//! Decode a raw `ENTITY_PROPERTY` payload (as dumped by `replay_bundle`) against a
//! chosen entity's client-server property table, and report how many bytes were left
//! over. A correct (entity, index) pairing consumes the payload exactly; a wrong one
//! either fails or leaves a tail. This is what settles "which entity was selected"
//! without needing a live session.
//!
//! usage: <game-dir> <entity> <index> <hex> [<hex>...]

use std::env;  use std::path::Path;
use wgtk::res::fs::ResFilesystem; use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::net::codec::Codec;
use wgtk::script::Value;

fn main() {
    let mut args = env::args().skip(1);
    let dir = args.next().expect("usage: <game-dir> <entity> <index> <hex>...");
    let entity = args.next().expect("need entity name");
    let index: usize = args.next().expect("need property index").parse().unwrap();
    let dir = Path::new(&dir);
    let fs = ResFilesystem::new(dir).unwrap();
    let dispatch = ScriptDispatch::new(script::load(&fs).unwrap());
    let (_tid, ed) = dispatch.entity_from_name(&entity).expect("entity not found");
    let prop = ed.properties.get(index).expect("index out of range");
    println!("{entity}[{index}] = {} (len={:?})\n", prop.name, prop.length);

    for hex in args {
        let raw: Vec<u8> = (0..hex.len()).step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let mut cursor = &raw[..];
        match <Value as Codec<_>>::read(&mut cursor, &prop.ty) {
            Ok(v) => {
                let left = cursor.len();
                let mark = if left == 0 { "EXACT" } else { "LEFTOVER" };
                println!("{hex}: {mark} ({left} bytes left) -> {v:?}");
            }
            Err(e) => println!("{hex}: FAILED: {e}"),
        }
    }
}
