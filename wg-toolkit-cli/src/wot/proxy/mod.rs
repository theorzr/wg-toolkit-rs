//! Proxy login and base app used for debugging exchanged messages.

use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use std::{fmt, io, thread};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use tracing::{error, info, warn, info_span, trace, trace_span};

use rsa::{RsaPrivateKey, RsaPublicKey};
use flate2::read::ZlibDecoder;
use blowfish::Blowfish;

use wgtk::net::element::{DebugElementUndefined, DebugElementVariable16, SimpleElement};
use wgtk::net::bundle::{Bundle, NextElementReader, ElementReader};

use wgtk::net::app::{proxy, login_proxy, base, client};
use wgtk::net::app::common::entity::Entity;
use wgtk::net::packet::Packet;

use wgtk::util::io::serde_pickle_de_options;

use crate::CliResult;
use super::r#gen;


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
        pending_clients: Mutex::new(HashMap::new()),
        pending_switches: Mutex::new(HashMap::new()),
    });

    let login_handler = LoginHandler {
        shared: Arc::clone(&shared),
    };

    let base_handler = BaseHandler {
        shared,
        next_tick: None,
        entity_types: r#gen::entity::collect_entity_types::<EntityTypeVec>().0,
        entities: HashMap::new(),
        selected_entity_id: None,
        player_entity_id: None,
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
    entity_types: Vec<Arc<EntityType>>,
    entities: HashMap<u32, Arc<EntityType>>,
    selected_entity_id: Option<u32>,
    player_entity_id: Option<u32>,
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
        base_app_addr: SocketAddrV4,
        _login_key: u32,
        _server_message: String,
    ) -> Result<SocketAddrV4, Self::Error> {

        info!(%addr, %base_app_addr, "Login success");
        self.shared.pending_clients.lock().unwrap().insert(addr, PendingClient {
            base_app_addr,
            blowfish,
        });

        // Return the proxy base app address instead of the expected one!
        Ok(self.shared.base_app_addr)

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
                if let Err(e) = self.read_in_bundle(peer, bundle) {
                    error!(%addr, "<- Error while reading bundle: {e}");
                }
            }
        }

        Ok(())

    }

}

impl BaseHandler {

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
                    let entity_type = self.entities.get(&entity_id).unwrap();
                    return (entity_type.base_entity_method)(&mut *self, addr, entity_id, elt);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "-> Base entity method (unknown selected entity): msg#{} {:?}", id - id::BASE_ENTITY_METHOD.first, elt.element);
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

    fn read_in_element(&mut self, peer: &mut proxy::Peer, mut elt: ElementReader) -> io::Result<bool> {

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

                // Restore player entity!
                if let Some((player_entity_id, player_entity)) = player_entity {
                    self.entities.insert(player_entity_id, player_entity);
                    self.player_entity_id = Some(player_entity_id);
                }

            }
            LoggedOff::ID => {
                let lo = elt.read_simple::<LoggedOff>()?;
                info!(%addr, id = LoggedOff::ID, request_id = ?lo.request_id, "<- Logged off: 0x{:02X}", lo.element.reason);
            }
            CreateBasePlayerHeader::ID => {

                let cbp = elt.read_simple_stable::<CreateBasePlayerHeader>()?;

                if let Some(entity_type) = cbp.element.entity_type_id.checked_sub(1).and_then(|i| self.entity_types.get(i as usize)) {
                    self.entities.insert(cbp.element.entity_id, Arc::clone(&entity_type));
                    self.player_entity_id = Some(cbp.element.entity_id);
                    return (entity_type.create_base_player)(&mut *self, addr, elt);
                }

                self.player_entity_id = None;
                // It's possible to skip it because its len is variable.
                let dbg = elt.read_simple::<DebugElementVariable16<0>>()?;
                warn!(%addr, id = CreateBasePlayerHeader::ID, request_id = ?dbg.request_id,
                    "<- Create base player with invalid entity type id: 0x{:02X}, {:?}",
                    cbp.element.entity_type_id, dbg.element);

            }
            CreateCellPlayer::ID => {
                let ccp = elt.read_simple::<CreateCellPlayer>()?;
                warn!(%addr, id = CreateCellPlayer::ID, request_id = ?ccp.request_id, "<- Create cell player: {:?}", ccp.element);
            }
            SelectPlayerEntity::ID => {
                let spe = elt.read_simple::<SelectPlayerEntity>()?;
                if let Some(player_entity_id) = self.player_entity_id {
                    info!(%addr, id = SelectPlayerEntity::ID, request_id = ?spe.request_id,
                        "<- Select player entity: {player_entity_id}");
                } else {
                    warn!(%addr, id = SelectPlayerEntity::ID, request_id = ?spe.request_id,
                        "<- Select player entity: no player entity")
                }
                self.selected_entity_id = self.player_entity_id;
            }
            SwitchBaseApp::ID => {

                let sba = elt.read_simple::<SwitchBaseApp>()?;
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
                        base_app_addr: sba.element.base_addr,
                        blowfish,
                    });
                } else {
                    warn!(%addr, "<- Switch base app: no blowfish key available, cannot pre-register the reconnection");
                }

                let mut real_addr_bytes = [0u8; 6];
                real_addr_bytes[..4].copy_from_slice(&sba.element.base_addr.ip().octets());
                real_addr_bytes[4..6].copy_from_slice(&sba.element.base_addr.port().to_be_bytes());

                let mut our_addr_bytes = [0u8; 6];
                our_addr_bytes[..4].copy_from_slice(&self.shared.base_app_addr.ip().octets());
                our_addr_bytes[4..6].copy_from_slice(&self.shared.base_app_addr.port().to_be_bytes());

                if !peer.patch_raw(&real_addr_bytes, &our_addr_bytes) {
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
            SelectEntity::ID => return trace_dbg!(elt, addr, SelectEntity),
            ForcedPosition::ID => return trace_dbg!(elt, addr, ForcedPosition),
            AvatarUpdateNoAliasDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdateNoAliasDetailed),
            AvatarUpdateAliasDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdateAliasDetailed),
            AvatarUpdatePlayerDetailed::ID => return trace_dbg!(elt, addr, AvatarUpdatePlayerDetailed),
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

                if let Some(entity_id) = self.selected_entity_id {
                    // Unwrap because selected entity should exist!
                    let entity_type = self.entities.get(&entity_id).unwrap();
                    return (entity_type.entity_method)(&mut *self, addr, entity_id, elt);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity method (unknown selected entity): msg#{} {:?}", id - id::ENTITY_METHOD.first, elt.element);
                return Ok(false);

            }
            id if id::ENTITY_PROPERTY.contains(id) => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, id, request_id = ?elt.request_id,
                    "<- Entity property: msg#{} {:?}", id - id::ENTITY_PROPERTY.first, elt.element);
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

    fn read_create_base_player<E>(&mut self, addr: SocketAddr, elt: ElementReader) -> io::Result<bool>
    where
        E: Entity + fmt::Debug,
    {

        use client::element::{CreateBasePlayer, id::CREATE_BASE_PLAYER};

        let cbp = elt.read_simple::<CreateBasePlayer<E>>()?;

        // The full entity data is logged as its own field (rather than written to a
        // separate `entity_<id>.txt` dump file) so it lands in the same ordered trace
        // while keeping the message itself a short, scannable one-liner.
        info!(%addr, id = CREATE_BASE_PLAYER, request_id = ?cbp.request_id, entity_data = ?cbp.element.entity_data,
            "<- Create base player: ({}) entity_type_id={}", cbp.element.entity_id, cbp.element.entity_type_id);

        Ok(true)

    }

    fn read_entity_method<E>(&mut self, addr: SocketAddr, entity_id: u32, elt: ElementReader) -> io::Result<bool>
    where
        E: Entity,
        E::ClientMethod: fmt::Debug,
    {
        use client::element::{EntityMethod, EntityMethodInner};

        let id = elt.id();
        let em = elt.read_simple::<EntityMethod<E::ClientMethod>>()?;
        match &em.element.inner {
            EntityMethodInner::Known(inner) => {
                info!(%addr, id, request_id = ?em.request_id, "<- Entity method: ({entity_id}) {:?}", inner);
            }
            EntityMethodInner::Unknown { exposed_id, data } => {
                // The exposed id table we generated from the shipped entity_defs doesn't cover
                // this one (see re-work/NOTES.md) -- `EntityMethod` still frames it correctly
                // (var8, matching the live client's own fallback), so bundle reading can safely
                // continue, we just can't decode its content.
                warn!(%addr, id, exposed_id, request_id = ?em.request_id,
                    "<- Entity method (unknown exposed id): ({entity_id}) raw: {}", hex(data));
            }
        }
        Ok(true)
    }

    fn read_base_entity_method<E>(&mut self, addr: SocketAddr, entity_id: u32, elt: ElementReader) -> io::Result<bool>
    where
        E: Entity,
        E::BaseMethod: fmt::Debug,
    {
        use base::element::{BaseEntityMethod, BaseEntityMethodInner};

        let id = elt.id();
        let em = elt.read_simple::<BaseEntityMethod<E::BaseMethod>>()?;
        match &em.element.inner {
            BaseEntityMethodInner::Known(inner) => {
                info!(%addr, id, request_id = ?em.request_id, "-> Base entity method: ({entity_id}) {:?}", inner);
            }
            BaseEntityMethodInner::Unknown { exposed_id, data } => {
                warn!(%addr, id, exposed_id, request_id = ?em.request_id,
                    "-> Base entity method (unknown exposed id): ({entity_id}) raw: {}", hex(data));
            }
        }
        Ok(true)
    }

}

/// Represent an entity type and its associated static functions.
#[derive(Debug)]
struct EntityType {
    create_base_player: fn(&mut BaseHandler, SocketAddr, ElementReader) -> io::Result<bool>,
    entity_method: fn(&mut BaseHandler, SocketAddr, u32, ElementReader) -> io::Result<bool>,
    base_entity_method: fn(&mut BaseHandler, SocketAddr, u32, ElementReader) -> io::Result<bool>,
}

impl EntityType {

    const fn new<E>() -> Self
    where
        E: Entity + fmt::Debug,
        E::ClientMethod: fmt::Debug,
        E::BaseMethod: fmt::Debug,
    {
        Self {
            create_base_player: BaseHandler::read_create_base_player::<E>,
            entity_method: BaseHandler::read_entity_method::<E>,
            base_entity_method: BaseHandler::read_base_entity_method::<E>,
        }
    }

}

/// Internal entity type vector.
struct EntityTypeVec(Vec<Arc<EntityType>>);
impl r#gen::entity::EntityTypeCollection for EntityTypeVec {

    fn new(len: usize) -> Self {
        Self(Vec::with_capacity(len))
    }

    fn add<E: Entity>(&mut self)
    where
        E: std::fmt::Debug,
        E::ClientMethod: std::fmt::Debug,
        E::BaseMethod: std::fmt::Debug,
        E::CellMethod: std::fmt::Debug,
    {
        self.0.push(Arc::new(EntityType::new::<E>()));
    }

}
