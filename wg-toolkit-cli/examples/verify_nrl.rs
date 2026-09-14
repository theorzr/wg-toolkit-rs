//! Decode captured `Nrl*` element payloads through the real codecs, check the decode
//! re-encodes byte-for-byte, and report how much of each element could be framed.
//!
//! Input: a file of `<Kind> <hex>` lines, where `<Kind>` is `NrlCreateNode`,
//! `NrlUnlinkTree`, `NrlUpdateNode`, `NrlMsgToClient`, `NrlMsgToCell` or `NrlData`.
//! Raw hex comes from the proxy's TRACE lines (`re-work/test/proxy-stdout-*.log`).
//!
//! Usage: cargo run -p wg-toolkit-cli --features wot --example verify_nrl -- <samples-file>

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Cursor;

use wgtk::app::client::element as client_el;
use wgtk::net::codec::SimpleCodec;

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// Decode, re-encode and report `(decoded, round-trips, framed bytes, messages, text)`.
/// `probe` yields `(undecoded tail length, number of messages framed)`.
macro_rules! check {
    ($ty:ty, $data:expr, $probe:expr) => {{
        let mut cursor = Cursor::new(&$data[..]);
        match <$ty as SimpleCodec>::read(&mut cursor) {
            Ok(value) => {
                let mut back = Vec::new();
                SimpleCodec::write(&value, &mut back).unwrap();
                let (rest, messages) = $probe(&value);
                (true, back == $data, $data.len() - rest, messages, format!("{value:?}"))
            }
            Err(e) => (false, false, 0, 0, format!("{e}")),
        }
    }};
}

fn main() {
    let path = env::args().nth(1).expect("usage: <samples-file>");
    let text = fs::read_to_string(&path).unwrap();

    // Per kind: total, decoded, byte-exact round-trips, fully framed (nothing left),
    // and at-least-one-message (so "decoded" cannot be true just by yielding nothing).
    let mut stats: BTreeMap<&str, [usize; 5]> = BTreeMap::new();
    let mut examples: BTreeMap<String, (String, String)> = BTreeMap::new();

    for line in text.lines() {
        let Some((kind, hex)) = line.split_once(' ') else { continue };
        let Some(data) = decode_hex(hex.trim()) else { continue };

        let (ok, exact, framed, messages, render) = match kind {
            "NrlCreateNode" => check!(client_el::NrlCreateNode, data,
                |v: &client_el::NrlCreateNode| (v.0.rest.len(), v.0.messages.len())),
            "NrlUnlinkTree" => check!(client_el::NrlUnlinkTree, data,
                |v: &client_el::NrlUnlinkTree| (v.0.rest.len(), v.0.messages.len())),
            // Client -> cell uses the same stream format, every message typed inline.
            "NrlMsgToClient" | "NrlMsgToCell" => check!(client_el::NrlMsgToClient, data,
                |v: &client_el::NrlMsgToClient| (v.0.rest.len(), v.0.messages.len())),
            "NrlUpdateNode" => check!(client_el::NrlUpdateNode, data,
                |_: &client_el::NrlUpdateNode| (0, 1)),
            "NrlData" => check!(client_el::NrlData, data, |_: &client_el::NrlData| (0, 1)),
            _ => continue,
        };

        let kind: &'static str = Box::leak(kind.to_string().into_boxed_str());
        let e = stats.entry(kind).or_default();
        e[0] += 1;
        e[1] += ok as usize;
        e[2] += exact as usize;
        e[3] += (ok && framed == data.len()) as usize;
        e[4] += (ok && messages > 0) as usize;

        // Keep one example per (kind, outcome) bucket, to eyeball what is being decoded.
        let bucket = format!("{kind} {} framed={}/{}", if ok { "ok" } else { "FAIL" }, framed, data.len());
        examples.entry(bucket).or_insert((hex.trim().to_string(), render));
    }

    println!("{:<26} {:>7} {:>8} {:>8} {:>9} {:>8}",
        "kind", "total", "decoded", "exact", "with msgs", "complete");
    for (kind, [total, ok, exact, complete, any]) in &stats {
        println!("{kind:<26} {total:>7} {ok:>8} {exact:>8} {any:>9} {complete:>8}");
    }

    println!("\n--- one example per outcome ---");
    for (bucket, (hex, render)) in &examples {
        println!("\n[{bucket}]\n  {hex}\n  {render}");
    }
}
