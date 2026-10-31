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

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::script::{
    Script, Interface, Method, Property, PropertyFlags, VariableHeaderSize, Component,
    Ty, TyKind, TyDict, TyDictProp, TySystem, Value,
};
use crate::net::element::ElementLength;
use crate::net::codec::Codec;


/// Every entity's dynamic dispatch tables, computed once from a loaded [`Script`] --
/// lets a caller resolve an entity purely by its script-declared name or by its wire
/// type id, without repeating the script's own entity-name lookup at every call site.
///
/// `entities` is indexed by wire type id minus one: entity type ids are allocated as a
/// contiguous 1-based sequence matching `script.entities`' own order (see
/// [`crate::script::Entity::id`]), followed immediately by one slot per
/// `script.dynamic_components` entry, same order. Dynamic components get their own,
/// independently creatable entity type id in the client's `EntityDescriptionMap`,
/// continuing the same counter right after the last real entity, in declaration order,
/// deduped by name (see `script::load`'s `seen_components`). `StaticComponents` never
/// get a slot of their own, only folded
/// into whatever entity's table references them via `<ofEntity>`. So a `Vec` slot still
/// maps directly to each id, no `HashMap` needed -- use [`Self::entity_from_id`] rather
/// than indexing it directly, since the wire id is off by one from the backing storage.
#[derive(Debug)]
pub struct ScriptDispatch {
    pub script: Script,
    pub entities: Vec<EntityDispatch>,
    /// The type id of each dynamic component that has an NRL component type id, in NRL
    /// order, see [`Self::component_from_nrl_id`].
    nrl_components: Vec<u16>,
}

impl ScriptDispatch {

    /// Compute the dispatch tables for every entity declared in `script`, in the same
    /// order as `script.entities`, followed by one per `script.dynamic_components` entry
    /// (see [`ScriptDispatch`]'s doc comment on why).
    pub fn new(mut script: Script) -> Self {

        let mut entities = Vec::with_capacity(script.entities.len() + script.dynamic_components.len());

        for entity in &script.entities {
            entities.push(EntityDispatch {
                base_methods: build_method_table(&script.interfaces, &entity.interface, base_methods_of, &script.static_components),
                cell_methods: build_method_table(&script.interfaces, &entity.interface, cell_methods_of, &script.static_components),
                client_methods: build_method_table(&script.interfaces, &entity.interface, client_methods_of, &script.static_components),
                client_methods_declared: build_declared_order(&script.interfaces, &entity.interface, client_methods_of, &script.static_components),
                properties: build_property_table(&script.interfaces, &entity.interface, &script.static_components),
                data_ty: build_entity_data_ty(&mut script.tys, &script.interfaces, &entity.interface, is_base_property_exposed),
                cell_data_ty: build_entity_data_ty(&mut script.tys, &script.interfaces, &entity.interface, is_cell_property_exposed),
                cell_components_data_ty: build_components_data_ty(&mut script.tys, &script.interfaces, &entity.interface, &script.static_components, is_cell_property_exposed),
            });
        }

        for component in &script.dynamic_components {
            entities.push(EntityDispatch {
                base_methods: build_method_table(&script.interfaces, &component.interface, base_methods_of, &[]),
                cell_methods: build_method_table(&script.interfaces, &component.interface, cell_methods_of, &[]),
                client_methods: build_method_table(&script.interfaces, &component.interface, client_methods_of, &[]),
                client_methods_declared: build_declared_order(&script.interfaces, &component.interface, client_methods_of, &[]),
                properties: build_property_table(&script.interfaces, &component.interface, &[]),
                data_ty: build_entity_data_ty(&mut script.tys, &script.interfaces, &component.interface, is_base_property_exposed),
                cell_data_ty: build_entity_data_ty(&mut script.tys, &script.interfaces, &component.interface, is_cell_property_exposed),
                cell_components_data_ty: build_components_data_ty(&mut script.tys, &script.interfaces, &component.interface, &[], is_cell_property_exposed),
            });
        }

        let nrl_components = script.dynamic_components.iter().enumerate()
            .filter(|(_, component)| component.has_client_class)
            .map(|(index, _)| (script.entities.len() + 1 + index) as u16)
            .collect();

        Self { script, entities, nrl_components }

    }

    /// Look up a dynamic component by the component type id an NRL `DynComponent` node
    /// carries, returning its own type id and dispatch tables.
    ///
    /// NRL numbers components like [`Self::entity_from_id`] does, continuing after the
    /// entities, but skips every component the client has no script class for (see
    /// [`crate::script::Component::has_client_class`]), so the two ids drift apart past
    /// the first such component. Checked against a live battle: every one of the 10
    /// distinct ids seen resolves to a component whose property and client method count
    /// fits the node's children (see `NrlNode::DynComponent`), and the two named
    /// instances (`ShotsReceiver_*`, `SequenceNetworkSync_*`) resolve to their own name.
    pub fn component_from_nrl_id(&self, component_type_id: u16) -> Option<(u16, &EntityDispatch)> {
        let index = usize::from(component_type_id).checked_sub(self.script.entities.len() + 1)?;
        let type_id = *self.nrl_components.get(index)?;
        Some((type_id, self.entity_from_id(type_id)?))
    }

    /// Look up an entity's dispatch tables by its wire type id (1-based -- see
    /// [`crate::script::Entity::id`]). Returns `None` for a type id out of range,
    /// including `0` (never a valid entity id), instead of panicking.
    pub fn entity_from_id(&self, type_id: u16) -> Option<&EntityDispatch> {
        self.entities.get(usize::from(type_id).checked_sub(1)?)
    }

    /// Look up an entity's or dynamic component's wire type id and dispatch tables by
    /// its script-declared name.
    pub fn entity_from_name(&self, name: &str) -> Option<(u16, &EntityDispatch)> {
        let type_id = if let Some(entity) = self.script.entities.iter().find(|e| &*e.interface.name == name) {
            entity.id as u16
        } else {
            let offset = self.script.dynamic_components.iter().position(|c| &*c.name == name)?;
            (self.script.entities.len() + 1 + offset) as u16
        };
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
    /// The name of each client method in declaration order, which is how NRL numbers a
    /// component's methods, see [`Self::client_method_declared`].
    pub client_methods_declared: Vec<Arc<str>>,
    /// This entity's client-visible property table.
    pub properties: Vec<PropertyDef>,
    /// The type describing this entity's `CreateBasePlayer` creation payload.
    pub data_ty: Ty,
    /// The type describing this entity's own part of the `CreateCellPlayer` creation
    /// payload.
    pub cell_data_ty: Ty,
    /// The type describing the static components' part of the `CreateCellPlayer` creation
    /// payload, which follows [`Self::cell_data_ty`] and a count byte.
    pub cell_components_data_ty: Ty,
}

/// One exposed method slot in a dynamically-computed, exposed-id-ordered method table --
/// see [`EntityDispatch`].
#[derive(Debug, Clone)]
pub struct MethodDef {
    pub name: Arc<str>,
    pub args: Vec<Ty>,
    pub length: ElementLength,
}

impl EntityDispatch {

    /// Look up a client method by its index in declaration order, deduplicated by name
    /// like the exposed table. An NRL method node carries this index (checked against
    /// all 11 of a live `OwnVehicle`'s), not the exposed id.
    pub fn client_method_declared(&self, index: u32) -> Option<&MethodDef> {
        let name = self.client_methods_declared.get(usize::try_from(index).ok()?)?;
        self.client_methods.iter().find(|method| method.name == *name)
    }

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
/// cell-hosted. Cell-hosted properties such as `AvatarObserver`'s `remoteCamera`/
/// `isObserverFPV`/`numOfObservers`, flagged plain `OWN_CLIENT` with no
/// `BASE_AND_CLIENT`, are not part of the base creation payload: they arrive inside
/// `CreateCellPlayer`'s own `cell_data` blob. A `CreateBasePlayer` entity_data type built
/// with the wider filter would read past its element. Only [`build_entity_data_ty`]
/// needs this narrower filter: the
/// exposed-id property table built by [`build_property_table`] legitimately covers
/// every client-visible property regardless of hosting, since an individual
/// property-change broadcast can originate from either side.
fn is_base_property_exposed(property: &Property) -> bool {
    matches!(property.flags, PropertyFlags::BaseAndClient)
}

/// The cell-hosted counterpart of [`is_base_property_exposed`]: client-visible and
/// cell-hosted, BigWorld's `FROM_CELL_TO_CLIENT_DATA` domain that `Witness::Witness`
/// streams into `CreateCellPlayer`'s `cell_data`.
fn is_cell_property_exposed(property: &Property) -> bool {
    matches!(property.flags, PropertyFlags::OwnClient | PropertyFlags::AllClients)
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
        TyKind::Dict(dict) => {
            // An `AllowNone` FIXED_DICT has no fixed stream size at all: it is a lone
            // presence byte when `None`, and that byte plus its fields otherwise (see
            // `Value`'s `Codec<Ty>` impl in `net/codec.rs`). Confirmed against the live
            // client, which gives such a property a variable-length slot -- counting it as
            // fixed sorts it into the wrong place in the property table and shifts every
            // slot after it.
            if dict.allow_none {
                return None;
            }
            dict.properties.iter()
                .map(|prop| ty_stream_size(&prop.ty))
                .sum()
        }
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
        // `VariableLengthHeaderSize` equivalent in the entity XML to read a preferred
        // size from. `Variable8` is the fallback because it matches Mercury's
        // `DEFAULT_VARIABLE_LENGTH_HEADER_SIZE`, the default `EntityMethod::read_length`
        // uses, and because a `Variable16` prefix fails on variable-sized properties such
        // as `Avatar::arenaExtraData` while the same entity's fixed-size properties
        // decode. A variable-sized property longer than 254 bytes would not fit it.
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

/// Visit every method reachable from `interface` in BigWorld's own declaration order:
/// everything pulled in through `<Implements>` first, then the interface's own. Entity
/// `<Parent>` inheritance is already flattened into `interface` by `script::load`.
fn walk_methods<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    methods_of: fn(&Interface) -> &[Method],
    visit: &mut dyn FnMut(&'m Method),
) {
    for implement_name in &interface.implements {
        walk_methods(interfaces, find_interface(interfaces, implement_name), methods_of, visit);
    }
    for method in methods_of(interface) {
        visit(method);
    }
}

/// As [`walk_methods`], for properties.
fn walk_properties<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    visit: &mut dyn FnMut(&'m Property),
) {
    for implement_name in &interface.implements {
        walk_properties(interfaces, find_interface(interfaces, implement_name), visit);
    }
    for property in &interface.properties {
        visit(property);
    }
}

/// Append `interface`'s exposed methods to `out`, in declaration order and not yet sorted.
///
/// `seen` spans every call that feeds one entity's table -- the entity's own members, then
/// each static component's -- so both rules that decide whether a method gets a slot live
/// here, once, rather than being restated per phase. Restating them is precisely what left
/// `is_method_exposed` out of the component phase and pushed
/// `checkPositionForEquipment` two slots past the id the client sends.
fn collect_methods<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    methods_of: fn(&Interface) -> &[Method],
    seen: &mut HashSet<Arc<str>>,
    out: &mut Vec<(&'m Method, ElementLength)>,
) {
    walk_methods(interfaces, interface, methods_of, &mut |method| {
        // An unexposed method has no exposed id at all. A redeclared name keeps the first
        // slot: `EntityMethodDescriptions::init` pushes to `exposedMethods_` only when the
        // name -> index insert is new, and a repeat merely records another implementing
        // component. Either way a second entry would lengthen the table and shift every
        // slot after it.
        if is_method_exposed(method) && seen.insert(Arc::clone(&method.name)) {
            out.push((method, method_length(method)));
        }
    });
}

/// Append `interface`'s exposed, client-visible properties to `out`, in declaration order
/// and not yet sorted. `seen` spans phases exactly as in [`collect_methods`].
fn collect_properties<'m>(
    interfaces: &'m [Interface],
    interface: &'m Interface,
    seen: &mut HashSet<Arc<str>>,
    out: &mut Vec<(&'m Property, ElementLength)>,
) {
    // Slots placed by *this* call. A property redeclared within one phase -- by an entity
    // and one of its interfaces, or by an entity and its `<Parent>` (live:
    // `ClientSelectableCameraVehicle` redeclares `ClientSelectableObject`'s `modelName`) --
    // is an override: `parseProperties` finds the name in the property map, reuses the
    // existing `index` and its already-allocated `clientServerFullIndex`, then overwrites
    // the slot with `properties_[index] = dataDescription`. So it keeps the first
    // declaration's place but contributes the last declaration's type, and therefore the
    // last one's stream size, which is what the caller's sort reads.
    let mut slot_of: HashMap<Arc<str>, usize> = HashMap::new();
    walk_properties(interfaces, interface, &mut |property| {
        // Only a client-server property is ever allocated a `clientServerFullIndex`, so a
        // redeclaration that merely widens visibility (live: `RepairBase`'s `CELL_PRIVATE`
        // `team`, made `ALL_CLIENTS` by `StepRepairPoint`) overrides nothing -- the
        // parent's copy never gets this far.
        if !is_property_exposed(property) {
            return;
        }
        match slot_of.get(&property.name) {
            Some(&slot) => out[slot] = (property, property_length(property)),
            None if seen.insert(Arc::clone(&property.name)) => {
                slot_of.insert(Arc::clone(&property.name), out.len());
                out.push((property, property_length(property)));
            }
            None => {}
        }
    });
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
    entity_interface: &Interface,
    methods_of: fn(&Interface) -> &[Method],
    static_components: &[Component],
) -> Vec<MethodDef> {

    // The entity's own and inherited (`implements`) methods occupy the leading slots.
    // `seen` then travels with each component call below, so a name the entity already
    // declares never takes a second slot -- live example: `Account::requestToken`, also in
    // `AccountAuthTokenProvider`. An extra entry lengthens the table and shifts every slot
    // after it, which is what made `Account`'s exposed id 13 read as
    // `accountDebugger_registerDebugTaskResult` (20 bytes) where the client meant
    // `doCmdInt3` (28), leaving 8 bytes unread on every call.
    let mut seen = HashSet::new();
    let mut collected = Vec::new();
    collect_methods(interfaces, entity_interface, methods_of, &mut seen, &mut collected);
    collected.sort_by_key(|&(_, length)| length_sort_key(length));

    // Then each static component targeting this entity, appended after that sort in
    // component order, its own methods size-sorted among themselves -- exactly as
    // [`build_property_table`] does for properties.
    //
    // For example, the client's own `MethodDescription` exposed index (int32 at +152)
    // puts `LaPingerComponent`'s `pingMeAndThenJustTouchMe` at **43** on `Account`.
    // `Account` owns 41 client methods (slots 0..40), and the only other component
    // methods targeting it are `AccountBattleRoyaleTournamentComponent`'s two, at 41 and
    // 42, so 43 is the only consistent arrangement.
    //
    // Going through [`collect_methods`] rather than reading `component.interface` directly
    // keeps the exposure and dedup rules identical to the entity's own: unexposed
    // component methods take no slot, so `AvatarInBattleVehicleSwitch`'s two unexposed
    // cell methods leave `Avatar`'s cell table at 24 entries, not 26.
    let entity_name = &*entity_interface.name;
    for component in static_components {
        if !component.of_entities.iter().any(|e| e == entity_name) {
            continue;
        }
        let mut of_component = Vec::new();
        collect_methods(interfaces, &component.interface, methods_of, &mut seen, &mut of_component);
        of_component.sort_by_key(|&(_, length)| length_sort_key(length));
        collected.extend(of_component);
    }

    collected.into_iter()
        .map(|(method, length)| MethodDef {
            name: method.name.clone(),
            args: method.args.iter().map(|arg| arg.ty.clone()).collect(),
            length,
        })
        .collect()

}

/// The names [`build_method_table`] collects, in declaration order rather than sorted.
fn build_declared_order(
    interfaces: &[Interface],
    entity_interface: &Interface,
    methods_of: fn(&Interface) -> &[Method],
    static_components: &[Component],
) -> Vec<Arc<str>> {
    let mut seen = HashSet::new();
    let mut collected = Vec::new();
    collect_methods(interfaces, entity_interface, methods_of, &mut seen, &mut collected);
    let entity_name = &*entity_interface.name;
    for component in static_components {
        if component.of_entities.iter().any(|e| e == entity_name) {
            collect_methods(interfaces, &component.interface, methods_of, &mut seen, &mut collected);
        }
    }
    collected.into_iter().map(|(method, _)| Arc::clone(&method.name)).collect()
}

/// Build the entity's flat, exposed-id-ordered client-visible property table (covering
/// either its base or cell slice, both share one id space on the wire), used for
/// property-update dispatch -- not entity creation, see [`build_entity_data_ty`] for
/// that. Same stable-sort/component-folding rule as [`build_method_table`].
fn build_property_table(
    interfaces: &[Interface],
    entity_interface: &Interface,
    static_components: &[Component],
) -> Vec<PropertyDef> {

    // The entity's own and inherited (`implements`, and `<Parent>` -- already flattened in
    // by `script::load`) properties, size-sorted. These occupy the leading slots, which is
    // why the live client's `DataDescription` set matches this list name-for-name and in
    // order (Avatar 28, Vehicle 50). [`collect_properties`] applies the exposure filter and
    // the override rule; `seen` then travels with each component call below.
    let mut seen = HashSet::new();
    let mut collected = Vec::new();
    collect_properties(interfaces, entity_interface, &mut seen, &mut collected);
    collected.sort_by_key(|&(_, length)| length_sort_key(length));

    // Then each static component targeting this entity, appended after that sort -- never
    // merged into it -- with its own properties size-sorted among themselves.
    //
    // Component properties really are part of the entity's client-server id space. That is
    // not inferred from the leaked vanilla source (which is ambiguous here) but read out of
    // the running client: the `clientServerIndex` field (int16 at `DataDescription+108`) of
    // `Avatar`'s two components reports
    //   AvatarInBattleVehicleSwitch: isVehicleConfirmed 28, spawnPoints 29,
    //                                spawnInfoForVehicle 30, vehicleSpawnList 31
    //   StoryModeAvatarComponent:    isPositionValid 32, wrongApplicationPoint 33
    // i.e. slots 28.. continuing straight on from the entity's own 28, one component after
    // the other, each internally ordered fixed-before-variable and ascending by size.
    //
    // Appending rather than re-sorting is what keeps the verified leading slots fixed: a
    // global sort would put the components' two one-byte booleans among the entity's own
    // small properties and push `ammoViews` from 26 to 28, contradicting its confirmed
    // element id `0xC1`. An earlier attempt to fold components into the sort also inflated
    // `Vehicle` to 139 entries with 14 duplicate names -- hence the shared `seen`.
    //
    // KNOWN GAP: within one component, properties that are *all* variable-length tie under
    // this sort and so keep declaration order, but the client orders those three in the
    // reverse (`spawnPoints` 29, `spawnInfoForVehicle` 30, `vehicleSpawnList` 31). The rule
    // behind that is not recoverable from the script model -- every variable type's
    // `streamSize()` is the same `-1` sentinel, so there is nothing left to sort on. The
    // effect is confined to *labels*: all three are `Variable8` and therefore
    // self-delimiting, so every element still frames identically and every following
    // element still decodes. Only a name printed for slots 29/31 may be swapped.
    let entity_name = &*entity_interface.name;
    for component in static_components {
        if !component.of_entities.iter().any(|e| e == entity_name) {
            continue;
        }
        let mut of_component = Vec::new();
        collect_properties(interfaces, &component.interface, &mut seen, &mut of_component);
        of_component.sort_by_key(|&(_, length)| length_sort_key(length));
        collected.extend(of_component);
    }

    collected.into_iter()
        .map(|(property, length)| PropertyDef { name: property.name.clone(), ty: property.ty.clone(), length })
        .collect()

}

/// Build the [`Ty`] (always a [`TyKind::Dict`]) describing an entity's own part of a
/// creation payload: every property reachable through `implements` that `exposed`
/// selects, either the *base-hosted* client-visible ones of `CreateBasePlayer`'s
/// `entity_data` (see [`is_base_property_exposed`]) or the *cell-hosted* ones of
/// `CreateCellPlayer`'s `cell_data` (see [`is_cell_property_exposed`]), in plain
/// declaration order (recursed depth-first, NOT sorted by stream size -- unlike
/// [`build_property_table`], this must match the nested-struct field order the wire's
/// `Codec` actually walks, not the method/property *exposed id* table). Registers (and
/// returns) a fresh anonymous type in `tys`.
fn build_entity_data_ty(
    tys: &mut TySystem,
    interfaces: &[Interface],
    entity_interface: &Interface,
    exposed: fn(&Property) -> bool,
) -> Ty {

    let mut properties = Vec::new();
    collect_data_props(interfaces, entity_interface, exposed, &mut properties);
    tys.register(None, TyKind::Dict(TyDict { properties, allow_none: false }))

}

/// Build the [`Ty`] (always a [`TyKind::Dict`]) describing the static components' part of
/// an entity's creation payload: the properties `exposed` selects of each static
/// component targeting this entity, in component order, each in declaration order.
///
/// Only `CreateCellPlayer` is known to carry this part, after the entity's own properties
/// and a count byte. Checked against a live `Avatar`: its two components'
/// properties at default values account for the 17 bytes after that count, and no
/// other arrangement does.
fn build_components_data_ty(
    tys: &mut TySystem,
    interfaces: &[Interface],
    entity_interface: &Interface,
    static_components: &[Component],
    exposed: fn(&Property) -> bool,
) -> Ty {

    let mut properties = Vec::new();
    let entity_name = &*entity_interface.name;
    for component in static_components {
        if component.of_entities.iter().any(|e| e == entity_name) {
            collect_data_props(interfaces, &component.interface, exposed, &mut properties);
        }
    }

    tys.register(None, TyKind::Dict(TyDict { properties, allow_none: false }))

}

/// The properties `exposed` selects, reachable through `implements`, in declaration order
/// (recursed depth-first), see [`build_entity_data_ty`].
fn collect_data_props(interfaces: &[Interface], interface: &Interface, exposed: fn(&Property) -> bool, out: &mut Vec<TyDictProp>) {
    for implement_name in &interface.implements {
        collect_data_props(interfaces, find_interface(interfaces, implement_name), exposed, out);
    }
    for property in &interface.properties {
        if exposed(property) {
            out.push(TyDictProp { name: property.name.clone(), ty: property.ty.clone(), default: None });
        }
    }
}


#[cfg(test)]
mod tests {

    use crate::script::{Arg, Interface, Method, TyKind, TySystem, VariableHeaderSize};

    use super::*;

    fn method(name: &str, args: Vec<Ty>) -> Method {
        Method {
            name: name.into(),
            exposed_to_all_clients: true,
            exposed_to_own_client: true,
            variable_header_size: VariableHeaderSize::Variable8,
            args: args.into_iter().map(|ty| Arg { ty }).collect(),
        }
    }

    fn interface(name: &str, implements: &[&str], base_methods: Vec<Method>) -> Interface {
        Interface {
            name: name.into(),
            implements: implements.iter().map(|s| s.to_string()).collect(),
            properties: Vec::new(),
            temp_properties: Vec::new(),
            client_methods: Vec::new(),
            base_methods,
            cell_methods: Vec::new(),
        }
    }

    fn property(name: &str, ty: Ty) -> Property {
        Property {
            name: name.into(), ty,
            persistent: false, identifier: false, indexed: false,
            database_len: None, default: None,
            flags: PropertyFlags::AllClients,
        }
    }

    fn component(name: &str, of_entities: &[&str], properties: Vec<Property>) -> Component {
        let mut interface = interface(name, &[], Vec::new());
        interface.properties = properties;
        Component {
            name: name.into(),
            of_entities: of_entities.iter().map(|s| s.to_string()).collect(),
            interface,
            has_client_class: true,
        }
    }

    /// Static-component properties continue the entity's client-server id space, appended
    /// after its own slots, one component at a time, each internally size-sorted.
    ///
    /// The shape here mirrors WoT's `Avatar` exactly, and the expected indices are the ones
    /// read out of the running client (`DataDescription+108`), not derived:
    ///   `isVehicleConfirmed` 28, `spawnInfoForVehicle` 30, `isPositionValid` 32,
    ///   `wrongApplicationPoint` 33.
    /// Slot 32 is the one that matters most -- the live server addresses it (element id
    /// `0xC7`) after every `SelectPlayerEntity`, and it stopped 24 bundles per battle while
    /// the table ended at 28.
    ///
    /// The two all-variable slots 29/31 are deliberately NOT asserted by name: the client
    /// orders those in reverse and the rule is unrecoverable (every variable type shares the
    /// same `-1` stream-size sentinel). They are `Variable8` either way, so framing is
    /// unaffected -- which is what this test does check.
    #[test]
    fn component_properties_continue_the_entity_id_space() {

        let mut tys = TySystem::default();
        let u8_ty = tys.register(None, TyKind::UInt8);
        let vec3_ty = tys.register(None, TyKind::Vector3);
        let str_ty = tys.register(None, TyKind::String);

        let mut entity = interface("Avatar", &[], Vec::new());
        entity.properties = vec![property("own_fixed", u8_ty.clone()), property("own_var", str_ty.clone())];

        let components = vec![
            component("AvatarInBattleVehicleSwitch", &["Avatar"], vec![
                property("vehicleSpawnList", str_ty.clone()),
                property("isVehicleConfirmed", u8_ty.clone()),
                property("spawnInfoForVehicle", str_ty.clone()),
                property("spawnPoints", str_ty.clone()),
            ]),
            component("StoryModeAvatarComponent", &["Avatar"], vec![
                property("wrongApplicationPoint", vec3_ty.clone()),
                property("isPositionValid", u8_ty.clone()),
            ]),
            component("Elsewhere", &["Vehicle"], vec![property("absent", u8_ty.clone())]),
        ];

        let table = build_property_table(&[], &entity, &components);
        let names: Vec<&str> = table.iter().map(|p| &*p.name).collect();

        assert_eq!(names.len(), 8, "2 own + 4 + 2 component properties, nothing from `Elsewhere`");
        assert_eq!(&names[..2], &["own_fixed", "own_var"], "the entity's own slots must not shift");
        // Ground truth from the client, shifted down by 26: this fixture gives the entity 2
        // own slots where the real `Avatar` has 28, so client 28 -> 2, 30 -> 4, 32 -> 6, 33 -> 7.
        assert_eq!(names[2], "isVehicleConfirmed", "client index 28");
        assert_eq!(names[4], "spawnInfoForVehicle", "client index 30");
        assert_eq!(names[6], "isPositionValid", "client index 32");
        assert_eq!(names[7], "wrongApplicationPoint", "client index 33");

        // The two slots whose order we cannot derive must at least stay self-delimiting,
        // since that is what keeps a mislabel from becoming a desync.
        for i in [3, 5] {
            assert!(matches!(table[i].length, ElementLength::Variable8),
                "slot {i} must be self-delimiting, was {:?}", table[i].length);
        }
    }

    /// Static-component methods continue the entity's exposed-id space, appended after its
    /// own slots, one component at a time, each internally size-sorted -- the same rule as
    /// [`build_property_table`] applies to properties.
    ///
    /// Ground truth: `MethodDescription`'s exposed index (int32 at +152) in the running
    /// client reports `LaPingerComponent::pingMeAndThenJustTouchMe` as **43** on `Account`,
    /// which owns 41 client methods (0..40) and takes 41/42 from
    /// `AccountBattleRoyaleTournamentComponent`, so the server addresses it as 43.
    #[test]
    fn component_methods_continue_the_entity_id_space() {

        let mut tys = TySystem::default();
        let u8_ty = tys.register(None, TyKind::UInt8);
        let u64_ty = tys.register(None, TyKind::UInt64);

        let entity = interface("Account", &[], vec![
            method("own_big", vec![u64_ty.clone()]),
            method("own_small", vec![u8_ty.clone()]),
        ]);

        let mut tournament = interface("Tournament", &[], vec![method("setTournamentToken", vec![u8_ty.clone()])]);
        tournament.base_methods = vec![method("setTournamentToken", vec![u8_ty.clone()])];
        let mut pinger = interface("LaPinger", &[], vec![method("pingMeAndThenJustTouchMe", vec![u8_ty.clone()])]);
        pinger.base_methods = vec![method("pingMeAndThenJustTouchMe", vec![u8_ty.clone()])];

        let components = vec![
            Component { name: "Tournament".into(), of_entities: vec!["Account".into()], interface: tournament, has_client_class: true },
            Component { name: "LaPinger".into(), of_entities: vec!["Account".into()], interface: pinger, has_client_class: true },
            Component { name: "Elsewhere".into(), of_entities: vec!["Avatar".into()],
                interface: interface("Elsewhere", &[], vec![method("absent", vec![u8_ty.clone()])]),
                has_client_class: true },
        ];

        let table = build_method_table(&[], &entity, base_methods_of, &components);
        let names: Vec<&str> = table.iter().map(|m| &*m.name).collect();

        assert_eq!(names, ["own_small", "own_big", "setTournamentToken", "pingMeAndThenJustTouchMe"],
            "components append in order after the entity's own size-sorted slots");
        // The entity's own slots must keep their indices, or every existing id shifts.
        assert_eq!(&names[..2], &["own_small", "own_big"]);
        // And `pingMeAndThenJustTouchMe` must be last, as the client reports.
        assert_eq!(*names.last().unwrap(), "pingMeAndThenJustTouchMe");
    }

    /// A component method with no `<Exposed/>` has no exposed id, so it must not take a
    /// slot -- exactly as an unexposed method of the entity's own does not.
    ///
    /// The shape and the expected index here are the live `Avatar` cell table, and 23 is
    /// what the running client actually sends. `AvatarInBattleVehicleSwitch` declares five
    /// cell methods but exposes only three (`confirmVehicleSelection`, `chooseVehicle`,
    /// `switchSetup`); `sendVehicleSpawnList` and `requestSwitch` have no `<Exposed/>`.
    /// Counting those two put `StoryModeAvatarComponent`'s `checkPositionForEquipment` at
    /// 25, so the client's id-23 call (16 bytes: `INT32` + `VECTOR3`) decoded as
    /// `sendVehicleSpawnList` and overran the element -- 24 stopped outbound bundles in one
    /// battle, the only errors left in that run.
    #[test]
    fn unexposed_component_methods_take_no_slot() {

        let mut tys = TySystem::default();
        let u8_ty = tys.register(None, TyKind::UInt8);
        let u32_ty = tys.register(None, TyKind::UInt32);
        let u64_ty = tys.register(None, TyKind::UInt64);

        fn unexposed(mut method: Method) -> Method {
            method.exposed_to_all_clients = false;
            method.exposed_to_own_client = false;
            method
        }

        // Stand-ins for `Avatar`'s own 20 exposed cell methods, reduced to the two that
        // bracket the component slots.
        let entity = interface("Avatar", &[], vec![
            method("vehicle_shoot", vec![]),
            method("reportClientStats", vec![u64_ty.clone()]),
        ]);

        let mut switcher = interface("AvatarInBattleVehicleSwitch", &[], Vec::new());
        switcher.base_methods = vec![
            unexposed(method("sendVehicleSpawnList", vec![u8_ty.clone()])),
            method("confirmVehicleSelection", vec![]),
            method("chooseVehicle", vec![u64_ty.clone()]),
            unexposed(method("requestSwitch", vec![u8_ty.clone()])),
            method("switchSetup", vec![u32_ty.clone()]),
        ];

        let mut story = interface("StoryModeAvatarComponent", &[], Vec::new());
        story.base_methods = vec![method("checkPositionForEquipment", vec![u64_ty.clone(), u64_ty.clone()])];

        let components = vec![
            Component { name: "AvatarInBattleVehicleSwitch".into(), of_entities: vec!["Avatar".into()], interface: switcher, has_client_class: true },
            Component { name: "StoryModeAvatarComponent".into(), of_entities: vec!["Avatar".into()], interface: story, has_client_class: true },
        ];

        let table = build_method_table(&[], &entity, base_methods_of, &components);
        let names: Vec<&str> = table.iter().map(|m| &*m.name).collect();

        assert_eq!(names, [
            "vehicle_shoot", "reportClientStats",
            "confirmVehicleSelection", "switchSetup", "chooseVehicle",
            "checkPositionForEquipment",
        ], "the two unexposed component methods must not appear at all");

        // The point of the whole test: the unexposed pair would push this slot out by two.
        assert_eq!(*names.last().unwrap(), "checkPositionForEquipment");
    }

    /// A method declared by both an entity and one of its interfaces must occupy a single
    /// exposed slot, as in BigWorld's `EntityMethodDescriptions::init` (which pushes to
    /// `exposedMethods_` only when the name->index insert is new). Live case:
    /// `Account::requestToken`, also declared by `AccountAuthTokenProvider`. The extra
    /// entry lengthened the table and shifted every later slot, so `Account`'s exposed id
    /// 13 decoded as a 20-byte method when the client meant a 28-byte one.
    #[test]
    fn redeclared_method_takes_one_exposed_slot() {

        let mut tys = TySystem::default();
        let u8_ty = tys.register(None, TyKind::UInt8);
        let u64_ty = tys.register(None, TyKind::UInt64);

        // `shared` is declared by the interface *and* the entity, with an equal signature.
        let iface = interface("Iface", &[], vec![
            method("shared", vec![u8_ty.clone()]),
        ]);
        let entity = interface("Entity", &["Iface"], vec![
            method("shared", vec![u8_ty.clone()]),
            method("big", vec![u64_ty.clone()]),
        ]);

        let interfaces = vec![iface];
        let table = build_method_table(&interfaces, &entity, base_methods_of, &[]);

        let names: Vec<&str> = table.iter().map(|m| &*m.name).collect();
        assert_eq!(names, ["shared", "big"], "the redeclared method must not be duplicated");

        // The point of deduplicating *before* the sort: a phantom entry would push `big`
        // to index 2, and the wire's index 1 would then decode with the wrong signature.
        assert_eq!(table[1].length, ElementLength::Fixed(8));

    }

    /// A property name redeclared by an entity and its `<Parent>` is an override, not a
    /// second slot. `EntityDescription::parseProperties` reuses the first declaration's
    /// `index` *and* its already-allocated `clientServerFullIndex` on a name hit, so the
    /// table must keep exactly one entry, at the position the first declaration earned.
    ///
    /// Live shape: `ClientSelectableCameraVehicle` redeclares `ClientSelectableObject`'s
    /// `modelName` (same `STRING`/`ALL_CLIENTS`, only `Editable` differs). Parent members
    /// are folded in ahead of the child's by `script::load::apply_parent_chain`, so they
    /// arrive here already flattened into one interface -- which is exactly what would
    /// have produced a duplicate without this dedup.
    #[test]
    fn overridden_property_keeps_one_slot() {

        let mut tys = TySystem::default();
        let string_ty = tys.register(None, TyKind::String);
        let u8_ty = tys.register(None, TyKind::UInt8);

        // As `flatten_parents` leaves it: the parent's `modelName`/`edgeMode` first, then
        // the child's redeclared `modelName`. The redeclaration is deliberately given a
        // *different* size here so the two halves of the rule are separable.
        let mut entity = interface("ClientSelectableCameraVehicle", &[], Vec::new());
        entity.properties = vec![
            property("modelName", string_ty.clone()),
            property("edgeMode", u8_ty.clone()),
            property("modelName", u8_ty.clone()),
        ];

        let table = build_property_table(&[], &entity, &[]);

        let names: Vec<&str> = table.iter().map(|p| &*p.name).collect();
        assert_eq!(names, ["modelName", "edgeMode"],
            "the overridden property must occupy one slot, not two");

        // `properties_[index] = dataDescription` -- the slot is the first declaration's,
        // the contents are the last's. The last one is a `UINT8`, which is why `modelName`
        // now sorts ahead of `edgeMode` instead of trailing it as a variable-length string.
        assert_eq!(table[0].length, ElementLength::Fixed(1),
            "the overriding declaration's type must win");
    }

}
