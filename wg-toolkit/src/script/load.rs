//! Loads a full script [`Model`] from the game's resources.

use std::collections::{HashMap, HashSet};
use std::io;

use tracing::debug;

use crate::res::fs::ResFilesystem;
use crate::pxml::{self, Value};

use super::def::{Component, Entity, Interface, Script};
use super::ty::TySystem;
use super::parse;


/// Load the full model of script resources (aliases, interfaces, entities and
/// extensions) from the given resource filesystem.
pub fn load(fs: &ResFilesystem) -> io::Result<Script> {

    let mut model = Script::default();

    let alias_reader = fs.read("scripts/entity_defs/alias.xml")?;
    let alias_elt = pxml::from_reader(alias_reader).unwrap();
    parse::parse_aliases(&alias_elt, &mut model.tys);
    debug!("read aliases");

    for interface_file in fs.read_dir("scripts/entity_defs/interfaces")? {

        let interface_file = interface_file?;
        let Some((interface_name, "")) = interface_file.name().split_once(".def") else {
            continue;
        };

        let interface_reader = fs.read(interface_file.path())?;
        let interface_elt = pxml::from_reader(interface_reader).unwrap();
        let interface = parse::parse_interface(&interface_elt, &mut model.tys, interface_name.to_string());
        model.interfaces.push(interface);
        debug!("read interface {interface_name}");

    }

    // A component's own `<Implements>` (e.g. many `DynamicComponents` implement
    // `ReplicableComponent`) resolves against this separate, parallel interfaces
    // directory -- confirmed to exist only at this one base-game location, no per-
    // extension equivalent found anywhere in the resource tree.
    if let Ok(component_interface_files) = fs.read_dir("scripts/component_defs/interfaces") {
        for interface_file in component_interface_files {

            let interface_file = interface_file?;
            let Some((interface_name, "")) = interface_file.name().split_once(".def") else {
                continue;
            };

            let interface_reader = fs.read(interface_file.path())?;
            let interface_elt = pxml::from_reader(interface_reader).unwrap();
            let interface = parse::parse_interface(&interface_elt, &mut model.tys, interface_name.to_string());
            model.interfaces.push(interface);
            debug!("read component interface {interface_name}");

        }
    }

    let entities_reader = fs.read("scripts/entities.xml")?;
    let entities_elt = pxml::from_reader(entities_reader).unwrap();
    let entities_elt = entities_elt.get_child("ClientServerEntities").unwrap().as_element().unwrap();
    for (index, (entity_name, _)) in entities_elt.iter_children_all().enumerate() {

        let entity_reader = fs.read(format!("scripts/entity_defs/{entity_name}.def"))?;
        let entity_elt = pxml::from_reader(entity_reader).unwrap();
        let entity = parse::parse_entity(&entity_elt, &mut model.tys, index + 1, entity_name.to_string(), None);
        debug!("read entity {entity_name}");
        model.entities.push(entity);

    }

    // The base game itself also declares its own "static"/"dynamic" components, the same
    // shape as an extension's (`<ofEntity>`-folded def files), just rooted at plain
    // `scripts/component_defs/` instead of `<ext>/scripts/component_defs/` and listed by
    // `scripts/components.xml` instead of an `extension.xml`'s `Components` block (e.g.
    // `AvatarInBattleVehicleSwitch`, folded into `Avatar`) -- previously not parsed at
    // all here, which silently dropped every base-game component's properties/methods
    // from every entity's exposed-id table (confirmed live: real property updates for
    // entity types like `Avatar` referenced exposed ids past the end of this project's
    // table, entirely accounted for by these missing base components once added back).
    // NOT empirically confirmed whether these fold in before or after every extension's
    // own components in the live client's actual exposed-id order -- placed first here
    // as the natural guess (base components predate any extension), matching how
    // `scripts/entities.xml`'s own entities are already ordered before extension ones.
    //
    // `seen_components` dedupes by name across this whole function (base game AND every
    // extension, `StaticComponents` AND `DynamicComponents` alike): the same component
    // name can legitimately appear more than once in the raw data (confirmed live in the
    // base game's own `scripts/components.xml` -- e.g. `SecondaryGunComponent` is listed
    // under both `StaticComponents` and `DynamicComponents`, and a handful of names like
    // `NetworkReplicationPointComponent` even repeat within the same `DynamicComponents`
    // list), but only ever gets ONE real slot: a live Frida dump of the running client's
    // `EntityDescriptionMap` (`re-work/frida/dump_entity_types.js`) showed each dynamic
    // component name exactly once, in first-occurrence order -- a second listing further
    // down was silently not assigned its own id. Skipping every occurrence past the first
    // keeps this project's dynamic-component id sequence aligned with that live table
    // (confirmed: before this fix, `EntityDescriptionMap` index 256 lived on
    // `LSVehicleShotChargerComponent`, but this project's un-deduped list put a duplicate
    // `SecondaryGunComponent` at index ~93, shifting every id after it by one).
    let mut seen_components = HashSet::new();

    {
        let components_reader = fs.read("scripts/components.xml")?;
        let components_elt = pxml::from_reader(components_reader).unwrap();

        for (list_name, components) in [
            ("StaticComponents", &mut model.static_components),
            ("DynamicComponents", &mut model.dynamic_components),
        ] {

            let Some(Value::Element(list_elt)) = components_elt.get_child(list_name) else {
                continue;
            };

            for (component_name, _) in list_elt.iter_children_all() {

                if !seen_components.insert(component_name.clone()) {
                    debug!("skipped duplicate component {component_name} ({list_name})");
                    continue;
                }

                let component_path = format!("scripts/component_defs/{component_name}.def");
                let component_reader = fs.read(&component_path)?;
                let component_elt = pxml::from_reader(component_reader).unwrap();

                let of_entities = parse::parse_of_entity(&component_elt);
                let interface = parse::parse_interface(&component_elt, &mut model.tys, component_name.clone());

                components.push(Component {
                    name: component_name.clone().into(),
                    of_entities,
                    interface,
                });
                debug!("read component {component_name} ({list_name})");

            }

        }
    }

    // WoT extensions (feature packages such as "la_pinger" or "battle_royale") each sit
    // at the root of the resource filesystem and, if active, carry an "extension.xml"
    // declaring a set of "static" components. Each static component is a def file (same
    // shape as an interface) under "<ext>/scripts/component_defs/" that also declares
    // which entity/entities it folds its methods/properties into via "<ofEntity>". This
    // is a WG-specific build step (no trace of it in vanilla BigWorld's entity_description
    // parsing), so its exact folding rule isn't authoritatively documented -- the order
    // used here (extensions in alphabetical directory order, static components in
    // declaration order within each extension) was empirically confirmed against a live
    // capture: la_pinger's "LaPingerComponent.pingMeAndThenJustTouchMe" lands exactly on
    // Account's exposed client method id 0x2B, right after battle_royale's
    // "AccountBattleRoyaleTournamentComponent" 2 client methods (0x29, 0x2A) and Account's
    // own last interface-derived method (0x28) -- see re-work/HANGAR_LOADING.md.
    //
    // An extension can also carry its own "Entities" section (`ClientServerEntities` --
    // has a real def file, same convention as `scripts/entity_defs/`; and
    // `ServerOnlyEntities` -- no def file exists anywhere for these, no client-visible
    // surface, so they're only logged, never parsed/generated). Continuing the main
    // list's numbering here (same ordering rule as component folding) was CONFIRMED
    // live (2026-08-29): a Frida script (`re-work/frida/dump_entity_types.js`) located
    // the running client's actual `BW::EntityDescriptionMap` vector directly in process
    // memory (no address/offset knowledge needed -- found by scanning for a known entity
    // name string, then walking the confirmed 808-byte `EntityDescription` stride outward
    // until the pattern breaks) and read off every entity in true index order. Every one
    // of the 10 currently-known extension entities (`battle_royale`'s `Mine`/`Loot`/
    // `Placement`/`InfluenceZone`/`BattleRoyaleRadio`/`ThunderStrike`, `comp7`'s
    // `Comp7Lighting`, `comp7_core`'s `ApplicationPoint`, `server_side_replay`'s
    // `ReplayAccount`, `story_mode`'s `SPGZone`) landed at exactly the id this generator
    // already assigned it. See `Entity::from_extension` and doc/ENTITY.md.
    //
    // The same live dump also turned up something not modeled by this codebase at all:
    // the id sequence keeps going past the last real entity (`SPGZone`, 0x32) into
    // dozens more slots with clearly component-shaped names (`DogTagComponent`,
    // `HealthComponent`, `VehicleBuff`, ...) all the way past 0x100 -- i.e. the live
    // client's `EntityDescriptionMap` is NOT limited to what `entities.xml`-derived
    // `Entity` values would suggest; static/dynamic extension components apparently get
    // their own real slots in this same id space too, distinct from the
    // already-confirmed exposed-method-id folding. Not yet understood what (if anything)
    // reads those ids off the wire -- worth revisiting for the "dynamic components" open
    // question in doc/ENTITY.md.
    let mut ext_names: Vec<String> = fs.read_dir("")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.stat().is_dir())
        .map(|entry| entry.name().to_string())
        .collect();
    ext_names.sort();

    for ext_name in ext_names {

        let Ok(ext_reader) = fs.read(format!("{ext_name}/extension.xml")) else {
            continue;
        };

        let ext_elt = pxml::from_reader(ext_reader).unwrap();

        if let Some(Value::Element(components_elt)) = ext_elt.get_child("Components") {

            for (list_name, components) in [
                ("StaticComponents", &mut model.static_components),
                ("DynamicComponents", &mut model.dynamic_components),
            ] {

                let Some(Value::Element(list_elt)) = components_elt.get_child(list_name) else {
                    continue;
                };

                for (component_name, _) in list_elt.iter_children_all() {

                    if !seen_components.insert(component_name.clone()) {
                        debug!("skipped duplicate component {ext_name}/{component_name} ({list_name})");
                        continue;
                    }

                    let component_path = format!("{ext_name}/scripts/component_defs/{component_name}.def");
                    let component_reader = fs.read(&component_path)?;
                    let component_elt = pxml::from_reader(component_reader).unwrap();

                    let of_entities = parse::parse_of_entity(&component_elt);
                    let interface = parse::parse_interface(&component_elt, &mut model.tys, component_name.clone());

                    components.push(Component {
                        name: component_name.clone().into(),
                        of_entities,
                        interface,
                    });
                    debug!("read component {ext_name}/{component_name} ({list_name})");

                }

            }

        }

        if let Some(Value::Element(ext_entities_elt)) = ext_elt.get_child("Entities") {

            for server_only_name in parse::parse_names(ext_entities_elt, "ServerOnlyEntities") {
                debug!("found server-only entity {ext_name}/{server_only_name}, no def, not modeled");
            }

            for entity_name in parse::parse_names(ext_entities_elt, "ClientServerEntities") {

                let entity_path = format!("{ext_name}/scripts/entity_defs/{entity_name}.def");
                let entity_reader = fs.read(&entity_path)?;
                let entity_elt = pxml::from_reader(entity_reader).unwrap();
                let id = model.entities.len() + 1;
                let entity = parse::parse_entity(&entity_elt, &mut model.tys, id, entity_name.clone(), Some(ext_name.clone()));
                debug!("read entity {ext_name}/{entity_name}");
                model.entities.push(entity);

            }

        }

    }

    // Only now that *every* entity exists can parents be folded in: a `<Parent>` may name
    // an entity declared later in the list, or one contributed by another extension.
    let Script { tys, entities, .. } = &mut model;
    flatten_parents(entities, |name, ext_name| {
        read_external_parent(fs, tys, ext_name, name, &mut HashSet::new())
    })?;

    debug!("loaded {} types", model.tys.count());

    Ok(model)

}

/// Read an entity def file by name, honouring the extension search path: an extension's
/// own `entity_defs/` shadows the base game's, exactly like BigWorld's `BW_RES_PATH`
/// lookup in `EntityDescription::parse` (`this->getDefsDir() + "/" + name + ".def"`).
/// `Ok(None)` when no def exists under either root.
fn read_entity_def(
    fs: &ResFilesystem,
    ext_name: Option<&str>,
    name: &str,
) -> io::Result<Option<Box<pxml::Element>>> {
    if let Some(ext_name) = ext_name {
        if let Ok(reader) = fs.read(format!("{ext_name}/scripts/entity_defs/{name}.def")) {
            return Ok(Some(pxml::from_reader(reader).unwrap()));
        }
    }
    match fs.read(format!("scripts/entity_defs/{name}.def")) {
        Ok(reader) => Ok(Some(pxml::from_reader(reader).unwrap())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Splice `parent`'s members in *front* of `child`'s, leaving `child`'s own name alone.
fn prepend_interface(child: &mut Interface, mut parent: Interface) {
    fn splice<T>(front: &mut Vec<T>, back: &mut Vec<T>) {
        front.append(back);
        std::mem::swap(front, back);
    }
    splice(&mut parent.implements, &mut child.implements);
    splice(&mut parent.temp_properties, &mut child.temp_properties);
    splice(&mut parent.properties, &mut child.properties);
    splice(&mut parent.client_methods, &mut child.client_methods);
    splice(&mut parent.base_methods, &mut child.base_methods);
    splice(&mut parent.cell_methods, &mut child.cell_methods);
}

/// Fold every entity's `<Parent>` chain into its own interface.
///
/// BigWorld's `EntityDescription::parse` reads `<Parent>` before anything else and
/// recursively parses the parent's whole def into the *same* description, only then
/// running `parseInterface` on the child's own section
/// (`lib/entitydef/entity_description.cpp:194`). So an entity inherits its parent's
/// `Implements`, properties and methods, and every inherited member comes *first* in
/// declaration order -- which is what the size-sorted exposed-id tables break ties on.
///
/// Nothing in `scripts/entities.xml` hints that this is happening. Found live, not
/// inferred: the server addressed `SPGZone` -- type id 0x32, from `story_mode`, whose
/// entire def is `<Parent>AreaOfEffect</Parent>` -- with exposed client method 0 and an
/// `AreaOfEffect::playEffect(STRING, VECTOR3, FLOAT32)` payload, while this loader gave it
/// an empty method table: 12 stopped bundles in one battle. 15 entities in the current
/// client use `<Parent>`, chained up to three deep (`HeroTank` ->
/// `ClientSelectableCameraVehicle` -> `ClientSelectableCameraObject` ->
/// `ClientSelectableObject`).
///
/// Run as a fixpoint over the already-parsed entities rather than by walking each child's
/// chain back to the root: an ancestor is then flattened once and reused by every
/// descendant (`AreaOfEffect` alone has four), instead of being re-read and re-parsed per
/// child, per level.
fn flatten_parents(
    entities: &mut [Entity],
    mut read_external: impl FnMut(&str, Option<&str>) -> io::Result<Option<Interface>>,
) -> io::Result<()> {

    let index_of: HashMap<String, usize> = entities.iter().enumerate()
        .map(|(index, entity)| (entity.interface.name.to_string(), index))
        .collect();

    // Parents read through `read_external`, cached so a name is only ever read once even
    // when several entities inherit it.
    let mut external: HashMap<String, Interface> = HashMap::new();

    // "Flat" = this entity's interface already holds everything it inherits. Parentless
    // entities start out flat; every other becomes flat once its parent is, so repeated
    // sweeps resolve one generation each, whatever order the entities are declared in.
    let mut flat: HashSet<String> = entities.iter()
        .filter(|entity| entity.parent.is_none())
        .map(|entity| entity.interface.name.to_string())
        .collect();

    while flat.len() < entities.len() {

        let mut progressed = false;

        for index in 0..entities.len() {

            let name = entities[index].interface.name.to_string();
            if flat.contains(&name) {
                continue;
            }

            // Not flat implies a parent: the parentless are all flat from the start.
            let parent_name = entities[index].parent.clone().unwrap();

            let parent = if let Some(&parent_index) = index_of.get(&parent_name) {
                if !flat.contains(&parent_name) {
                    continue;  // Wait for a later sweep to flatten the parent first.
                }
                entities[parent_index].interface.clone()
            } else {
                // Not an entity type: read its def, on demand and once. `RepairBase` is
                // the only one in the current client -- see `read_external_parent`.
                if !external.contains_key(&parent_name) {
                    let ext_name = entities[index].from_extension.clone();
                    match read_external(&parent_name, ext_name.as_deref())? {
                        Some(parent) => { external.insert(parent_name.clone(), parent); }
                        None => {
                            debug!("entity {name} declares <Parent>{parent_name}</Parent> \
                                    but no such def exists");
                            flat.insert(name);
                            progressed = true;
                            continue;
                        }
                    }
                }
                external[&parent_name].clone()
            };

            debug!("entity {name} inherits {}", parent.name);
            prepend_interface(&mut entities[index].interface, parent);
            flat.insert(name);
            progressed = true;

        }

        if !progressed {
            // Every entity left is waiting on a parent that is itself waiting: a
            // `<Parent>` cycle. Leave them unflattened rather than spinning forever.
            let stuck: Vec<&str> = entities.iter()
                .map(|entity| &*entity.interface.name)
                .filter(|name| !flat.contains(*name))
                .collect();
            debug!("cyclic <Parent> chain among {stuck:?}, left unresolved");
            break;
        }

    }

    Ok(())

}

/// Read a `<Parent>` that is not itself an entity type, from its def file, with its own
/// `<Parent>` chain already folded in. `Ok(None)` when no def exists under either root.
///
/// A parent need not be an entity type at all: BigWorld resolves `<Parent>` by filename
/// (`getDefsDir() + "/" + name + ".def"`), never through the entity registry, so a def can
/// exist purely to be inherited. `RepairBase` is the only one in the current client -- 51
/// def files against 50 registered types -- and it is `StepRepairPoint`'s parent.
///
/// `seen` guards against a `<Parent>` cycle among such defs, which would otherwise recurse
/// forever; [`flatten_parents`] handles cycles among real entities separately.
fn read_external_parent(
    fs: &ResFilesystem,
    tys: &mut TySystem,
    ext_name: Option<&str>,
    name: &str,
    seen: &mut HashSet<String>,
) -> io::Result<Option<Interface>> {

    if !seen.insert(name.to_string()) {
        debug!("cyclic <Parent> chain at {name}, left unresolved");
        return Ok(None);
    }

    let Some(elt) = read_entity_def(fs, ext_name, name)? else {
        return Ok(None);
    };

    let mut interface = parse::parse_interface(&elt, tys, name.to_string());

    // An external parent's own parent is resolved the same way, from disk: it is outside
    // the entity list too, or -- not seen in any shipped def so far -- an entity type that
    // the fixpoint may not have flattened yet, which reading the def sidesteps.
    if let Some(grandparent) = elt.get_child("Parent").and_then(Value::as_string) {
        if let Some(grandparent) = read_external_parent(fs, tys, ext_name, grandparent, seen)? {
            prepend_interface(&mut interface, grandparent);
        }
    }

    Ok(Some(interface))

}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::script::def::{Method, PropertyFlags, Property, VariableHeaderSize};
    use crate::script::ty::TyKind;

    fn iface(name: &str, implements: &[&str], property: &str, method: &str) -> Interface {
        let mut tys = TySystem::default();
        let ty = tys.register(None, TyKind::UInt8);
        Interface {
            name: name.into(),
            implements: implements.iter().map(|s| s.to_string()).collect(),
            properties: vec![Property {
                name: property.into(), ty,
                persistent: false, identifier: false, indexed: false,
                database_len: None, default: None,
                flags: PropertyFlags::AllClients,
            }],
            temp_properties: vec![format!("temp_{name}")],
            client_methods: vec![Method {
                name: method.into(),
                exposed_to_all_clients: true,
                exposed_to_own_client: true,
                variable_header_size: VariableHeaderSize::Variable8,
                args: Vec::new(),
            }],
            base_methods: Vec::new(),
            cell_methods: Vec::new(),
        }
    }

    fn entity(id: usize, interface: Interface, parent: Option<&str>) -> Entity {
        Entity {
            interface,
            parent: parent.map(str::to_string),
            id,
            from_extension: None,
        }
    }

    /// `EntityDescription::parse` recurses into `<Parent>` *before* parsing the child's own
    /// section, so every inherited member precedes the child's in declaration order --
    /// which is what the size-sorted exposed-id tables break ties on. Grandparents
    /// therefore come before parents, and the child stays last.
    #[test]
    fn parents_are_folded_in_ahead_of_the_child() {

        let mut child = iface("HeroTank", &["ChildIface"], "vehicleGunPitch", "childMethod");

        // Applied the way a chain resolves: nearest ancestor first, each prepend pushing
        // the older generation further to the front.
        prepend_interface(&mut child, iface("ClientSelectableCameraVehicle", &["ParentIface"], "modelName", "parentMethod"));
        prepend_interface(&mut child, iface("ClientSelectableObject", &["GrandIface"], "edgeMode", "grandMethod"));

        assert_eq!(&*child.name, "HeroTank", "folding must not rename the child");
        assert_eq!(child.implements, ["GrandIface", "ParentIface", "ChildIface"]);
        assert_eq!(
            child.properties.iter().map(|p| &*p.name).collect::<Vec<_>>(),
            ["edgeMode", "modelName", "vehicleGunPitch"]);
        assert_eq!(
            child.client_methods.iter().map(|m| &*m.name).collect::<Vec<_>>(),
            ["grandMethod", "parentMethod", "childMethod"]);
        assert_eq!(
            child.temp_properties,
            ["temp_ClientSelectableObject", "temp_ClientSelectableCameraVehicle", "temp_HeroTank"]);

    }

    /// The fixpoint resolves a chain whatever order the entities are declared in -- a
    /// child may well precede its parent in `scripts/entities.xml` (live:
    /// `ClientSelectableCameraObject` is id 14, three slots *before* nothing it needs, but
    /// `HeroTank` at 20 depends on `ClientSelectableCameraVehicle` at 15) -- and reuses a
    /// shared ancestor rather than re-reading it per descendant.
    #[test]
    fn flatten_resolves_chains_in_any_declaration_order() {

        // Deliberately declared child-first, deepest-first.
        let mut entities = vec![
            entity(1, iface("HeroTank", &[], "vehicleGunPitch", "childMethod"), Some("CameraVehicle")),
            entity(2, iface("CameraVehicle", &[], "modelName", "parentMethod"), Some("Selectable")),
            entity(3, iface("Selectable", &[], "edgeMode", "grandMethod"), None),
            entity(4, iface("Sibling", &[], "siblingProp", "siblingMethod"), Some("Selectable")),
        ];

        // No entity here has an out-of-`entities` parent, so the resolver is never called.
        flatten_parents(&mut entities, |name, _| panic!("unexpected external parent {name}")).unwrap();

        assert_eq!(
            entities[0].interface.client_methods.iter().map(|m| &*m.name).collect::<Vec<_>>(),
            ["grandMethod", "parentMethod", "childMethod"],
            "a two-deep chain must resolve outermost-ancestor-first");
        assert_eq!(
            entities[3].interface.client_methods.iter().map(|m| &*m.name).collect::<Vec<_>>(),
            ["grandMethod", "siblingMethod"],
            "a shared ancestor must be folded into every descendant");
        assert_eq!(
            entities[2].interface.client_methods.iter().map(|m| &*m.name).collect::<Vec<_>>(),
            ["grandMethod"],
            "a parentless entity must be left alone");

    }

}
