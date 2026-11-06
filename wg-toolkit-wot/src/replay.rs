//! Decoding a base proxy capture offline.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::cell::RefCell;
use std::rc::Rc;

use tracing::{info, info_span, warn};

use wgtk::app::proxy::capture::CaptureReader;
use wgtk::net::bundle::{self, RoundTrip};
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::script::Script;
use wgtk::app::proxy;
use wgtk::pickle;

use crate::{init_tracing, load_script, CliResult, ReplayArgs};
use crate::proxy::{hex, BaseHandler, Shared};


pub fn run(args: ReplayArgs, game_dir: &Path) -> CliResult<()> {

    // A replay gets its own trace file, so it never overwrites the live session's.
    init_tracing("replay-trace.jsonl")?;

    let script = load_script(game_dir)?;
    replay(&args.capture, args.real_login_app, script, args.round_trip)

}

/// Decode a packet capture written by [`BaseProxy`] offline, through the same handler, so it
/// logs exactly what the live proxy would have.
///
/// The real login app address only serves the sanity check on `SwitchBaseApp`
/// addresses; when not given, the real address of the first captured packet stands in
/// for it, being a base app of the same network.
pub fn replay(
    capture_path: &Path,
    real_login_app: Option<SocketAddrV4>,
    script: Script,
    round_trip: bool,
) -> CliResult<()> {

    let open = || CaptureReader::open(capture_path)
        .map_err(|e| format!("Failed to open packet capture at {}: {e}", capture_path.display()));

    let real_login_app = match real_login_app {
        Some(addr) => addr,
        None => match open()?.next() {
            Some(Ok(record)) => match record.real_addr {
                SocketAddr::V4(addr) => addr,
                SocketAddr::V6(addr) => return Err(format!("Unexpected IPv6 real address {addr}, give the real login app address")),
            },
            Some(Err(e)) => return Err(format!("Failed to read packet capture: {e}")),
            None => return Err(format!("The packet capture is empty")),
        },
    };

    let capture = open()?;
    info!(unix_time = capture.start_unix().as_secs(), "Replaying: {}",
        capture_path.display());

    let mut handler = BaseHandler::new(Shared {
        base_app: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
        real_login_app,
        dispatch: ScriptDispatch::new(script),
    });

    let round_trips = round_trip.then(start_round_trip_check);

    let _span = info_span!("base").entered();
    let mut app = proxy::App::replay(capture);
    let truncated = loop {
        match app.poll() {
            proxy::Event::CaptureEnd(end) => break end.truncated,
            proxy::Event::IoError(error) => return Err(format!("Failed to replay packet capture: {}", error.error)),
            event => {
                // When it was recorded, for analyses of the trace over time.
                let t = app.replay_time().unwrap_or_default().as_secs_f64();
                let _record = info_span!("record", t).entered();
                handler.handle_event(&mut app, event)
            }
        }
    };

    if let Some(round_trips) = round_trips {
        bundle::set_round_trip_check(None);
        let _ = std::panic::take_hook();
        print_round_trips(&round_trips.borrow());
    }

    if truncated {
        warn!("Replayed a capture ending on an incomplete record: it was not finished properly");
    }
    handler.log_census();
    info!("Replayed the capture");

    Ok(())

}


/// Round-trip outcomes of one element type, see [`start_round_trip_check`].
#[derive(Default)]
struct RoundTripStats {
    ids: BTreeSet<u8>,
    same: u32,
    /// Different bytes, but only in how pickles are laid out: every pickle unpickles to
    /// the same value, Python 2 `str` and `unicode` kept apart, and every byte around
    /// them matches except the pickles' own lengths.
    pickle_equal: u32,
    different: u32,
    failed: u32,
    not_encodable: u32,
    /// The first difference or failure, to show.
    example: Option<String>,
}

/// Enable the bundle round-trip check, collecting outcomes per element type.
fn start_round_trip_check() -> Rc<RefCell<BTreeMap<&'static str, RoundTripStats>>> {

    // An element without an encoder panics in `write`, which the check catches; only
    // those panics are silenced, every other one still reports.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.payload().downcast_ref::<String>().map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if !message.contains("read-only") {
            previous(info);
        }
    }));

    let stats = Rc::new(RefCell::new(BTreeMap::<&'static str, RoundTripStats>::new()));
    let collected = Rc::clone(&stats);
    bundle::set_round_trip_check(Some(Box::new(move |id, type_name, outcome| {
        let mut stats = collected.borrow_mut();
        let entry = stats.entry(type_name).or_default();
        entry.ids.insert(id);
        match outcome {
            RoundTrip::Same => entry.same += 1,
            RoundTrip::NotEncodable => entry.not_encodable += 1,
            RoundTrip::Failed(e) => {
                entry.failed += 1;
                entry.example.get_or_insert_with(|| format!("id 0x{id:02X}: {e}"));
            }
            RoundTrip::Different { reason: "body", original, encoded } if pickle_equal(&original, &encoded) => {
                entry.pickle_equal += 1
            }
            RoundTrip::Different { reason, original, encoded } => {
                entry.different += 1;
                entry.example.get_or_insert_with(|| format!("id 0x{id:02X}, {reason}: read {} encoded {}\n    {}",
                    hex(&original[..original.len().min(48)]), hex(&encoded[..encoded.len().min(48)]),
                    describe_difference(&original, &encoded)));
            }
        }
    })));

    stats

}

/// A body split into its pickles (protocol 2, starting `80 02`), decoded without
/// running anything, and the bytes around them. A pickle's own packed length, when it
/// directly precedes it, is left out of those bytes: it follows from the pickle.
fn split_pickles(data: &[u8]) -> (Vec<&[u8]>, Vec<pickle::Value>) {
    let mut around = Vec::new();
    let mut values = Vec::new();
    let (mut at, mut search) = (0, 0);
    while let Some(start) = data[search..].windows(2).position(|w| w == [0x80, 0x02]).map(|i| search + i) {
        let Ok((value, len)) = pickle::from_bytes_prefix(&data[start..]) else {
            search = start + 1;
            continue;
        };
        let mut before = &data[at..start];
        if before.last() == Some(&(len as u8)) && len < 0xFF {
            before = &before[..before.len() - 1];
        } else if before.len() >= 4 && before[before.len() - 4] == 0xFF
            && before[before.len() - 3..] == (len as u32).to_le_bytes()[..3] {
            before = &before[..before.len() - 4];
        }
        around.push(before);
        values.push(value);
        at = start + len;
        search = at;
    }
    around.push(&data[at..]);
    (around, values)
}

/// Where two bodies differ, in terms of their pickles, for a round-trip report.
fn describe_difference(original: &[u8], encoded: &[u8]) -> String {
    let ((around_a, values_a), (around_b, values_b)) = (split_pickles(original), split_pickles(encoded));
    if values_a.len() != values_b.len() {
        return format!("{} pickles read, {} encoded", values_a.len(), values_b.len());
    }
    if let Some(i) = around_a.iter().zip(&around_b).position(|(a, b)| a != b) {
        let (a, b) = (around_a[i], around_b[i]);
        let at = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        return format!("bytes around the pickles, part {i}, from offset {at}: read {} encoded {}",
            hex(&a[at..a.len().min(at + 24)]), hex(&b[at..b.len().min(at + 24)]));
    }
    let Some(i) = values_a.iter().zip(&values_b).position(|(a, b)| a != b) else {
        return "no difference found".to_string();
    };
    match (&values_a[i], &values_b[i]) {
        (pickle::Value::Dict(a), pickle::Value::Dict(b)) => {
            let differing: Vec<String> = a.keys().chain(b.keys()).collect::<BTreeSet<_>>().into_iter()
                .filter(|key| a.get(key) != b.get(key))
                .take(4)
                .map(|key| format!("{key:?}: read {} encoded {}",
                    preview(a.get(key)), preview(b.get(key))))
                .collect();
            format!("pickle {i}, keys {}", differing.join("; "))
        }
        (a, b) => format!("pickle {i}: read {} encoded {}", preview(Some(a)), preview(Some(b))),
    }
}

fn preview(value: Option<&pickle::Value>) -> String {
    let text = format!("{value:?}");
    if text.len() > 160 { format!("{}…", &text[..text.floor_char_boundary(160)]) } else { text }
}

/// Whether two bodies only differ in how their pickles are laid out, see
/// [`RoundTripStats::pickle_equal`].
fn pickle_equal(original: &[u8], encoded: &[u8]) -> bool {
    let (a, b) = (split_pickles(original), split_pickles(encoded));
    !a.1.is_empty() && a == b
}

fn print_round_trips(stats: &BTreeMap<&'static str, RoundTripStats>) {
    // Drop module paths, including inside generics (`Reply<…u8::MAX>>` stays readable):
    // each `::` replaces the identifier before it with the part after it.
    let short = |name: &str| {
        let mut out = String::new();
        for part in name.split("::") {
            let ident = out.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').count();
            out.truncate(out.len() - ident);
            out.push_str(part);
        }
        out
    };
    println!("{:<48} {:>8} {:>8} {:>8} {:>8} {:>8}  ids", "element", "same", "pickle=", "differ", "failed", "no enc");
    let mut rows: Vec<_> = stats.iter().collect();
    rows.sort_by_key(|(_, s)| (s.same > 0 && s.different == 0 && s.failed == 0 && s.not_encodable == 0, std::cmp::Reverse(s.different + s.failed + s.not_encodable)));
    for (name, s) in rows {
        let ids: Vec<String> = s.ids.iter().map(|id| format!("{id:02X}")).collect();
        let ids = if ids.len() > 6 { format!("{} ids", ids.len()) } else { ids.join(" ") };
        println!("{:<48} {:>8} {:>8} {:>8} {:>8} {:>8}  {ids}", short(name), s.same, s.pickle_equal, s.different, s.failed, s.not_encodable);
        if let Some(example) = &s.example {
            println!("    {example}");
        }
    }
}
