//! Loads a full script [`Model`] from the game's resources.

use std::io;

use tracing::debug;

use crate::res::fs::ResFilesystem;
use crate::pxml::{self, Value};

use super::def::{Component, Script};
use super::parse;


/// Load the full model of script resources (aliases, interfaces, entities and
/// extensions) from the given resource filesystem.
pub fn load(fs: &ResFilesystem, version: String) -> io::Result<Script> {

    let mut model = Script {
        version,
        ..Default::default()
    };

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

    debug!("loaded {} types", model.tys.count());

    Ok(model)

}
