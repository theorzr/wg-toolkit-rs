//! Print the script model's component type ids, to resolve the `componentTypeId` field that
//! `ClientDynComponentNode`'s NRL create record carries (a `u16`, little-endian).
//!
//! Usage: cargo run -p wg-toolkit-cli --features wot --example dump_component_ids -- <game-dir> [id...]

use std::env;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;

fn main() {
    let mut args = env::args().skip(1);
    let dir = args.next().expect("usage: <game-dir> [id...]");
    let wanted: Vec<u32> = args.filter_map(|a| a.parse().ok()).collect();
    let dir = Path::new(&dir);

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();

    println!("entities: {}", script.entities.len());
    println!("dynamic_components: {}", script.dynamic_components.len());
    println!("static_components: {}", script.static_components.len());

    // The entity dispatch id space, confirmed against a live dump: real entities first
    // (1-based), then one id per dynamic component in declaration order.
    let name_of = |id: u32| -> String {
        let i = id as usize;
        if i >= 1 && i <= script.entities.len() {
            format!("entity {}", script.entities[i - 1].interface.name)
        } else if i > script.entities.len() && i - script.entities.len() - 1 < script.dynamic_components.len() {
            format!("dynamic component {}", script.dynamic_components[i - script.entities.len() - 1].name)
        } else {
            "<out of range>".to_string()
        }
    };

    if wanted.is_empty() {
        for (i, c) in script.dynamic_components.iter().enumerate() {
            println!("  dyn[{}] type_id={} {}", i, script.entities.len() + 1 + i, c.name);
        }
        for (i, c) in script.static_components.iter().enumerate() {
            println!("  static[{}] {}", i, c.name);
        }
    } else {
        for id in wanted {
            println!("  id {id} -> {}", name_of(id));
        }
    }
}
