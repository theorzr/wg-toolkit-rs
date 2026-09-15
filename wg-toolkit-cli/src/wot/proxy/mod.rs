//! Proxy login and base app used for debugging exchanged messages.

use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{io, thread};

use tracing::{error, info, warn, info_span, trace, trace_span};

use rsa::{RsaPrivateKey, RsaPublicKey};
use flate2::read::ZlibDecoder;
use blowfish::Blowfish;

use wgtk::net::element::{DebugElementUndefined, DebugElementVariable16, Element, SimpleElement};
use wgtk::net::bundle::{Bundle, NextElementReader, ElementReader};
use wgtk::net::codec::WgSocketAddrV4;

use wgtk::app::{proxy, login_proxy, base, client};
use wgtk::app::dispatch::{ScriptDispatch, MethodCall, MethodDef};
use wgtk::app::math::{PackedXyz, PackedXz, calculate_reference_position};
use glam::Vec3;
use wgtk::app::base::element::CellMethodTables;
use wgtk::net::packet::Packet;
use wgtk::script::{Script, Value};

use wgtk::util::io::serde_pickle_de_options;

use crate::CliResult;


/// Sentinel `id_alias` value meaning "no alias assigned" -- confirmed against the leaked
/// BigWorld 14.4.1 SDK (`server/cellapp/entity_cache.hpp`: `const IDAlias NO_ID_ALIAS =
/// 0xff;`). The server only allocates a real alias to entities with volatile (position)
/// data, and only while its per-client alias pool (`Witness::freeAliases_`, an 8-bit
/// free-list, `server/cellapp/witness.cpp`) still has a free slot; everything else keeps
/// this sentinel.
const NO_ID_ALIAS: u8 = 0xff;

/// Renders an `id_alias` for logging, spelling out the `NO_ID_ALIAS` sentinel. It is a
/// real slot in the client's table (see `id_aliases`), so it is worth being able to tell
/// "entity has no volatile data" from an ordinary alias when reading a trace.
fn fmt_id_alias(id_alias: u8) -> String {
    if id_alias == NO_ID_ALIAS {
        format!("{id_alias} (NO_ID_ALIAS)")
    } else {
        id_alias.to_string()
    }
}

/// Hex-encode bytes for embedding in a log line (used for raw payloads that failed to
/// decode into anything more structured, so they still end up in the protocol trace).
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Read an element whose exact wire framing (length style) has been confirmed live
/// against the real client (see `re-work/NOTES.md`) even though `wg-toolkit` doesn't
/// yet understand what it means, so it can be read (and the bundle safely continued)
/// as its already-defined placeholder type. Logged at `trace` level with structured
/// `id`/`request_id` fields -- like every other event here, the same call feeds both
/// the console (filtered by `RUST_LOG`) and the always-on `proxy-trace.jsonl` file
/// (see `cmd_wot`), so there's no separate dump path to keep in sync.
/// Append every inbound packet to `$WGTK_DUMP_PACKETS`, when set. Opened once, lazily;
/// any I/O error disables the dump rather than disturbing the proxy.
fn dump_packets(raw: &[String]) {
    use std::io::Write as _;
    use std::sync::{Mutex, OnceLock};
    static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let sink = SINK.get_or_init(|| {
        let path = std::env::var("WGTK_DUMP_PACKETS").ok()?;
        match std::fs::File::create(&path) {
            Ok(f) => Some(Mutex::new(f)),
            Err(e) => { warn!("failed to open WGTK_DUMP_PACKETS={path}: {e}"); None }
        }
    });
    if let Some(sink) = sink {
        if let Ok(mut f) = sink.lock() {
            for p in raw { let _ = writeln!(f, "{p}"); }
        }
    }
}

macro_rules! trace_dbg {
    ($elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        trace!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
            "{}: {:?}", stringify!($ty), e.element);
        Ok(true)
    }};
}

/// Like [`trace_dbg`], but for the `AVATAR_UPDATE_NO_ALIAS_*` (AVUPMSG) family and
/// `AvatarUpdateNoAliasDetailed`, which carry a full `entity_id`.
///
/// **Every avatar update re-targets the stream.** Confirmed against the leaked BigWorld
/// 14.4.1 SDK (`lib/connection/server_connection.cpp`): `IMPLEMENT_AVUPMSG` opens with
/// `selectedEntityID_ = id;` *unconditionally*, before it even checks
/// `isControlledLocally()`. So every `ENTITY_PROPERTY`/`ENTITY_METHOD` element after a
/// position update belongs to *that* entity, not to whatever `SelectEntity`/
/// `SelectPlayerEntity` last named.
///
/// Not tracking this is what made ~20k of one battle's property updates decode as the
/// player `Avatar`'s `denunciationsLeft` (its index 13) when they were really each
/// vehicle's `gunAnglesPacked` (`Vehicle`'s index 13) -- and likewise made `Vehicle`'s
/// `avatarID` (index 16, an `OBJECT_ID`, hence the entity-id-looking values) read as
/// `Avatar`'s `arenaTypeID`.
macro_rules! trace_dbg_entity {
    ($self:expr, $elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        let entity_id = e.element.entity_id;
        $self.selected_entity_id = Some(entity_id);
        // Resolve the packed offset into world coordinates (see `PositionTracker`).
        // `None` means the element carries no position, or the reference/scale isn't
        // known yet -- in which case the packed bytes below are still the honest record.
        let world = $self.position.resolve(Some(entity_id), &e.element.position);
        // INFO, like the aliased family, so the values land in `proxy-trace.jsonl`
        // (which is INFO-and-up; TRACE there is transport bookkeeping, not protocol
        // content).
        info!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
            entity_id, world = ?world.map(|w| w.to_string()),
            "<- {}: {:?}", stringify!($ty), e.element);
        Ok(true)
    }};
}

/// Like [`trace_dbg`], but for the `AVATAR_UPDATE_ALIAS_*` (AVUPMSG) family: also
/// resolves `element.id_alias` back to a full entity id via `self.id_aliases` (see that
/// field's doc comment) and logs it alongside, so per-entity correlation (e.g. of the
/// still-unexplained trailing `unk` bytes) doesn't require cross-referencing `EnterAoi`
/// by hand.
macro_rules! trace_dbg_alias {
    ($self:expr, $elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        let entity_id = $self.id_aliases.get(&e.element.id_alias).copied();
        let world = $self.position.resolve(entity_id, &e.element.position);
        // Re-target exactly like the `NoAlias` family does (see [`trace_dbg_entity`]):
        // `IMPLEMENT_AVUPMSG` sets `selectedEntityID_` unconditionally, and the aliased
        // forms are the *common* case in a battle, so skipping this pins the stream to
        // whatever `SelectPlayerEntity` last named (the player `Avatar`) and decodes
        // every other vehicle's properties against the wrong table. An alias we can't
        // resolve clears the selection rather than leaving a stale one, so the element
        // stops on "no dispatch table" instead of silently mis-decoding.
        $self.selected_entity_id = entity_id;
        // Logged at INFO so it reaches `proxy-trace.jsonl`, which is INFO-and-up by
        // design (TRACE there is transport bookkeeping). This replaced a bounded raw-hex
        // sampler that existed only while the AVUPMSG field splits were unknown; now that
        // all 24 layouts are settled, the parsed values are what's actually wanted.
        //
        // The packed bytes are still printed alongside `world`: `world` is only as good
        // as the tracked reference position, and keeping the raw field means a suspect
        // coordinate can always be re-derived by hand from the same trace line.
        info!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id, entity_id = ?entity_id,
            world = ?world.map(|w| w.to_string()),
            "<- {}: {:?} (alias {} -> {:?})", stringify!($ty), e.element,
            fmt_id_alias(e.element.id_alias), entity_id);
        Ok(true)
    }};
}


pub fn run(
    login_app_addr: SocketAddrV4,
    real_login_app_addr: SocketAddrV4,
    base_app_addr: SocketAddrV4,
    encryption_key: Option<Arc<RsaPrivateKey>>,
    real_encryption_key: Option<Arc<RsaPublicKey>>,
    script: Script,
) -> CliResult<()> {

    let mut login_app = login_proxy::App::new(login_app_addr.into(), real_login_app_addr.into(), real_encryption_key)
        .map_err(|e| format!("Failed to bind login app: {e}"))?;

    if let Some(encryption_key) = encryption_key {
        login_app.set_encryption(encryption_key);
    }

    let mut base_app = proxy::App::new(base_app_addr.into())
        .map_err(|e| format!("Failed to bind base app: {e}"))?;

    let shared = Arc::new(Shared {
        base_app_addr,
        login_app_addr,
        real_login_app_addr,
        dispatch: ScriptDispatch::new(script),
        pending_clients: Mutex::new(HashMap::new()),
        pending_switches: Mutex::new(HashMap::new()),
    });

    let login_handler = LoginHandler {
        shared: Arc::clone(&shared),
    };

    let base_handler = BaseHandler {
        shared,
        next_tick: None,
        entities: HashMap::new(),
        created_entity_types: HashMap::new(),
        selected_entity_id: None,
        player_entity_id: None,
        id_aliases: HashMap::new(),
        warned_aliases: HashSet::new(),
        avup_samples: HashMap::new(),
        partial_resources: HashMap::new(),
        session_keys: HashMap::new(),
        position: PositionTracker::new(),
    };

    thread::scope(move |scope| {

        scope.spawn(move || {
            let _span = info_span!("login").entered();
            if let Err(e) = login_app.run(login_handler) {
                error!("Unexpected hard error: ({}) {e}", e.kind());
            }
        });

        scope.spawn(move || {
            let _span = info_span!("base").entered();
            if let Err(e) = base_app.run(base_handler) {
                error!("Unexpected hard error: ({}) {e}", e.kind());
            }
        });

    });

    Ok(())

}


#[derive(Debug)]
struct LoginHandler {
    shared: Arc<Shared>,
}

/// A volatile position resolved into world space.
///
/// `y` is `None` for the `OnGround` family, which genuinely does not send it -- the
/// client takes that entity's height from the terrain, which this proxy has no access
/// to. Kept optional rather than defaulted to `0.0` so a trace never shows an altitude
/// that was never on the wire.
#[derive(Debug, Clone, Copy)]
struct WorldPos {
    x: f32,
    y: Option<f32>,
    z: f32,
}

impl std::fmt::Display for WorldPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.y {
            Some(y) => write!(f, "({:.2}, {:.2}, {:.2})", self.x, y, self.z),
            None => write!(f, "({:.2}, ~ground, {:.2})", self.x, self.z),
        }
    }
}

/// The packed-position field of an `AVATAR_UPDATE_*` element, resolved against an origin.
///
/// Implemented for the three shapes the family uses -- [`PackedXyz`] (`FullPos`),
/// [`PackedXz`] (`OnGround`) and `()` (`NoPos`) -- so the two AVUPMSG macros can resolve
/// a position generically without matching on the element type.
trait VolatilePosition {
    fn resolve(&self, origin: Vec3, xz_scale: f32) -> Option<WorldPos>;
}

impl VolatilePosition for PackedXyz {
    fn resolve(&self, origin: Vec3, xz_scale: f32) -> Option<WorldPos> {
        let o = self.unpack(xz_scale);
        // `AVATAR_UPDATE_GET_POS_FullPos` adds all three components.
        Some(WorldPos { x: o.x + origin.x, y: Some(o.y + origin.y), z: o.z + origin.z })
    }
}

impl VolatilePosition for PackedXz {
    fn resolve(&self, origin: Vec3, xz_scale: f32) -> Option<WorldPos> {
        let (x, z) = self.unpack(xz_scale);
        // No altitude is reported, because the wire carries none and the client does not
        // recover one either: in the `OnGround` path it sets `y` to BigWorld's
        // `NO_POSITION` sentinel (-13000.0), which WoT's build then happens to bias by
        // the reference's own `y` -- the client binary literally computes
        // `y = referencePosition_.y - 13000.0` off a vehicle, and the bare `-13000.0` on
        // one. Either way it is a sentinel meaning "ask the terrain", never a height, so
        // surfacing it as a coordinate would be pure noise.
        Some(WorldPos { x: x + origin.x, y: None, z: z + origin.z })
    }
}

impl VolatilePosition for () {
    fn resolve(&self, _origin: Vec3, _xz_scale: f32) -> Option<WorldPos> {
        None
    }
}

/// Everything needed to turn the *relative* positions in the `AVATAR_UPDATE_*` stream
/// into world coordinates.
///
/// WoT runs BigWorld's relative mode (`VOLATILE_POSITIONS_ARE_ABSOLUTE == 0`, confirmed
/// in the client binary), so a volatile update carries only an offset. The client
/// reconstructs the absolute position as
/// `pos = unpack(packed, packed_xz_scale) + origin`, where
/// `origin = if on a vehicle { ZERO } else { reference_position }`
/// (`AVATAR_UPDATE_GET_POS_ORIGIN`). This mirrors the connection state that feeds that
/// expression -- see each field.
#[derive(Debug)]
struct PositionTracker {
    /// The client's own recent outgoing positions, indexed by the `ref_num` it stamped on
    /// each one. Mirrors `ServerConnection::sentPositions_[256]`, which the client fills
    /// in `server_connection.cpp:757` and the server later names back at it via
    /// [`RelativePositionReference`]. Boxed because 256 * 12 bytes is more than belongs
    /// inline in a handler that is moved around.
    ///
    /// Entries start as `None`: a reference naming a slot the proxy never saw (it joined
    /// mid-session, or the update was lost) must be detectable, not silently read as the
    /// origin.
    sent_positions: Box<[Option<Vec3>; 256]>,
    /// Mirrors `ServerConnection::referencePosition_` (one Vec3 at `this+0x13a8` in the
    /// client). Deliberately a single global value, **not** per entity: the binary has
    /// exactly three stores to that field and none of them are keyed by entity.
    ///
    /// `None` until a writer has run, so positions decoded before the first reference are
    /// reported as unresolved instead of silently offset from the origin.
    reference_position: Option<Vec3>,
    /// Which vehicle each entity is riding, from [`SetVehicle`]; absent or `0` means
    /// none. Only used for the `vehicle_id == 0` test that picks the origin.
    vehicles: HashMap<u32, u32>,
    /// The space's `packed_xz_scale`, announced by `CreateCellPlayer`. Without it the
    /// packed x/z cannot be scaled, so positions stay unresolved rather than being
    /// reported at an arbitrary scale.
    packed_xz_scale: Option<f32>,
}

impl PositionTracker {

    fn new() -> Self {
        Self {
            sent_positions: Box::new([None; 256]),
            reference_position: None,
            vehicles: HashMap::new(),
            packed_xz_scale: None,
        }
    }

    /// Full reset, mirroring the client's connection reset -- the same `memset` that
    /// clears `idAlias_`, so this belongs wherever that is handled (`ResetEntities`).
    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Record one of the client's own outgoing positions (upstream
    /// `AvatarUpdateImplicit`/`AvatarUpdateExplicit`), so a later
    /// [`RelativePositionReference`] naming this `ref_num` can resolve.
    fn record_sent(&mut self, ref_num: u8, position: Vec3) {
        self.sent_positions[ref_num as usize] = Some(position);
    }

    /// `ServerConnection::relativePosition`: assigned verbatim, **no rounding**.
    fn set_reference_exact(&mut self, position: Vec3) {
        self.reference_position = Some(position);
    }

    /// `ServerConnection::relativePositionReference`: the reference is a *rounded* copy of
    /// a position the client itself sent earlier. Returns the resolved position, or `None`
    /// if that slot was never observed (in which case the reference is left untouched
    /// rather than being set to something invented).
    fn set_reference_from_sent(&mut self, sequence_number: u8) -> Option<Vec3> {
        let sent = self.sent_positions[sequence_number as usize]?;
        self.reference_position = Some(calculate_reference_position(sent));
        Some(reference)
    }

    /// `ServerConnection::detailedPositionReceived`: an accurate, uncompressed position
    /// re-bases the reference, but **only** for the player's own entity and only while it
    /// is not on a vehicle. Both conditions are the SDK's, and both matter: without the
    /// entity test every other vehicle's detailed position would clobber the reference.
    ///
    /// Returns whether it applied, so callers can log the distinction.
    fn detailed_position_received(&mut self, entity_id: u32, player_entity_id: Option<u32>, position: Vec3) -> bool {
        if player_entity_id != Some(entity_id) || self.vehicle_of(entity_id) != 0 {
            return false;
        }
        self.reference_position = Some(calculate_reference_position(position));
        true
    }

    fn set_vehicle(&mut self, passenger_id: u32, vehicle_id: u32) {
        if vehicle_id == 0 {
            self.vehicles.remove(&passenger_id);
        } else {
            self.vehicles.insert(passenger_id, vehicle_id);
        }
    }

    /// The vehicle `entity_id` is riding, or `0` for none (`NULL_ENTITY_ID`).
    fn vehicle_of(&self, entity_id: u32) -> u32 {
        self.vehicles.get(&entity_id).copied().unwrap_or(0)
    }

    /// Resolve a volatile position for `entity_id` into world coordinates, or `None` if
    /// the element carries no position or the state needed isn't known yet.
    /// `entity_id` of `None` (an `id_alias` that didn't resolve) yields `None`: which
    /// origin applies depends on whether *that* entity is on a vehicle, so an unknown
    /// target means an unknowable position, not one to guess at.
    fn resolve<P: VolatilePosition>(&self, entity_id: Option<u32>, packed: &P) -> Option<WorldPos> {
        let xz_scale = self.packed_xz_scale?;
        // An entity riding a vehicle reports positions relative to that vehicle, so the
        // origin is zero and no reference is needed -- that case resolves even before the
        // first reference position arrives.
        let origin = if self.vehicle_of(entity_id?) != 0 {
            Vec3::ZERO
        } else {
            self.reference_position?
        };
        packed.resolve(origin, xz_scale)
    }

}

#[derive(Debug)]
struct BaseHandler {
    shared: Arc<Shared>,
    next_tick: Option<u8>,
    /// Every entity created so far, keyed by its id, holding its wire type id and its
    /// actual decoded data -- this is what lets a base/client method call be decoded
    /// generically by just looking up the entity id, resolving its dispatch tables from
    /// `Shared::dispatch` by type id, and dispatching against those, without this proxy
    /// needing to know any concrete entity type statically.
    entities: HashMap<u32, (u16, Value)>,
    /// Entity types learned from `CreateEntity`/`CreateEntityDetailed`. Kept separate from
    /// `entities` because only decoders whose element framing is independent of the table
    /// may use a merely-plausible type: currently `Nested`/`SliceEntityProperty`, both
    /// always `Variable16`.
    ///
    /// `ENTITY_PROPERTY`/`ENTITY_METHOD` used to be excluded, since they derive the
    /// element's wire length from the table (see `EntityMethod::read_length`, keyed off
    /// `client_methods.len()`): a wrong type there mis-frames that element and everything
    /// after it in the bundle, which can corrupt a downstream `SwitchBaseApp` into a
    /// garbage address that this proxy would then adopt, killing the connection.
    ///
    /// That exclusion has been lifted, because the premise behind "merely-plausible" was
    /// wrong: `CreateEntity`/`CreateEntityDetailed` carry the entity's *declared*
    /// `entity_type_id`, which is exactly as authoritative as the one `CreateBasePlayer`
    /// puts into `entities` -- it was only ever kept apart out of caution, not evidence.
    /// Withholding it was costing real traffic: once AVUPMSG re-targeting landed (see
    /// `trace_dbg_entity`), 2728 of one battle's property updates resolved to vehicles
    /// known only from `CreateEntity` and were dropped here, at element ids that decode
    /// against `Vehicle` as `gunAnglesPacked` (1966), `engineMode` (619), `isStrafing`
    /// (143) and `avatarID` -- i.e. ordinary per-tick vehicle telemetry, all correctly
    /// sized for that table.
    ///
    /// The `SwitchBaseApp` failure mode this once guarded against keeps its own, more
    /// direct defences (the `0.x.x.x` / port-0 / `/16`-match checks in that arm), which
    /// are what actually stopped it recurring.
    created_entity_types: HashMap<u32, u16>,
    selected_entity_id: Option<u32>,
    player_entity_id: Option<u32>,
    /// Resolves an AVUPMSG `id_alias` byte back to the full entity id it currently
    /// refers to. This mirrors the *client*'s `ServerConnection::idAlias_[256]` rather
    /// than the server's `Witness::freeAliases_` pool, because it has to answer the same
    /// question the client answers when it decodes the same bytes.
    ///
    /// That distinction decides the lifetime rules, and they are narrower than they look:
    /// the only writer is `enterAoI` (shared by `EnterAoi`/`EnterAoiOnVehicle`), which
    /// records the mapping *including* the `NO_ID_ALIAS`/255 slot, and the only bulk
    /// clear is the full connection reset (`ResetEntities`). In particular `LeaveAoi`
    /// does **not** remove anything -- the server reuses an alias by sending a fresh
    /// `enterAoI` that overwrites the slot. See those three arms for the details.
    id_aliases: HashMap<u8, u32>,
    /// Id aliases already reported as unresolvable, so the warning below fires once per
    /// distinct alias instead of once per position update (thousands per battle).
    warned_aliases: HashSet<(u8, u8)>,
    /// How many raw AVUPMSG samples have been logged per element id (bounded).
    avup_samples: HashMap<u8, usize>,
    partial_resources: HashMap<u16, PartialResource>,
    /// The session key last sent by each client, as observed on the initial handshake
    /// with the base app. When a SwitchBaseApp is intercepted and rewritten to keep the
    /// client pointed at this proxy (see below), the client itself has no reason to redo
    /// this handshake since its own view of the base app's address never changed, so the
    /// proxy replays it on the client's behalf toward the new real address instead.
    session_keys: HashMap<SocketAddr, u32>,
    /// Relative-position state: see [`PositionTracker`].
    position: PositionTracker,
}

#[derive(Debug)]
struct Shared {
    base_app_addr: SocketAddrV4,
    #[allow(unused)]
    login_app_addr: SocketAddrV4,
    /// The real login app this proxy forwards to. Used to sanity-check a `SwitchBaseApp`
    /// address against the operator's own network -- see that element's handling.
    real_login_app_addr: SocketAddrV4,
    /// The script model's dynamic dispatch tables, resolved once at startup -- see
    /// `wgtk::app::script::ScriptDispatch`.
    dispatch: ScriptDispatch,
    pending_clients: Mutex<HashMap<SocketAddr, PendingClient>>,
    /// Same idea as `pending_clients`, but keyed by IP only: after a SwitchBaseApp, the
    /// client tears down its whole connection and reconnects with a fresh, unpredictable
    /// local port (matching the real BigWorld client's own disconnect + reconnect
    /// behavior on this message, confirmed live), so we can't know the exact address to
    /// expect in advance the way a normal post-login connection lets us.
    pending_switches: Mutex<HashMap<IpAddr, PendingClient>>,
}

#[derive(Debug)]
struct PendingClient {
    base_app_addr: SocketAddrV4,
    blowfish: Arc<Blowfish>,
}

/// Describe a partial resource being download, a header must have been sent.
#[derive(Debug)]
struct PartialResource {
    /// The byte description sent in the resource header.
    description: Vec<u8>,
    /// Fragments received so far, keyed by their sequence number so they can be
    /// reassembled in order regardless of the order they actually arrived in.
    fragments: BTreeMap<u8, Vec<u8>>,
    /// The sequence number of the fragment marked `last`, once it has been seen.
    /// The download is complete once every sequence number in `0..=last_sequence_num`
    /// has a matching entry in `fragments`.
    last_sequence_num: Option<u8>,
}

impl login_proxy::Handler for LoginHandler {

    type Error = io::Error;

    fn receive_ping(&mut self,
        addr: SocketAddr,
        latency: Duration,
    ) -> Result<(), Self::Error> {
        info!(%addr, "Ping-Pong: {:?}", latency);
        Ok(())
    }

    fn receive_login_success(&mut self,
        addr: SocketAddr,
        blowfish: Arc<Blowfish>,
        base_app_addr: WgSocketAddrV4,
        _login_key: u32,
        _server_message: String,
    ) -> Result<WgSocketAddrV4, Self::Error> {

        info!(%addr, %base_app_addr, "Login success");
        self.shared.pending_clients.lock().unwrap().insert(addr, PendingClient {
            base_app_addr: base_app_addr.addr,
            blowfish,
        });

        // Return the proxy base app address instead of the expected one!
        Ok(self.shared.base_app_addr.into())

    }

    fn receive_login_error(&mut self,
        addr: SocketAddr,
        error: login_proxy::element::LoginError,
        data: String,
    ) -> Result<(), Self::Error> {
        info!(%addr, "Login error: {:?} ({data:?})", error);
        Ok(())
    }



}

impl proxy::Handler for BaseHandler {

    type Error = io::Error;

    fn accept_peer(&mut self,
        addr: SocketAddr,
    ) -> Result<Option<proxy::PeerConfig>, Self::Error> {

        if let Some(pending_client) = self.shared.pending_clients.lock().unwrap().remove(&addr) {
            info!(%addr, "Forwarding new peer to {}", pending_client.base_app_addr);
            return Ok(Some(proxy::PeerConfig {
                real_addr: SocketAddr::V4(pending_client.base_app_addr),
                blowfish: Some(pending_client.blowfish),
            }));
        }

        // The client tears down and reconnects from a fresh, unpredictable local port
        // after a SwitchBaseApp (confirmed live), so we can't know the exact address to
        // expect in advance -- only match by IP, keyed at switch time (see the
        // SwitchBaseApp element handling below).
        if let Some(pending_client) = self.shared.pending_switches.lock().unwrap().remove(&addr.ip()) {
            info!(%addr, "Forwarding reconnected peer (post-switch) to {}", pending_client.base_app_addr);
            return Ok(Some(proxy::PeerConfig {
                real_addr: SocketAddr::V4(pending_client.base_app_addr),
                blowfish: Some(pending_client.blowfish),
            }));
        }

        warn!(%addr, "Rejected an unknown peer");
        Ok(None)

    }

    fn receive_invalid_packet_encryption(&mut self,
        peer: proxy::Peer,
        _packet: Packet,
        direction: proxy::PacketDirection,
    ) -> Result<(), Self::Error> {
        error!(addr = %peer.addr(), "Failed to decrypt a packet: ({direction:?})");
        Ok(())
    }

    fn receive_bundle(&mut self,
        peer: proxy::Peer,
        bundle: Bundle,
        direction: proxy::PacketDirection,
        _channel: Option<proxy::PacketChannel>,
    ) -> Result<(), Self::Error> {

        let addr = peer.addr();

        match direction {
            proxy::PacketDirection::Out => {
                if let Err(e) = self.read_out_bundle(peer, bundle) {
                    error!(%addr, "-> Error while reading bundle: {e}");
                }
            }
            proxy::PacketDirection::In => {
                // On success this clone is wasted work, but bundles are tiny (a handful
                // of packets at most) and this is the only way to get the *whole*
                // bundle's raw bytes for forensic logging: by the time `read_in_bundle`
                // returns an error, the `ElementReader` has already consumed part of it,
                // and nothing else exposes the pre-parse bytes.
                let raw: Vec<String> = bundle.iter().map(|p| hex(p.slice())).collect();
                // Optional full-corpus capture: the error path below only ever preserves
                // the handful of bundles that failed, which is far too sparse to replay
                // offline -- the selection/alias state machine needs every packet in order
                // to be meaningful. Set `WGTK_DUMP_PACKETS=<file>` to append every inbound
                // packet (one hex line each) for `examples/replay_selection.rs`.
                dump_packets(&raw);
                if let Err(e) = self.read_in_bundle(peer, bundle) {
                    error!(%addr, raw_packets = ?raw, "<- Error while reading bundle: {e}");
                }
            }
        }

        Ok(())

    }

}

impl BaseHandler {

/// Why `selected_entity_id` failed to resolve a dispatch table, for the "no dispatch
    /// table" warnings: distinguishes "nothing is selected" (an AVUPMSG whose id alias we
    /// never saw an `EnterAoi` for clears the selection) from "selected, but we were never
    /// told this entity's type" (no `CreateEntity`/`CreateBasePlayer` for it).
    fn dispatch_miss_reason(&self) -> String {
        match self.selected_entity_id {
            None => "no entity selected (unresolved id alias?)".to_string(),
            Some(entity_id) => {
                let known_alias = self.id_aliases.values().any(|&v| v == entity_id);
                match (self.entities.get(&entity_id), self.created_entity_types.get(&entity_id)) {
                    (None, None) => format!(
                        "entity {entity_id} has no known type (in_aoi_aliases={known_alias}, \
                         entities={}, created={})", self.entities.len(), self.created_entity_types.len()),
                    (e, c) => format!("entity {entity_id} type known ({e:?}/{c:?}) but no dispatch table"),
                }
            }
        }
    }

    /// What `SelectPlayerEntity` (id: `id::SELECT_PLAYER_ENTITY`) targets: always the
    /// entity from the most recent `CreateBasePlayer`, never the `Vehicle` that
    /// `CreateCellPlayer` announces alongside it.
    ///
    /// This used to prefer the `CreateCellPlayer` vehicle over `player_entity_id`, on the
    /// strength of a v2.3.1.3 capture where battle property updates appeared to decode
    /// only against `Vehicle`'s table. That was a false positive, disproven on a v2.4.0.0
    /// battle capture:
    ///
    /// * Entering a battle fires a *second* `CreateBasePlayer`, with `entity_type_id=2`
    ///   (`Avatar`) -- so the old reasoning's "the base/`Account` entity" premise simply
    ///   doesn't hold in battle: by then the player entity *is* the `Avatar`.
    /// * `Vehicle[19] debuff` is `Fixed(4)`; `Avatar[19] ownVehicleAuxPhysicsData` is
    ///   `Fixed(8)`. Reading the latter as the former consumes half the element, and the
    ///   4 trailing bytes then get eaten as the next element's id -- which is why the
    ///   leftovers kept landing on further *plausible-looking* `Vehicle` properties
    ///   (`isStrafing`, `engineMode`, `gunAnglesPacked`) and made the wrong table look
    ///   right. Live proof: all 3908 `debuff` reads in one battle were followed by a
    ///   decode error or a bundle stop, never once by a clean continuation.
    /// * Replaying that battle's 304 failing packets settles it: as `Vehicle` all 304
    ///   fail; as `Avatar` all 304 decode to completion, recovering 73% more elements,
    ///   with `ownVehicleAuxPhysicsData` exactly where the bogus `debuff` used to be.
    ///
    /// The `Vehicle` is still registered in `entities` by `CreateCellPlayer` so that an
    /// explicit `SelectEntity` naming it resolves a real dispatch table -- it just no
    /// longer hijacks `SelectPlayerEntity`.
    fn select_player_entity_id(&self) -> Option<u32> {
        self.player_entity_id
    }

    fn read_out_bundle(&mut self, mut peer: proxy::Peer, bundle: Bundle) -> io::Result<()> {

        // Every event traced underneath this span (down to the trace file too, see
        // `cmd_wot`) is tagged with `out` for free, alongside the `base` span already
        // entered around this whole handler's thread in `run()`.
        let _span = trace_span!("out").entered();

        let mut reader = bundle.element_reader();
        while let Some(elt) = reader.next() {
            match elt {
                NextElementReader::Element(elt) => {
                    if !self.read_out_element(&mut peer, elt)? {
                        break;
                    }
                }
                NextElementReader::Reply(reply) => {
                    let request_id = reply.request_id();
                    let _elt = reply.read_simple::<()>()?;
                    warn!(addr = %peer.addr(), request_id, "-> Reply (unknown payload length, bundle reading stopped here)");
                    break;
                }
            }
        }

        Ok(())

    }

    fn read_out_element(&mut self, peer: &mut proxy::Peer, elt: ElementReader) -> io::Result<bool> {

        use base::element::*;

        let addr = peer.addr();

        match elt.id() {
            // LoginKey::ID => {}  // This should not be encrypted so we just ignore it!
            SessionKey::ID => {
                let elt = elt.read_simple::<SessionKey>()?;
                info!(%addr, id = SessionKey::ID, request_id = ?elt.request_id,
                    "-> Session key: 0x{:08X}", elt.element.session_key);
                self.session_keys.insert(addr, elt.element.session_key);
            }
            EnableEntities::ID => {
                let ee = elt.read_simple::<EnableEntities>()?;
                info!(%addr, id = EnableEntities::ID, request_id = ?ee.request_id, "-> Enable entities");
            }
            DisconnectClient::ID => {
                let dc = elt.read_simple::<DisconnectClient>()?;
                info!(%addr, id = DisconnectClient::ID, request_id = ?dc.request_id,
                    "-> Disconnect: 0x{:02X}", dc.element.reason);
            }
            // The following ids are known (id, length style and length param all confirmed
            // live against v2.3.1.3, see `re-work/NOTES.md`) but not yet given a proper
            // structured codec -- read them through their placeholder type so the bundle
            // can safely keep being read, and trace their raw content.
            PingDatacenter::ID => return trace_dbg!(elt, addr, PingDatacenter),
            AvatarUpdateImplicit::ID => {
                // The client stamps each of its own outgoing positions with a sequence
                // number and remembers it (`sentPositions_[seq] = globalPos`); the server
                // later re-names one of those as the base for relative updates, by that
                // number alone (see `RelativePositionReference`). So this is the only
                // place the proxy can learn what such a reference will mean.
                let e = elt.read_simple::<AvatarUpdateImplicit>()?;
                self.position.record_sent(e.element.ref_num, e.element.position);
                trace!(%addr, id = AvatarUpdateImplicit::ID, request_id = ?e.request_id,
                    "-> {}: {:?}", stringify!(AvatarUpdateImplicit), e.element);
                return Ok(true);
            }
            AvatarUpdateExplicit::ID => {
                // Same as `AvatarUpdateImplicit`, but names the vehicle explicitly. The
                // SDK records the position either way (the `sentPositions_` write happens
                // before the implicit/explicit branch), so this does too.
                let e = elt.read_simple::<AvatarUpdateExplicit>()?;
                self.position.record_sent(e.element.ref_num, e.element.position);
                trace!(%addr, id = AvatarUpdateExplicit::ID, request_id = ?e.request_id,
                    "-> {}: {:?}", stringify!(AvatarUpdateExplicit), e.element);
                return Ok(true);
            }
            AckPhysicsCorrection::ID => return trace_dbg!(elt, addr, AckPhysicsCorrection),
            RequestEntityUpdate::ID => return trace_dbg!(elt, addr, RequestEntityUpdate),
            NrlMsgToCell::ID => return trace_dbg!(elt, addr, NrlMsgToCell),
            AvatarUpdateWardImplicit::ID => return trace_dbg!(elt, addr, AvatarUpdateWardImplicit),
            AvatarUpdateWardExplicit::ID => return trace_dbg!(elt, addr, AvatarUpdateWardExplicit),
            AckWardPhysicsCorrection::ID => return trace_dbg!(elt, addr, AckWardPhysicsCorrection),
            RestoreClientAck::ID => return trace_dbg!(elt, addr, RestoreClientAck),
            ClientToServerHeartbeat::ID => return trace_dbg!(elt, addr, ClientToServerHeartbeat),
            SendToCell::ID => return trace_dbg!(elt, addr, SendToCell),
            id if id::BASE_ENTITY_METHOD.contains(id) => {

                // Account::doCmdInt3 (AccountCommands.CMD_SYNC_DATA), exposed id: 0x0E, message id: 0x95

                if let Some(entity_id) = self.player_entity_id {
                    // Unwrap because selected entity should exist!
                    let &(type_id, _) = self.entities.get(&entity_id).unwrap();
                    let Some(dispatch) = self.shared.dispatch.entity_from_id(type_id) else {
                        warn!(%addr, id, "-> Base entity method (no dispatch table for entity type 0x{type_id:02X}): ({entity_id})");
                        return Ok(true);
                    };
                    // The element's framing is always Variable16 regardless of exposed
                    // id, so this never fails to decode structurally -- an unrecognized
                    // exposed id surfaces as `MethodCall::Unknown`, not an `Err`, so the
                    // bundle reader always safely advances here.
                    let call = elt.read::<BaseEntityMethod, _>(&dispatch.base_methods)?.element.call;
                    match &call {
                        MethodCall::Known { .. } => info!(%addr, id, "-> Base entity method: ({entity_id}) {call:?}"),
                        MethodCall::Unknown { .. } => warn!(%addr, id, "-> Base entity method (unrecognized exposed id): ({entity_id}) {call:?}"),
                    }
                    return Ok(true);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "-> Base entity method (unknown selected entity): msg#{} {:?}", id - id::BASE_ENTITY_METHOD.first, elt.element);
                return Ok(false);

            }
            id if id::CELL_ENTITY_METHOD.contains(id) => {

                if let Some(player_entity_id) = self.player_entity_id {
                    let tables = CellTables { handler: self, player_entity_id };
                    let m = elt.read::<CellEntityMethod, _>(&tables)?.element;
                    // A wire id of 0 means "my own entity" (the base app substitutes its
                    // own id), so resolve it for the log but keep an explicit target
                    // visible when the call names one -- in a battle that is the common
                    // case, since gun aiming is addressed to the player's *vehicle*.
                    let target = if m.entity_id == 0 {
                        format!("{player_entity_id} (self)")
                    } else {
                        format!("{}", m.entity_id)
                    };
                    let call = m.call;
                    match &call {
                        MethodCall::Known { .. } => info!(%addr, id, "-> Cell entity method: ({target}) {call:?}"),
                        MethodCall::Unknown { .. } => warn!(%addr, id, "-> Cell entity method (unrecognized exposed id): ({target}) {call:?}"),
                    }
                    return Ok(true);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "-> Cell entity method (unknown selected entity): msg#{} {:?}", id - id::CELL_ENTITY_METHOD.first, elt.element);
                return Ok(false);

            }
            id => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                error!(%addr, id, request_id = ?elt.request_id, "-> Unknown element: {:?}", elt.element);
                return Ok(false);
            }
        }

        Ok(true)

    }

    fn read_in_bundle(&mut self, mut peer: proxy::Peer, bundle: Bundle) -> io::Result<()> {

        let _span = trace_span!("in").entered();

        let mut reader = bundle.element_reader();
        while let Some(elt) = reader.next() {
            match elt {
                NextElementReader::Element(elt) => {
                    if !self.read_in_element(&mut peer, elt)? {
                        break;
                    }
                }
                NextElementReader::Reply(reply) => {
                    let request_id = reply.request_id();
                    let _elt = reply.read_simple::<()>()?;
                    warn!(addr = %peer.addr(), request_id, "<- Reply (unknown payload length, bundle reading stopped here)");
                    break;
                }
            }
        }

        Ok(())

    }

    fn read_in_element(&mut self, peer: &mut proxy::Peer, elt: ElementReader) -> io::Result<bool> {

        use client::element::*;

        let addr = peer.addr();

        match elt.id() {
            UpdateFrequencyNotification::ID => {
                let ufn = elt.read_simple::<UpdateFrequencyNotification>()?;
                info!(%addr, id = UpdateFrequencyNotification::ID, request_id = ?ufn.request_id,
                    "<- Update frequency: {} Hz, game time: {}", ufn.element.frequency, ufn.element.game_time);
            }
            TickSync::ID => {
                let ts = elt.read_simple::<TickSync>()?;
                if let Some(next_tick) = self.next_tick {
                    if next_tick != ts.element.tick {
                        trace!(%addr, "<- Tick missed, expected {next_tick}, got {}", ts.element.tick);
                    }
                }
                trace!(%addr, id = TickSync::ID, request_id = ?ts.request_id, "<- Tick sync: {}", ts.element.tick);
                self.next_tick = Some(ts.element.tick.wrapping_add(1));
            }
            ResetEntities::ID => {

                let re = elt.read_simple::<ResetEntities>()?;

                info!(%addr, id = ResetEntities::ID, request_id = ?re.request_id,
                    "<- Reset entities, keep player on base: {}, entities: {}",
                    re.element.keep_player_on_base, self.entities.len());

                // Mirrors the client's connection reset, which clears the position state
                // along with `idAlias_`. Keeping a stale reference across a space change
                // would offset every position in the new space by the old space's origin.
                self.position.reset();

                // Don't delete player entity if requested...
                let mut player_entity = None;
                if re.element.keep_player_on_base {
                    if let Some(player_entity_id) = self.player_entity_id {
                        player_entity = Some(self.entities.remove_entry(&player_entity_id).unwrap());
                    }
                }

                self.entities.clear();
                // Entity ids are reused across spaces, so a leftover mapping would resolve
                // a wrong type for a reused id. Dropped wholesale: unlike `entities`, this
                // never holds the player, so nothing here needs preserving.
                self.created_entity_types.clear();
                // The alias table is per-connection state in the real client and is wiped
                // by the same reset that clears `id_`/`spaceID_`/`controlledEntities_`
                // (`memset( idAlias_, 0, sizeof( idAlias_ ) )`). Aliases are small ints
                // reused from scratch in the next space, so keeping them across a reset
                // would silently resolve a new space's alias to a dead entity from the
                // old one -- the same class of bug as the `entities` map, which is why
                // that is cleared here too.
                self.id_aliases.clear();
                self.player_entity_id = None;

                // Restore player entity!
                if let Some((player_entity_id, player_entity)) = player_entity {
                    self.entities.insert(player_entity_id, player_entity);
                    self.player_entity_id = Some(player_entity_id);
                }

                // `selected_entity_id` isn't necessarily the player entity (see
                // `SelectEntity`/`SelectAliasedEntity`, not tracked here), but whatever
                // it pointed to is gone now that `entities` was just cleared -- keeping
                // it around would leave the `ENTITY_METHOD`/`ENTITY_PROPERTY` arms below
                // holding a dangling id into `entities`, which used to `unwrap()` a
                // `None` and panic this connection's poll worker thread silently (no
                // `JoinHandle::join()` anywhere to surface it -- see `ThreadPoll`).
                self.selected_entity_id = self.select_player_entity_id();

            }
            LoggedOff::ID => {
                let lo = elt.read_simple::<LoggedOff>()?;
                info!(%addr, id = LoggedOff::ID, request_id = ?lo.request_id, "<- Logged off: 0x{:02X}", lo.element.reason);
            }
            id::CREATE_BASE_PLAYER => {

                // An unrecognized entity type id surfaces as an `Err` here (propagated by
                // `?`), same as everywhere else this project has decided it's safer to
                // stop reading a bundle than to guess -- see `CreateBasePlayer`'s
                // `Element<ScriptDispatch>` impl.
                let full = elt.read::<CreateBasePlayer, _>(&self.shared.dispatch)?;

                // The full entity data is logged as its own field (rather than a
                // separate `entity_<id>.txt` dump file) so it lands in the same
                // ordered trace while keeping the message itself a short one-liner.
                info!(%addr, id = id::CREATE_BASE_PLAYER, request_id = ?full.request_id,
                    entity_data = ?full.element.entity_data,
                    "<- Create base player: ({}) entity_type_id={} entity_components_count={}",
                    full.element.entity_id, full.element.entity_type_id, full.element.entity_components_count);

                self.entities.insert(full.element.entity_id, (full.element.entity_type_id, full.element.entity_data.into_owned()));
                self.player_entity_id = Some(full.element.entity_id);

            }
            CreateCellPlayer::ID => {

                let ccp = elt.read_simple::<CreateCellPlayer>()?;
                warn!(%addr, id = CreateCellPlayer::ID, request_id = ?ccp.request_id, "<- Create cell player: {:?}", ccp.element);

                // The space's packed-position scale, without which no volatile position can
                // be resolved at all, plus the initial reference: `createCellPlayer` ends
                // with `detailedPositionReceived( id_, ... )`, so entering a space bases the
                // reference on the player's own starting position.
                self.position.packed_xz_scale = Some(ccp.element.packed_xz_scale);
                if let Some(id) = self.player_entity_id {
                    self.position.detailed_position_received(id, Some(id), ccp.element.position);
                }

                // `vehicle_id` is a distinct `Vehicle` entity, not the player's own base
                // entity (confirmed live -- see the doc comment on
                // `client::element::CreateCellPlayer::vehicle_id`). Register it so a later
                // `SelectEntity` targeting it can resolve a real dispatch table, instead of
                // whatever entity happened to be selected before (previously misattributed
                // to the player's `Account` entity, which then failed to decode the
                // vehicle's own properties past `Account`'s shorter property table).
                match self.shared.dispatch.entity_from_name("Vehicle") {
                    Some((vehicle_type_id, _)) => {
                        self.entities.insert(ccp.element.vehicle_id, (vehicle_type_id, Value::Dict(BTreeMap::new())));
                    }
                    None => warn!(%addr, "<- Create cell player: no 'Vehicle' entity type in the loaded script model"),
                }

            }
            SelectEntity::ID => {
                let se = elt.read_simple::<SelectEntity>()?;
                // A type known only from `CreateEntity` still lets the framing-independent
                // decoders resolve a table, so it counts as tracked here.
                if self.entities.contains_key(&se.element.entity_id)
                    || self.created_entity_types.contains_key(&se.element.entity_id) {
                    info!(%addr, id = SelectEntity::ID, request_id = ?se.request_id,
                        "<- Select entity: {}", se.element.entity_id);
                } else {
                    warn!(%addr, id = SelectEntity::ID, request_id = ?se.request_id,
                        "<- Select entity (not tracked): {}", se.element.entity_id);
                }
                self.selected_entity_id = Some(se.element.entity_id);
            }
            SelectPlayerEntity::ID => {
                let spe = elt.read_simple::<SelectPlayerEntity>()?;
                if let Some(player_entity_id) = self.select_player_entity_id() {
                    info!(%addr, id = SelectPlayerEntity::ID, request_id = ?spe.request_id,
                        "<- Select player entity: {player_entity_id}");
                } else {
                    warn!(%addr, id = SelectPlayerEntity::ID, request_id = ?spe.request_id,
                        "<- Select player entity: no player entity")
                }
                self.selected_entity_id = self.select_player_entity_id();
            }
            SwitchBaseApp::ID => {

                let sba = elt.read_simple::<SwitchBaseApp>()?;

                // A `0.x.x.x` address, or a real-looking address with port 0, is never
                // valid for a real game server -- seeing one here means the bundle's read
                // position already desynced on an earlier element (confirmed live: a busy
                // battle with other entities in view exercises `SelectAliasedEntity`/
                // `CreateEntity`, still unconfirmed/undecoded, see their doc comments) and
                // these 9 bytes are garbage, not a real `SwitchBaseApp`. Trusting it would
                // register the garbage address as this peer's reconnect target below, so
                // when the client's real reconnect arrives, this proxy forwards it into a
                // black hole it can never reach -- confirmed live to be exactly what an
                // "instant" battle disconnect turned out to be. Confirmed live a second
                // time, worse: forwarding a subsequent packet to a port-0 address makes
                // the OS-level send fail with `EINVAL`, which isn't a `ConnectionReset`
                // and so isn't handled as a per-peer drop -- it propagates all the way out
                // of the base app's `run()` loop and kills request handling for every
                // connected client, not just this one. Bail out loudly instead of
                // chaining into either failure mode.
                // The `0.x.x.x`/port-0 checks below are necessary but not sufficient: a
                // desynced read can yield an address that passes both, and adopting it as
                // the peer's real address kills the connection (inbound stops dead while
                // the client resends its session key forever). A genuine base-app switch
                // stays inside the operator's network, so require the same /16 as the real
                // login app -- the tightest bound derivable from this proxy's own config
                // without hardcoding an IP range.
                let real_login_octets = self.shared.real_login_app_addr.ip().octets();
                let new_octets = sba.element.base_addr.addr.ip().octets();
                let same_network = new_octets[..2] == real_login_octets[..2];

                if !same_network {
                    error!(%addr, id = SwitchBaseApp::ID, request_id = ?sba.request_id,
                        "<- Switch base app: address {:?} is outside the real login app's network ({}), \
                         likely bundle desync upstream -- ignoring",
                        sba.element.base_addr, self.shared.real_login_app_addr.ip());
                    return Ok(true);
                }

                if sba.element.base_addr.addr.ip().octets()[0] == 0 || sba.element.base_addr.addr.port() == 0 {
                    error!(%addr, id = SwitchBaseApp::ID, request_id = ?sba.request_id,
                        "<- Switch base app: implausible address {:?}, likely bundle desync upstream -- ignoring",
                        sba.element.base_addr);
                    return Ok(true);
                }

                info!(%addr, id = SwitchBaseApp::ID, request_id = ?sba.request_id,
                    "<- Switch base app to: {:?} (reset entities: {})", sba.element.base_addr, sba.element.reset_entities);

                // Change the real base address for this peer, so our own forwarding
                // starts targeting it -- but let the packet itself reach the client
                // completely as-is otherwise (same framing, same sequence number, same
                // channel), just with the embedded address patched to point back at
                // this proxy instead. This way the real application still sees (and
                // acknowledges, and retransmits if needed) the exact packet it sent,
                // while the client only ever learns about our own address.
                peer.set_real_addr(sba.element.base_addr.into());

                // The client tears down its whole connection and reconnects fresh after
                // this (confirmed live), from an unpredictable new local port, so
                // pre-register the session by IP alone for `accept_peer` to pick up.
                if let Some(blowfish) = peer.blowfish_arc() {
                    self.shared.pending_switches.lock().unwrap().insert(addr.ip(), PendingClient {
                        base_app_addr: sba.element.base_addr.addr,
                        blowfish,
                    });
                } else {
                    warn!(%addr, "<- Switch base app: no blowfish key available, cannot pre-register the reconnection");
                }

                // Patch the embedded address in place: same framing (prefix, flags,
                // sequence number, channel), just pointing at us instead of the real
                // base app, so the real application still sees (and acknowledges, and
                // retransmits if needed) the exact packet it sent. Our own salt is
                // always 0 (`WgSocketAddrV4::from<SocketAddrV4>`'s default), matching
                // how we represent this address everywhere else (e.g. the login-time
                // hand-off), rather than reusing whatever the real message carried --
                // see `WgSocketAddrV4`'s doc comment for why that's correct here.
                let our_addr = WgSocketAddrV4::from(self.shared.base_app_addr);
                if !peer.patch_raw(&sba.element.base_addr.to_bytes(), &our_addr.to_bytes()) {
                    warn!(%addr, "<- Switch base app: could not locate the address bytes to patch in the raw packet");
                }

            }
            ResourceHeader::ID => {

                let rh = elt.read_simple::<ResourceHeader>()?;
                info!(%addr, id = ResourceHeader::ID, request_id = ?rh.request_id, "<- Resource header: {:?}", rh.element);

                // Intentionally overwrite any previous downloading resource!
                self.partial_resources.insert(rh.element.id, PartialResource {
                    description: rh.element.description,
                    fragments: BTreeMap::new(),
                    last_sequence_num: None,
                });

            }
            ResourceFragment::ID => {

                let rf = elt.read_simple::<ResourceFragment>()?;
                let res_id = rf.element.id;

                let Some(partial_resource) = self.partial_resources.get_mut(&res_id) else {
                    warn!(%addr, id = ResourceFragment::ID, request_id = ?rf.request_id,
                        "<- Resource fragment: {res_id}, len: {}, missing header", rf.element.data.len());
                    return Ok(true);
                };

                // Fragments can arrive out of order (piggybacked retransmits, bundles
                // racing each other, ...), so just file each one under its sequence
                // number instead of assuming they arrive already in order, and only
                // reassemble once every sequence number up to the one marked `last`
                // has actually been seen.
                if partial_resource.fragments.insert(rf.element.sequence_num, rf.element.data.clone()).is_some() {
                    warn!(%addr, id = ResourceFragment::ID, request_id = ?rf.request_id,
                        "<- Resource fragment: {res_id}, len: {}, duplicate sequence number {}",
                        rf.element.data.len(), rf.element.sequence_num);
                }

                if rf.element.last {
                    partial_resource.last_sequence_num = Some(rf.element.sequence_num);
                }

                trace!(%addr, id = ResourceFragment::ID, request_id = ?rf.request_id,
                    "<- Resource fragment: {res_id}, len: {}, sequence number: {}",
                    rf.element.data.len(), rf.element.sequence_num);

                // Only proceed once we know the final sequence number and every
                // fragment up to it has been received, regardless of arrival order.
                let Some(last_sequence_num) = partial_resource.last_sequence_num else {
                    return Ok(true);
                };
                let complete = (0..=last_sequence_num)
                    .all(|seq| partial_resource.fragments.contains_key(&seq));
                if !complete {
                    return Ok(true);
                }

                let resource = self.partial_resources.remove(&res_id).unwrap();
                let data: Vec<u8> = resource.fragments.into_values().flatten().collect();

                // See: scripts/client/game.py#L223
                let (total_len, crc32) = match serde_pickle::value_from_reader(&resource.description[..], serde_pickle_de_options()) {
                    Ok(serde_pickle::Value::Tuple(values)) if values.len() == 2 => {
                        if let &[serde_pickle::Value::I64(total_len), serde_pickle::Value::I64(crc32)] = &values[..] {
                            (total_len as u32, crc32 as u32)
                        } else {
                            warn!(%addr, "<- Invalid resource description: unexpected values: {values:?}");
                            return Ok(true);
                        }
                    }
                    Ok(v) => {
                        warn!(%addr, "<- Invalid resource description: python: {v}");
                        return Ok(true);
                    }
                    Err(e) => {
                        warn!(%addr, "<- Invalid resource description: {e}");
                        return Ok(true);
                    }
                };

                let actual_total_len = data.len();
                if actual_total_len != total_len as usize {
                    warn!(%addr, "<- Invalid resource length, expected: {total_len}, got: {actual_total_len}");
                    return Ok(true);
                }

                let actual_crc32 = crc32fast::hash(&data);
                if actual_crc32 != crc32 {
                    warn!(%addr, "<- Invalid resource crc32, expected: 0x{crc32:08X}, got: 0x{actual_crc32:08X}");
                    return Ok(true);
                }

                // TODO: onCmdResponse for requested SYNC use RES_SUCCESS=0, RES_STREAM=1, RES_CACHE=2 for result_id
                //       When RES_STREAM is used, then a resource (header+fragment) is expected with the associated request_id.

                // The full data is a zlib-compressed pickle. Decoded and logged in full
                // here (rather than written to a separate dump file) so it lands in the
                // same ordered trace as everything else.
                match serde_pickle::value_from_reader(ZlibDecoder::new(&data[..]), serde_pickle_de_options()) {
                    Ok(val) => {
                        info!(%addr, res_id, len = actual_total_len, crc32 = format!("0x{crc32:08X}"), python = %val,
                            "<- Resource completed");
                    }
                    Err(e) => {

                        // FIXME: It appears that the current serde-pickle impl doesn't
                        // support recursive structures, however the structure that is
                        // initially requested with 'CMD_SYNC_DATA' contains some.
                        // FIXME: The resource that is received by the from the chat
                        // command contains a "deque" object, which cannot be parsed
                        // so we get a "unresolved global reference" error.

                        let mut raw = Vec::new();
                        let _ = std::io::copy(&mut ZlibDecoder::new(&data[..]), &mut raw);
                        warn!(%addr, res_id, len = actual_total_len, crc32 = format!("0x{crc32:08X}"), raw = hex(&raw),
                            "<- Resource completed but failed to decode as python: {e}");

                    }
                }

            }
            // The following ids are known (id, length style and length param all confirmed
            // live against v2.3.1.3, see `re-work/NOTES.md`) but not yet given a proper
            // structured codec -- read them through their placeholder/real type so the
            // bundle can safely keep being read, and trace their content.
            // Note: ids 0x29..=0x40 (the AVATAR_UPDATE_*_CALLBACK combinatorial messages)
            // have no known length style yet and so aren't covered here -- they still fall
            // through to the generic catch-all below, which stops reading the bundle.
            Authenticate::ID => return trace_dbg!(elt, addr, Authenticate),
            BandwidthNotification::ID => return trace_dbg!(elt, addr, BandwidthNotification),
            SetGameTime::ID => return trace_dbg!(elt, addr, SetGameTime),
            DummyPacket::ID => return trace_dbg!(elt, addr, DummyPacket),
            SpaceProperty::ID => return trace_dbg!(elt, addr, SpaceProperty),
            AddSpaceGeometryMapping::ID => return trace_dbg!(elt, addr, AddSpaceGeometryMapping),
            RemoveSpaceGeometryMapping::ID => return trace_dbg!(elt, addr, RemoveSpaceGeometryMapping),
            CreateEntity::ID => {
                // Record the entity's type so a later `SelectEntity` targeting it can
                // resolve a dispatch table. Without this, only the player's own base
                // entity (`CreateBasePlayer`) and vehicle (`CreateCellPlayer`) were ever
                // known, so every message aimed at any *other* entity fell through to a
                // "not tracked" warning -- which is what kept the `Nested`/
                // `SliceEntityProperty` codecs from ever being exercised against real
                // traffic, since those target exactly such entities. See
                // `created_entity_types` for why this deliberately doesn't go into
                // `entities`.
                let e = match elt.read_simple::<CreateEntity>() {
                    Ok(e) => e,
                    // Worth its own warning rather than just propagating: this is the only
                    // thing that ever learns a non-player entity's type, so a failure here
                    // is what turns every later property of that entity into a "no dispatch
                    // table" stop. `CreateEntity`'s payload is wrapped in a
                    // `CompressionIStream`, and a zlib-compressed body is rejected outright
                    // (no zlib dependency here), so that is the first suspect.
                    Err(e) => {
                        warn!(%addr, id = CreateEntity::ID, "<- Create entity FAILED (entity type will be unknown): {e}");
                        return Err(e);
                    }
                };
                self.created_entity_types.insert(e.element.entity_id, e.element.entity_type_id);
                // `ServerConnection::createEntity` ends in `detailedPositionReceived`, so a
                // create for the player's own entity re-bases the reference.
                self.position.detailed_position_received(e.element.entity_id, self.player_entity_id, e.element.position);
                trace!(%addr, id = CreateEntity::ID, request_id = ?e.request_id, "{}: {:?}", stringify!(CreateEntity), e.element);
                return Ok(true);
            }
            CreateEntityDetailed::ID => {
                let e = elt.read_simple::<CreateEntityDetailed>()?;
                self.created_entity_types.insert(e.element.entity_id, e.element.entity_type_id);
                self.position.detailed_position_received(e.element.entity_id, self.player_entity_id, e.element.position);
                trace!(%addr, id = CreateEntityDetailed::ID, request_id = ?e.request_id, "{}: {:?}", stringify!(CreateEntityDetailed), e.element);
                return Ok(true);
            }
            CellAppSuspended::ID => return trace_dbg!(elt, addr, CellAppSuspended),
            CellAppResumed::ID => return trace_dbg!(elt, addr, CellAppResumed),
            ClientSuspensionDetectionEnabled::ID => return trace_dbg!(elt, addr, ClientSuspensionDetectionEnabled),
            EnterAoi::ID => {
                let e = elt.read_simple::<EnterAoi>()?;
                // Recorded even when the alias is `NO_ID_ALIAS` (0xFF), matching
                // `ServerConnection::enterAoI`, which does `idAlias_[ idAlias ] = id;`
                // under the explicit comment "Set this even if args.idAlias is
                // NO_ID_ALIAS." 0xFF is a real, addressable slot in the client's 256-entry
                // table, so a later `selectAliasedEntity( 0xFF )` resolves to whichever
                // entity last entered without an alias. Skipping it made that lookup miss.
                self.id_aliases.insert(e.element.id_alias, e.element.entity_id);
                info!(%addr, id = EnterAoi::ID, request_id = ?e.request_id,
                    "<- Enter AoI: entity {} alias {}", e.element.entity_id,
                    fmt_id_alias(e.element.id_alias));
                return Ok(true);
            }
            EnterAoiOnVehicle::ID => {
                let e = elt.read_simple::<EnterAoiOnVehicle>()?;
                // Same slot-0xFF handling as `EnterAoi` above: `enterAoI` is the shared
                // implementation behind both messages.
                self.id_aliases.insert(e.element.id_alias, e.element.entity_id);
                info!(%addr, id = EnterAoiOnVehicle::ID, request_id = ?e.request_id,
                    "<- Enter AoI on vehicle: entity {} vehicle {} alias {}",
                    e.element.entity_id, e.element.vehicle_id,
                    fmt_id_alias(e.element.id_alias));
                return Ok(true);
            }
            LeaveAoi::ID => {
                let e = elt.read_simple::<LeaveAoi>()?;
                // The wire message only carries the entity id (see `Witness::onLeaveAoI`
                // in the leaked BigWorld source), not the alias itself, so a slot can only
                // be found by value rather than by key -- reported here, but deliberately
                // *not* removed.
                //
                // The real client never unmaps an alias on leave: `idAlias_` is written
                // only by `ServerConnection::enterAoI` and cleared only by the full
                // connection reset (`memset( idAlias_, 0, sizeof( idAlias_ ) )`). The
                // server reuses a freed alias by sending a fresh `enterAoI` for it, which
                // overwrites the slot, so dropping it here buys nothing and costs real
                // data: an unresolvable alias now *clears* `selected_entity_id` (see
                // `trace_dbg_alias`), so a volatile update that arrives slightly after the
                // leave -- reordering is normal, these are unreliable messages -- would
                // stop the bundle on "no dispatch table" instead of decoding as the client
                // decodes it.
                let stale: Vec<u8> = self.id_aliases.iter()
                    .filter(|(_, v)| **v == e.element.entity_id).map(|(k, _)| *k).collect();
                info!(%addr, id = LeaveAoi::ID, request_id = ?e.request_id,
                    "<- Leave AoI: entity {} (aliases now stale, kept as the client keeps them: {:?})",
                    e.element.entity_id, stale);
                return Ok(true);
            }
            TickSyncPeriodic::ID => return trace_dbg!(elt, addr, TickSyncPeriodic),
            RelativePositionReference::ID => {
                let e = elt.read_simple::<RelativePositionReference>()?;
                let seq = e.element.sequence_number;
                match self.position.set_reference_from_sent(seq) {
                    Some(reference) => trace!(%addr, id = RelativePositionReference::ID, request_id = ?e.request_id,
                        "<- Relative position reference: seq {seq} -> {reference:?}"),
                    // Expected when the proxy joined mid-session or an upstream update was
                    // lost: the slot was never observed. Leave the old reference in place
                    // rather than inventing one -- positions stay slightly stale instead of
                    // jumping to a fabricated origin.
                    None => warn!(%addr, id = RelativePositionReference::ID,
                        "<- Relative position reference: seq {seq} names a position never seen upstream; reference unchanged"),
                }
                return Ok(true);
            }
            RelativePosition::ID => {
                // Assigned verbatim -- `ServerConnection::relativePosition` does not round,
                // unlike every other writer of this field.
                let e = elt.read_simple::<RelativePosition>()?;
                self.position.set_reference_exact(e.element.position);
                trace!(%addr, id = RelativePosition::ID, request_id = ?e.request_id,
                    "<- Relative position: {:?}", e.element.position);
                return Ok(true);
            }
            SetVehicle::ID => {
                let e = elt.read_simple::<SetVehicle>()?;
                self.position.set_vehicle(e.element.passenger_id, e.element.vehicle_id);
                trace!(%addr, id = SetVehicle::ID, request_id = ?e.request_id,
                    "<- Set vehicle: entity {} -> vehicle {}", e.element.passenger_id, e.element.vehicle_id);
                return Ok(true);
            }
            SelectAliasedEntity::ID => {
                // `ServerConnection::selectAliasedEntity`: `selectedEntityID_ = idAlias_[args.idAlias]`.
                // The element is a `DebugElementFixed<_, 1>`, so its single data byte *is*
                // the alias. Unresolvable alias clears the selection, as in `trace_dbg_alias`.
                let e = elt.read_simple::<SelectAliasedEntity>()?;
                let id_alias = e.element.data[0];
                let entity_id = self.id_aliases.get(&id_alias).copied();
                if entity_id.is_none() && self.warned_aliases.insert((SelectAliasedEntity::ID, id_alias)) {
                    let mut known: Vec<u8> = self.id_aliases.keys().copied().collect();
                    known.sort_unstable();
                    warn!(%addr, "<- SelectAliasedEntity: unresolved id_alias {id_alias}; known aliases ({}): {known:?}", known.len());
                }
                self.selected_entity_id = entity_id;
                trace!(%addr, id = SelectAliasedEntity::ID, request_id = ?e.request_id,
                    id_alias, entity_id = ?entity_id, "SelectAliasedEntity");
                return Ok(true);
            }
            ForcedPosition::ID => return trace_dbg!(elt, addr, ForcedPosition),
            AvatarUpdateNoAliasDetailed::ID => {
                // Re-targets like the rest of the AVUPMSG family (see `trace_dbg_entity`),
                // and additionally re-bases the reference position: this one carries a
                // full uncompressed `Vec3`, which is what `detailedPositionReceived`
                // rounds into `referencePosition_` (for the player's own entity only).
                let e = elt.read_simple::<AvatarUpdateNoAliasDetailed>()?;
                let entity_id = e.element.entity_id;
                self.selected_entity_id = Some(entity_id);
                let rebased = self.position.detailed_position_received(entity_id, self.player_entity_id, e.element.position);
                info!(%addr, id = AvatarUpdateNoAliasDetailed::ID, request_id = ?e.request_id, entity_id, rebased,
                    "<- {}: {:?}", stringify!(AvatarUpdateNoAliasDetailed), e.element);
                return Ok(true);
            }
            AvatarUpdateAliasDetailed::ID => {
                // As `AvatarUpdateNoAliasDetailed`, but the target arrives as an alias.
                let e = elt.read_simple::<AvatarUpdateAliasDetailed>()?;
                let entity_id = self.id_aliases.get(&e.element.id_alias).copied();
                self.selected_entity_id = entity_id;
                let rebased = match entity_id {
                    Some(id) => self.position.detailed_position_received(id, self.player_entity_id, e.element.position),
                    None => false,
                };
                info!(%addr, id = AvatarUpdateAliasDetailed::ID, request_id = ?e.request_id, entity_id = ?entity_id, rebased,
                    "<- {}: {:?} (alias {} -> {:?})", stringify!(AvatarUpdateAliasDetailed), e.element,
                    fmt_id_alias(e.element.id_alias), entity_id);
                return Ok(true);
            }
            AvatarUpdatePlayerDetailed::ID => {
                // `ServerConnection::avatarUpdatePlayerDetailed` does `selectedEntityID_ = id_`
                // -- the player entity, same target as `SelectPlayerEntity`.
                let e = elt.read_simple::<AvatarUpdatePlayerDetailed>()?;
                self.selected_entity_id = self.select_player_entity_id();
                // This one is always the player's own entity, so it re-bases the reference
                // whenever the player isn't riding a vehicle.
                let rebased = match self.player_entity_id {
                    Some(id) => self.position.detailed_position_received(id, self.player_entity_id, e.element.position),
                    None => false,
                };
                trace!(%addr, id = AvatarUpdatePlayerDetailed::ID, request_id = ?e.request_id, rebased,
                    "{}: {:?}", stringify!(AvatarUpdatePlayerDetailed), e.element);
                return Ok(true);
            }
            AvatarUpdateNoAliasFullPosYawPitchRoll::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasFullPosYawPitchRoll),
            AvatarUpdateNoAliasFullPosYawPitch::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasFullPosYawPitch),
            AvatarUpdateNoAliasFullPosYaw::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasFullPosYaw),
            AvatarUpdateNoAliasFullPosNoDir::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasFullPosNoDir),
            AvatarUpdateNoAliasOnGroundYawPitchRoll::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasOnGroundYawPitchRoll),
            AvatarUpdateNoAliasOnGroundYawPitch::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasOnGroundYawPitch),
            AvatarUpdateNoAliasOnGroundYaw::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasOnGroundYaw),
            AvatarUpdateNoAliasOnGroundNoDir::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasOnGroundNoDir),
            AvatarUpdateNoAliasNoPosYawPitchRoll::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasNoPosYawPitchRoll),
            AvatarUpdateNoAliasNoPosYawPitch::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasNoPosYawPitch),
            AvatarUpdateNoAliasNoPosYaw::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasNoPosYaw),
            AvatarUpdateNoAliasNoPosNoDir::ID => return trace_dbg_entity!(self, elt, addr, AvatarUpdateNoAliasNoPosNoDir),
            AvatarUpdateAliasFullPosYawPitchRoll::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasFullPosYawPitchRoll),
            AvatarUpdateAliasFullPosYawPitch::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasFullPosYawPitch),
            AvatarUpdateAliasFullPosYaw::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasFullPosYaw),
            AvatarUpdateAliasFullPosNoDir::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasFullPosNoDir),
            AvatarUpdateAliasOnGroundYawPitchRoll::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasOnGroundYawPitchRoll),
            AvatarUpdateAliasOnGroundYawPitch::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasOnGroundYawPitch),
            AvatarUpdateAliasOnGroundYaw::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasOnGroundYaw),
            AvatarUpdateAliasOnGroundNoDir::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasOnGroundNoDir),
            AvatarUpdateAliasNoPosYawPitchRoll::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasNoPosYawPitchRoll),
            AvatarUpdateAliasNoPosYawPitch::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasNoPosYawPitch),
            AvatarUpdateAliasNoPosYaw::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasNoPosYaw),
            AvatarUpdateAliasNoPosNoDir::ID => return trace_dbg_alias!(self, elt, addr, AvatarUpdateAliasNoPosNoDir),
            AvatarUpdateVolatileProperties::ID => return trace_dbg!(elt, addr, AvatarUpdateVolatileProperties),
            ChangeVolatilePackerType::ID => return trace_dbg!(elt, addr, ChangeVolatilePackerType),
            NrlCreateNode::ID => return trace_dbg!(elt, addr, NrlCreateNode),
            NrlUnlinkTree::ID => return trace_dbg!(elt, addr, NrlUnlinkTree),
            NrlUpdateNode::ID => return trace_dbg!(elt, addr, NrlUpdateNode),
            NrlUnlinkTreeFlag::ID => return trace_dbg!(elt, addr, NrlUnlinkTreeFlag),
            NrlUpdateNodeFlag::ID => return trace_dbg!(elt, addr, NrlUpdateNodeFlag),
            NrlData::ID => return trace_dbg!(elt, addr, NrlData),
            NrlMsgToClient::ID => return trace_dbg!(elt, addr, NrlMsgToClient),
            NrlUnreliableMsgToClient::ID => return trace_dbg!(elt, addr, NrlUnreliableMsgToClient),
            ControlEntity::ID => return trace_dbg!(elt, addr, ControlEntity),
            VoiceData::ID => return trace_dbg!(elt, addr, VoiceData),
            RestoreClient::ID => return trace_dbg!(elt, addr, RestoreClient),
            DetailedPosition::ID => {
                // Carries no id of its own -- it applies to the currently-selected entity,
                // which is also what `ServerConnection::detailedPosition` uses
                // (`getVehicleID( selectedEntityID_ )`).
                let e = elt.read_simple::<DetailedPosition>()?;
                let rebased = match self.selected_entity_id {
                    Some(id) => self.position.detailed_position_received(id, self.player_entity_id, e.element.position),
                    None => false,
                };
                trace!(%addr, id = DetailedPosition::ID, request_id = ?e.request_id, rebased,
                    "<- {}: {:?}", stringify!(DetailedPosition), e.element);
                return Ok(true);
            }
            id @ NestedEntityProperty::ID => {

                // Buffer the raw payload first so a decode failure (or a missing dispatch
                // table) can still report the exact bytes as plain hex, which is what makes
                // such a failure diagnosable at all. Reading the correctly-framed element
                // rather than an unbounded `DebugElementUndefined` also lets every path
                // below continue the bundle safely.
                let raw = elt.read_simple::<DebugElementVariable16<{ NestedEntityProperty::ID }>>()?;
                let data = raw.element.data;

                // Same dispatch-table requirement as `ENTITY_PROPERTY` below: decoding
                // the compressed path needs the correct entity's own property table to
                // know each container's shape (see `NestedEntityProperty`'s doc comment).
                // Safe to consult `created_entity_types` here (unlike
                // `ENTITY_PROPERTY`/`ENTITY_METHOD`): this element is always `Variable16`,
                // so its framing doesn't depend on the table and a wrong guess can only
                // produce a bad decode of this one element, never a bundle desync.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                let Some((entity_id, dispatch)) = dispatch else {
                    warn!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Nested entity property (no dispatch table)");
                    return Ok(true);
                };

                let mut read = io::Cursor::new(&data[..]);
                match NestedEntityProperty::read(&mut read, &dispatch.properties, data.len(), id) {
                    Ok(p) => info!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Nested entity property: ({entity_id}) {:?} = {:?}", p.path, p.value),
                    // Unlike `ENTITY_PROPERTY`, a decode failure here doesn't risk
                    // desyncing the rest of the bundle (the payload is fully consumed
                    // above), so log and keep going rather than stop the whole bundle.
                    Err(e) => warn!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Nested entity property (failed to decode): ({entity_id}) {e}"),
                }

                return Ok(true);

            }
            id @ SliceEntityProperty::ID => {

                // See the `NestedEntityProperty` arm above for why the payload is
                // buffered and hex-logged before any decode is attempted.
                let raw = elt.read_simple::<DebugElementVariable16<{ SliceEntityProperty::ID }>>()?;
                let data = raw.element.data;

                // Safe to consult `created_entity_types` here (unlike
                // `ENTITY_PROPERTY`/`ENTITY_METHOD`): this element is always `Variable16`,
                // so its framing doesn't depend on the table and a wrong guess can only
                // produce a bad decode of this one element, never a bundle desync.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                let Some((entity_id, dispatch)) = dispatch else {
                    warn!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Slice entity property (no dispatch table)");
                    return Ok(true);
                };

                let mut read = io::Cursor::new(&data[..]);
                match SliceEntityProperty::read(&mut read, &dispatch.properties, data.len(), id) {
                    // Several readings parsed and the append-consistent one was picked;
                    // keep that visibly distinct so a wrong pick can't become fact.
                    Ok(p) if p.seq_len_inferred => info!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Slice entity property (append-inferred): ({entity_id}) {:?} [{}..{}] = {:?}",
                        p.path, p.start, p.end, p.values),
                    Ok(p) => info!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Slice entity property: ({entity_id}) {:?} [{}..{}] = {:?}",
                        p.path, p.start, p.end, p.values),
                    Err(e) => warn!(%addr, id, len = data.len(), data = %hex(&data),
                        "<- Slice entity property (failed to decode): ({entity_id}) {e}"),
                }

                return Ok(true);

            }
            UpdateEntity::ID => return trace_dbg!(elt, addr, UpdateEntity),
            SetCellAppExtAddress::ID => return trace_dbg!(elt, addr, SetCellAppExtAddress),
            LastProxyMessageAfterDirectCellAppConnection::ID => return trace_dbg!(elt, addr, LastProxyMessageAfterDirectCellAppConnection),
            id if id::ENTITY_METHOD.contains(id) => {

                // Account::msg#37 = onClanInfoReceived
                // Account::msg#39 = showGUI

                // `EntityMethod::read_length` needs the *correct* entity's own
                // `client_methods.len()` to even know this element's framing (it decides
                // per id whether a sub-id byte follows, which shifts Fixed/Variable8 vs.
                // Variable16 -- see that fn's doc comment): without a real dispatch table
                // there is no safe length to guess, unlike a merely *unrecognized exposed
                // id* within a table we do have (that case still surfaces safely as
                // `MethodCall::Unknown`, not consulted here). So every branch below that
                // can't resolve one falls through to the same unbounded, stop-reading
                // path at the bottom instead of trying to skip a guessed number of bytes.
                // Consults `created_entity_types` as well as `entities` -- see that
                // field's doc comment for why this was once restricted to `entities`, and
                // why that restriction has been lifted.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                if let Some((entity_id, dispatch)) = dispatch {
                    // See the `BASE_ENTITY_METHOD` arm in `read_out_element` for why an
                    // unrecognized exposed id surfaces as `MethodCall::Unknown`, not
                    // an `Err`.
                    let call = elt.read::<EntityMethod, _>(&dispatch.client_methods)?.element.call;
                    return Ok(match &call {
                        MethodCall::Known { .. } => {
                            info!(%addr, id, "<- Entity method: ({entity_id}) {call:?}");
                            true
                        }
                        MethodCall::Unknown { .. } => {
                            // Unlike `BASE_ENTITY_METHOD` (always Variable16, confirmed
                            // safe regardless of exposed id), `EntityMethod`'s fallback
                            // for an exposed id outside our table is Variable8 (max 254
                            // bytes) -- confirmed live to be wrong here: our reconstructed
                            // `client_methods` table is missing this id (most likely a
                            // dynamic-component method, see the TODO on
                            // `CreateBasePlayer::entity_components_count` -- this project
                            // doesn't model those at all), not something the real client
                            // fails to recognize too, and the real call was longer than
                            // 254 bytes. Confirmed by a live capture: an entity name
                            // string (a bot vehicle's) got split across two separately
                            // logged "Unknown" calls, and reading continued into unrelated
                            // bytes afterward, eventually misparsing them as a bogus
                            // `CreateBasePlayerHeader` with garbage "python" content. So
                            // stop reading the rest of this bundle rather than trust the
                            // position past a truncated read.
                            warn!(%addr, id, "<- Entity method (unrecognized exposed id, stopping bundle): ({entity_id}) {call:?}");
                            false
                        }
                    });
                }

                // No dispatch table to resolve this element's framing against (nothing
                // selected, the selected entity is no longer tracked, or its type has no
                // dispatch table) -- reading must stop here instead of risking a
                // misinterpreted length desyncing the rest of this bundle (confirmed
                // live: this is exactly how a decode desync turned into a garbage
                // `SwitchBaseApp` that stranded a reconnecting client).
                let reason = self.dispatch_miss_reason();
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity method (no dispatch table: {reason}): msg#{} {:?}", id - id::ENTITY_METHOD.first, elt.element);
                return Ok(false);

            }
            id if id::ENTITY_PROPERTY.contains(id) => {

                // Same reasoning as `ENTITY_METHOD` above: without the correct entity's
                // own `properties.len()`, `EntityProperty::read_length` can't resolve
                // this element's framing at all (see that fn's doc comment -- unlike
                // methods, it has no generic fallback length either), so every branch
                // that can't resolve a dispatch table falls through to the same
                // unbounded, stop-reading path at the bottom.
                // Consults `created_entity_types` as well as `entities` -- see that
                // field's doc comment for why this was once restricted to `entities`, and
                // why that restriction has been lifted.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                if let Some((entity_id, dispatch)) = dispatch {
                    // An `Err` here is expected for any property belonging to a *dynamic*
                    // component (attached per-instance, see `entity_components_count` --
                    // this project's model can't predict their exposed ids, see
                    // `doc/ENTITY.md`), not just a real mismatch -- and also, less
                    // obviously, for a property this table *does* recognize but whose
                    // reconstructed `Ty`/length is itself wrong (e.g. mis-sized, so
                    // `Value::read` over- or under-runs the bounded reader). Unlike entity
                    // methods, there's no safe fallback framing to guess here (see
                    // `EntityProperty`'s doc comment) -- reading must stop rather than
                    // risk desyncing the rest of this bundle. Caught explicitly (rather
                    // than propagated via `?` to the bundle-level catch-all) so the
                    // exposed id and property name -- unavailable once the error reaches
                    // that outer, entity-agnostic context -- can still be logged.
                    match elt.read::<EntityProperty, _>(&dispatch.properties) {
                        Ok(p) => {
                            info!(%addr, id, "<- Entity property: ({entity_id}) {}={:?}", p.element.name, p.element.value);
                            return Ok(true);
                        }
                        Err(e) => {
                            let exposed_id = id::ENTITY_PROPERTY.to_exposed_id_checked(dispatch.properties.len() as u16, id);
                            let name = exposed_id.and_then(|e| dispatch.properties.get(e as usize)).map(|def| &*def.name);
                            // Propagated (not just `Ok(false)`) so the bundle-level catch-all
                            // logs this bundle's full raw bytes -- needed to reproduce this
                            // class of failure byte-exact via `replay_bundle.rs`, since the
                            // AsciiFmt preview elsewhere is lossy/ambiguous around literal
                            // quote bytes in the data (confirmed the hard way: hand-parsing
                            // one back to bytes silently produced 640 bytes instead of the
                            // declared 88).
                            return Err(io::Error::new(e.kind(), format!(
                                "Entity property (failed to decode, stopping bundle): ({entity_id}) exposed_id={exposed_id:?} name={name:?}: {e}")));
                        }
                    }
                }

                let reason = self.dispatch_miss_reason();
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity property (no dispatch table: {reason}): msg#{} {:?}", id - id::ENTITY_PROPERTY.first, elt.element);
                return Ok(false);
            }
            id => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                error!(%addr, id, request_id = ?elt.request_id, "<- Unknown element: {:?}", elt.element);
                return Ok(false);
            }
        }

        Ok(true)

    }

}



/// Resolves a client-to-cell call's method table from the entity it names, for
/// [`CellEntityMethod`]. Borrowed per call rather than stored on [`BaseHandler`] so it
/// always reflects the current entity/type maps.
struct CellTables<'a> {
    handler: &'a BaseHandler,
    /// Substituted for a wire entity id of 0, exactly as the base app does.
    player_entity_id: u32,
}

impl CellMethodTables for CellTables<'_> {
    fn cell_methods(&self, entity_id: u32) -> Option<&[MethodDef]> {
        let entity_id = if entity_id == 0 { self.player_entity_id } else { entity_id };
        // Same two-map lookup the property/method dispatch above uses: an entity can be
        // known either from its full creation or from a `CreateEntity` type record.
        let type_id = self.handler.created_entity_types.get(&entity_id).copied()
            .or_else(|| self.handler.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
        Some(&self.handler.shared.dispatch.entity_from_id(type_id)?.cell_methods)
    }
}


#[cfg(test)]
mod position_tests {

    use super::*;

    /// A tracker with the two things every resolve needs, so each test states only what
    /// it is actually about.
    fn ready() -> PositionTracker {
        let mut t = PositionTracker::new();
        t.packed_xz_scale = Some(1.0);
        t.set_reference_exact(Vec3::new(100.0, 20.0, -50.0));
        t
    }

    /// The round trip the whole feature rests on: an upstream position, named back by
    /// sequence number, becomes the (rounded) reference.
    #[test]
    fn reference_resolves_through_sent_positions() {
        let mut t = PositionTracker::new();
        t.record_sent(7, Vec3::new(10.4, -3.6, 2.5));
        assert_eq!(t.set_reference_from_sent(7), Some(Vec3::new(10.0, -4.0, 3.0)));
        assert_eq!(t.reference_position, Some(Vec3::new(10.0, -4.0, 3.0)));
    }

    /// A reference naming a slot never observed must not silently become the origin, and
    /// must not discard the reference already in hand.
    #[test]
    fn unknown_sequence_number_leaves_reference_untouched() {
        let mut t = ready();
        let before = t.reference_position;
        assert_eq!(t.set_reference_from_sent(200), None);
        assert_eq!(t.reference_position, before);
    }

    /// `relativePosition` assigns verbatim; `relativePositionReference` rounds. Getting
    /// these the same way round would shift every position by up to half a metre.
    #[test]
    fn only_the_sequence_path_rounds() {
        let mut t = PositionTracker::new();
        t.set_reference_exact(Vec3::new(1.4, 2.6, -0.5));
        assert_eq!(t.reference_position, Some(Vec3::new(1.4, 2.6, -0.5)), "must not round");

        t.record_sent(0, Vec3::new(1.4, 2.6, -0.5));
        t.set_reference_from_sent(0);
        assert_eq!(t.reference_position, Some(Vec3::new(1.0, 3.0, 0.0)), "must round");
    }

    /// Only the player's own entity re-bases the reference. Without this test the
    /// regression is invisible: every other vehicle's detailed position would quietly
    /// clobber the origin and drag every decoded position with it.
    #[test]
    fn detailed_position_rebases_only_for_the_player_off_vehicle() {
        let mut t = ready();
        let original = t.reference_position;

        assert!(!t.detailed_position_received(42, Some(7), Vec3::new(1.0, 1.0, 1.0)),
            "another entity must not re-base");
        assert_eq!(t.reference_position, original);

        t.set_vehicle(7, 99);
        assert!(!t.detailed_position_received(7, Some(7), Vec3::new(1.0, 1.0, 1.0)),
            "the player while riding a vehicle must not re-base");
        assert_eq!(t.reference_position, original);

        t.set_vehicle(7, 0);
        assert!(t.detailed_position_received(7, Some(7), Vec3::new(1.4, 1.6, -2.5)));
        assert_eq!(t.reference_position, Some(Vec3::new(1.0, 2.0, -2.0)));
    }

    /// The origin switches to zero for a passenger, so a vehicle-relative offset is not
    /// reported as if it were measured from the player's reference.
    #[test]
    fn passenger_positions_are_vehicle_relative() {
        let mut t = ready();
        let packed = PackedXyz::pack(Vec3::new(3.0, 1.0, 4.0), 1.0);

        let on_foot = t.resolve(Some(7), &packed).expect("reference and scale are set");
        assert!((on_foot.x - 103.0).abs() < 0.5, "x was {}", on_foot.x);
        assert!((on_foot.z - (-46.0)).abs() < 0.5, "z was {}", on_foot.z);

        t.set_vehicle(7, 99);
        let riding = t.resolve(Some(7), &packed).expect("no reference needed on a vehicle");
        assert!((riding.x - 3.0).abs() < 0.5, "x was {}", riding.x);
        assert!((riding.z - 4.0).abs() < 0.5, "z was {}", riding.z);
    }

    /// Everything the resolve depends on must be able to say "I don't know" rather than
    /// produce a confident wrong coordinate.
    #[test]
    fn resolve_reports_unknown_rather_than_guessing() {
        let packed = PackedXyz::pack(Vec3::new(1.0, 2.0, 3.0), 1.0);

        let mut no_scale = PositionTracker::new();
        no_scale.set_reference_exact(Vec3::ZERO);
        assert!(no_scale.resolve(Some(7), &packed).is_none(), "no packed_xz_scale");

        let mut no_reference = PositionTracker::new();
        no_reference.packed_xz_scale = Some(1.0);
        assert!(no_reference.resolve(Some(7), &packed).is_none(), "no reference yet");

        assert!(ready().resolve(None, &packed).is_none(), "unresolved id_alias");
        assert!(ready().resolve(Some(7), &()).is_none(), "NoPos carries no position");
    }

    /// `OnGround` genuinely omits y, and must not be reported with a fabricated one.
    #[test]
    fn on_ground_has_no_altitude() {
        let t = ready();
        let w = t.resolve(Some(7), &PackedXz::pack(2.0, 3.0, 1.0)).unwrap();
        assert!(w.y.is_none(), "OnGround must not invent an altitude");
        assert!(w.to_string().contains("ground"), "rendered as {w}");
    }

    /// A space change must not carry the old space's origin into the new one.
    #[test]
    fn reset_clears_everything() {
        let mut t = ready();
        t.record_sent(3, Vec3::ONE);
        t.set_vehicle(7, 99);
        t.reset();
        assert!(t.reference_position.is_none());
        assert!(t.packed_xz_scale.is_none());
        assert_eq!(t.vehicle_of(7), 0);
        assert_eq!(t.set_reference_from_sent(3), None);
    }

}
