//! Dump the dispatch tables computed from a game's script model.
//!
//! Without a name, lists every entity type id: the entities first, then one id per
//! dynamic component, which is also how the `componentTypeId` of an NRL create record
//! resolves. With an entity or component name (or its type id), dumps its client-visible
//! property table and its client, cell and base method tables, in exposed-id order, with
//! the element id each slot travels with and its length style.
//!
//! Usage: cargo run -p wg-toolkit-wot --example dump_entity -- <game-dir> [name|type-id]

use std::path::Path;
use std::env;

use wgtk::app::dispatch::{ScriptDispatch, MethodDef};
use wgtk::net::element::ElementIdRange;
use wgtk::app::client::element as client;
use wgtk::app::base::element as base;
use wgtk::res::fs::ResFilesystem;
use wgtk::script;


fn main() {

    let mut args = env::args().skip(1);
    let dir = args.next().expect("usage: <game-dir> [name|type-id]");
    let want = args.next();

    let fs = ResFilesystem::new(Path::new(&dir).join("res")).expect("failed to open resources");
    let dispatch = ScriptDispatch::new(script::load(&fs).expect("failed to load script model"));
    let script = &dispatch.script;

    let Some(want) = want else {

        println!("== entities ==");
        for (i, entity) in script.entities.iter().enumerate() {
            println!("{:>5}  {}", i + 1, entity.interface.name);
        }

        println!("== dynamic components ==");
        for (i, component) in script.dynamic_components.iter().enumerate() {
            println!("{:>5}  {}", script.entities.len() + 1 + i, component.name);
        }

        println!("== static components (no type id) ==");
        for component in &script.static_components {
            println!("       {}", component.name);
        }

        return;

    };

    let found = match want.parse::<u16>() {
        Ok(type_id) => dispatch.entity_from_id(type_id).map(|ed| (type_id, ed)),
        Err(_) => dispatch.entity_from_name(&want),
    };
    let Some((type_id, ed)) = found else {
        eprintln!("no entity or dynamic component {want}");
        std::process::exit(1);
    };

    println!("== {want} (type id {type_id}): {} properties ==", ed.properties.len());
    for (i, prop) in ed.properties.iter().enumerate() {
        println!("{i:>4} {:<10} {:<40} {:<14} {}",
            element_id(client::id::ENTITY_PROPERTY, ed.properties.len(), i),
            prop.name, format!("{:?}", prop.length), prop.ty.name());
    }

    print_methods("client", client::id::ENTITY_METHOD, &ed.client_methods);
    print_methods("cell", base::id::CELL_ENTITY_METHOD, &ed.cell_methods);
    print_methods("base", base::id::BASE_ENTITY_METHOD, &ed.base_methods);

}

fn print_methods(label: &str, range: ElementIdRange, methods: &[MethodDef]) {
    println!("== {label} methods: {} ==", methods.len());
    for (i, method) in methods.iter().enumerate() {
        let args = method.args.iter().map(|a| format!("{:?}", a.kind())).collect::<Vec<_>>().join(", ");
        println!("{i:>4} {:<10} {:<40} {:<14} ({args})",
            element_id(range, methods.len(), i), method.name, format!("{:?}", method.length));
    }
}

/// The element id of an exposed slot, with its sub-id when the table overflows the range.
fn element_id(range: ElementIdRange, count: usize, exposed_id: usize) -> String {
    match range.from_exposed_id(count as u16, exposed_id as u16) {
        (id, None) => format!("0x{id:02X}"),
        (id, Some(sub_id)) => format!("0x{id:02X}/{sub_id}"),
    }
}
