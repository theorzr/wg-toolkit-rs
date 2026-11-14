//! Proxy login and base app used for debugging exchanged messages.

mod python;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::time::Instant;
use std::path::Path;
use std::sync::Arc;
use std::io;

use tracing::{debug, error, info, warn, info_span, trace, trace_span};
use flate2::read::ZlibDecoder;
use blowfish::cipher::KeyInit;
use blowfish::Blowfish;
use glam::Vec3;

use wgtk::net::element::{DebugElementUndefined, DebugElementVariable16, DebugElementVariable32, SimpleElement, REPLY_ID};
use wgtk::net::bundle::{Bundle, NextElementReader, ElementReader};
use wgtk::app::dispatch::{ScriptDispatch, MethodCall, MethodDef, PropertyDef};
use wgtk::app::math::{PackedXyz, PackedXz, calculate_reference_position};
use wgtk::app::base::element::{CellMethodTables, BaseAppLoginReply};
use wgtk::app::proxy::capture::CaptureWriter;
use wgtk::app::{proxy, base, client};
use wgtk::pickle;
use wgtk::net::codec::WgSocketAddrV4;
use wgtk::script::{Script, Value};
use wgtk::app::nrl::NrlTree;

use crate::{init_tracing, load_script, AppProxyArgs, CliResult};
use crate::link::{ClientHost, LinkEvent, LinkStream, Message, LINK_POLL};


/// Tracing target for the Python-level descriptions (see [`python`]), so they can be
/// filtered on their own: `RUST_LOG=wot::python=info`.
const PYTHON_TARGET: &str = "wot::python";


pub fn run(args: AppProxyArgs, game_dir: &Path) -> CliResult<()> {

    init_tracing("proxy-trace.jsonl")?;

    let script = load_script(game_dir)?;
    let link = LinkStream::connect(args.login_link)
        .map_err(|e| format!("Failed to link with the login proxy: {e}"))?;
    BaseProxy::new(args.bind, args.real_login_app, script, args.capture.capture.as_deref(), link)?.run();

    Ok(())

}

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

/// Follow a decoded compressed path to the container it addresses, for applying an
/// update to [`BaseHandler::entity_properties`].
///
/// The path's first step is always the top-level property name; the rest descend through
/// dicts (`Field`) and sequences (`Index`). Returns `None` when the path doesn't fit the
/// value we hold -- a stale or never-seeded property -- rather than inventing structure,
/// since a wrong array length is worse than a missing one (see the caller).
fn resolve_path<'v>(
    properties: &'v mut HashMap<Arc<str>, Value>,
    path: &[client::element::EntityPropertyPathStep],
) -> Option<&'v mut Value> {

    use client::element::EntityPropertyPathStep as Step;

    let (Step::Field(name), rest) = path.split_first()? else {
        return None;  // A top-level property is always addressed by name.
    };

    let mut value = properties.get_mut(name)?;

    for step in rest {
        value = match (step, value) {
            (Step::Field(name), Value::Dict(dict)) => dict.get_mut(name)?,
            (Step::Index(index), Value::Seq(items)) => items.get_mut(*index as usize)?,
            _ => return None,
        };
    }

    Some(value)

}

/// Hex-encode bytes for embedding in a log line (used for raw payloads that failed to
/// decode into anything more structured, so they still end up in the protocol trace).
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Read an element as `$ty` and log it at TRACE, with structured `id`/`request_id`
/// fields. Used for elements whose framing is known, so the bundle can continue past
/// them, whatever their body decodes to.
///
/// Like every other event here, the same call feeds both the console (filtered by
/// `RUST_LOG`) and the always-on `proxy-trace.jsonl` file (see `init_tracing`), so there is no
/// separate dump path to keep in sync.
macro_rules! trace_dbg {
    ($elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        trace!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
            "{}: {:?}", stringify!($ty), e.element);
        Ok(true)
    }};
}

/// Like [`trace_dbg`], but for elements that still have no real codec: logs the body at
/// INFO so samples accumulate in the session logs rather than vanishing at TRACE.
///
/// These elements are rare, a few per session at most, so a layout has to be worked out
/// from samples gathered over many sessions. Logging them at INFO keeps every sample in
/// the session logs instead of losing it at TRACE.
///
/// Capped per element id so that one of these turning out to be high-rate (several are
/// named "periodic" or "heartbeat") cannot drown the log: past the cap it falls back to
/// TRACE. The census still counts every one, so the cap never hides how often it fires.
macro_rules! sample_dbg {
    ($self:expr, $elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        if $self.sample_budget(<$ty as SimpleElement>::ID) {
            info!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
                "UNDECODED {}: {:?}", stringify!($ty), e.element);
        } else {
            trace!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
                "{}: {:?}", stringify!($ty), e.element);
        }
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
/// Ignoring this attributes the following properties to the wrong entity without any
/// decode error: index 13 is `Avatar`'s `denunciationsLeft` but `Vehicle`'s
/// `gunAnglesPacked`, and both decode cleanly from the same bytes.
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


/// The base app of the proxy, forwarding the clients of the login proxy to their real
/// base app.
#[derive(Debug)]
pub struct BaseProxy {
    app: proxy::App,
    handler: BaseHandler,
    addr: SocketAddrV4,
    /// The link with the login proxy, none once stopped by it.
    link: Option<LinkStream>,
}

impl BaseProxy {

    /// The base proxy at this address, taking the clients of the login proxy it is
    /// linked with. The real login app only serves to check `SwitchBaseApp` addresses.
    pub fn new(
        addr: SocketAddrV4,
        real_login_app: SocketAddrV4,
        script: Script,
        capture_path: Option<&Path>,
        link: LinkStream,
    ) -> CliResult<Self> {

        let mut app = proxy::App::new(addr.into())
            .map_err(|e| format!("Failed to bind base app: {e}"))?;

        if let Some(capture_path) = capture_path {
            let capture = CaptureWriter::create(capture_path)
                .map_err(|e| format!("Failed to create packet capture at {}: {e}", capture_path.display()))?;
            app.set_capture(capture);
            info!("Capturing to: {}", capture_path.display());
        }

        let handler = BaseHandler::new(Shared {
            base_app: addr,
            real_login_app,
            dispatch: ScriptDispatch::new(script),
        });

        Ok(Self { app, handler, addr, link: Some(link) })

    }

    pub fn run(mut self) {

        let _span = info_span!("base").entered();
        info!("Running on: {}", self.addr);

        loop {
            self.link_events();
            let event = self.app.poll_until(Instant::now() + LINK_POLL);
            self.handler.handle_event(&mut self.app, event);
        }

    }

    /// Handle the events of the link with the login proxy, keeping the clients it hands
    /// over until they connect.
    fn link_events(&mut self) {

        let Some(link) = &self.link else { return };

        while let Some(event) = link.try_event() {
            match event {
                LinkEvent::Linked(()) => {
                    if let Err(e) = link.send(&Message::BaseStatus { load: 0 }) {
                        warn!("Failed to register with the login proxy: {e}");
                    }
                }
                LinkEvent::Message((), Message::ClientHost { request_id, client }) => {
                    let hosted = pending_client(&client)
                        .map(|pending_client| self.handler.pending_clients.insert(client.addr, pending_client))
                        .is_some();
                    let message = Message::ClientHosted { request_id, base_app: hosted.then_some(self.addr) };
                    if let Err(e) = link.send(&message) {
                        warn!(addr = %client.addr, "Failed to answer the login proxy: {e}");
                    }
                }
                LinkEvent::Message((), Message::BaseStop) => {
                    info!("Stopped by the login proxy, no longer taking clients");
                    self.link = None;
                    return;
                }
                LinkEvent::Message((), message) => warn!("Received an unexpected message from the login proxy: {message:?}"),
                LinkEvent::Unlinked(()) => {}
            }
        }

    }

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
        let reference = calculate_reference_position(sent);
        self.reference_position = Some(reference);
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
pub struct BaseHandler {
    shared: Shared,
    /// Clients the login proxy handed over, by their address.
    pending_clients: HashMap<SocketAddr, PendingClient>,
    /// Same idea as `pending_clients`, but keyed by IP only: after a SwitchBaseApp, the
    /// client tears down its whole connection and reconnects with a fresh, unpredictable
    /// local port (the real BigWorld client's disconnect + reconnect behaviour on this
    /// message), so we can't know the exact address to expect in advance the way a
    /// normal post-login connection lets us.
    pending_switches: HashMap<IpAddr, PendingClient>,
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
    /// `CreateEntity`/`CreateEntityDetailed` carry the entity's declared
    /// `entity_type_id`, as authoritative as the one `CreateBasePlayer` puts into
    /// `entities`, so every decoder may use it, including `ENTITY_PROPERTY` and
    /// `ENTITY_METHOD`. Those two derive the element's wire length from the table (see
    /// `EntityMethod::read_length`), so a wrong type would mis-frame the rest of the
    /// bundle; the type here is not a guess, which is what makes that safe.
    ///
    /// A mis-framed bundle can corrupt a downstream `SwitchBaseApp` into a garbage
    /// address. That arm has its own checks against it (the `0.x.x.x` / port-0 /
    /// `/16`-match tests) rather than relying on framing always being right.
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
    /// The current value of every client-visible property this proxy has seen set, per
    /// entity.
    ///
    /// A compressed property path is only decodable against the *live* length of every
    /// `Array` it crosses: the length decides how many bits the index and range fields
    /// occupy. Without a tracked value for an array, a path through it is reported as
    /// undecodable rather than guessed at.
    ///
    /// Seeded from each entity's creation payload (`CreateEntity::client_properties`),
    /// from `updateEntity` runs, and from whole-property updates (`ENTITY_PROPERTY`), then
    /// kept current by the nested and slice updates themselves.
    entity_properties: HashMap<u32, HashMap<Arc<str>, Value>>,
    /// How many times each element id was seen, inbound and outbound, and how many have
    /// been counted since the last report.
    ///
    /// This is coverage instrumentation, not protocol state. The list of ids still
    /// decoded by a `DebugElement*` placeholder is only a work list next to how much
    /// traffic each one actually carries, and `trace_dbg!` logs at TRACE, so an
    /// INFO-level session otherwise says nothing about them. See [`Self::count_element`].
    /// The Python-level view of account commands, tokens and streams: see
    /// [`python::Requests`].
    requests: python::Requests,
    census_in: HashMap<u8, u32>,
    census_out: HashMap<u8, u32>,
    census_pending: u32,
    /// BaseApp peers whose `baseAppLogin` reply has already been seen.
    ///
    /// `baseAppLogin` is the first request a client makes on a fresh connection, so its
    /// reply is the first reply that connection carries. That ordering is how the reply
    /// is identified, because the request itself is sent unencrypted and fails
    /// decryption here, leaving no request id to pair with. See
    /// [`wgtk::app::base::element::BaseAppLoginReply`].
    base_login_seen: HashSet<SocketAddr>,
    /// How many bodies have been surfaced at INFO per element id, for [`sample_dbg`]'s cap.
    sample_counts: HashMap<u8, u32>,
    /// Each connection's NRL tree, which dynamic-component updates decode against.
    nrl: HashMap<SocketAddr, NrlTree>,
}

#[derive(Debug)]
pub struct Shared {
    pub base_app: SocketAddrV4,
    /// The real login app this proxy forwards to. Used to sanity-check a `SwitchBaseApp`
    /// address against the operator's own network -- see that element's handling.
    pub real_login_app: SocketAddrV4,
    /// The script model's dynamic dispatch tables, resolved once at startup -- see
    /// `wgtk::app::script::ScriptDispatch`.
    pub dispatch: ScriptDispatch,
}

#[derive(Debug)]
struct PendingClient {
    base_app: SocketAddrV4,
    blowfish: Arc<Blowfish>,
}

/// The client of the login proxy to forward to its real base app, if valid.
fn pending_client(client: &ClientHost) -> Option<PendingClient> {
    let Some(real_base_app) = client.real_base_app else {
        warn!(addr = %client.addr, "Refused a client handed without its real base app");
        return None;
    };
    match Blowfish::new_from_slice(&client.blowfish_key) {
        Ok(blowfish) => Some(PendingClient {
            base_app: real_base_app,
            blowfish: Arc::new(blowfish),
        }),
        Err(_) => {
            warn!(addr = %client.addr, "Refused a client with an invalid blowfish key: {:?}", client.blowfish_key);
            None
        }
    }
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

impl BaseHandler {

    /// Handle an event of the proxy app, live or replayed.
    pub fn handle_event(&mut self, app: &mut proxy::App, event: proxy::Event) {
        match event {
            proxy::Event::Timeout | 
            proxy::Event::CaptureEnd(_) => {}
            proxy::Event::IoError(error) => {
                if let Some(addr) = error.addr {
                    warn!(%addr, "Failed to talk with a peer: {}", error.error);
                } else {
                    warn!("Failed to use the socket: {}", error.error);
                }
            }
            proxy::Event::UnknownPeer(unknown) => {
                if let Some(config) = self.accept_peer(unknown.addr) {
                    if let Err(e) = app.accept_peer(unknown.addr, config) {
                        warn!(addr = %unknown.addr, "Failed to accept a peer: {e}");
                    }
                }
            }
            proxy::Event::InvalidPacketEncryption(invalid) => {
                if let Some(peer) = app.peer(invalid.addr) {
                    self.receive_invalid_packet_encryption(peer, invalid.direction);
                }
            }
            proxy::Event::Bundle(bundle) => {
                if let Some(peer) = app.peer(bundle.addr) {
                    self.receive_bundle(peer, bundle.bundle, bundle.direction);
                }
            }
        }
    }

    fn accept_peer(&mut self, addr: SocketAddr) -> Option<proxy::PeerConfig> {

        if let Some(pending_client) = self.pending_clients.remove(&addr) {
            info!(%addr, real_base_app = %pending_client.base_app, "Forwarding a new peer");
            return Some(proxy::PeerConfig {
                real_addr: SocketAddr::V4(pending_client.base_app),
                blowfish: Some(pending_client.blowfish),
            });
        }

        // The client tears down and reconnects from a fresh, unpredictable local port
        // after a SwitchBaseApp, so we can't know the exact address to
        // expect in advance -- only match by IP, keyed at switch time (see the
        // SwitchBaseApp element handling below).
        if let Some(pending_client) = self.pending_switches.remove(&addr.ip()) {
            info!(%addr, real_base_app = %pending_client.base_app, "Forwarding a peer reconnected after a switch");
            return Some(proxy::PeerConfig {
                real_addr: SocketAddr::V4(pending_client.base_app),
                blowfish: Some(pending_client.blowfish),
            });
        }

        warn!(%addr, "Refused an unknown peer");
        None

    }

    fn receive_invalid_packet_encryption(&mut self, peer: proxy::Peer, direction: proxy::PacketDirection) {
        // One outbound packet per BaseApp connection is expected to fail here: the
        // client's `baseAppLogin` request, which the client sends unencrypted before it
        // enables Blowfish on the channel (`BaseAppLoginRequestProtocol::sendRequest`).
        // Decrypting plaintext fails, which is not a fault. Every later packet is
        // encrypted and decrypts normally.
        //
        // Logged at DEBUG for a peer whose login reply has not been seen yet. Anything
        // failing after that reply is a genuine problem and stays at ERROR.
        let addr = peer.addr();
        if self.base_login_seen.contains(&addr) {
            error!(%addr, "Failed to decrypt a packet: ({direction:?})");
        } else {
            debug!(%addr, "Could not decrypt the pre-login packet ({direction:?}): \
                expected, baseAppLogin is sent unencrypted");
        }
    }

    fn receive_bundle(&mut self, peer: proxy::Peer, bundle: Bundle, direction: proxy::PacketDirection) {

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
                if let Err(e) = self.read_in_bundle(peer, bundle) {
                    error!(%addr, raw_packets = ?raw, "<- Error while reading bundle: {e}");
                }
            }
        }

    }

}

impl BaseHandler {

    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            pending_clients: HashMap::new(),
            pending_switches: HashMap::new(),
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
            entity_properties: HashMap::new(),
            requests: python::Requests::default(),
            census_in: HashMap::new(),
            census_out: HashMap::new(),
            census_pending: 0,
            base_login_seen: HashSet::new(),
            sample_counts: HashMap::new(),
            nrl: HashMap::new(),
        }
    }

    /// Whether this element id still has room to have a body logged at INFO.
    ///
    /// Twenty samples is far more than enough to recover a layout, and well short of
    /// anything that would bury the rest of the log.
    fn sample_budget(&mut self, id: u8) -> bool {
        let seen = self.sample_counts.entry(id).or_insert(0);
        *seen += 1;
        *seen <= 20
    }

    /// Tally one element id, and periodically log the running census.
    ///
    /// Reported periodically rather than at exit because the proxy has no shutdown hook,
    /// so the last report before you stop it is the session's census. The interval is
    /// sized to report several times per battle, which is frequent enough to watch a
    /// session progress without burying the decode warnings this log exists for.
    fn count_element(&mut self, outbound: bool, id: u8) {

        {
            let census = if outbound { &mut self.census_out } else { &mut self.census_in };
            *census.entry(id).or_insert(0) += 1;
        }

        self.census_pending += 1;
        if self.census_pending >= 10_000 {
            self.census_pending = 0;
            self.log_census();
        }

    }

    pub fn log_census(&self) {
        let fmt = |census: &HashMap<u8, u32>| {
            let mut counts: Vec<(u32, u8)> =
                census.iter().map(|(&id, &n)| (n, id)).collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            counts.iter().map(|(n, id)| format!("0x{id:02X}:{n}"))
                .collect::<Vec<_>>().join(" ")
        };
        info!("element census <- {}", fmt(&self.census_in));
        info!("element census -> {}", fmt(&self.census_out));
    }

    /// Seed [`Self::entity_properties`] from an entity-creation payload's initial property
    /// values, so a compressed path crossing one of this entity's `Array`s has a real
    /// length to resolve against from the very first update rather than only after the
    /// server happens to resend the whole property.
    ///
    /// `decode` is the create element's own `client_properties`, passed as a closure only
    /// because the property table it needs is looked up here, from the type id.
    ///
    /// A run that stops early is logged, not dropped: whatever decoded before the stop is
    /// still the server's own state, and for a `Vehicle` -- whose table is short by its
    /// dynamic components -- the prefix is the only seeding there is.
    fn seed_entity_properties(
        &mut self,
        addr: SocketAddr,
        entity_id: u32,
        type_id: u16,
        decode: impl FnOnce(&[PropertyDef]) -> client::element::ChangedProperties,
    ) {

        let Some(dispatch) = self.shared.dispatch.entity_from_id(type_id) else { return };
        let decoded = decode(&dispatch.properties);

        // Worth a warning rather than silence: this is the same exposed-id table the rest
        // of dispatch relies on, so a mismatch here is evidence about that table.
        if let Some(stop) = &decoded.stopped {
            // The ids that *did* decode are the diagnosis, not noise: they say whether the
            // run was walking the table correctly right up to the stop (so the table is
            // simply short) or had already drifted (so an earlier property's length is
            // wrong). See `ChangedProperties` for why ascending ids make that readable.
            let got: Vec<String> = decoded.values.iter()
                .map(|(id, name, _)| format!("{id}:{name}")).collect();
            warn!(%addr, rest = %hex(&stop.rest),
                "entity {entity_id} (type 0x{type_id:02X}) initial properties stopped early: \
                 kept {} of {} [{}], {}",
                decoded.values.len(), stop.expected, got.join(" "), stop.reason);
        }

        if decoded.values.is_empty() {
            return;
        }

        let names: Vec<&str> = decoded.values.iter().map(|(_, name, _)| &**name).collect();
        trace!(%addr, "entity {entity_id} initial properties: {names:?}");

        let tracked = self.entity_properties.entry(entity_id).or_default();
        for (_, name, value) in decoded.values {
            tracked.insert(name, value);
        }

    }

    /// Apply a decoded slice update to the tracked value of the entity's property, so the
    /// `Array` it targets keeps its true length for later compressed-path decodes.
    ///
    /// A no-op when the property was never seen whole: the decoder guesses lengths for it
    /// either way, and inventing a baseline here would turn a guess into recorded fact.
    fn apply_slice(
        &mut self,
        entity_id: u32,
        path: &[client::element::EntityPropertyPathStep],
        start: u32,
        end: u32,
        values: &[Value],
    ) {

        let Some(properties) = self.entity_properties.get_mut(&entity_id) else { return };
        let Some(Value::Seq(items)) = resolve_path(properties, path) else { return };

        let (start, end) = (start as usize, end as usize);
        if start > end || end > items.len() {
            // The decode disagrees with the length we hold, so one of them is wrong. Drop the
            // tracked value rather than splice at a bogus offset -- guessing again is
            // recoverable, a silently corrupted array is not.
            warn!("entity {entity_id} {path:?}: slice [{start}..{end}] outside tracked length {}, dropping tracked value",
                items.len());
            if let Some(client::element::EntityPropertyPathStep::Field(name)) = path.first() {
                let name = Arc::clone(name);
                properties.remove(&name);
            }
            return;
        }

        items.splice(start..end, values.iter().cloned());

    }

    /// Apply a decoded nested (single-field) update to the tracked value, same contract as
    /// [`Self::apply_slice`].
    fn apply_nested(
        &mut self,
        entity_id: u32,
        path: &[client::element::EntityPropertyPathStep],
        value: &Value,
    ) {

        let Some(properties) = self.entity_properties.get_mut(&entity_id) else { return };
        let Some(slot) = resolve_path(properties, path) else { return };

        *slot = value.clone();

    }

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
    /// In battle the player entity is the `Avatar`: entering a battle fires a second
    /// `CreateBasePlayer`, with `entity_type_id=2`. The vehicle is a separate entity.
    ///
    /// Choosing the wrong one fails quietly rather than loudly, because the two tables
    /// overlap at the same indices with different sizes. `Avatar[19]`
    /// `ownVehicleAuxPhysicsData` is `Fixed(8)` while `Vehicle[19]` `debuff` is
    /// `Fixed(4)`; reading the first as the second leaves four bytes that are then taken
    /// as the next element id, and those often land on other plausible `Vehicle`
    /// properties, which makes the wrong table look right.
    ///
    /// The `Vehicle` is still registered in `entities` by `CreateCellPlayer`, so an
    /// explicit `SelectEntity` naming it resolves its own dispatch table.
    fn select_player_entity_id(&self) -> Option<u32> {
        self.player_entity_id
    }

    fn read_out_bundle(&mut self, mut peer: proxy::Peer, bundle: Bundle) -> io::Result<()> {

        // Every event traced underneath this span (down to the trace file too, see
        // `init_tracing`) is tagged with `out` for free, alongside the `base` span already
        // entered around this whole handler's thread in `run()`.
        let _span = trace_span!("out").entered();

        let mut reader = bundle.element_reader();
        while let Some(elt) = reader.next() {
            match elt {
                NextElementReader::Element(elt) => {
                    self.count_element(true, elt.id());
                    if !self.read_out_element(&mut peer, elt)? {
                        break;
                    }
                }
                NextElementReader::Reply(reply) => {
                    // `InterfaceElement::REPLY` is a variable-length message with a
                    // 4-byte length, which is exactly what `DebugElementVariable32` reads,
                    // so taking the body verbatim leaves the reader on the next element.
                    // An unknown reply type is a decoding gap, not a framing one, so it
                    // must not stop the bundle. `UDPBundleProcessor::dispatchMessages`
                    // draws the same line: it warns when a reply handler leaves bytes
                    // behind, and keeps iterating.
                    //
                    // Both sides log their request id, so pairing a reply with the request
                    // it answers is a join over the trace, not state to keep here.
                    let request_id = reply.request_id();
                    let body = reply.read_simple::<DebugElementVariable32<REPLY_ID>>()?;
                    info!(addr = %peer.addr(), request_id, "-> Reply (undecoded): {body:?}");
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
            // Elements with a known framing but no codec yet: read through their
            // placeholder type so the bundle keeps being read, with their raw content
            // sampled.
            PingDatacenter::ID => return sample_dbg!(self, elt, addr, PingDatacenter),
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
            RequestEntityUpdate::ID => {
                // At INFO, with `rest` as hex. Every observed request carries no stamps,
                // so a populated stamp list, or a ragged `rest`, is worth seeing.
                let e = elt.read_simple::<RequestEntityUpdate>()?;
                let r = &e.element;
                info!(%addr, id = RequestEntityUpdate::ID, rest = %hex(&r.rest),
                    "-> Request entity update: ({}) stamps={:?}", r.entity_id, r.stamps);
                return Ok(true);
            }
            NrlMsgToCell::ID => {
                // At INFO, unlike the other `Nrl*` elements, with `rest` as hex: the
                // trailing value is the part no schema describes, so it is the part worth
                // seeing. On node 1 it is the server tick (see `NrlMsgToCell`).
                let e = elt.read_simple::<NrlMsgToCell>()?;
                let NrlMsgToCell(stream) = &e.element;
                info!(%addr, id = NrlMsgToCell::ID, messages = stream.messages.len(),
                    rest = %hex(&stream.rest),
                    "-> Nrl msg to cell: {:?}", stream.messages);
                return Ok(true);
            }
            AvatarUpdateWardImplicit::ID => return sample_dbg!(self, elt, addr, AvatarUpdateWardImplicit),
            AvatarUpdateWardExplicit::ID => return sample_dbg!(self, elt, addr, AvatarUpdateWardExplicit),
            AckWardPhysicsCorrection::ID => return sample_dbg!(self, elt, addr, AckWardPhysicsCorrection),
            RestoreClientAck::ID => return sample_dbg!(self, elt, addr, RestoreClientAck),
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
                        MethodCall::Known { name, args, .. } => {
                            info!(%addr, id, "-> Base entity method: ({entity_id}) {call:?}");
                            if let Some(meaning) = self.requests.on_base_call(name, args) {
                                info!(target: PYTHON_TARGET, %addr, entity_id, "-> {meaning}");
                            }
                        }
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
                    self.count_element(false, elt.id());
                    if !self.read_in_element(&mut peer, elt)? {
                        break;
                    }
                }
                NextElementReader::Reply(reply) => {
                    // `InterfaceElement::REPLY` is a variable-length message with a
                    // 4-byte length, which is exactly what `DebugElementVariable32` reads,
                    // so taking the body verbatim leaves the reader on the next element.
                    // An unknown reply type is a decoding gap, not a framing one, so it
                    // must not stop the bundle. `UDPBundleProcessor::dispatchMessages`
                    // draws the same line: it warns when a reply handler leaves bytes
                    // behind, and keeps iterating.
                    //
                    // Both sides log their request id, so pairing a reply with the request
                    // it answers is a join over the trace, not state to keep here.
                    let addr = peer.addr();
                    let request_id = reply.request_id();
                    let body = reply.read_simple::<DebugElementVariable32<REPLY_ID>>()?;

                    // Read as raw bytes first, then typed: a reply carries no element id,
                    // so nothing in it says what shape it is. The one known shape is
                    // `baseAppLogin`'s, recognised by position rather than content: the
                    // first reply on a connection, four bytes long. Guarding on both
                    // keeps a later four-byte reply to some *other* request from being
                    // mislabelled a session key.
                    let first_on_peer = self.base_login_seen.insert(addr);
                    match <[u8; 4]>::try_from(&body.data[..]) {
                        Ok(bytes) if first_on_peer => {
                            let reply = BaseAppLoginReply { session_key: u32::from_le_bytes(bytes) };
                            info!(%addr, request_id,
                                "<- Base app login reply: session key 0x{:08X}", reply.session_key);
                        }
                        _ => info!(%addr, request_id, "<- Reply (undecoded): {body:?}"),
                    }
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
                    "<- Update frequency: {} Hz, unknown: {}, game time: {}", ufn.element.frequency, ufn.element.unknown, ufn.element.game_time);
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
                // Same reasoning as `created_entity_types`: entity ids are reused across
                // spaces, and a stale `Array` length carried into a new space would be
                // worse than none. The decoder trusts a tracked length outright, whereas
                // without one it reports the path as undecodable. The player is not
                // preserved either: its properties are re-sent on entering the new space.
                self.entity_properties.clear();
                // The alias table is per-connection state in the real client and is wiped
                // by the same reset that clears `id_`/`spaceID_`/`controlledEntities_`
                // (`memset( idAlias_, 0, sizeof( idAlias_ ) )`). Aliases are small ints
                // reused from scratch in the next space, so keeping them across a reset
                // would silently resolve a new space's alias to a dead entity from the
                // old one, for the same reason `entities` is cleared above.
                self.id_aliases.clear();
                // Request ids are allocated per account entity, so an answer to an
                // outstanding request cannot arrive across a reset, and a reused id would
                // otherwise be matched to the old request.
                self.requests.clear();
                self.player_entity_id = None;

                // Restore player entity!
                if let Some((player_entity_id, player_entity)) = player_entity {
                    self.entities.insert(player_entity_id, player_entity);
                    self.player_entity_id = Some(player_entity_id);
                }

                // `selected_entity_id` may name any entity (see `SelectEntity`/
                // `SelectAliasedEntity`), and whatever it named is gone now that
                // `entities` was cleared. Keeping it would leave the `ENTITY_METHOD`/
                // `ENTITY_PROPERTY` arms holding a dangling id into `entities`. A panic
                // there would kill this connection's poll worker thread with nothing to
                // report it, since no `JoinHandle::join()` surfaces it (see `ThreadPoll`).
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

                let ccp = elt.read_simple::<CreateCellPlayer>()?.element;
                info!(%addr, id = CreateCellPlayer::ID, unk_flag = ccp.unk_flag, unk_short = ccp.unk_short,
                    "<- Create cell player: space {}, vehicle {}, position {:?}, direction {:?}, packed xz scale {}",
                    ccp.space_id, ccp.vehicle_id, ccp.position, ccp.direction, ccp.packed_xz_scale);

                // The rest is the player entity's own cell-hosted, client-visible properties,
                // decoded against its type -- the player entity being the one this cell
                // part is created for.
                let player = self.player_entity_id.and_then(|entity_id| {
                    let &(type_id, _) = self.entities.get(&entity_id)?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });
                match player {
                    Some((entity_id, dispatch)) => match ccp.read_cell_data(dispatch) {
                        Ok(data) => {
                            info!(%addr, id = CreateCellPlayer::ID, components_count = data.components_count,
                                "<- Create cell player: ({entity_id}) {:?} {:?}", data.properties, data.components_properties);
                            // Seeds the tracked values that compressed property paths are
                            // decoded against, see `entity_properties`.
                            let tracked = self.entity_properties.entry(entity_id).or_default();
                            for properties in [data.properties, data.components_properties] {
                                if let Value::Dict(properties) = properties {
                                    tracked.extend(properties);
                                }
                            }
                        }
                        Err(e) => warn!(%addr, id = CreateCellPlayer::ID, data = %hex(&ccp.cell_data),
                            "<- Create cell player: ({entity_id}) cell data failed to decode: {e}"),
                    },
                    None => warn!(%addr, id = CreateCellPlayer::ID, data = %hex(&ccp.cell_data),
                        "<- Create cell player: no dispatch table for the player entity"),
                }

                // The space's packed-position scale, without which no volatile position can
                // be resolved at all, plus the initial reference: `createCellPlayer` ends
                // with `detailedPositionReceived( id_, ... )`, so entering a space bases the
                // reference on the player's own starting position.
                self.position.packed_xz_scale = Some(ccp.packed_xz_scale);
                if let Some(id) = self.player_entity_id {
                    self.position.detailed_position_received(id, Some(id), ccp.position);
                }

                // `vehicle_id` is a distinct `Vehicle` entity, not the player's own base
                // entity (see `client::element::CreateCellPlayer::vehicle_id`). Register it
                // so a later `SelectEntity` targeting it resolves the `Vehicle` dispatch
                // table rather than whatever entity was selected before.
                match self.shared.dispatch.entity_from_name("Vehicle") {
                    Some((vehicle_type_id, _)) => {
                        self.entities.insert(ccp.vehicle_id, (vehicle_type_id, Value::Dict(BTreeMap::new())));
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
                // valid for a real game server. Seeing one means the bundle's read position
                // desynced on an earlier element and these 9 bytes are garbage, not a real
                // `SwitchBaseApp`. Trusting it has two consequences. The garbage address
                // becomes this peer's reconnect target, so the client's real reconnect is
                // forwarded somewhere it can never arrive and the session drops. And
                // sending to a port-0 address fails with `EINVAL`, which is not a
                // `ConnectionReset`, so it is not handled as a per-peer drop: it propagates
                // out of the base app's `run()` loop and stops request handling for every
                // connected client. Bail out loudly instead.
                // The `0.x.x.x`/port-0 checks below are necessary but not sufficient: a
                // desynced read can yield an address that passes both, and adopting it as
                // the peer's real address kills the connection (inbound stops dead while
                // the client resends its session key forever). A genuine base-app switch
                // stays inside the operator's network, so require the same /16 as the real
                // login app -- the tightest bound derivable from this proxy's own config
                // without hardcoding an IP range.
                let real_login_octets = self.shared.real_login_app.ip().octets();
                let new_octets = sba.element.base_addr.addr.ip().octets();
                let same_network = new_octets[..2] == real_login_octets[..2];

                if !same_network {
                    error!(%addr, id = SwitchBaseApp::ID, request_id = ?sba.request_id,
                        "<- Switch base app: address {:?} is outside the real login app's network ({}), \
                         likely bundle desync upstream -- ignoring",
                        sba.element.base_addr, self.shared.real_login_app.ip());
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
                // this, from an unpredictable new local port, so pre-register the session
                // by IP alone for `accept_peer` to pick up.
                if let Some(blowfish) = peer.blowfish_arc() {
                    self.pending_switches.insert(addr.ip(), PendingClient {
                        base_app: sba.element.base_addr.addr,
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
                let our_addr = WgSocketAddrV4::from(self.shared.base_app);
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
                let (total_len, crc32) = match pickle::from_reader(&resource.description[..]) {
                    Ok(pickle::Value::Tuple(values)) if values.len() == 2 => {
                        if let &[pickle::Value::I64(total_len), pickle::Value::I64(crc32)] = &values[..] {
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

                // What this stream answers: the command whose `RES_STREAM` result it
                // carries, or a predefined stream (see `python::Requests`).
                let answers = self.requests.on_stream_complete(res_id);
                if let Some(answers) = &answers {
                    info!(target: PYTHON_TARGET, %addr, res_id, len = actual_total_len, "<- stream for {answers} complete");
                }
                let answers = answers.unwrap_or_else(|| "unknown".to_string());

                // The full data is a zlib-compressed pickle. Decoded and logged in full
                // here (rather than written to a separate dump file) so it lands in the
                // same ordered trace as everything else.
                match pickle::from_reader(ZlibDecoder::new(&data[..])) {
                    Ok(val) => {
                        info!(%addr, res_id, answers, len = actual_total_len, crc32 = format!("0x{crc32:08X}"), python = %val,
                            "<- Resource completed");
                    }
                    Err(e) => {
                        let mut raw = Vec::new();
                        let _ = std::io::copy(&mut ZlibDecoder::new(&data[..]), &mut raw);
                        warn!(%addr, res_id, answers, len = actual_total_len, crc32 = format!("0x{crc32:08X}"), raw = hex(&raw),
                            "<- Resource completed but failed to decode as python: {e}");
                    }
                }

            }
            // Elements with a known framing, read through their own type so the bundle
            // keeps being read. Those still without a codec go through `sample_dbg!`.
            Authenticate::ID => return trace_dbg!(elt, addr, Authenticate),
            BandwidthNotification::ID => return trace_dbg!(elt, addr, BandwidthNotification),
            SetGameTime::ID => return trace_dbg!(elt, addr, SetGameTime),
            DummyPacket::ID => return sample_dbg!(self, elt, addr, DummyPacket),
            SpaceProperty::ID => return sample_dbg!(self, elt, addr, SpaceProperty),
            AddSpaceGeometryMapping::ID => {
                // At INFO, and `rest` as hex: a non-empty `rest` means the layout does not
                // fit this payload, which is worth seeing whenever it happens.
                let e = elt.read_simple::<AddSpaceGeometryMapping>()?;
                let m = &e.element;
                info!(%addr, id = AddSpaceGeometryMapping::ID, rest = %hex(&m.rest),
                    "<- Add space geometry mapping: space {} entry {} {:?} unk={} transform={:?}",
                    m.space_id, m.entry_id, m.path, m.unk, m.transform);
                return Ok(true);
            }
            RemoveSpaceGeometryMapping::ID => return sample_dbg!(self, elt, addr, RemoveSpaceGeometryMapping),
            CreateEntity::ID => {
                // Record the entity's type so a later `SelectEntity` targeting it can
                // resolve a dispatch table. This is the only source of type information
                // for entities other than the player's own base entity and vehicle, which
                // are the entities `Nested`/`SliceEntityProperty` mostly target. See
                // `created_entity_types` for why this does not go into `entities`.
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
                self.seed_entity_properties(addr, e.element.entity_id, e.element.entity_type_id,
                    |properties| e.element.client_properties(properties));
                // `ServerConnection::createEntity` ends in `detailedPositionReceived`, so a
                // create for the player's own entity re-bases the reference.
                self.position.detailed_position_received(e.element.entity_id, self.player_entity_id, e.element.position);
                trace!(%addr, id = CreateEntity::ID, request_id = ?e.request_id, "{}: {:?}", stringify!(CreateEntity), e.element);
                return Ok(true);
            }
            CreateEntityDetailed::ID => {
                let e = elt.read_simple::<CreateEntityDetailed>()?;
                self.created_entity_types.insert(e.element.entity_id, e.element.entity_type_id);
                self.seed_entity_properties(addr, e.element.entity_id, e.element.entity_type_id,
                    |properties| e.element.client_properties(properties));
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
            TickSyncPeriodic::ID => return sample_dbg!(self, elt, addr, TickSyncPeriodic),
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
                // Unresolvable alias clears the selection, as in `trace_dbg_alias`.
                let e = elt.read_simple::<SelectAliasedEntity>()?;
                let id_alias = e.element.id_alias;
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
            AvatarUpdateVolatileProperties::ID => return sample_dbg!(self, elt, addr, AvatarUpdateVolatileProperties),
            ChangeVolatilePackerType::ID => return sample_dbg!(self, elt, addr, ChangeVolatilePackerType),
            id @ (NrlCreateNode::ID | NrlUnlinkTree::ID | NrlMsgToClient::ID) => {
                let mut stream = match id {
                    NrlCreateNode::ID => elt.read_simple::<NrlCreateNode>()?.element.0,
                    NrlUnlinkTree::ID => elt.read_simple::<NrlUnlinkTree>()?.element.0,
                    _ => elt.read_simple::<NrlMsgToClient>()?.element.0,
                };
                match stream.decode_components(&self.shared.dispatch) {
                    Ok(()) => info!(%addr, id, rest = %hex(&stream.rest),
                        "<- Nrl messages: {:?}", stream.messages),
                    Err(e) => warn!(%addr, id, rest = %hex(&stream.rest),
                        "<- Nrl messages (component failed to decode: {e}): {:?}", stream.messages),
                }
                self.nrl.entry(addr).or_default().apply(&stream, &self.shared.dispatch);
            }
            NrlData::ID => {
                let data = elt.read_simple::<NrlData>()?.element;
                self.nrl.entry(addr).or_default().push_data(&data.fragment);
            }
            NrlUpdateNodeFlag::ID => {
                elt.read_simple::<NrlUpdateNodeFlag>()?;
                match self.nrl.entry(addr).or_default().update_node(&self.shared.dispatch) {
                    Ok(update) => info!(%addr, id = NrlUpdateNodeFlag::ID, "<- Nrl update: {update:?}"),
                    Err(e) => warn!(%addr, id = NrlUpdateNodeFlag::ID, "<- Nrl update (failed to decode, buffer dropped): {e}"),
                }
            }
            NrlUnlinkTreeFlag::ID => {
                elt.read_simple::<NrlUnlinkTreeFlag>()?;
                match self.nrl.entry(addr).or_default().unlink_tree_node() {
                    Ok(node_id) => info!(%addr, id = NrlUnlinkTreeFlag::ID, "<- Nrl unlink tree: {node_id}"),
                    Err(e) => warn!(%addr, id = NrlUnlinkTreeFlag::ID, "<- Nrl unlink tree (empty buffer): {e}"),
                }
            }
            NrlUpdateNode::ID => {
                let e = elt.read_simple::<NrlUpdateNode>()?.element;
                let tree = self.nrl.entry(addr).or_default();
                let mut payload = &e.payload[..];
                match tree.decode_update(&self.shared.dispatch, e.node_id, &mut payload) {
                    Ok(update) if payload.is_empty() => info!(%addr, id = NrlUpdateNode::ID, "<- Nrl update: {update:?}"),
                    Ok(update) => warn!(%addr, id = NrlUpdateNode::ID, rest = %hex(payload),
                        "<- Nrl update (bytes left): {update:?}"),
                    Err(e) => warn!(%addr, id = NrlUpdateNode::ID, "<- Nrl update (failed to decode): {e}"),
                }
            }
            NrlUnreliableMsgToClient::ID => {
                let e = elt.read_simple::<NrlUnreliableMsgToClient>()?.element;
                if let client::element::NrlUnreliableMsgToClientPayload::Raw(data) = &e.payload {
                    warn!(%addr, id = NrlUnreliableMsgToClient::ID, kind = e.kind, data = %hex(data),
                        "<- Nrl unreliable msg (unknown payload): tick {}", e.tick);
                } else {
                    trace!(%addr, id = NrlUnreliableMsgToClient::ID, "<- Nrl unreliable msg: {e:?}");
                }
            }
            ControlEntity::ID => return trace_dbg!(elt, addr, ControlEntity),
            VoiceData::ID => return sample_dbg!(self, elt, addr, VoiceData),
            RestoreClient::ID => return sample_dbg!(self, elt, addr, RestoreClient),
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

                // Same dispatch-table requirement, and same safety argument, as the
                // `SliceEntityProperty` arm below.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                let Some((entity_id, dispatch)) = dispatch else {
                    let raw = elt.read_simple::<DebugElementVariable16<{ NestedEntityProperty::ID }>>()?;
                    warn!(%addr, id, len = raw.element.data.len(), data = %hex(&raw.element.data),
                        "<- Nested entity property (no dispatch table)");
                    return Ok(true);
                };

                let config = client::element::PropertyPathConfig::with_values(
                    &dispatch.properties, self.entity_properties.get(&entity_id));

                match elt.read::<NestedEntityProperty, _>(&config)?.element {
                    NestedEntityProperty::Known { path, value } => {
                        info!(%addr, id, "<- Nested entity property: ({entity_id}) {path:?} = {value:?}");
                        self.apply_nested(entity_id, &path, &value);
                    }
                    NestedEntityProperty::Unknown { data, reason } =>
                        warn!(%addr, id, len = data.len(), data = %hex(&data),
                            "<- Nested entity property (failed to decode): ({entity_id}) {reason}"),
                }

                return Ok(true);

            }
            id @ SliceEntityProperty::ID => {

                // Resolving the compressed path needs the selected entity's own property
                // table. Safe to consult `created_entity_types` here (unlike
                // `ENTITY_PROPERTY`/`ENTITY_METHOD`): this element is always `Variable16`,
                // so its framing doesn't depend on the table and a wrong guess can only
                // produce a bad decode of this one element, never a bundle desync.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                // With no table there is nothing to decode against, but the element still
                // has to be consumed for the bundle to continue -- so read it raw and hex
                // it, which is also what makes such a case diagnosable at all.
                let Some((entity_id, dispatch)) = dispatch else {
                    let raw = elt.read_simple::<DebugElementVariable16<{ SliceEntityProperty::ID }>>()?;
                    warn!(%addr, id, len = raw.element.data.len(), data = %hex(&raw.element.data),
                        "<- Slice entity property (no dispatch table)");
                    return Ok(true);
                };

                // Tracked `Array` lengths pin the decode where it would otherwise have to
                // guess -- see `entity_properties`.
                let config = client::element::PropertyPathConfig::with_values(
                    &dispatch.properties, self.entity_properties.get(&entity_id));

                match elt.read::<SliceEntityProperty, _>(&config)?.element {
                    SliceEntityProperty::Known { path, start, end, values } => {
                        info!(%addr, id, "<- Slice entity property: ({entity_id}) {path:?} [{start}..{end}] = {values:?}");
                        self.apply_slice(entity_id, &path, start, end, &values);
                    }
                    SliceEntityProperty::Unknown { data, reason } =>
                        warn!(%addr, id, len = data.len(), data = %hex(&data),
                            "<- Slice entity property (failed to decode): ({entity_id}) {reason}"),
                }

                return Ok(true);

            }
            id @ UpdateEntity::ID => {

                // Applies to the *selected* entity, like `ENTITY_PROPERTY`
                // (`ServerConnection::updateEntity` -> `onEntityProperties(
                // selectedEntityID_, ...)`). Safe to consult `created_entity_types`: this
                // element is `Variable16`, so its framing never depends on the table.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let type_id = self.created_entity_types.get(&entity_id).copied()
                        .or_else(|| self.entities.get(&entity_id).map(|&(type_id, _)| type_id))?;
                    Some((entity_id, self.shared.dispatch.entity_from_id(type_id)?))
                });

                let Some((entity_id, dispatch)) = dispatch else {
                    let raw = elt.read_simple::<DebugElementVariable16<{ UpdateEntity::ID }>>()?;
                    warn!(%addr, id, len = raw.element.data.len(), data = %hex(&raw.element.data),
                        "<- Update entity (no dispatch table)");
                    return Ok(true);
                };

                let UpdateEntity(decoded) = elt.read::<UpdateEntity, _>(&dispatch.properties)?.element;

                if let Some(stop) = &decoded.stopped {
                    warn!(%addr, id, rest = %hex(&stop.rest),
                        "<- Update entity (stopped early): ({entity_id}) kept {} of {}, {}",
                        decoded.values.len(), stop.expected, stop.reason);
                }

                if !decoded.values.is_empty() {
                    info!(%addr, id, "<- Update entity: ({entity_id}) {}",
                        decoded.values.iter().map(|(_, name, value)| format!("{name}={value:?}"))
                            .collect::<Vec<_>>().join(", "));
                    // These are whole property values, so they are as authoritative as
                    // `ENTITY_PROPERTY`'s -- and they are the only source for properties
                    // held back from the creation payload by level of detail, which is
                    // what otherwise leaves an `Array` length unknown.
                    let tracked = self.entity_properties.entry(entity_id).or_default();
                    for (_, name, value) in decoded.values {
                        tracked.insert(name, value);
                    }
                }

                return Ok(true);

            }
            SetCellAppExtAddress::ID => {
                let e = elt.read_simple::<SetCellAppExtAddress>()?;
                info!(%addr, id = SetCellAppExtAddress::ID,
                    "<- Set cell app ext address: {}", e.element.0);
                return Ok(true);
            }
            LastProxyMessageAfterDirectCellAppConnection::ID => return sample_dbg!(self, elt, addr, LastProxyMessageAfterDirectCellAppConnection),
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
                        MethodCall::Known { name, args, .. } => {
                            info!(%addr, id, "<- Entity method: ({entity_id}) {call:?}");
                            if let Some(meaning) = self.requests.on_client_call(name, args) {
                                info!(target: PYTHON_TARGET, %addr, entity_id, "<- {meaning}");
                            }
                            true
                        }
                        MethodCall::Unknown { .. } => {
                            // Unlike `BASE_ENTITY_METHOD` (always Variable16), an
                            // `EntityMethod` whose exposed id is outside our table falls
                            // back to Variable8, at most 254 bytes. An id missing from the
                            // reconstructed `client_methods` table is most likely a
                            // dynamic-component method, and its real call can be longer
                            // than that, so the read can end mid-argument and everything
                            // after it in the bundle is misread. Stop reading the bundle
                            // rather than trust the position after a possibly truncated
                            // read.
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
                    // component (attached per-instance --
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
                            // The only unambiguous source of a property's whole value, so
                            // this is what seeds the live `Array` lengths that compressed
                            // paths are decoded against.
                            self.entity_properties.entry(entity_id).or_default()
                                .insert(Arc::clone(&p.element.name), p.element.value);
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
