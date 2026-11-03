//! Typed bindings to the entities and dynamic components of a loaded script.
//!
//! A schema declares, with [`entity_schema!`], the members of an entity type that a
//! server uses: client methods it calls, base and cell methods it receives, and
//! properties it sets, each with Rust types. Registering a schema with an app binds it
//! once against the loaded script: every member is looked up by name and its types
//! checked, so that a game version that renamed or retyped one fails at startup, naming
//! it, rather than mid-battle. After that, calls and property changes go by the indices
//! found then, with no name lookup.
//!
//! A dynamic component, declared with [`component_schema!`], is bound the same way, a
//! component type having the tables of an entity type. It is added to an entity in a
//! space with [`Entity::add_component`], given who gets it, and clients get it over NRL
//! (see the `Nrl*` elements of [`super::client::element`]): the app numbers its nodes,
//! and the clients knowing the entity get it at the end of the tick, with every property
//! given a value by then, then what changes of it, each tick.
//!
//! Values cross into the script as [`Value`]s, through [`ScriptType`], and are encoded
//! by `Value`'s own codec, the one decoding live traffic: a Rust type only has to
//! convert, never to encode.

use std::ops::{Deref, DerefMut};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::any::TypeId;
use std::sync::Arc;
use std::fmt;
use std::io;

use glam::{Vec2, Vec3, Vec4};

use crate::util::io::{serde_pickle_de_options, serde_pickle_ser_options, WgWriteExt};
use crate::script::{PropertyFlags, PythonValue, StringValue, Ty, TyKind, Value};
use crate::net::codec::Codec;

use super::client::element::{
    encode_nested_path, ChangedProperties, EntityMethod, IndexedEntityProperty, NrlBody, NrlChild, NrlComponentData,
    NrlData, NrlIdList, NrlMessage, NrlNode, NrlUnlinkTreeFlag, NrlUpdateNodeFlag, UpdateEntity,
};
use super::dispatch::{EntityDispatch, MethodCall, MethodDef, ScriptDispatch};
use super::slots::{SlotKey, Slots};
use super::outbox::ClientOutbox;
use super::io_invalid_data;

#[doc(hidden)]
pub use paste::paste as __paste;
/// A step of a path into a property's value, see `set_<property>_at`.
pub use super::client::element::EntityPropertyPathStep as PathStep;


// ============ //
//    VALUES    //
// ============ //

/// A Rust type that values of some script types convert to and from.
pub trait ScriptType: Sized {

    /// Whether values of the given type convert to and from this one. Checked once,
    /// when a schema is bound.
    fn check(ty: &Ty) -> bool;

    /// The value of the given type, which [`Self::check`] accepted.
    fn into_value(self, ty: &Ty) -> Value;

    /// This value back, if it is one this type holds.
    fn from_value(value: Value) -> Option<Self>;

}

/// A Rust type of method arguments: a tuple of [`ScriptType`]s, implemented up to 16.
pub trait ScriptArgs: Sized {

    /// Each argument's type.
    const TYPES: &'static [TypeDecl];

    /// The arguments as values of the given types, which [`Self::TYPES`] accepted.
    fn into_values(self, tys: &[Ty]) -> Vec<Value>;

    /// The arguments back, if they are ones this type holds.
    fn from_values(values: Vec<Value>) -> Option<Self>;

}

/// A Rust type, as checked against a script type when a schema is bound.
#[derive(Clone, Copy)]
pub struct TypeDecl {
    pub check: fn(&Ty) -> bool,
    pub name: fn() -> &'static str,
}

impl TypeDecl {
    pub const fn of<T: ScriptType>() -> Self {
        Self { check: T::check, name: std::any::type_name::<T> }
    }
}

/// A type's kind, through its aliases.
pub fn kind(ty: &Ty) -> &TyKind {
    match ty.kind() {
        TyKind::Alias(inner) => kind(inner),
        kind => kind,
    }
}

macro_rules! impl_scalar {
    ($($rust:ty => $variant:ident),* $(,)?) => {
        $(impl ScriptType for $rust {
            fn check(ty: &Ty) -> bool {
                matches!(kind(ty), TyKind::$variant)
            }
            fn into_value(self, _ty: &Ty) -> Value {
                Value::$variant(self)
            }
            fn from_value(value: Value) -> Option<Self> {
                match value {
                    Value::$variant(v) => Some(v),
                    _ => None,
                }
            }
        })*
    };
}

impl_scalar! {
    i8 => Int8, i16 => Int16, i32 => Int32, i64 => Int64,
    u8 => UInt8, u16 => UInt16, u32 => UInt32, u64 => UInt64,
    f32 => Float32, f64 => Float64,
    Vec2 => Vector2, Vec3 => Vector3, Vec4 => Vector4,
}

/// `BOOL`, an alias of `UINT8`.
impl ScriptType for bool {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::UInt8)
    }
    fn into_value(self, _ty: &Ty) -> Value {
        Value::UInt8(self as u8)
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::UInt8(v) => Some(v != 0),
            _ => None,
        }
    }
}

/// A `STRING` holding text.
impl ScriptType for String {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::String)
    }
    fn into_value(self, _ty: &Ty) -> Value {
        Value::String(StringValue::String(self))
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::String(StringValue::String(s)) => Some(s),
            Value::String(StringValue::Raw(raw)) => String::from_utf8(raw).ok(),
            _ => None,
        }
    }
}

/// The bytes of a `STRING` or a `PYTHON` value, whatever they hold: a compact
/// descriptor, say, or an already encoded blob.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl ScriptType for Bytes {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::String | TyKind::Python)
    }
    fn into_value(self, ty: &Ty) -> Value {
        match kind(ty) {
            TyKind::Python => Value::Python(PythonValue::Raw(self.0)),
            _ => Value::String(StringValue::Raw(self.0)),
        }
    }
    fn from_value(value: Value) -> Option<Self> {
        let pickle = |value| serde_pickle::value_to_vec(&value, serde_pickle_ser_options()).ok();
        Some(Self(match value {
            Value::String(StringValue::Raw(raw)) | Value::Python(PythonValue::Raw(raw)) => raw,
            Value::String(StringValue::String(s)) => s.into_bytes(),
            Value::String(StringValue::Python(value)) | Value::Python(PythonValue::Decoded(value)) => pickle(value)?,
            _ => return None,
        }))
    }
}

/// A pickled Python value, in a `PYTHON` or a `STRING` value.
#[derive(Debug, Clone, PartialEq)]
pub struct Pickle(pub serde_pickle::Value);

impl ScriptType for Pickle {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::String | TyKind::Python)
    }
    fn into_value(self, ty: &Ty) -> Value {
        match kind(ty) {
            TyKind::Python => Value::Python(PythonValue::Decoded(self.0)),
            _ => Value::String(StringValue::Python(self.0)),
        }
    }
    fn from_value(value: Value) -> Option<Self> {
        let unpickle = |raw: Vec<u8>| serde_pickle::value_from_slice(&raw, serde_pickle_de_options()).ok();
        Some(Self(match value {
            Value::String(StringValue::Python(value)) | Value::Python(PythonValue::Decoded(value)) => value,
            Value::String(StringValue::Raw(raw)) | Value::Python(PythonValue::Raw(raw)) => unpickle(raw)?,
            _ => return None,
        }))
    }
}

/// An `ARRAY` or a `TUPLE`.
impl<T: ScriptType> ScriptType for Vec<T> {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::Array(seq) | TyKind::Tuple(seq) if T::check(&seq.ty))
    }
    fn into_value(self, ty: &Ty) -> Value {
        let (TyKind::Array(seq) | TyKind::Tuple(seq)) = kind(ty) else { unreachable!("checked") };
        Value::Seq(self.into_iter().map(|v| v.into_value(&seq.ty)).collect())
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Seq(values) => values.into_iter().map(T::from_value).collect(),
            _ => None,
        }
    }
}

/// An `ARRAY` or a `TUPLE` of this size, or of any size.
impl<T: ScriptType, const N: usize> ScriptType for [T; N] {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::Array(seq) | TyKind::Tuple(seq)
            if T::check(&seq.ty) && seq.size.is_none_or(|size| size as usize == N))
    }
    fn into_value(self, ty: &Ty) -> Value {
        let (TyKind::Array(seq) | TyKind::Tuple(seq)) = kind(ty) else { unreachable!("checked") };
        Value::Seq(self.into_iter().map(|v| v.into_value(&seq.ty)).collect())
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Seq(values) => values.into_iter().map(T::from_value).collect::<Option<Vec<_>>>()?.try_into().ok(),
            _ => None,
        }
    }
}

/// A `FIXED_DICT` declaring `AllowNone`, `None` when Python's `None`.
impl<T: ScriptType> ScriptType for Option<T> {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::Dict(dict) if dict.allow_none) && T::check(ty)
    }
    fn into_value(self, ty: &Ty) -> Value {
        match self {
            Some(v) => v.into_value(ty),
            None => Value::None,
        }
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::None => Some(None),
            value => T::from_value(value).map(Some),
        }
    }
}

/// Any value, unchecked: for what has no Rust type yet.
impl ScriptType for Value {
    fn check(_ty: &Ty) -> bool {
        true
    }
    fn into_value(self, _ty: &Ty) -> Value {
        self
    }
    fn from_value(value: Value) -> Option<Self> {
        Some(value)
    }
}

/// The type of the given field of a `FIXED_DICT`, for [`script_dict!`].
#[doc(hidden)]
pub fn dict_field_ty<'t>(ty: &'t Ty, name: &str) -> Option<&'t Ty> {
    match kind(ty) {
        TyKind::Dict(dict) => dict.properties.iter().find(|prop| &*prop.name == name).map(|prop| &prop.ty),
        _ => None,
    }
}

/// Whether the given type is a `FIXED_DICT` of exactly these fields, each passing its
/// check, for [`script_dict!`].
#[doc(hidden)]
pub fn dict_check(ty: &Ty, fields: &[(&str, fn(&Ty) -> bool)]) -> bool {
    match kind(ty) {
        TyKind::Dict(dict) => dict.properties.len() == fields.len()
            && fields.iter().all(|&(name, check)| dict_field_ty(ty, name).is_some_and(check)),
        _ => false,
    }
}

macro_rules! impl_args {
    ($($arg:ident $name:ident),*) => {
        impl<$($arg: ScriptType),*> ScriptArgs for ($($arg,)*) {

            const TYPES: &'static [TypeDecl] = &[$(TypeDecl::of::<$arg>()),*];

            #[allow(unused)]
            fn into_values(self, tys: &[Ty]) -> Vec<Value> {
                let ($($name,)*) = self;
                let mut tys = tys.iter();
                vec![$($name.into_value(tys.next().expect("checked"))),*]
            }

            #[allow(unused)]
            fn from_values(values: Vec<Value>) -> Option<Self> {
                let mut values = values.into_iter();
                let args = ($($arg::from_value(values.next()?)?,)*);
                values.next().is_none().then_some(args)
            }

        }
    };
}

impl_args!();
impl_args!(A0 a0);
impl_args!(A0 a0, A1 a1);
impl_args!(A0 a0, A1 a1, A2 a2);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10, A11 a11);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10, A11 a11, A12 a12);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10, A11 a11, A12 a12, A13 a13);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10, A11 a11, A12 a12, A13 a13, A14 a14);
impl_args!(A0 a0, A1 a1, A2 a2, A3 a3, A4 a4, A5 a5, A6 a6, A7 a7, A8 a8, A9 a9, A10 a10, A11 a11, A12 a12, A13 a13, A14 a14, A15 a15);


// ============= //
//    SCHEMAS    //
// ============= //

/// A method a schema declares, with its arguments' types.
pub struct MethodDecl {
    pub name: &'static str,
    pub args: &'static [TypeDecl],
}

/// A property a schema declares, with the side hosting it and its type.
pub struct PropertyDecl {
    pub name: &'static str,
    pub side: Side,
    pub ty: TypeDecl,
}

/// A side of an entity: its base or its cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Base,
    Cell,
}

/// The members of an entity or dynamic component type that a server uses, a subset of
/// the script's, implemented by [`entity_schema!`] and [`component_schema!`].
pub trait Schema: 'static {
    /// The type's name in the script.
    const NAME: &'static str;
    /// The client methods it calls.
    const CLIENT_METHODS: &'static [MethodDecl];
    /// The base and cell methods it receives.
    const BASE_METHODS: &'static [MethodDecl];
    const CELL_METHODS: &'static [MethodDecl];
    /// The client-visible properties it sets.
    const PROPERTIES: &'static [PropertyDecl];
}

/// The schema of an entity type, whose [`Entity`] views deref to [`Self::View`].
pub trait EntitySchema: Schema {
    type View<'a>: EntityView<'a>;
}

/// The view an [`Entity`] derefs to, generated by [`entity_schema!`] with a method per
/// declared member.
pub trait EntityView<'a> {
    fn new(raw: RawEntity<'a>) -> Self;
    fn raw(&self) -> &RawEntity<'a>;
    fn raw_mut(&mut self) -> &mut RawEntity<'a>;
}

/// The schema of a dynamic component type, whose [`Component`] views deref to
/// [`Self::View`].
pub trait ComponentSchema: Schema {
    type View<'a>: ComponentView<'a>;
}

/// The view a [`Component`] derefs to, generated by [`component_schema!`] with a method
/// per declared member.
pub trait ComponentView<'a> {
    fn new(raw: RawComponent<'a>) -> Self;
    fn raw(&self) -> &RawComponent<'a>;
    fn raw_mut(&mut self) -> &mut RawComponent<'a>;
}

/// The methods of one side of a schema, as an enum generated by [`entity_schema!`] with
/// one variant per method, holding its decoded arguments.
pub trait Methods: Sized {
    type Schema: Schema;
    const SIDE: Side;
    /// Decode a call of the declared method at the given index.
    fn decode(index: usize, args: Vec<Value>) -> Option<Self>;
}

/// A schema bound to an entity or component type of the loaded script, see [`bind`].
#[derive(Debug)]
pub struct Bound {
    schema: TypeId,
    /// The exposed id of each declared client method and property.
    client_methods: Vec<u16>,
    properties: Vec<u16>,
    /// The declaration index of each base and cell method, by exposed id.
    base_methods: Vec<Option<u16>>,
    cell_methods: Vec<Option<u16>>,
}

impl Bound {

    /// Whether this is the given schema's binding.
    pub fn is<S: Schema>(&self) -> bool {
        self.schema == TypeId::of::<S>()
    }

    /// Decode a call of one of the entity's base or cell methods, if it is one the
    /// given calls declare.
    pub fn decode<M: Methods>(&self, call: &MethodCall) -> Option<M> {
        if !self.is::<M::Schema>() {
            return None;
        }
        let MethodCall::Known { exposed_id, args, .. } = call else { return None };
        let methods = match M::SIDE {
            Side::Base => &self.base_methods,
            Side::Cell => &self.cell_methods,
        };
        let index = (*methods.get(*exposed_id as usize)?)?;
        M::decode(index.into(), args.clone())
    }

}

/// Why a schema did not bind, listing every member that did not.
#[derive(Debug)]
pub struct BindError {
    pub schema: &'static str,
    pub errors: Vec<String>,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "schema {} doesn't match the script: {}", self.schema, self.errors.join("; "))
    }
}

impl std::error::Error for BindError {}

/// Bind a schema to its type's dispatch tables, checking each declared member.
pub fn bind<S: Schema>(dispatch: &EntityDispatch) -> Result<Bound, BindError> {

    let mut errors = Vec::new();

    let mut bind_methods = |kind: &str, decls: &[MethodDecl], table: &[MethodDef]| -> Vec<u16> {
        decls.iter().filter_map(|decl| {
            let Some(exposed_id) = table.iter().position(|def| &*def.name == decl.name) else {
                errors.push(format!("no {kind} method {}", decl.name));
                return None;
            };
            let def = &table[exposed_id];
            if def.args.len() != decl.args.len() {
                errors.push(format!("{kind} method {} takes {} arguments, not {}", decl.name, def.args.len(), decl.args.len()));
            }
            for (i, (ty, arg)) in def.args.iter().zip(decl.args).enumerate() {
                if !(arg.check)(ty) {
                    errors.push(format!("{kind} method {} argument {i} is a {}, not a {}", decl.name, describe(ty), (arg.name)()));
                }
            }
            Some(exposed_id as u16)
        }).collect()
    };

    let client_methods = bind_methods("client", S::CLIENT_METHODS, &dispatch.client_methods);
    let by_exposed_id = |exposed_ids: Vec<u16>, len: usize| {
        let mut indices = vec![None; len];
        for (index, exposed_id) in exposed_ids.into_iter().enumerate() {
            indices[exposed_id as usize] = Some(index as u16);
        }
        indices
    };
    let base_methods = by_exposed_id(bind_methods("base", S::BASE_METHODS, &dispatch.base_methods), dispatch.base_methods.len());
    let cell_methods = by_exposed_id(bind_methods("cell", S::CELL_METHODS, &dispatch.cell_methods), dispatch.cell_methods.len());

    let properties = S::PROPERTIES.iter().filter_map(|decl| {
        let Some(exposed_id) = dispatch.properties.iter().position(|def| &*def.name == decl.name) else {
            errors.push(format!("no client-visible property {}", decl.name));
            return None;
        };
        let def = &dispatch.properties[exposed_id];
        let side = match def.flags {
            PropertyFlags::BaseAndClient => Side::Base,
            _ => Side::Cell,
        };
        if side != decl.side {
            errors.push(format!("property {} is a {side:?} property ({:?})", decl.name, def.flags));
        }
        if !(decl.ty.check)(&def.ty) {
            errors.push(format!("property {} is a {}, not a {}", decl.name, describe(&def.ty), (decl.ty.name)()));
        }
        Some(exposed_id as u16)
    }).collect();

    if errors.is_empty() {
        Ok(Bound { schema: TypeId::of::<S>(), client_methods, properties, base_methods, cell_methods })
    } else {
        Err(BindError { schema: S::NAME, errors })
    }

}

/// The schemas of entity types, or of component types, registered with an app, by type
/// id.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    bindings: HashMap<u16, Bound>,
    schema_types: HashMap<TypeId, u16>,
}

impl Registry {

    /// Bind a schema to its type, see [`bind`].
    pub fn register<S: Schema>(&mut self, script: &ScriptDispatch) -> Result<(), BindError> {

        let Some((type_id, dispatch)) = script.entity_from_name(S::NAME) else {
            return Err(BindError { schema: S::NAME, errors: vec![format!("no type {}", S::NAME)] });
        };

        self.bindings.insert(type_id, bind::<S>(dispatch)?);
        self.schema_types.insert(TypeId::of::<S>(), type_id);
        Ok(())

    }

    /// The type of a registered schema.
    pub fn type_of<S: Schema>(&self) -> io::Result<u16> {
        self.schema_types.get(&TypeId::of::<S>()).copied()
            .ok_or_else(|| io_invalid_data(format_args!("schema {} is not registered", S::NAME)))
    }

    /// The binding of a type, if a schema is registered for it.
    pub fn bound(&self, type_id: u16) -> Option<&Bound> {
        self.bindings.get(&type_id)
    }

}

/// A type, briefly: its name, or its kind when it has none.
fn describe(ty: &Ty) -> String {
    match (ty.name(), ty.kind()) {
        ("", TyKind::Alias(inner)) => describe(inner),
        ("", TyKind::Dict(_)) => "FIXED_DICT".to_string(),
        ("", TyKind::Array(seq)) => format!("ARRAY of {}", describe(&seq.ty)),
        ("", TyKind::Tuple(seq)) => format!("TUPLE of {}", describe(&seq.ty)),
        ("", kind) => format!("{kind:?}"),
        (name, _) => name.to_string(),
    }
}


// ========== //
//    KEYS    //
// ========== //

/// An entity of an app, of the given schema: what the app gave when it created it, or
/// decoded a call to it with. Stale once the entity is gone, even if another one takes
/// its place.
pub struct EntityKey<T> {
    pub(crate) slot: SlotKey,
    pub(crate) id: u32,
    _schema: PhantomData<fn() -> T>,
}

/// The schema of an [`EntityKey`] to an entity of any type.
pub enum AnyEntity {}

impl<T> EntityKey<T> {

    pub(crate) fn new(slot: SlotKey, id: u32) -> Self {
        Self { slot, id, _schema: PhantomData }
    }

    /// The entity's id, as the protocol carries it.
    pub fn id(&self) -> EntityId {
        EntityId(self.id)
    }

    /// The same key, to an entity of any type.
    pub fn any(self) -> EntityKey<AnyEntity> {
        EntityKey::new(self.slot, self.id)
    }

    /// The same key, to an entity of the given type, which the caller checked it is.
    pub(crate) fn cast<U>(self) -> EntityKey<U> {
        EntityKey::new(self.slot, self.id)
    }

}

impl<T> Clone for EntityKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for EntityKey<T> {}

impl<T> PartialEq for EntityKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot
    }
}

impl<T> Eq for EntityKey<T> {}

impl<T> std::hash::Hash for EntityKey<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
    }
}

impl<T> fmt::Debug for EntityKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntityKey({})", self.id)
    }
}

impl<T> fmt::Display for EntityKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.id, f)
    }
}

/// An entity's id as the protocol carries it, an `OBJECT_ID` say: in method arguments
/// and properties, where a key cannot go since one cannot be read back from a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub u32);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// An `INT32` or a `UINT32`.
impl ScriptType for EntityId {
    fn check(ty: &Ty) -> bool {
        matches!(kind(ty), TyKind::Int32 | TyKind::UInt32)
    }
    fn into_value(self, ty: &Ty) -> Value {
        match kind(ty) {
            TyKind::Int32 => Value::Int32(self.0 as i32),
            _ => Value::UInt32(self.0),
        }
    }
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Int32(id) => Some(Self(id as u32)),
            Value::UInt32(id) => Some(Self(id)),
            _ => None,
        }
    }
}


/// A component of an entity, of the given schema: what [`Entity::add_component`] gave.
/// Stale once the component or its entity is gone.
pub struct ComponentKey<C> {
    pub(crate) entity: EntityKey<AnyEntity>,
    pub(crate) slot: SlotKey,
    _schema: PhantomData<fn() -> C>,
}

impl<C> ComponentKey<C> {

    /// Its entity.
    pub fn entity(&self) -> EntityKey<AnyEntity> {
        self.entity
    }

}

impl<C> Clone for ComponentKey<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for ComponentKey<C> {}

impl<C> PartialEq for ComponentKey<C> {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity && self.slot == other.slot
    }
}

impl<C> Eq for ComponentKey<C> {}

impl<C> fmt::Debug for ComponentKey<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ComponentKey({}, {:?})", self.entity, self.slot)
    }
}



// =========== //
//    VIEWS    //
// =========== //

/// An entity of a client, as a schema sees it: derefs to its schema's view, whose
/// methods call its client methods and set and get its properties.
pub struct Entity<'a, T: EntitySchema> {
    view: T::View<'a>,
}

impl<'a, T: EntitySchema> Entity<'a, T> {

    pub(crate) fn new(raw: RawEntity<'a>) -> Self {
        Self { view: T::View::new(raw) }
    }

    /// The entity's key.
    pub fn key(&self) -> EntityKey<T> {
        let raw = self.view.raw();
        EntityKey::new(raw.slot, raw.id)
    }

    /// Call a client method the schema doesn't declare, by name, with values left
    /// unchecked: for calls replayed from a capture.
    pub fn call_dynamic(&mut self, name: &str, args: Vec<Value>) -> io::Result<()> {
        self.view.raw_mut().call_dynamic(name, args)
    }

    /// Set properties the schema doesn't declare, given in the entity type's property
    /// table, with values left unchecked, in one element: for changes replayed from a
    /// capture.
    pub fn set_dynamic(&mut self, properties: ChangedProperties) -> io::Result<()> {
        self.view.raw_mut().set_dynamic(properties)
    }

    /// Add a dynamic component of the given schema to the entity, which must be in a
    /// space: the clients of the given audience knowing the entity get it at the end of
    /// the tick, with the properties set by then, see [`super::component`].
    pub fn add_component<C: ComponentSchema>(&mut self, audience: Audience) -> io::Result<Component<'_, C>> {
        let raw = self.view.raw_mut();
        let entity = EntityKey::new(raw.slot, raw.id);
        raw.components.as_mut()
            .ok_or_else(|| io_invalid_data(format_args!("entity {entity} is in no space")))?
            .add(entity, audience)
    }

}

impl<'a, T: EntitySchema> Deref for Entity<'a, T> {
    type Target = T::View<'a>;
    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl<'a, T: EntitySchema> DerefMut for Entity<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.view
    }
}

/// What a view goes through: the entity, its property values, its tables, its binding
/// and where what its clients get goes.
pub struct RawEntity<'a> {
    pub(crate) slot: SlotKey,
    pub(crate) id: u32,
    /// Its client-visible property values, by exposed id, when known.
    pub(crate) properties: &'a mut [Option<Value>],
    pub(crate) dispatch: &'a EntityDispatch,
    pub(crate) bound: &'a Bound,
    pub(crate) sink: EntitySink<'a>,
    /// Its dynamic components, when in a space.
    pub(crate) components: Option<EntityComponents<'a>>,
}

/// Where what an entity's clients get goes.
pub(crate) enum EntitySink<'a> {
    /// The outbox of the one client of a base entity, selecting it as its player or by
    /// its id.
    Client { out: &'a mut ClientOutbox, is_player: bool },
    /// The events of the space of an entity in one, which go to the clients knowing it.
    Space(&'a mut Vec<EntityEvent>),
}

/// What an entity in a space gave its clients, for each of them to get at the end of
/// the tick.
#[derive(Debug)]
pub(crate) struct EntityEvent {
    pub entity_id: u32,
    /// Whether it goes to its own client only, rather than to every client knowing it.
    pub own_client: bool,
    pub kind: EntityEventKind,
}

#[derive(Debug)]
pub(crate) enum EntityEventKind {
    Call { exposed_id: u16, args: Vec<Value> },
    Property { exposed_id: u16, value: Value },
    /// Several properties in one element.
    Update(ChangedProperties),
}

impl EntityEventKind {

    /// Write it to an outbox, its entity already selected, against its entity type's
    /// tables.
    pub fn write(&self, out: &mut ClientOutbox, dispatch: &EntityDispatch) -> io::Result<()> {
        match self {
            &Self::Call { exposed_id, ref args } => {
                let name = Arc::clone(&dispatch.client_methods[exposed_id as usize].name);
                let call = EntityMethod { call: MethodCall::Known { exposed_id, name, args: args.clone() } };
                out.write_selected(call, &dispatch.client_methods)
            }
            &Self::Property { exposed_id, ref value } => {
                out.write_selected(IndexedEntityProperty { exposed_id, value: value.clone() }, &dispatch.properties[..])
            }
            Self::Update(properties) => out.write_selected(UpdateEntity(properties.clone()), &dispatch.properties[..]),
        }
    }

}

impl RawEntity<'_> {

    /// Give what an entity gave to its clients.
    fn emit(&mut self, kind: EntityEventKind, own_client: bool) -> io::Result<()> {
        match &mut self.sink {
            EntitySink::Client { out, is_player } => {
                if *is_player {
                    out.select_player()?;
                } else {
                    out.select_entity(self.id)?;
                }
                kind.write(out, self.dispatch)
            }
            EntitySink::Space(events) => {
                events.push(EntityEvent { entity_id: self.id, own_client, kind });
                Ok(())
            }
        }
    }

    /// Set a property by exposed id, and give it to the clients it is for.
    fn write_property(&mut self, exposed_id: u16, value: Value) -> io::Result<()> {
        self.properties[exposed_id as usize] = Some(value.clone());
        let own_client = self.dispatch.properties[exposed_id as usize].is_own_client();
        self.emit(EntityEventKind::Property { exposed_id, value }, own_client)
    }

    /// Call the declared client method at the given index.
    pub fn call_client<A: ScriptArgs>(&mut self, index: usize, args: A) -> io::Result<()> {
        let exposed_id = self.bound.client_methods[index];
        let def = &self.dispatch.client_methods[exposed_id as usize];
        let args = args.into_values(&def.args);
        self.write_call(exposed_id, args)
    }

    /// Set the declared property at the given index.
    pub fn set_property<V: ScriptType>(&mut self, index: usize, value: V) -> io::Result<()> {
        let exposed_id = self.bound.properties[index];
        let value = value.into_value(&self.dispatch.properties[exposed_id as usize].ty);
        self.write_property(exposed_id, value)
    }

    /// The declared property at the given index, if its value is known: given at
    /// creation, or set since.
    pub fn get_property<V: ScriptType>(&self, index: usize) -> Option<V> {
        let exposed_id = self.bound.properties[index];
        V::from_value(self.properties[exposed_id as usize].clone()?)
    }

    fn call_dynamic(&mut self, name: &str, args: Vec<Value>) -> io::Result<()> {
        let exposed_id = self.dispatch.client_methods.iter().position(|def| &*def.name == name)
            .ok_or_else(|| io_invalid_data(format_args!("unknown client method: {name}")))?;
        self.write_call(exposed_id as u16, args)
    }

    fn set_dynamic(&mut self, properties: ChangedProperties) -> io::Result<()> {
        let mut own_client = false;
        for (exposed_id, _, value) in &properties.values {
            let def = self.dispatch.properties.get(usize::from(*exposed_id))
                .ok_or_else(|| io_invalid_data(format_args!("unknown property: {exposed_id}")))?;
            own_client |= def.is_own_client();
            self.properties[usize::from(*exposed_id)] = Some(value.clone());
        }
        self.emit(EntityEventKind::Update(properties), own_client)
    }

    fn write_call(&mut self, exposed_id: u16, args: Vec<Value>) -> io::Result<()> {
        self.emit(EntityEventKind::Call { exposed_id, args }, false)
    }

}


/// A component of an entity, as a schema sees it: derefs to its schema's view, whose
/// methods call its client methods and set and get its properties. What they give goes
/// to the clients that get it at the end of the tick.
pub struct Component<'a, C: ComponentSchema> {
    view: C::View<'a>,
}

impl<'a, C: ComponentSchema> Component<'a, C> {

    pub(crate) fn new(raw: RawComponent<'a>) -> Self {
        Self { view: C::View::new(raw) }
    }

    /// The component's key.
    pub fn key(&self) -> ComponentKey<C> {
        let raw = self.view.raw();
        ComponentKey { entity: raw.entity, slot: raw.slot, _schema: PhantomData }
    }

}

impl<'a, C: ComponentSchema> Deref for Component<'a, C> {
    type Target = C::View<'a>;
    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

impl<'a, C: ComponentSchema> DerefMut for Component<'a, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.view
    }
}

/// What a component view goes through: the component, its tables and its binding.
pub struct RawComponent<'a> {
    pub(crate) entity: EntityKey<AnyEntity>,
    pub(crate) slot: SlotKey,
    pub(crate) state: &'a mut ComponentState,
    pub(crate) dispatch: &'a EntityDispatch,
    pub(crate) bound: &'a Bound,
}

impl RawComponent<'_> {

    /// Call the declared client method at the given index.
    pub fn call_client<A: ScriptArgs>(&mut self, index: usize, args: A) -> io::Result<()> {
        let method = self.bound.client_methods[index];
        let args = args.into_values(&self.dispatch.client_methods[usize::from(method)].args);
        self.state.events.push(ComponentEvent::Call { method, args });
        Ok(())
    }

    /// Set the declared property at the given index. Set again in the same tick, its
    /// clients only get its last value.
    pub fn set_property<V: ScriptType>(&mut self, index: usize, value: V) -> io::Result<()> {

        let property = self.bound.properties[index];
        let value = value.into_value(&self.dispatch.properties[usize::from(property)].ty);
        self.state.values[usize::from(property)] = Some(value);

        let set = ComponentEvent::Set { property };
        if !self.state.events.contains(&set) {
            self.state.events.push(set);
        }

        Ok(())

    }

    /// Set a value inside the declared property at the given index, at the given path
    /// from it: its clients only get that value. The property must have a value, with
    /// the path in it.
    pub fn set_property_at<V: ScriptType>(&mut self, index: usize, path: Vec<PathStep>, value: V) -> io::Result<()> {

        let property = self.bound.properties[index];
        let invalid = || io_invalid_data(format_args!("no value at {path:?}"));
        let ty = path.iter().try_fold(&self.dispatch.properties[usize::from(property)].ty, |ty, step| match (kind(ty), step) {
            (TyKind::Dict(dict), PathStep::Field(name)) => dict.properties.iter().find(|prop| prop.name == *name).map(|prop| &prop.ty),
            (TyKind::Array(seq) | TyKind::Tuple(seq), PathStep::Index(_)) => Some(&seq.ty),
            _ => None,
        }).ok_or_else(invalid)?;
        let value = value.into_value(ty);

        let root = self.state.values[usize::from(property)].as_mut().ok_or_else(invalid)?;
        let slot = path.iter().try_fold(root, |value, step| match (value, step) {
            (Value::Dict(dict), PathStep::Field(name)) => dict.get_mut(name),
            (Value::Seq(items), PathStep::Index(index)) => items.get_mut(*index as usize),
            _ => None,
        }).ok_or_else(invalid)?;
        *slot = value.clone();

        self.state.events.push(ComponentEvent::SetAt { property, path, value });
        Ok(())

    }

    /// The declared property at the given index, if given a value.
    pub fn get_property<V: ScriptType>(&self, index: usize) -> Option<V> {
        let property = self.bound.properties[index];
        V::from_value(self.state.values[usize::from(property)].clone()?)
    }

}

/// Insert at `start` the packed length of what follows it in an update record.
fn insert_length(fragment: &mut Vec<u8>, start: usize) -> io::Result<()> {
    let mut prefix = [0; 4];
    let mut write = &mut prefix[..];
    write.write_packed_u24((fragment.len() - start) as u32)?;
    let len = 4 - write.len();
    fragment.splice(start..start, prefix[..len].iter().copied());
    Ok(())
}

/// The components of an entity in a space, for [`Entity::add_component`]: its
/// components and its NRL node, the space's NRL node ids, and the app's tables.
pub(crate) struct EntityComponents<'a> {
    pub components: &'a mut Slots<ComponentState>,
    pub nrl_node: &'a mut Option<u16>,
    pub nrl_nodes: &'a mut NrlNodeIds,
    pub registry: &'a Registry,
    pub script: &'a ScriptDispatch,
}

impl EntityComponents<'_> {

    /// Add a component of the given schema, see [`Entity::add_component`].
    pub fn add<C: ComponentSchema>(&mut self, entity: EntityKey<AnyEntity>, audience: Audience) -> io::Result<Component<'_, C>> {

        let type_id = self.registry.type_of::<C>()?;
        let bound = self.registry.bound(type_id).expect("registered");
        let dispatch = self.script.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown component type: {type_id}")))?;
        let nrl_type_id = self.script.component_nrl_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("component {} has no NRL type", C::NAME)))?;

        if self.nrl_node.is_none() {
            *self.nrl_node = Some(self.nrl_nodes.take(1)?);
        }
        let properties = dispatch.properties.len();
        let node = self.nrl_nodes.take(1 + properties + dispatch.client_methods.len() + 1)?;

        let slot = self.components.insert(ComponentState {
            type_id,
            nrl_type_id,
            audience,
            node,
            values: vec![None; properties],
            events: Vec::new(),
        });

        let state = self.components.get_mut(slot).expect("inserted");
        Ok(Component::new(RawComponent { entity, slot, state, dispatch, bound }))

    }

}



// ================ //
//    COMPONENTS    //
// ================ //

/// Who gets a component, of the clients knowing its entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Every one of them.
    AllClients,
    /// Its entity's own client only: the client of the entity itself, or of the one
    /// controlling it (see `EntityInit::controlled_by`).
    OwnClient,
}

/// A dynamic component of an entity in a space: its type, its NRL nodes and its property
/// values.
#[derive(Debug)]
pub(crate) struct ComponentState {
    /// Its type id, numbered as entity types are, and the one NRL gives it.
    type_id: u16,
    nrl_type_id: u16,
    audience: Audience,
    /// Its node, its children's following it: one per property, one per client method,
    /// then one more.
    node: u16,
    /// Its property values, by index in its table, when given.
    values: Vec<Option<Value>>,
    /// What changed of it during the tick, in order.
    events: Vec<ComponentEvent>,
}

/// Something that changed of a component during the tick, see [`ComponentState`].
#[derive(Debug, PartialEq)]
enum ComponentEvent {
    /// A property was set, its value being the current one.
    Set { property: u16 },
    /// A value inside a property was set, at the given path from it.
    SetAt { property: u16, path: Vec<PathStep>, value: Value },
    Call { method: u16, args: Vec<Value> },
}

impl ComponentState {

    /// Its type id, numbered as entity types are.
    pub fn type_id(&self) -> u16 {
        self.type_id
    }

    /// Whether the given client gets it, if it is its entity's own client or not.
    pub fn is_for(&self, own: bool) -> bool {
        own || self.audience == Audience::AllClients
    }

    /// Forget what changed of it during the tick, every client having got it.
    pub fn end_tick(&mut self) {
        self.events.clear();
    }

    /// What its clients need to remove it once removed from the given slot.
    pub fn removed(&self, slot: SlotKey) -> RemovedComponent {
        RemovedComponent { slot, node: self.node }
    }

    /// The NRL message creating it under its entity's node, with every property given a
    /// value. Its method children all carry their method, in table order.
    pub fn create(&self, entity_node: u16, script: &ScriptDispatch) -> io::Result<NrlMessage> {

        let dispatch = self.dispatch(script)?;
        let methods = dispatch.client_methods.len();
        let count = self.values.len() + methods + 1;
        let ids: Vec<u32> = (1..=count as u32).map(|i| u32::from(self.node) + i).collect();

        // The children present, in order, each with a bit in the presence mask.
        let mut present = vec![0u8; count.div_ceil(8)];
        let mut children = Vec::new();
        let mut add = |index: usize, child| {
            present[index / 8] |= 1 << (index % 8);
            children.push((ids[index], child));
        };

        for (index, value) in self.values.iter().enumerate() {
            if let Some(value) = value {
                add(index, NrlChild::Property { name: Arc::clone(&dispatch.properties[index].name), value: value.clone() });
            }
        }
        for (method_index, def) in dispatch.client_methods.iter().enumerate() {
            add(self.values.len() + method_index, NrlChild::Method { name: Arc::clone(&def.name), method_index: method_index as u32 });
        }
        add(count - 1, NrlChild::Trailing);

        Ok(NrlMessage {
            node_id: self.node,
            body: NrlBody::CreateNode {
                parent_id: entity_node,
                node: NrlNode::DynComponent {
                    component_type_id: self.nrl_type_id,
                    name: String::new(),
                    data: Some(NrlComponentData::encode(script, self.nrl_type_id, children)?),
                    children: NrlIdList::new(self.node.into(), ids, Some(present)),
                },
            },
        })

    }

    /// Write what changed of it during the tick to a client. Just created on it, it has
    /// its properties already, and only gets its calls.
    pub fn write_events(&self, out: &mut ClientOutbox, script: &ScriptDispatch, created: bool) -> io::Result<()> {

        let dispatch = self.dispatch(script)?;

        for event in &self.events {

            // An update record, `[node id: u16 big-endian][update]`, goes to the client's
            // NRL buffer, then a flag applies it.
            let fragment = match *event {
                ComponentEvent::Set { .. } if created => continue,
                ComponentEvent::Set { property } => {

                    let def = &dispatch.properties[usize::from(property)];
                    let value = self.values[usize::from(property)].as_ref().expect("set");
                    let mut fragment = self.record(property);

                    if def.volatile {
                        value.write(&mut fragment, &def.ty)?;
                    } else {
                        // `[kind 0: the whole value][packed length][value]`.
                        fragment.push(0);
                        let start = fragment.len();
                        value.write(&mut fragment, &def.ty)?;
                        insert_length(&mut fragment, start)?;
                    }

                    fragment

                }
                ComponentEvent::SetAt { .. } if created => continue,
                ComponentEvent::SetAt { property, ref path, ref value } => {

                    // `[kind 2: a value inside][packed length][path][value]`, the path
                    // starting at the component's property table. Its `Array` lengths
                    // come from the property's value.
                    let name = &dispatch.properties[usize::from(property)].name;
                    let current = self.values[usize::from(property)].clone().expect("set");
                    let values = HashMap::from([(Arc::clone(name), current)]);
                    let full_path: Vec<PathStep> = std::iter::once(PathStep::Field(Arc::clone(name)))
                        .chain(path.iter().cloned())
                        .collect();

                    let mut fragment = self.record(property);
                    fragment.push(2);
                    let start = fragment.len();
                    encode_nested_path(&dispatch.properties, &values, &full_path, value, &mut fragment)?;
                    insert_length(&mut fragment, start)?;
                    fragment

                }
                ComponentEvent::Call { method, ref args } => {
                    let mut fragment = self.record(self.values.len() as u16 + method);
                    dispatch.client_methods[usize::from(method)].write_args(&mut fragment, args)?;
                    fragment
                }
            };

            out.write_simple(NrlData { fragment })?;
            out.write_simple(NrlUpdateNodeFlag)?;

        }

        Ok(())

    }

    /// An update record's start: the node id of its child at the given index, see
    /// [`Self::create`].
    fn record(&self, child: u16) -> Vec<u8> {
        (self.node + 1 + child).to_be_bytes().to_vec()
    }

    fn dispatch<'s>(&self, script: &'s ScriptDispatch) -> io::Result<&'s EntityDispatch> {
        script.entity_from_id(self.type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown component type: {}", self.type_id)))
    }

}

/// A component removed during the tick, which its clients still have.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RemovedComponent {
    pub slot: SlotKey,
    node: u16,
}

impl RemovedComponent {

    /// Write its removal to a client: its node's id goes to the client's NRL buffer, then
    /// a flag unlinks that node and its children, as captured for a fire put out.
    pub fn write(&self, out: &mut ClientOutbox) -> io::Result<()> {
        out.write_simple(NrlData { fragment: self.node.to_be_bytes().to_vec() })?;
        out.write_simple(NrlUnlinkTreeFlag)
    }

}

/// The NRL node ids of a space's entities and components, never reused: an id the
/// client may still have is not given again, see `App`.
#[derive(Debug)]
pub(crate) struct NrlNodeIds {
    next: u16,
}

impl Default for NrlNodeIds {
    fn default() -> Self {
        Self { next: FIRST_NODE }
    }
}

impl NrlNodeIds {

    /// The first of `count` consecutive new ids.
    fn take(&mut self, count: usize) -> io::Result<u16> {
        let first = self.next;
        self.next = u16::try_from(count).ok()
            .and_then(|count| first.checked_add(count))
            .ok_or_else(|| io_invalid_data(format_args!("no NRL node id left in the space")))?;
        Ok(first)
    }

}

/// The root node every entity node goes under, which the client has from the start.
const ROOT_NODE: u16 = 1;

/// The first node id given, after the root's.
const FIRST_NODE: u16 = 2;

/// The NRL message creating an entity's node.
pub(crate) fn create_entity_node(entity_id: u32, node: u16) -> NrlMessage {
    NrlMessage {
        node_id: node,
        body: NrlBody::CreateNode { parent_id: ROOT_NODE, node: NrlNode::Entity { entity_id } },
    }
}



// ============ //
//    MACROS    //
// ============ //

/// Declare a struct converting to and from a `FIXED_DICT`, field for field: binding
/// checks the dict has exactly these fields, of these types.
///
/// ```ignore
/// script_dict! {
///     #[derive(Debug, Clone)]
///     pub struct GunReloadTime {
///         pub baseTime: f32,
///         pub clipTime: f32,
///         pub endTime: f32,
///         pub timeLeft: f32,
///     }
/// }
/// ```
#[macro_export]
macro_rules! script_dict {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $( $(#[$field_meta:meta])* $field_vis:vis $field:ident : $ty:ty ),* $(,)?
        }
    ) => {

        $(#[$meta])*
        #[allow(non_snake_case)]
        $vis struct $name {
            $( $(#[$field_meta])* $field_vis $field: $ty ),*
        }

        impl $crate::app::entity::ScriptType for $name {

            fn check(ty: &$crate::script::Ty) -> bool {
                $crate::app::entity::dict_check(ty, &[
                    $( (stringify!($field), <$ty as $crate::app::entity::ScriptType>::check) ),*
                ])
            }

            fn into_value(self, ty: &$crate::script::Ty) -> $crate::script::Value {
                let mut dict = ::std::collections::BTreeMap::new();
                $(
                    let field_ty = $crate::app::entity::dict_field_ty(ty, stringify!($field)).expect("checked");
                    dict.insert(::std::sync::Arc::from(stringify!($field)), $crate::app::entity::ScriptType::into_value(self.$field, field_ty));
                )*
                $crate::script::Value::Dict(dict)
            }

            fn from_value(value: $crate::script::Value) -> ::std::option::Option<Self> {
                let $crate::script::Value::Dict(mut dict) = value else { return None };
                Some(Self {
                    $( $field: $crate::app::entity::ScriptType::from_value(dict.remove(stringify!($field))?)? ),*
                })
            }

        }

    };
}

/// Declare the members of an entity type that a server uses, each with Rust types:
/// generates the schema, the view its [`Entity`] derefs to, with a method per client
/// method and a getter and a setter per property, and an enum per side of the methods it
/// receives.
///
/// ```ignore
/// entity_schema! {
///     pub Vehicle {
///         client {
///             fn showShooting(burst_count: u8, gun_index: i8, shot_kind: u8);
///         }
///         cell {
///             fn setRemoteCamera(data: Value);
///             health: i16,
///             gunAnglesPacked: u16,
///         }
///     }
/// }
///
/// let mut vehicle = app.entity::<Vehicle>(entity_id)?;
/// vehicle.showShooting(1, 0, 0)?;
/// vehicle.set_health(900)?;
/// ```
///
/// Sections are optional but come in this order: `client`, `base`, `cell`. The client
/// section declares the client methods called. The base and cell sections declare, in
/// any order, the methods of that side received (`fn` lines, ending with `;`) and the
/// properties of that side set (`name: Type` lines, separated by `,`). The methods of a
/// side decode to an enum named after the schema and the side, each to a variant named
/// after its method in camel case (`vehicle_moveWith` to `VehicleMoveWith`), with a
/// field per argument: `VehicleCell::SetRemoteCamera { data }`.
#[macro_export]
macro_rules! entity_schema {

    (
        $(#[$meta:meta])*
        $vis:vis $name:ident {
            $( client { $( $client:tt )* } )?
            $( base { $( $base:tt )* } )?
            $( cell { $( $cell:tt )* } )?
        }
    ) => {
        $crate::entity_schema!(@base
            [ [ $(#[$meta])* $vis $name ] [ $( $( $client )* )? ] [ $( $( $cell )* )? ] ]
            [] [] $( $( $base )* )?);
    };

    // Sort a side's members into its methods and its properties, the base side then the
    // cell side.
    (@base $state:tt [ $( $fns:tt )* ] $props:tt
        $(#[$fn_meta:meta])* fn $fn_name:ident ( $( $args:tt )* ); $( $rest:tt )*
    ) => {
        $crate::entity_schema!(@base $state [ $( $fns )* { $(#[$fn_meta])* $fn_name ( $( $args )* ) } ] $props $( $rest )*);
    };
    (@base $state:tt $fns:tt [ $( $props:tt )* ]
        $(#[$prop_meta:meta])* $prop:ident : $ty:ty $( , $( $rest:tt )* )?
    ) => {
        $crate::entity_schema!(@base $state $fns [ $( $props )* { $(#[$prop_meta])* $prop : $ty } ] $( $( $rest )* )?);
    };
    (@base [ $header:tt $client:tt [ $( $cell:tt )* ] ] $base_fns:tt $base_props:tt) => {
        $crate::entity_schema!(@cell [ $header $client $base_fns $base_props ] [] [] $( $cell )*);
    };
    (@cell $state:tt [ $( $fns:tt )* ] $props:tt
        $(#[$fn_meta:meta])* fn $fn_name:ident ( $( $args:tt )* ); $( $rest:tt )*
    ) => {
        $crate::entity_schema!(@cell $state [ $( $fns )* { $(#[$fn_meta])* $fn_name ( $( $args )* ) } ] $props $( $rest )*);
    };
    (@cell $state:tt $fns:tt [ $( $props:tt )* ]
        $(#[$prop_meta:meta])* $prop:ident : $ty:ty $( , $( $rest:tt )* )?
    ) => {
        $crate::entity_schema!(@cell $state $fns [ $( $props )* { $(#[$prop_meta])* $prop : $ty } ] $( $( $rest )* )?);
    };
    (@cell [ $header:tt $client:tt $base_fns:tt $base_props:tt ] $cell_fns:tt $cell_props:tt) => {
        $crate::entity_schema!(@emit $header $client $base_fns $base_props $cell_fns $cell_props);
    };

    (@emit
        [ $(#[$meta:meta])* $vis:vis $name:ident ]
        [ $( $(#[$client_meta:meta])* fn $client:ident ( $( $client_arg:ident : $client_ty:ty ),* $(,)? ); )* ]
        [ $( { $(#[$base_meta:meta])* $base:ident ( $( $base_arg:ident : $base_ty:ty ),* $(,)? ) } )* ]
        [ $( { $(#[$base_prop_meta:meta])* $base_prop:ident : $base_prop_ty:ty } )* ]
        [ $( { $(#[$cell_meta:meta])* $cell:ident ( $( $cell_arg:ident : $cell_ty:ty ),* $(,)? ) } )* ]
        [ $( { $(#[$cell_prop_meta:meta])* $cell_prop:ident : $cell_prop_ty:ty } )* ]
    ) => { $crate::app::entity::__paste! {

        $(#[$meta])*
        $vis enum $name {}

        #[doc(hidden)]
        #[allow(non_snake_case, non_camel_case_types, dead_code)]
        mod [<__ $name:snake _schema>] {
            pub enum Client { $( $client, )* }
            pub enum Base { $( $base, )* }
            pub enum Cell { $( $cell, )* }
            pub enum Props { $( $base_prop, )* $( $cell_prop, )* }
        }

        impl $crate::app::entity::Schema for $name {
            const NAME: &'static str = stringify!($name);
            const CLIENT_METHODS: &'static [$crate::app::entity::MethodDecl] = &[ $(
                $crate::app::entity::MethodDecl {
                    name: stringify!($client),
                    args: <( $( $client_ty, )* ) as $crate::app::entity::ScriptArgs>::TYPES,
                },
            )* ];
            const BASE_METHODS: &'static [$crate::app::entity::MethodDecl] = &[ $(
                $crate::app::entity::MethodDecl {
                    name: stringify!($base),
                    args: <( $( $base_ty, )* ) as $crate::app::entity::ScriptArgs>::TYPES,
                },
            )* ];
            const CELL_METHODS: &'static [$crate::app::entity::MethodDecl] = &[ $(
                $crate::app::entity::MethodDecl {
                    name: stringify!($cell),
                    args: <( $( $cell_ty, )* ) as $crate::app::entity::ScriptArgs>::TYPES,
                },
            )* ];
            const PROPERTIES: &'static [$crate::app::entity::PropertyDecl] = &[
                $( $crate::app::entity::PropertyDecl {
                    name: stringify!($base_prop),
                    side: $crate::app::entity::Side::Base,
                    ty: $crate::app::entity::TypeDecl::of::<$base_prop_ty>(),
                }, )*
                $( $crate::app::entity::PropertyDecl {
                    name: stringify!($cell_prop),
                    side: $crate::app::entity::Side::Cell,
                    ty: $crate::app::entity::TypeDecl::of::<$cell_prop_ty>(),
                }, )*
            ];
        }

        impl $crate::app::entity::EntitySchema for $name {
            type View<'a> = [<$name View>]<'a>;
        }

        /// The view of an entity, see its schema.
        $vis struct [<$name View>]<'a>($crate::app::entity::RawEntity<'a>);

        impl<'a> $crate::app::entity::EntityView<'a> for [<$name View>]<'a> {
            fn new(raw: $crate::app::entity::RawEntity<'a>) -> Self { Self(raw) }
            fn raw(&self) -> &$crate::app::entity::RawEntity<'a> { &self.0 }
            fn raw_mut(&mut self) -> &mut $crate::app::entity::RawEntity<'a> { &mut self.0 }
        }

        #[allow(non_snake_case, dead_code)]
        impl [<$name View>]<'_> {
            $(
                $(#[$client_meta])*
                pub fn $client(&mut self $(, $client_arg: $client_ty )*) -> ::std::io::Result<()> {
                    self.0.call_client([<__ $name:snake _schema>]::Client::$client as usize, ( $( $client_arg, )* ))
                }
            )*
            $(
                $(#[$base_prop_meta])*
                pub fn $base_prop(&self) -> ::std::option::Option<$base_prop_ty> {
                    self.0.get_property([<__ $name:snake _schema>]::Props::$base_prop as usize)
                }
                $(#[$base_prop_meta])*
                pub fn [<set_ $base_prop>](&mut self, value: $base_prop_ty) -> ::std::io::Result<()> {
                    self.0.set_property([<__ $name:snake _schema>]::Props::$base_prop as usize, value)
                }
            )*
            $(
                $(#[$cell_prop_meta])*
                pub fn $cell_prop(&self) -> ::std::option::Option<$cell_prop_ty> {
                    self.0.get_property([<__ $name:snake _schema>]::Props::$cell_prop as usize)
                }
                $(#[$cell_prop_meta])*
                pub fn [<set_ $cell_prop>](&mut self, value: $cell_prop_ty) -> ::std::io::Result<()> {
                    self.0.set_property([<__ $name:snake _schema>]::Props::$cell_prop as usize, value)
                }
            )*
        }

        $crate::entity_schema!(@methods $vis $name Base [ $( { $(#[$base_meta])* $base ( $( $base_arg : $base_ty ),* ) } )* ]);
        $crate::entity_schema!(@methods $vis $name Cell [ $( { $(#[$cell_meta])* $cell ( $( $cell_arg : $cell_ty ),* ) } )* ]);

    } };

    // The enum of the methods of one side, when it receives any.
    (@methods $vis:vis $name:ident $side:ident []) => {};
    (@methods $vis:vis $name:ident $side:ident
        [ $( { $(#[$fn_meta:meta])* $fn_name:ident ( $( $arg:ident : $ty:ty ),* ) } )+ ]
    ) => { $crate::app::entity::__paste! {

        #[doc = concat!("The ", stringify!($side), " methods of [`", stringify!($name), "`] received.")]
        // A schema declares every argument the client sends, used or not.
        #[allow(dead_code)]
        #[derive(Debug)]
        $vis enum [<$name $side>] {
            $( $(#[$fn_meta])* [<$fn_name:camel>] { $( $arg: $ty ),* }, )*
        }

        impl $crate::app::entity::Methods for [<$name $side>] {
            type Schema = $name;
            const SIDE: $crate::app::entity::Side = $crate::app::entity::Side::$side;
            #[allow(unused)]
            fn decode(index: usize, args: ::std::vec::Vec<$crate::script::Value>) -> ::std::option::Option<Self> {
                $(
                    if index == [<__ $name:snake _schema>]::$side::$fn_name as usize {
                        let ( $( $arg, )* ) = <( $( $ty, )* ) as $crate::app::entity::ScriptArgs>::from_values(args)?;
                        return Some(Self::[<$fn_name:camel>] { $( $arg ),* });
                    }
                )*
                None
            }
        }

    } };

}


/// Declare the members of a dynamic component type that a server uses, each with Rust
/// types: generates the schema and the view its [`Component`] derefs to, with a method
/// per client method and a getter and a setter per property, as [`entity_schema!`] does
/// for entities: its properties are on the cell side.
///
/// ```ignore
/// component_schema! {
///     pub OwnVehicle {
///         client {
///             fn onObservedByEnemy(vehicle_id: i16);
///         }
///         cell {
///             vehicleGunReloadTime: GunReloadTime,
///         }
///     }
/// }
///
/// let mut own = app.entity(vehicle)?.add_component::<OwnVehicle>(Audience::OwnClient)?;
/// own.set_vehicleGunReloadTime(reload)?;
/// ```
///
/// Both sections are optional but come in this order. The cell section declares the
/// properties set, separated by `,`.
#[macro_export]
macro_rules! component_schema {
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident {
            $( client {
                $( $(#[$client_meta:meta])* fn $client:ident ( $( $client_arg:ident : $client_ty:ty ),* $(,)? ); )*
            } )?
            $( cell {
                $( $(#[$prop_meta:meta])* $prop:ident : $prop_ty:ty ),* $(,)?
            } )?
        }
    ) => { $crate::app::entity::__paste! {

        $(#[$meta])*
        $vis enum $name {}

        #[doc(hidden)]
        #[allow(non_snake_case, non_camel_case_types, dead_code)]
        mod [<__ $name:snake _component>] {
            pub enum Client { $( $( $client, )* )? }
            pub enum Props { $( $( $prop, )* )? }
        }

        impl $crate::app::entity::Schema for $name {
            const NAME: &'static str = stringify!($name);
            const CLIENT_METHODS: &'static [$crate::app::entity::MethodDecl] = &[ $( $(
                $crate::app::entity::MethodDecl {
                    name: stringify!($client),
                    args: <( $( $client_ty, )* ) as $crate::app::entity::ScriptArgs>::TYPES,
                },
            )* )? ];
            const PROPERTIES: &'static [$crate::app::entity::PropertyDecl] = &[ $( $(
                $crate::app::entity::PropertyDecl {
                    name: stringify!($prop),
                    side: $crate::app::entity::Side::Cell,
                    ty: $crate::app::entity::TypeDecl::of::<$prop_ty>(),
                },
            )* )? ];
            const BASE_METHODS: &'static [$crate::app::entity::MethodDecl] = &[];
            const CELL_METHODS: &'static [$crate::app::entity::MethodDecl] = &[];
        }

        impl $crate::app::entity::ComponentSchema for $name {
            type View<'a> = [<$name View>]<'a>;
        }

        /// The view of a component, see its schema.
        $vis struct [<$name View>]<'a>($crate::app::entity::RawComponent<'a>);

        impl<'a> $crate::app::entity::ComponentView<'a> for [<$name View>]<'a> {
            fn new(raw: $crate::app::entity::RawComponent<'a>) -> Self { Self(raw) }
            fn raw(&self) -> &$crate::app::entity::RawComponent<'a> { &self.0 }
            fn raw_mut(&mut self) -> &mut $crate::app::entity::RawComponent<'a> { &mut self.0 }
        }

        #[allow(non_snake_case, dead_code)]
        impl [<$name View>]<'_> {
            $( $(
                $(#[$client_meta])*
                pub fn $client(&mut self $(, $client_arg: $client_ty )*) -> ::std::io::Result<()> {
                    self.0.call_client([<__ $name:snake _component>]::Client::$client as usize, ( $( $client_arg, )* ))
                }
            )* )?
            $( $(
                $(#[$prop_meta])*
                pub fn $prop(&self) -> ::std::option::Option<$prop_ty> {
                    self.0.get_property([<__ $name:snake _component>]::Props::$prop as usize)
                }
                $(#[$prop_meta])*
                pub fn [<set_ $prop>](&mut self, value: $prop_ty) -> ::std::io::Result<()> {
                    self.0.set_property([<__ $name:snake _component>]::Props::$prop as usize, value)
                }
                $(#[$prop_meta])*
                pub fn [<set_ $prop _at>]<V: $crate::app::entity::ScriptType>(
                    &mut self,
                    path: ::std::vec::Vec<$crate::app::entity::PathStep>,
                    value: V,
                ) -> ::std::io::Result<()> {
                    self.0.set_property_at([<__ $name:snake _component>]::Props::$prop as usize, path, value)
                }
            )* )?
        }

    } };
}


#[cfg(test)]
mod tests {

    use crate::net::element::ElementLength;
    use crate::script::{TyDict, TyDictProp, TySeq, TySystem};
    use crate::app::dispatch::PropertyDef;

    use super::*;

    crate::script_dict! {
        #[derive(Debug, Clone, PartialEq)]
        pub struct Marker {
            pub gunPosition: Vec3,
            pub dispersion: f32,
        }
    }

    crate::entity_schema! {
        pub Tank {
            client {
                fn showShooting(burst: u8, gun: i8);
                fn setMarker(marker: Marker);
            }
            base {
                name: String,
            }
            cell {
                fn vehicle_shoot();
                health: i16,
                fn vehicle_moveWith(flags: u8);
                engineMode: [u8; 2]
            }
        }
    }

    crate::entity_schema! {
        pub Wrong {
            client {
                fn showShooting(burst: u16, gun: i8);
                fn missing();
            }
            base {
                health: i16,
            }
        }
    }

    fn method(name: &str, args: Vec<Ty>) -> MethodDef {
        MethodDef { name: name.into(), args, length: ElementLength::Variable8 }
    }

    fn property(name: &str, ty: Ty, flags: PropertyFlags) -> PropertyDef {
        PropertyDef { name: name.into(), ty, length: ElementLength::Variable8, flags, volatile: false, detail_level: None }
    }

    fn dispatch() -> EntityDispatch {
        let mut tys = TySystem::default();
        let u8_ty = tys.register(None, TyKind::UInt8);
        let i8_ty = tys.register(None, TyKind::Int8);
        let i16_ty = tys.register(None, TyKind::Int16);
        let f32_ty = tys.register(None, TyKind::Float32);
        let vec3_ty = tys.register(None, TyKind::Vector3);
        let str_ty = tys.register(None, TyKind::String);
        let marker_ty = tys.register(None, TyKind::Dict(TyDict { allow_none: false, properties: vec![
            TyDictProp { name: "gunPosition".into(), ty: vec3_ty, default: None },
            TyDictProp { name: "dispersion".into(), ty: f32_ty, default: None },
        ] }));
        let mode_ty = tys.register(None, TyKind::Array(TySeq { ty: u8_ty.clone(), size: Some(2) }));
        let empty = tys.register(None, TyKind::Dict(TyDict::default()));
        EntityDispatch {
            base_methods: Vec::new(),
            // The calls' exposed ids are not their declaration order.
            cell_methods: vec![
                method("vehicle_moveWith", vec![u8_ty.clone()]),
                method("other", vec![]),
                method("vehicle_shoot", vec![]),
            ],
            client_methods: vec![
                method("setMarker", vec![marker_ty]),
                method("showShooting", vec![u8_ty, i8_ty]),
            ],
            properties: vec![
                property("engineMode", mode_ty, PropertyFlags::AllClients),
                property("health", i16_ty, PropertyFlags::AllClients),
                property("name", str_ty, PropertyFlags::BaseAndClient),
            ],
            data_ty: empty.clone(),
            cell_data_ty: empty.clone(),
            cell_components_data_ty: empty,
            static_components_count: 0,
        }
    }

    #[test]
    fn binds_by_name_whatever_the_exposed_ids() {
        let bound = bind::<Tank>(&dispatch()).unwrap();
        assert_eq!(bound.client_methods, [1, 0]);
        assert_eq!(bound.properties, [2, 1, 0]);
        assert_eq!(bound.cell_methods, [Some(1), None, Some(0)]);
    }

    #[test]
    fn reports_every_mismatch() {
        let error = bind::<Wrong>(&dispatch()).unwrap_err();
        assert_eq!(error.errors.len(), 3, "{error}");
        assert!(error.errors[0].contains("showShooting argument 0"), "{error}");
        assert!(error.errors[1].contains("no client method missing"), "{error}");
        assert!(error.errors[2].contains("health is a Cell property"), "{error}");
    }

    #[test]
    fn decodes_calls_into_their_enum() {
        let bound = bind::<Tank>(&dispatch()).unwrap();
        let call = |exposed_id, args| MethodCall::Known { exposed_id, name: "".into(), args };
        assert!(matches!(bound.decode::<TankCell>(&call(0, vec![Value::UInt8(3)])), Some(TankCell::VehicleMoveWith { flags: 3 })));
        assert!(matches!(bound.decode::<TankCell>(&call(2, vec![])), Some(TankCell::VehicleShoot {})));
        assert!(bound.decode::<TankCell>(&call(1, vec![])).is_none(), "undeclared");
        assert!(bound.decode::<TankCell>(&call(0, vec![Value::Int8(3)])).is_none(), "wrong type");
    }

    #[test]
    fn dict_round_trips() {
        let dispatch = dispatch();
        let ty = &dispatch.client_methods[0].args[0];
        assert!(Marker::check(ty));
        let marker = Marker { gunPosition: Vec3::new(1.0, 2.0, 3.0), dispersion: 0.5 };
        let value = marker.clone().into_value(ty);
        assert!(ty.is_instance(&value));
        assert_eq!(Marker::from_value(value), Some(marker));
    }

}
