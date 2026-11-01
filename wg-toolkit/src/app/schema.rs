//! Typed bindings to the entities of a loaded script.
//!
//! A schema declares, with [`entity_schema!`], the members of an entity type that a
//! server uses: client methods it calls, base and cell methods it receives, and
//! properties it sets, each with Rust types. Registering a schema with an app binds it
//! once against the loaded script: every member is looked up by name and its types
//! checked, so that a game version that renamed or retyped one fails at startup, naming
//! it, rather than mid-battle. After that, calls and property changes go by the indices
//! found then, with no name lookup.
//!
//! Values cross into the script as [`Value`]s, through [`ScriptType`], and are encoded
//! by `Value`'s own codec, the one decoding live traffic: a Rust type only has to
//! convert, never to encode.

use std::any::TypeId;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::fmt;
use std::io;

use glam::{Vec2, Vec3, Vec4};

use crate::script::{PropertyFlags, PythonValue, StringValue, Ty, TyKind, Value};
use crate::util::io::{serde_pickle_de_options, serde_pickle_ser_options};

use super::client::element::{EntityMethod, IndexedEntityProperty};
use super::dispatch::{EntityDispatch, MethodCall, MethodDef};
use super::outbox::ClientOutbox;

#[doc(hidden)]
pub use paste::paste as __paste;


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

/// The members of an entity type that a server uses, a subset of the script's,
/// implemented by [`entity_schema!`].
pub trait Schema: 'static {
    /// The entity type's name in the script.
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
    type View<'a>: View<'a>;
}

/// The view an [`Entity`] derefs to, generated by [`entity_schema!`] with a method per
/// declared member.
pub trait View<'a> {
    fn new(raw: RawEntity<'a>) -> Self;
    fn raw(&self) -> &RawEntity<'a>;
    fn raw_mut(&mut self) -> &mut RawEntity<'a>;
}

/// The calls of one side that a schema receives, decoded into an enum generated by
/// [`entity_schema!`].
pub trait Calls: Sized {
    type Schema: Schema;
    const SIDE: Side;
    /// Decode a call of the declared method at the given index.
    fn decode(index: usize, args: Vec<Value>) -> Option<Self>;
}

/// A schema bound to an entity type of the loaded script, see [`bind`].
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
    pub fn decode<C: Calls>(&self, call: &MethodCall) -> Option<C> {
        if !self.is::<C::Schema>() {
            return None;
        }
        let MethodCall::Known { exposed_id, args, .. } = call else { return None };
        let methods = match C::SIDE {
            Side::Base => &self.base_methods,
            Side::Cell => &self.cell_methods,
        };
        let index = (*methods.get(*exposed_id as usize)?)?;
        C::decode(index.into(), args.clone())
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

/// Bind a schema to its entity type's dispatch tables, checking each declared member.
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


// =========== //
//    VIEWS    //
// =========== //

/// An entity of a client, as a schema sees it: derefs to its schema's view, whose
/// methods call and set its members.
pub struct Entity<'a, T: EntitySchema> {
    view: T::View<'a>,
}

impl<'a, T: EntitySchema> Entity<'a, T> {

    pub(crate) fn new(raw: RawEntity<'a>) -> Self {
        Self { view: T::View::new(raw) }
    }

    /// The entity's id.
    pub fn entity_id(&self) -> u32 {
        self.view.raw().entity_id
    }

    /// Call a client method the schema doesn't declare, by name, with values left
    /// unchecked: for calls replayed from a capture.
    pub fn call_dynamic(&mut self, name: &str, args: Vec<Value>) -> io::Result<()> {
        self.view.raw_mut().call_dynamic(name, args)
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

/// What a view writes through: the entity, its tables, its binding and the outbox of
/// the client it is on.
pub struct RawEntity<'a> {
    pub(crate) out: &'a mut ClientOutbox,
    pub(crate) dispatch: &'a EntityDispatch,
    pub(crate) bound: &'a Bound,
    pub(crate) entity_id: u32,
    /// Whether it is its client's player entity, selected as such.
    pub(crate) is_player: bool,
}

impl RawEntity<'_> {

    fn select(&mut self) -> io::Result<()> {
        if self.is_player {
            self.out.select_player()
        } else {
            self.out.select_entity(self.entity_id)
        }
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
        self.select()?;
        self.out.write_selected(IndexedEntityProperty { exposed_id, value }, &self.dispatch.properties[..])
    }

    fn call_dynamic(&mut self, name: &str, args: Vec<Value>) -> io::Result<()> {
        let exposed_id = self.dispatch.client_methods.iter().position(|def| &*def.name == name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unknown client method: {name}")))?;
        self.write_call(exposed_id as u16, args)
    }

    fn write_call(&mut self, exposed_id: u16, args: Vec<Value>) -> io::Result<()> {
        let name = Arc::clone(&self.dispatch.client_methods[exposed_id as usize].name);
        self.select()?;
        self.out.write_selected(EntityMethod { call: MethodCall::Known { exposed_id, name, args } }, &self.dispatch.client_methods)
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

        impl $crate::app::schema::ScriptType for $name {

            fn check(ty: &$crate::script::Ty) -> bool {
                $crate::app::schema::dict_check(ty, &[
                    $( (stringify!($field), <$ty as $crate::app::schema::ScriptType>::check) ),*
                ])
            }

            fn into_value(self, ty: &$crate::script::Ty) -> $crate::script::Value {
                let mut dict = ::std::collections::BTreeMap::new();
                $(
                    let field_ty = $crate::app::schema::dict_field_ty(ty, stringify!($field)).expect("checked");
                    dict.insert(::std::sync::Arc::from(stringify!($field)), $crate::app::schema::ScriptType::into_value(self.$field, field_ty));
                )*
                $crate::script::Value::Dict(dict)
            }

            fn from_value(value: $crate::script::Value) -> ::std::option::Option<Self> {
                let $crate::script::Value::Dict(mut dict) = value else { return None };
                Some(Self {
                    $( $field: $crate::app::schema::ScriptType::from_value(dict.remove(stringify!($field))?)? ),*
                })
            }

        }

    };
}

/// Declare the members of an entity type that a server uses, each with Rust types:
/// generates the schema, the view its [`Entity`] derefs to, with a method per client
/// method and a setter per property, and an enum per side of the calls it receives.
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
    ) => { $crate::app::schema::__paste! {

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

        impl $crate::app::schema::Schema for $name {
            const NAME: &'static str = stringify!($name);
            const CLIENT_METHODS: &'static [$crate::app::schema::MethodDecl] = &[ $(
                $crate::app::schema::MethodDecl {
                    name: stringify!($client),
                    args: <( $( $client_ty, )* ) as $crate::app::schema::ScriptArgs>::TYPES,
                },
            )* ];
            const BASE_METHODS: &'static [$crate::app::schema::MethodDecl] = &[ $(
                $crate::app::schema::MethodDecl {
                    name: stringify!($base),
                    args: <( $( $base_ty, )* ) as $crate::app::schema::ScriptArgs>::TYPES,
                },
            )* ];
            const CELL_METHODS: &'static [$crate::app::schema::MethodDecl] = &[ $(
                $crate::app::schema::MethodDecl {
                    name: stringify!($cell),
                    args: <( $( $cell_ty, )* ) as $crate::app::schema::ScriptArgs>::TYPES,
                },
            )* ];
            const PROPERTIES: &'static [$crate::app::schema::PropertyDecl] = &[
                $( $crate::app::schema::PropertyDecl {
                    name: stringify!($base_prop),
                    side: $crate::app::schema::Side::Base,
                    ty: $crate::app::schema::TypeDecl::of::<$base_prop_ty>(),
                }, )*
                $( $crate::app::schema::PropertyDecl {
                    name: stringify!($cell_prop),
                    side: $crate::app::schema::Side::Cell,
                    ty: $crate::app::schema::TypeDecl::of::<$cell_prop_ty>(),
                }, )*
            ];
        }

        impl $crate::app::schema::EntitySchema for $name {
            type View<'a> = [<$name View>]<'a>;
        }

        /// The view of an entity, see its schema.
        $vis struct [<$name View>]<'a>($crate::app::schema::RawEntity<'a>);

        impl<'a> $crate::app::schema::View<'a> for [<$name View>]<'a> {
            fn new(raw: $crate::app::schema::RawEntity<'a>) -> Self { Self(raw) }
            fn raw(&self) -> &$crate::app::schema::RawEntity<'a> { &self.0 }
            fn raw_mut(&mut self) -> &mut $crate::app::schema::RawEntity<'a> { &mut self.0 }
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
                pub fn [<set_ $base_prop>](&mut self, value: $base_prop_ty) -> ::std::io::Result<()> {
                    self.0.set_property([<__ $name:snake _schema>]::Props::$base_prop as usize, value)
                }
            )*
            $(
                $(#[$cell_prop_meta])*
                pub fn [<set_ $cell_prop>](&mut self, value: $cell_prop_ty) -> ::std::io::Result<()> {
                    self.0.set_property([<__ $name:snake _schema>]::Props::$cell_prop as usize, value)
                }
            )*
        }

        $crate::entity_schema!(@calls $vis $name Base [ $( { $(#[$base_meta])* $base ( $( $base_arg : $base_ty ),* ) } )* ]);
        $crate::entity_schema!(@calls $vis $name Cell [ $( { $(#[$cell_meta])* $cell ( $( $cell_arg : $cell_ty ),* ) } )* ]);

    } };

    // The enum of the calls of one side, when it receives any.
    (@calls $vis:vis $name:ident $side:ident []) => {};
    (@calls $vis:vis $name:ident $side:ident
        [ $( { $(#[$fn_meta:meta])* $fn_name:ident ( $( $arg:ident : $ty:ty ),* ) } )+ ]
    ) => { $crate::app::schema::__paste! {

        #[doc = concat!("The ", stringify!($side), " methods of [`", stringify!($name), "`] received.")]
        #[derive(Debug)]
        $vis enum [<$name $side>] {
            $( $(#[$fn_meta])* [<$fn_name:camel>] { $( $arg: $ty ),* }, )*
        }

        impl $crate::app::schema::Calls for [<$name $side>] {
            type Schema = $name;
            const SIDE: $crate::app::schema::Side = $crate::app::schema::Side::$side;
            #[allow(unused)]
            fn decode(index: usize, args: ::std::vec::Vec<$crate::script::Value>) -> ::std::option::Option<Self> {
                $(
                    if index == [<__ $name:snake _schema>]::$side::$fn_name as usize {
                        let ( $( $arg, )* ) = <( $( $ty, )* ) as $crate::app::schema::ScriptArgs>::from_values(args)?;
                        return Some(Self::[<$fn_name:camel>] { $( $arg ),* });
                    }
                )*
                None
            }
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
        PropertyDef { name: name.into(), ty, length: ElementLength::Variable8, flags }
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
