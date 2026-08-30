//! Dynamic entity dispatch: computes, straight from a loaded [`crate::script::Script`],
//! the exposed-id method/property tables and entity-creation layout that
//! [`crate::app::base::App`] uses to resolve an entity/method/property purely by *name*
//! at runtime, against whatever script model was loaded for the current game version --
//! see [`ScriptDispatch`], the entry point.
//!
//! This mirrors what `wg-toolkit-cli`'s `bootstrap` command used to compute at codegen
//! time (`generate_entity_methods`, `generate_entity_properties`, and their shared
//! stable-sort rule), just reimplemented here at runtime instead of requiring a
//! per-game generated Rust type ahead of time.

use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::net::element::ElementLength;
use crate::net::codec::Codec;
use crate::script::{
    Script, Interface, Method, Property, Component, PropertyFlags, VariableHeaderSize,
    Ty, TyKind, TyDict, TyDictProp, TySystem, Value,
};


/// Every entity's dynamic dispatch tables, computed once from a loaded [`Script`] --
/// lets a caller resolve an entity purely by its script-declared name or by its wire
/// type id, without repeating the script's own entity-name lookup at every call site.
///
/// `entities` is indexed by wire type id minus one: entity type ids are allocated as a
/// contiguous 1-based sequence matching `script.entities`' own order (see
/// [`crate::script::Entity::id`]), so a `Vec` slot maps directly to each entity, no
/// `HashMap` needed -- use [`Self::entity`] rather than indexing it directly, since the
/// wire id is off by one from the backing storage.
#[derive(Debug)]
pub struct ScriptDispatch {
    pub script: Script,
    pub entities: Vec<EntityDispatch>,
}

impl ScriptDispatch {

    /// Compute the dispatch tables for every entity declared in `script`, in the same
    /// order as `script.entities`.
    pub fn new(mut script: Script) -> Self {

        let mut entities = Vec::with_capacity(script.entities.len());

        for entity in &script.entities {
            entities.push(EntityDispatch {
                base_methods: build_method_table(&script.interfaces, &script.static_components, &entity.interface, base_methods_of),
                cell_methods: build_method_table(&script.interfaces, &script.static_components, &entity.interface, cell_methods_of),
                client_methods: build_method_table(&script.interfaces, &script.static_components, &entity.interface, client_methods_of),
                properties: build_property_table(&script.interfaces, &script.static_components, &entity.interface),
                data_ty: build_entity_data_ty(&mut script.tys, &script.interfaces, &entity.interface),
            });
        }

        Self { script, entities }

    }

    /// Look up an entity's dispatch tables by its wire type id (1-based -- see
    /// [`crate::script::Entity::id`]). Returns `None` for a type id out of range,
    /// including `0` (never a valid entity id), instead of panicking.
    pub fn entity_from_id(&self, type_id: u16) -> Option<&EntityDispatch> {
        self.entities.get(usize::from(type_id).checked_sub(1)?)
    }

    /// Look up an entity's wire type id and dispatch tables by its script-declared name.
    pub fn entity_from_name(&self, name: &str) -> Option<(u16, &EntityDispatch)> {
        let type_id = self.script.entities.iter().find(|e| &*e.interface.name == name)?.id as u16;
        Some((type_id, self.entity_from_id(type_id)?))
    }

}

/// Per entity-type dynamic dispatch tables, computed once from a [`Script`] -- see
/// [`ScriptDispatch`], which builds and exposes these.
#[derive(Debug)]
pub struct EntityDispatch {
    pub base_methods: Vec<MethodDef>,
    pub cell_methods: Vec<MethodDef>,
    pub client_methods: Vec<MethodDef>,
    /// This entity's client-visible property table.
    pub properties: Vec<PropertyDef>,
    /// The type describing this entity's `CreateBasePlayer` creation payload.
    pub data_ty: Ty,
}

/// One exposed method slot in a dynamically-computed, exposed-id-ordered method table --
/// see [`EntityDispatch`].
#[derive(Debug, Clone)]
pub struct MethodDef {
    pub name: Arc<str>,
    pub args: Vec<Ty>,
    pub length: ElementLength,
}

impl MethodDef {

    /// Decode this method's arguments, in declared order, from the given reader.
    pub fn read_args(&self, read: &mut dyn Read) -> io::Result<Vec<Value>> {
        self.args.iter().map(|ty| Value::read(read, ty)).collect()
    }

    /// Encode this method's arguments, in declared order, into the given writer.
    pub fn write_args(&self, write: &mut dyn Write, args: &[Value]) -> io::Result<()> {
        for (ty, value) in self.args.iter().zip(args) {
            value.write(write, ty)?;
        }
        Ok(())
    }

}

/// One exposed client-visible property slot in a dynamically-computed, exposed-id-ordered
/// property table -- see [`EntityDispatch`].
#[derive(Debug, Clone)]
pub struct PropertyDef {
    pub name: Arc<str>,
    pub ty: Ty,
    pub length: ElementLength,
}

/// The dynamically-decoded result of a method call (base- or cell-directed), produced by
/// [`crate::app::base::App`] while dispatching against a [`MethodDef`] table.
#[derive(Debug, Clone)]
pub enum MethodCall {
    /// A call whose exposed id is present in the entity's method table for this
    /// direction.
    Known {
        name: Arc<str>,
        args: Vec<Value>,
    },
    /// An exposed id missing from the table (e.g. a mismatch between the loaded script
    /// model and what the live game actually sends).
    Unknown {
        exposed_id: u16,
        data: Vec<u8>,
    },
}

/// Whether this method reaches the client at all -- either flag alone is enough
/// (`exposed_to_all_clients` is always set for client methods, `exposed_to_own_client`
/// is the base/cell-only equivalent restricted to the entity's own owning client). Same
/// filter as `wg-toolkit-cli`'s codegen (`is_method_exposed`).
fn is_method_exposed(method: &Method) -> bool {
    method.exposed_to_all_clients || method.exposed_to_own_client
}

/// Same filter BigWorld's `DataDescription::isClientServerData()` uses: any of
/// `OTHER_CLIENT`/`OWN_CLIENT`/`BASE` flag bits reaches the client one way or another,
/// `CellPublic`/`CellPrivate` alone (ghosted server-to-server replication only) does not.
/// Same filter as `wg-toolkit-cli`'s codegen (`is_property_exposed`).
fn is_property_exposed(property: &Property) -> bool {
    matches!(property.flags, PropertyFlags::AllClients | PropertyFlags::OwnClient | PropertyFlags::BaseAndClient)
}

/// Whether this property is both client-visible *and* base-hosted (BigWorld's
/// `DataDescription::isBaseData()`, gated on the `DATA_BASE` flag bit specifically) --
/// unlike [`is_property_exposed`], this excludes `AllClients`/`OwnClient`, which are
/// cell-hosted (confirmed live: a `CreateBasePlayer` entity_data built with the wider
/// filter reads past its own declared element length, because cell-hosted properties
/// like `AvatarObserver`'s `remoteCamera`/`isObserverFPV`/`numOfObservers`, all flagged
/// plain `OWN_CLIENT` with no `BASE_AND_CLIENT`, aren't actually part of the base
/// creation payload at all -- they arrive later, opaque, inside `CreateCellPlayer`'s own
/// `cell_data` blob). Only [`build_entity_data_ty`] needs this narrower filter: the
/// exposed-id property table built by [`build_property_table`] legitimately covers
/// every client-visible property regardless of hosting, since an individual
/// property-change broadcast can originate from either side.
fn is_base_property_exposed(property: &Property) -> bool {
    matches!(property.flags, PropertyFlags::BaseAndClient)
}

/// Return the fixed on-wire size of this type in bytes, or `None` if it has no fixed
/// size (e.g. a string, or a sequence/dict containing one).
fn ty_stream_size(ty: &Ty) -> Option<usize> {
    match ty.kind() {
        TyKind::Int8 | TyKind::UInt8 => Some(1),
        TyKind::Int16 | TyKind::UInt16 => Some(2),
        TyKind::Int32 | TyKind::UInt32 => Some(4),
        TyKind::Int64 | TyKind::UInt64 => Some(8),
        TyKind::Float32 => Some(4),
        TyKind::Float64 => Some(8),
        TyKind::Vector2 => Some(4 * 2),
        TyKind::Vector3 => Some(4 * 3),
        TyKind::Vector4 => Some(4 * 4),
        TyKind::String => None,
        TyKind::Python => None,
        TyKind::Mailbox => None,
        TyKind::Alias(inner) => ty_stream_size(inner),
        TyKind::Dict(dict) =>
            dict.properties.iter()
                .map(|prop| ty_stream_size(&prop.ty))
                .sum(),
        TyKind::Array(seq) | TyKind::Tuple(seq) =>
            seq.size.map(|len| len as usize)
                .zip(ty_stream_size(&seq.ty))
                .map(|(len, element_size)| len * element_size),
    }
}

fn method_length(method: &Method) -> ElementLength {
    let size: Option<usize> = method.args.iter().map(|arg| ty_stream_size(&arg.ty)).sum();
    match size {
        Some(size) => ElementLength::Fixed(size as u32),
        None => match method.variable_header_size {
            VariableHeaderSize::Variable8 => ElementLength::Variable8,
            VariableHeaderSize::Variable16 => ElementLength::Variable16,
            VariableHeaderSize::Variable24 => ElementLength::Variable24,
            VariableHeaderSize::Variable32 => ElementLength::Variable32,
        }
    }
}

fn property_length(property: &Property) -> ElementLength {
    match ty_stream_size(&property.ty) {
        Some(size) => ElementLength::Fixed(size as u32),
        // Unlike methods (`method_length` above), a property has no per-declaration
        // `VariableLengthHeaderSize`-equivalent in this project's script model to read a
        // real preferred size from -- properties don't expose one in the entity XML the
        // way methods do. `Variable8` here is a live-evidence-backed guess, not a
        // confirmed-live constant like the one on `EntityMethod::read_length`: every
        // variable-sized property observed live so far that uses this fallback (e.g.
        // `Avatar::arenaExtraData`, a `PYTHON` property) failed to decode with the
        // previous `Variable16` guess (`failed to fill whole buffer`, consistently, on
        // this one property, while every *fixed*-size property on the same entity
        // decoded fine) -- exactly what a too-wide length prefix looks like. `Variable8`
        // also mirrors `EntityMethod`'s own confirmed-live default for anything without a
        // declared preferred size (Mercury's `DEFAULT_VARIABLE_LENGTH_HEADER_SIZE`
        // sentinel), which is at least a principled guess rather than an arbitrary one.
        // Flag for re-evaluation if a *larger* (>254 byte) variable-sized property is
        // ever observed failing under this new guess instead.
        None => ElementLength::Variable8,
    }
}

/// Sort key mirroring `wg-toolkit-cli`'s codegen stable `sort_by_key(stream_size)` rule:
/// any fixed size sorts before any variable one, fixed sizes sort ascending among
/// themselves, and (because [`Vec::sort_by_key`] is stable) anything comparing equal
/// keeps its original relative order.
fn length_sort_key(length: ElementLength) -> (u8, u32) {
    match length {
        ElementLength::Fixed(size) => (0, size),
        ElementLength::Variable8 => (1, 0),
        ElementLength::Variable16 => (2, 0),
        ElementLength::Variable24 => (3, 0),
        ElementLength::Variable32 => (4, 0),
        ElementLength::Undefined => (5, 0),
    }
}

fn find_interface<'m>(interfaces: &'m [Interface], name: &str) -> &'m Interface {
    interfaces.iter().find(|i| &*i.name == name)
        .unwrap_or_else(|| panic!("unknown implemented interface: {name}"))
}

fn collect_methods<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    methods_of: fn(&Interface) -> &[Method],
    out: &mut Vec<(&'m Method, ElementLength)>,
) {
    for implement_name in &interface.implements {
        collect_methods(interfaces, find_interface(interfaces, implement_name), methods_of, out);
    }
    for method in methods_of(interface) {
        if is_method_exposed(method) {
            out.push((method, method_length(method)));
        }
    }
}

fn collect_properties<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    out: &mut Vec<(&'m Property, ElementLength)>,
) {
    for implement_name in &interface.implements {
        collect_properties(interfaces, find_interface(interfaces, implement_name), out);
    }
    for property in &interface.properties {
        if is_property_exposed(property) {
            out.push((property, property_length(property)));
        }
    }
}

/// Selects an interface's client methods, for use with [`build_method_table`].
fn client_methods_of(interface: &Interface) -> &[Method] { &interface.client_methods }
/// Selects an interface's base methods, for use with [`build_method_table`].
fn base_methods_of(interface: &Interface) -> &[Method] { &interface.base_methods }
/// Selects an interface's cell methods, for use with [`build_method_table`].
fn cell_methods_of(interface: &Interface) -> &[Method] { &interface.cell_methods }

/// Build the exposed-id-ordered method table for `entity_interface`'s methods in the
/// direction selected by `methods_of` (one of [`client_methods_of`], [`base_methods_of`],
/// [`cell_methods_of`]): every method reachable through `implements`, stable-sorted by
/// stream size (fixed first ascending, then variable ascending), with static-extension-
/// component methods folded in afterward keeping their own order (not resorted) --
/// mirrors real BigWorld's `allocateClientServerFullIndexes`
/// (`entitydef/entity_description.cpp`), reimplemented here at runtime instead of at
/// `wg-toolkit-cli`'s codegen time.
fn build_method_table(
    interfaces: &[Interface],
    static_components: &[Component],
    entity_interface: &Interface,
    methods_of: fn(&Interface) -> &[Method],
) -> Vec<MethodDef> {

    let mut collected = Vec::new();
    collect_methods(interfaces, entity_interface, methods_of, &mut collected);
    collected.sort_by_key(|&(_, length)| length_sort_key(length));

    let mut defs: Vec<MethodDef> = collected.into_iter()
        .map(|(method, length)| MethodDef {
            name: method.name.clone(),
            args: method.args.iter().map(|arg| arg.ty.clone()).collect(),
            length,
        })
        .collect();

    for component in static_components {

        if !component.of_entities.iter().any(|e| **e == *entity_interface.name) {
            continue;
        }

        for method in methods_of(&component.interface) {
            if is_method_exposed(method) {
                defs.push(MethodDef {
                    name: method.name.clone(),
                    args: method.args.iter().map(|arg| arg.ty.clone()).collect(),
                    length: method_length(method),
                });
            }
        }

    }

    defs

}

/// Build the entity's flat, exposed-id-ordered client-visible property table (covering
/// either its base or cell slice, both share one id space on the wire), used for
/// property-update dispatch -- not entity creation, see [`build_entity_data_ty`] for
/// that. Same stable-sort/component-folding rule as [`build_method_table`].
fn build_property_table(
    interfaces: &[Interface],
    static_components: &[Component],
    entity_interface: &Interface,
) -> Vec<PropertyDef> {

    let mut collected = Vec::new();
    collect_properties(interfaces, entity_interface, &mut collected);
    collected.sort_by_key(|&(_, length)| length_sort_key(length));

    let mut defs: Vec<PropertyDef> = collected.into_iter()
        .map(|(property, length)| PropertyDef { name: property.name.clone(), ty: property.ty.clone(), length })
        .collect();

    for component in static_components {

        if !component.of_entities.iter().any(|e| **e == *entity_interface.name) {
            continue;
        }

        for property in &component.interface.properties {
            if is_property_exposed(property) {
                defs.push(PropertyDef {
                    name: property.name.clone(),
                    ty: property.ty.clone(),
                    length: property_length(property),
                });
            }
        }

    }

    defs

}

/// Build the [`Ty`] (always a [`TyKind::Dict`]) describing an entity's base creation
/// payload (`CreateBasePlayer`'s `entity_data`): every *base-hosted*, client-visible
/// property reachable through `implements` (see [`is_base_property_exposed`] -- cell-hosted
/// properties arrive later, opaque, inside `CreateCellPlayer`'s `cell_data`), in plain
/// declaration order (recursed depth-first, NOT sorted by stream size -- unlike
/// [`build_property_table`], this must match the nested-struct field order the wire's
/// `Codec` actually walks, not the method/property *exposed id* table). Registers (and
/// returns) a fresh anonymous type in `tys`.
fn build_entity_data_ty(tys: &mut TySystem, interfaces: &[Interface], entity_interface: &Interface) -> Ty {

    fn collect(interfaces: &[Interface], interface: &Interface, out: &mut Vec<TyDictProp>) {
        for implement_name in &interface.implements {
            collect(interfaces, find_interface(interfaces, implement_name), out);
        }
        for property in &interface.properties {
            if is_base_property_exposed(property) {
                out.push(TyDictProp { name: property.name.clone(), ty: property.ty.clone(), default: None });
            }
        }
    }

    let mut properties = Vec::new();
    collect(interfaces, entity_interface, &mut properties);
    tys.register(None, TyKind::Dict(TyDict { properties, allow_none: false }))

}
