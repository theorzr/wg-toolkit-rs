//! Attribute each client-visible property wg-toolkit collects for an entity to the
//! interface that declares it, and mark whether the live client actually gives it a
//! clientServerIndex (per a ground-truth list from re-work/frida/dump_property_table.js).
//!
//! Usage: <game-dir> <entity> <ground-truth-file>

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script::{self, Interface, PropertyFlags, Script};

fn exposed(flags: PropertyFlags) -> bool {
    matches!(flags, PropertyFlags::AllClients | PropertyFlags::OwnClient | PropertyFlags::BaseAndClient)
}

fn walk<'m>(script: &'m Script, iface: &'m Interface, out: &mut Vec<(&'m str, &'m str, PropertyFlags)>) {
    for name in &iface.implements {
        let next = script.interfaces.iter().find(|i| &*i.name == name.as_str()).expect("unknown interface");
        walk(script, next, out);
    }
    for p in &iface.properties {
        if exposed(p.flags) {
            out.push((&iface.name, &p.name, p.flags));
        }
    }
}

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> <entity> <truth-file>");
    let entity = env::args().nth(2).expect("need entity");
    let truth_path = env::args().nth(3).expect("need truth file");
    let dir = Path::new(&dir);

    let fs_ = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs_).unwrap();

    let truth: HashSet<String> = fs::read_to_string(&truth_path).unwrap()
        .lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();

    let ent = script.entities.iter().find(|e| &*e.interface.name == entity.as_str()).expect("entity not found");
    let mut collected = Vec::new();
    walk(&script, &ent.interface, &mut collected);

    println!("{entity}: collected {} exposed properties, truth has {}", collected.len(), truth.len());

    // Per declaring interface: how many of its properties the client actually exposes.
    let mut ifaces: Vec<&str> = collected.iter().map(|&(i, _, _)| i).collect();
    ifaces.dedup();
    let mut seen = HashSet::new();
    println!("\n{:<44} {:>5} {:>5} {:>5}", "declaring interface", "ours", "truth", "extra");
    for i in ifaces {
        if !seen.insert(i) { continue; }
        let mine: Vec<_> = collected.iter().filter(|&&(f, _, _)| f == i).collect();
        let in_truth = mine.iter().filter(|&&&(_, n, _)| truth.contains(n)).count();
        println!("{:<44} {:>5} {:>5} {:>5}", i, mine.len(), in_truth, mine.len() - in_truth);
    }

    // Which components does the folding pull in, and are they static or dynamic?
    println!("\n--- components whose <ofEntity> names {entity} ---");
    for (kind, list) in [("static", &script.static_components), ("dynamic", &script.dynamic_components)] {
        for c in list.iter() {
            if !c.of_entities.iter().any(|e| e.as_str() == entity.as_str()) { continue; }
            let exposed_n = c.interface.properties.iter().filter(|p| exposed(p.flags)).count();
            if exposed_n == 0 { continue; }
            let in_truth = c.interface.properties.iter()
                .filter(|p| exposed(p.flags) && truth.contains(&*p.name)).count();
            println!("  {kind:<8} {:<44} exposed={exposed_n:<4} in_truth={in_truth}", c.name);
        }
    }

    println!("\n--- extras, grouped by flags ---");
    let mut by_flag: Vec<(PropertyFlags, usize)> = Vec::new();
    for &(_, n, f) in &collected {
        if !truth.contains(n) {
            match by_flag.iter_mut().find(|(bf, _)| *bf == f) {
                Some((_, c)) => *c += 1,
                None => by_flag.push((f, 1)),
            }
        }
    }
    for (f, c) in by_flag { println!("  {c:>4}  {f:?}"); }
}
