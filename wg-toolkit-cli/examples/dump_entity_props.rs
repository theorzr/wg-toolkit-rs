use std::env;  use std::path::Path;
use wgtk::res::fs::ResFilesystem; use wgtk::script; use wgtk::app::dispatch::ScriptDispatch;
fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> <entity>...");
    let dir = Path::new(&dir);
    let fs = ResFilesystem::new(dir).unwrap();
    let dispatch = ScriptDispatch::new(script::load(&fs).unwrap());
    let args: Vec<String> = env::args().skip(2).collect();
    let names: Vec<String> = if args.iter().any(|a| a == "ALL") {
        dispatch.script.entities.iter().map(|e| e.interface.name.to_string()).collect()
    } else { args };
    for want in names {
        match dispatch.entity_from_name(&want) {
            Some((tid, ed)) => {
                println!("== {want} (type_id={tid}) properties = {} ==", ed.properties.len());
                for (i, p) in ed.properties.iter().enumerate() {
                    println!("  [{i:>3}] {:<40} len={:?}", p.name, p.length);
                }
            }
            None => println!("{want}: not found"),
        }
        println!();
    }
}
