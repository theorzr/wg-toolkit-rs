use std::env;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::app::client::element::id;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir>");
    let dir = Path::new(&dir);

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();
    let dispatch = ScriptDispatch::new(script);

    let type_id: u16 = 2;
    let ed = dispatch.entity_from_id(type_id).expect("type 2 not found");
    let name = &*dispatch.script.entities[(type_id - 1) as usize].interface.name;
    println!("type_id={type_id} name={name} client_methods.len()={}", ed.client_methods.len());
    for (i, m) in ed.client_methods.iter().enumerate() {
        println!("  [{i}] {}", m.name);
    }

    println!();
    println!("ENTITY_METHOD range: {:?}", id::ENTITY_METHOD);

    // Forward direction: what exposed_id does wire element_id=0x89 map to, given Avatar's
    // real client_methods.len()? (matches exactly what read_in_element's ENTITY_METHOD
    // arm computes live.)
    let checked = id::ENTITY_METHOD.to_exposed_id_checked(ed.client_methods.len() as u16, 0x89);
    println!("to_exposed_id_checked(len={}, elt_id=0x89) = {checked:?}", ed.client_methods.len());
    // If that's None (needs a sub-id byte), show what happens for a few candidate sub-ids.
    for sub in 0u8..=5 {
        let mut it = std::iter::once(sub);
        let exposed = id::ENTITY_METHOD.to_exposed_id(ed.client_methods.len() as u16, 0x89, || it.next().unwrap());
        println!("  to_exposed_id(len={}, elt_id=0x89, sub_id={sub}) = {exposed}", ed.client_methods.len());
    }

    // List dynamic components mentioning "story" or "SM" and their method counts.
    println!();
    println!("Dynamic components:");
    for component in &dispatch.script.dynamic_components {
        if let Some((cid, cd)) = dispatch.entity_from_name(&component.name) {
            if cd.client_methods.len() > 0 || component.name.to_lowercase().contains("sm") || component.name.to_lowercase().contains("stor") {
                println!("  id={cid} name={} client_methods={}", component.name, cd.client_methods.len());
                for (i, m) in cd.client_methods.iter().enumerate() {
                    println!("      [{i}] {}", m.name);
                }
            }
        }
    }
}
