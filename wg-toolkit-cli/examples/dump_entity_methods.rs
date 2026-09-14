use std::env;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> [entity-name]");
    let want = env::args().nth(2).unwrap_or_else(|| "Avatar".to_string());
    let dir = Path::new(&dir);

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();
    let dispatch = ScriptDispatch::new(script);

    println!("== all entities: type_id, name, client_methods.len() ==");
    for (i, e) in dispatch.script.entities.iter().enumerate() {
        let type_id = (i + 1) as u16;
        let n = dispatch.entity_from_id(type_id).map(|d| d.client_methods.len()).unwrap_or(0);
        println!("  type_id={type_id:<3} {:<28} client_methods={n}", e.interface.name);
    }

    if let Some((type_id, ed)) = dispatch.entity_from_name(&want) {
        println!();
        for (label, table) in [
            ("client", &ed.client_methods),
            ("cell", &ed.cell_methods),
            ("base", &ed.base_methods),
        ] {
            println!();
            println!("== {want} (type_id={type_id}) {label}_methods = {} ==", table.len());
            for (i, m) in table.iter().enumerate() {
                let args = m.args.iter().map(|a| format!("{:?}", a.kind())).collect::<Vec<_>>().join(", ");
                println!("  [{i:>3}] {:<44} len={:?} args=[{args}]", m.name, m.length);
            }
        }
    } else {
        println!("entity {want} not found");
    }

    println!();
    println!("== dynamic components having client methods ==");
    let mut total = 0usize;
    for component in &dispatch.script.dynamic_components {
        if let Some((cid, cd)) = dispatch.entity_from_name(&component.name) {
            if !cd.client_methods.is_empty() {
                total += cd.client_methods.len();
                println!("  id={cid:<4} {:<34} client_methods={}", component.name, cd.client_methods.len());
                for (i, m) in cd.client_methods.iter().enumerate() {
                    println!("        [{i}] {}", m.name);
                }
            }
        }
    }
    println!("  -- total component client methods: {total}");
}
