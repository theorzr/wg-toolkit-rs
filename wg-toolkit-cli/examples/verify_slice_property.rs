//! Brute-force a captured `SliceEntityProperty`/`NestedEntityProperty` payload against
//! *every* entity and dynamic-component property table in the loaded script model, under
//! both the nested-single and slice leaf interpretations, to find which (if any) table
//! the bytes actually decode against.
//!
//! Pass a game dir, and optionally a file of hex payloads (one per line) to replace the
//! built-in samples. Output marks each decode as certain (`SLICE `), append-inferred
//! (`SLICE~`, see `decode_compressed_path`) or ambiguous (`SLICE?`).
//!
//! Note the per-table summary only lists tables scoring above zero, so count the
//! ambiguous column too when judging whether a table is the right one.

use std::env;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::app::client::element::{SliceEntityProperty, NestedEntityProperty};
use wgtk::net::element::Element;

fn decode_hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir>");
    let dir = Path::new(&dir);

    let fs = ResFilesystem::new(dir).unwrap();
    let script = script::load(&fs).unwrap();
    let dispatch = ScriptDispatch::new(script);

    // Every table, labelled, in wire-type-id order: real entities first, then dynamic
    // components (see `ScriptDispatch`'s doc comment).
    let mut names: Vec<String> = dispatch.script.entities.iter()
        .map(|e| e.interface.name.to_string())
        .collect();
    names.extend(dispatch.script.dynamic_components.iter().map(|c| c.name.to_string()));

    println!("{} tables to try ({} entities + {} dynamic components)",
        names.len(), dispatch.script.entities.len(), dispatch.script.dynamic_components.len());

    // Fresh, healthy-stream capture (2026-09-10, zero `Unknown element` errors in the
    // session): every one of these was preceded by `Select entity (not tracked)`, so the
    // proxy had no dispatch table and could only log the raw payload.
    let samples: &[(&str, &str)] = &[
        // Previously ambiguous on real traffic.
        ("amb1 a0000100", "a0000100"),
        ("amb2 80003893", "80003893"),
        ("amb3 a0004600", "a0004600"),
        ("amb4 c0001c23", "c0001c23"),
        ("amb5 af009f00", "af009f00"),
        // Controls: these already decoded unambiguously.
        ("ok1  ac003400", "ac003400"),
        ("ok2  a820007100", "a820007100"),
    ];

    // An optional file of hex payloads (one per line, `#` comments allowed) replaces the
    // built-in list, so a capture's worth of real samples can be measured in one go.
    let from_file: Vec<(String, Vec<u8>)> = match env::args().nth(2) {
        Some(path) => fs::read_to_string(&path).unwrap().lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .enumerate()
            .map(|(i, l)| (format!("s{i:03} {l}"), decode_hex(l)))
            .collect(),
        None => Vec::new(),
    };

    let decoded: Vec<(&str, Vec<u8>)> = if from_file.is_empty() {
        samples.iter().map(|(label, hex)| (*label, decode_hex(hex))).collect()
    } else {
        from_file.iter().map(|(l, d)| (l.as_str(), d.clone())).collect()
    };

    // All of these are the same message id against (mostly) the same two entities, so the
    // right table should decode *every* sample -- a far stronger filter than any single
    // 4-byte payload, which is short enough to match plenty of tables by chance.
    println!("\n=== per-table results (only tables decoding >=1 sample are listed) ===");

    let mut best: Vec<(usize, String, usize, usize)> = Vec::new();

    for (i, name) in names.iter().enumerate() {
        let type_id = (i + 1) as u16;
        let Some(ed) = dispatch.entity_from_id(type_id) else { continue };

        let mut slice_ok = 0;
        let mut nested_ok = 0;
        let mut ambiguous = 0;
        let mut lines = Vec::new();

        for (label, data) in &decoded {
            let mut cursor = Cursor::new(&data[..]);
            match SliceEntityProperty::read(&mut cursor, &ed.properties, data.len(), SliceEntityProperty::ID) {
                Ok(p) => {
                    slice_ok += 1;
                    lines.push(format!("      SLICE{} {label}: path={:?} [{}..{}] = {:?}",
                        if p.seq_len_inferred { "~" } else { " " }, p.path, p.start, p.end, p.values));
                }
                // An "ambiguous" failure is a near miss worth seeing -- the path resolved
                // and only the array-length guess was undecidable -- unlike "no consistent
                // decode", which just means this table is wrong for these bytes.
                Err(e) => {
                    let e = e.to_string();
                    if e.contains("ambiguous") {
                        ambiguous += 1;
                        lines.push(format!("      SLICE? {label}: {e}"));
                    }
                }
            }
            let mut cursor = Cursor::new(&data[..]);
            if let Ok(p) = NestedEntityProperty::read(&mut cursor, &ed.properties, data.len(), NestedEntityProperty::ID) {
                nested_ok += 1;
                lines.push(format!("      NESTED {label}: path={:?} = {:?}", p.path, p.value));
            }
        }

        if slice_ok + nested_ok + ambiguous > 0 {
            println!("  [{type_id}] {name} (props={}): slice {slice_ok}/{}, nested {nested_ok}/{}, ambiguous {ambiguous}",
                ed.properties.len(), decoded.len(), decoded.len());
            for line in &lines {
                println!("{line}");
            }
            best.push((slice_ok.max(nested_ok), name.clone(), slice_ok, nested_ok));
        }
    }

    best.sort_by(|a, b| b.0.cmp(&a.0));
    println!("\n=== best candidates (tables decoding the most samples) ===");
    for (score, name, s, n) in best.iter().take(10) {
        println!("  {score}/{}  {name} (slice {s}, nested {n})", decoded.len());
    }
    if best.is_empty() {
        println!("  none");
    }
}
