//! Proxy login and base app used for debugging exchanged messages.

use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{io, thread};

use tracing::{error, info, warn, info_span, trace, trace_span};

use rsa::{RsaPrivateKey, RsaPublicKey};
use flate2::read::ZlibDecoder;
use blowfish::Blowfish;

use wgtk::net::element::{DebugElementUndefined, SimpleElement};
use wgtk::net::bundle::{Bundle, NextElementReader, ElementReader};
use wgtk::net::codec::WgSocketAddrV4;

use wgtk::app::{proxy, login_proxy, base, client};
use wgtk::app::dispatch::{ScriptDispatch, MethodCall};
use wgtk::net::packet::Packet;
use wgtk::script::{Script, Value};

use wgtk::util::io::serde_pickle_de_options;

use crate::CliResult;


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
macro_rules! trace_dbg {
    ($elt:expr, $addr:expr, $ty:ty) => {{
        let e = $elt.read_simple::<$ty>()?;
        trace!(addr = %$addr, id = <$ty as SimpleElement>::ID, request_id = ?e.request_id,
            "{}: {:?}", stringify!($ty), e.element);
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
        selected_entity_id: None,
        player_entity_id: None,
        cell_player_entity_id: None,
        partial_resources: HashMap::new(),
        session_keys: HashMap::new(),
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
    selected_entity_id: Option<u32>,
    player_entity_id: Option<u32>,
    /// The `Vehicle` entity registered by the last [`client::element::CreateCellPlayer`]
    /// (id: `id::CREATE_CELL_PLAYER`, see that handling below), if any -- confirmed live
    /// (WoT v2.3.1.3, actual battle capture) that once this exists, `SelectPlayerEntity`
    /// targets *this* entity rather than `player_entity_id` (the base/`Account` entity):
    /// every property update observed after entering a battle decoded correctly only
    /// against the `Vehicle` entity's own property table, never `Account`'s. Kept
    /// separate from `player_entity_id` rather than overwriting it, since client-directed
    /// *base* entity method calls (`BASE_ENTITY_METHOD`, below) must still always target
    /// the base/`Account` entity regardless of any cell/vehicle presence.
    cell_player_entity_id: Option<u32>,
    partial_resources: HashMap<u16, PartialResource>,
    /// The session key last sent by each client, as observed on the initial handshake
    /// with the base app. When a SwitchBaseApp is intercepted and rewritten to keep the
    /// client pointed at this proxy (see below), the client itself has no reason to redo
    /// this handshake since its own view of the base app's address never changed, so the
    /// proxy replays it on the client's behalf toward the new real address instead.
    session_keys: HashMap<SocketAddr, u32>,
}

#[derive(Debug)]
struct Shared {
    base_app_addr: SocketAddrV4,
    #[allow(unused)]
    login_app_addr: SocketAddrV4,
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
                if let Err(e) = self.read_in_bundle(peer, bundle) {
                    error!(%addr, raw_packets = ?raw, "<- Error while reading bundle: {e}");
                }
            }
        }

        Ok(())

    }

}

impl BaseHandler {

    /// What `SelectPlayerEntity` (id: `id::SELECT_PLAYER_ENTITY`) actually targets --
    /// confirmed live (WoT v2.3.1.3) that once a `Vehicle` has been registered by
    /// `CreateCellPlayer`, it takes over from the base/`Account` entity for this
    /// purpose, see the doc comment on `cell_player_entity_id`.
    fn select_player_entity_id(&self) -> Option<u32> {
        self.cell_player_entity_id.or(self.player_entity_id)
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
            AvatarUpdateImplicit::ID => return trace_dbg!(elt, addr, AvatarUpdateImplicit),
            AvatarUpdateExplicit::ID => return trace_dbg!(elt, addr, AvatarUpdateExplicit),
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

                if let Some(entity_id) = self.player_entity_id {
                    // Unwrap because selected entity should exist!
                    let &(type_id, _) = self.entities.get(&entity_id).unwrap();
                    let Some(dispatch) = self.shared.dispatch.entity_from_id(type_id) else {
                        warn!(%addr, id, "-> Cell entity method (no dispatch table for entity type 0x{type_id:02X}): ({entity_id})");
                        return Ok(true);
                    };
                    let call = elt.read::<CellEntityMethod, _>(&dispatch.cell_methods)?.element.call;
                    match &call {
                        MethodCall::Known { .. } => info!(%addr, id, "-> Cell entity method: ({entity_id}) {call:?}"),
                        MethodCall::Unknown { .. } => warn!(%addr, id, "-> Cell entity method (unrecognized exposed id): ({entity_id}) {call:?}"),
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

                // Don't delete player entity if requested...
                let mut player_entity = None;
                if re.element.keep_player_on_base {
                    if let Some(player_entity_id) = self.player_entity_id {
                        player_entity = Some(self.entities.remove_entry(&player_entity_id).unwrap());
                    }
                }

                self.entities.clear();
                self.player_entity_id = None;

                // The vehicle (if any) doesn't survive a reset either -- a new battle
                // means a fresh `CreateCellPlayer` for a new vehicle id.
                self.cell_player_entity_id = None;

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
                        // Confirmed live: once this exists, `SelectPlayerEntity` targets
                        // the vehicle, not the base/`Account` entity -- see the doc
                        // comment on `cell_player_entity_id`.
                        self.cell_player_entity_id = Some(ccp.element.vehicle_id);
                    }
                    None => warn!(%addr, "<- Create cell player: no 'Vehicle' entity type in the loaded script model"),
                }

            }
            SelectEntity::ID => {
                let se = elt.read_simple::<SelectEntity>()?;
                if self.entities.contains_key(&se.element.entity_id) {
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
            CreateEntity::ID => return trace_dbg!(elt, addr, CreateEntity),
            CreateEntityDetailed::ID => return trace_dbg!(elt, addr, CreateEntityDetailed),
            CellAppSuspended::ID => return trace_dbg!(elt, addr, CellAppSuspended),
            CellAppResumed::ID => return trace_dbg!(elt, addr, CellAppResumed),
            ClientSuspensionDetectionEnabled::ID => return trace_dbg!(elt, addr, ClientSuspensionDetectionEnabled),
            EnterAoi::ID => return trace_dbg!(elt, addr, EnterAoi),
            EnterAoiOnVehicle::ID => return trace_dbg!(elt, addr, EnterAoiOnVehicle),
            LeaveAoi::ID => return trace_dbg!(elt, addr, LeaveAoi),
            TickSyncPeriodic::ID => return trace_dbg!(elt, addr, TickSyncPeriodic),
            RelativePositionReference::ID => return trace_dbg!(elt, addr, RelativePositionReference),
            RelativePosition::ID => return trace_dbg!(elt, addr, RelativePosition),
            SetVehicle::ID => return trace_dbg!(elt, addr, SetVehicle),
            SelectAliasedEntity::ID => return trace_dbg!(elt, addr, SelectAliasedEntity),
            ForcedPosition::ID => return trace_dbg!(elt, addr, ForcedPosition),
            AvatarUpdateNoAliasDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasDetailed),
            AvatarUpdateAliasDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasDetailed),
            AvatarUpdatePlayerDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdatePlayerDetailed),
            AvatarUpdateNoAliasFullPosYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasFullPosYawPitchRoll),
            AvatarUpdateNoAliasFullPosYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasFullPosYawPitch),
            AvatarUpdateNoAliasFullPosYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasFullPosYaw),
            AvatarUpdateNoAliasFullPosNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasFullPosNoDir),
            AvatarUpdateNoAliasOnGroundYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasOnGroundYawPitchRoll),
            AvatarUpdateNoAliasOnGroundYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasOnGroundYawPitch),
            AvatarUpdateNoAliasOnGroundYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasOnGroundYaw),
            AvatarUpdateNoAliasOnGroundNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasOnGroundNoDir),
            AvatarUpdateNoAliasNoPosYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasNoPosYawPitchRoll),
            AvatarUpdateNoAliasNoPosYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasNoPosYawPitch),
            AvatarUpdateNoAliasNoPosYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasNoPosYaw),
            AvatarUpdateNoAliasNoPosNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasNoPosNoDir),
            AvatarUpdateAliasFullPosYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasFullPosYawPitchRoll),
            AvatarUpdateAliasFullPosYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasFullPosYawPitch),
            AvatarUpdateAliasFullPosYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasFullPosYaw),
            AvatarUpdateAliasFullPosNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasFullPosNoDir),
            AvatarUpdateAliasOnGroundYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasOnGroundYawPitchRoll),
            AvatarUpdateAliasOnGroundYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasOnGroundYawPitch),
            AvatarUpdateAliasOnGroundYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasOnGroundYaw),
            AvatarUpdateAliasOnGroundNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasOnGroundNoDir),
            AvatarUpdateAliasNoPosYawPitchRoll::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasNoPosYawPitchRoll),
            AvatarUpdateAliasNoPosYawPitch::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasNoPosYawPitch),
            AvatarUpdateAliasNoPosYaw::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasNoPosYaw),
            AvatarUpdateAliasNoPosNoDir::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasNoPosNoDir),
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
            DetailedPosition::ID => return trace_dbg!(elt, addr, DetailedPosition),
            NestedEntityProperty::ID => return trace_dbg!(elt, addr, NestedEntityProperty),
            SliceEntityProperty::ID => return trace_dbg!(elt, addr, SliceEntityProperty),
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
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let &(type_id, _) = self.entities.get(&entity_id)?;
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
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity method (no dispatch table): msg#{} {:?}", id - id::ENTITY_METHOD.first, elt.element);
                return Ok(false);

            }
            id if id::ENTITY_PROPERTY.contains(id) => {

                // Same reasoning as `ENTITY_METHOD` above: without the correct entity's
                // own `properties.len()`, `EntityProperty::read_length` can't resolve
                // this element's framing at all (see that fn's doc comment -- unlike
                // methods, it has no generic fallback length either), so every branch
                // that can't resolve a dispatch table falls through to the same
                // unbounded, stop-reading path at the bottom.
                let dispatch = self.selected_entity_id.and_then(|entity_id| {
                    let &(type_id, _) = self.entities.get(&entity_id)?;
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
                            warn!(%addr, id, "<- Entity property (failed to decode, stopping bundle): ({entity_id}) exposed_id={exposed_id:?} name={name:?}: {e}");
                            return Ok(false);
                        }
                    }
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity property (no dispatch table): msg#{} {:?}", id - id::ENTITY_PROPERTY.first, elt.element);
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

