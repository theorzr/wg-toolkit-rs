//! Report duplicate member names in the generated exposed tables. BigWorld's
//! `EntityMethodDescriptions::init` inserts into a name->index map and only pushes a new
//! entry (and a new exposed slot) when the name is *new*, so a method re-declared by both
//! an entity and one of its interfaces occupies ONE slot, not two. Any duplicate here
//! means our table is longer than the client's and every slot after it is shifted.

use std::env;  use std::path::Path; use std::collections::HashMap;
use wgtk::res::fs::ResFilesystem; use wgtk::script; use wgtk::app::dispatch::ScriptDispatch;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir>");
    let dir = Path::new(&dir);
    let fs = ResFilesystem::new(dir).unwrap();
    let dispatch = ScriptDispatch::new(script::load(&fs).unwrap());

    let mut total = 0;
    for (i, e) in dispatch.script.entities.iter().enumerate() {
        let type_id = (i + 1) as u16;
        let Some(ed) = dispatch.entity_from_id(type_id) else { continue };
        let name = &e.interface.name;
        let tables: [(&str, Vec<String>); 4] = [
            ("client", ed.client_methods.iter().map(|m| m.name.to_string()).collect()),
            ("cell", ed.cell_methods.iter().map(|m| m.name.to_string()).collect()),
            ("base", ed.base_methods.iter().map(|m| m.name.to_string()).collect()),
            ("prop", ed.properties.iter().map(|p| p.name.to_string()).collect()),
        ];
        for (label, names) in tables {
            let mut seen: HashMap<&str, usize> = HashMap::new();
            for n in &names { *seen.entry(n).or_insert(0) += 1; }
            let mut dups: Vec<_> = seen.iter().filter(|(_, c)| **c > 1).collect();
            if dups.is_empty() { continue }
            dups.sort();
            total += dups.len();
            println!("{name} ({label}, len={}): {:?}", names.len(),
                dups.iter().map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>());
        }
    }
    println!("\ntotal duplicated names: {total}");
}
