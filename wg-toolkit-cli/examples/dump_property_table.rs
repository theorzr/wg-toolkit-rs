//! Dump an entity's client-visible property table in exposed-id order, with the stream
//! length each slot was assigned, to check the table's ordering against the wire.

use std::env;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> <entity>");
    let dir = Path::new(&dir);
    let name = env::args().nth(2).unwrap_or_else(|| "Avatar".to_string());

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();
    let dispatch = ScriptDispatch::new(script);
    let (type_id, ed) = dispatch.entity_from_name(&name).expect("entity not found");

    println!("{name} (type_id={type_id}): {} properties, {} client methods",
        ed.properties.len(), ed.client_methods.len());
    println!("{:>4} {:>5}  {:<34} {:<12} {}", "idx", "id", "name", "length", "type");
    for (i, p) in ed.properties.iter().enumerate() {
        // Element id for a property slot, per `id::ENTITY_PROPERTY`'s 0xA7 base.
        let id = 0xA7u32 + i as u32;
        println!("{i:>4} {:>5}  {:<34} {:<12} {}",
            format!("0x{id:02X}"), p.name, format!("{:?}", p.length), p.ty.name());
    }

    println!("--- client methods ---");
    for (i, m) in ed.client_methods.iter().enumerate() {
        let id = 0x4Eu32 + i as u32;
        println!("{i:>4} {:>5}  {:<40} {:?}", format!("0x{id:02X}"), m.name, m.length);
    }
}
