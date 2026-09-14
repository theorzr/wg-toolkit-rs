use std::env;  use std::path::Path;
use wgtk::res::fs::ResFilesystem; use wgtk::script;
use wgtk::script::{Interface, PropertyFlags};
use wgtk::app::dispatch::ScriptDispatch;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> <entity>");
    let want = env::args().nth(2).unwrap_or_else(|| "Avatar".into());
    let dir = Path::new(&dir);
    let fs_ = ResFilesystem::new(dir).unwrap();
    let model = script::load(&fs_).unwrap();

    let entity = model.entities.iter().find(|e| &*e.interface.name == want).expect("entity not found");
    let iface = &entity.interface;

    fn walk<'m>(ifaces: &'m [Interface], i: &'m Interface, out: &mut Vec<(String, PropertyFlags, String)>) {
        for imp in &i.implements {
            if let Some(x) = ifaces.iter().find(|f| &*f.name == &**imp) { walk(ifaces, x, out); }
        }
        for p in &i.properties {
            out.push((p.name.to_string(), p.flags, i.name.to_string()));
        }
    }
    let mut decl = Vec::new();
    walk(&model.interfaces, iface, &mut decl);

    println!("== {want}: {} declared properties (declaration order) ==", decl.len());
    for (n, (name, flags, owner)) in decl.iter().enumerate() {
        let exposed = matches!(flags, PropertyFlags::AllClients | PropertyFlags::OwnClient | PropertyFlags::BaseAndClient);
        println!("  decl[{n:>3}] {:<44} {:<16} exposed={:<5} from {}", name, format!("{flags:?}"), exposed, owner);
    }

    println!();
    println!("== flag histogram ==");
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, f, _) in &decl { *counts.entry(format!("{f:?}")).or_default() += 1; }
    for (f, c) in counts { println!("  {f:<18} {c}"); }

    let dispatch = ScriptDispatch::new(model);
    if let Some((tid, ed)) = dispatch.entity_from_name(&want) {
        println!();
        println!("== resulting client-server table (type_id={tid}), {} entries ==", ed.properties.len());
        for (i, p) in ed.properties.iter().enumerate() {
            println!("  [{i:>3}] {:<44} len={:?}", p.name, p.length);
        }
    }
}
