use std::env;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::script::{Ty, TyKind};

fn ty_json(ty: &Ty) -> String {
    match ty.kind() {
        TyKind::Alias(inner) => ty_json(inner),
        TyKind::Dict(dict) => {
            let props: Vec<String> = dict.properties.iter()
                .map(|p| format!("{{\"name\":{:?},\"ty\":{}}}", &*p.name, ty_json(&p.ty)))
                .collect();
            format!("{{\"kind\":\"Dict\",\"properties\":[{}]}}", props.join(","))
        }
        TyKind::Array(seq) | TyKind::Tuple(seq) => {
            format!("{{\"kind\":\"Array\",\"elem\":{}}}", ty_json(&seq.ty))
        }
        other => format!("{{\"kind\":\"Scalar\",\"name\":{:?}}}", format!("{other:?}")),
    }
}

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir>");
    let dir = Path::new(&dir);

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();
    let dispatch = ScriptDispatch::new(script);

    let (type_id, ed) = dispatch.entity_from_name("Avatar").expect("Avatar not found");
    println!("type_id={type_id} name=Avatar properties.len()={}", ed.properties.len());
    let props: Vec<String> = ed.properties.iter()
        .map(|p| format!("{{\"name\":{:?},\"ty\":{}}}", &*p.name, ty_json(&p.ty)))
        .collect();
    println!("JSON:[{}]", props.join(","));

    println!();
    println!("client_methods.len()={}", ed.client_methods.len());
    let checked = wgtk::app::client::element::id::ENTITY_METHOD.to_exposed_id_checked(ed.client_methods.len() as u16, 103);
    println!("to_exposed_id_checked(len={}, elt_id=103) = {checked:?}", ed.client_methods.len());
    for (i, m) in ed.client_methods.iter().enumerate() {
        let args: Vec<String> = m.args.iter().map(|a| format!("{:?}", a.kind())).collect();
        println!("  [{i}] {}({})", m.name, args.join(", "));
    }
}
