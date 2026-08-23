//! Proxy login and base app used for debugging exchanged messages.

use std::net::{SocketAddr, SocketAddrV4};
use std::time::Duration;
use std::{fmt, fs, io, thread};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::io::Write;
use std::fs::File;

use tracing::{error, info, warn, info_span, trace};

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

    let dump_dir = PathBuf::from("proxy-dump");
    let _ = fs::remove_dir_all(&dump_dir);
    fs::create_dir_all(&dump_dir).map_err(|e| format!("Failed to create proxy dump directory: {e}"))?;

    let shared = Arc::new(Shared {
        base_app_addr,
        login_app_addr,
        dump_dir,
        pending_clients: Mutex::new(HashMap::new()),
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
}

#[derive(Debug)]
struct Shared {
    base_app_addr: SocketAddrV4,
    #[allow(unused)]
    login_app_addr: SocketAddrV4,
    dump_dir: PathBuf,
    pending_clients: Mutex<HashMap<SocketAddr, PendingClient>>,
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
    /// The next sequence number expected, any other sequence number abort the download
    /// with an error.
    sequence_num: u8,
    /// The full assembled data.
    data: Vec<u8>,
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
        
        info!(%addr, "Login success");
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
            Ok(Some(proxy::PeerConfig {
                real_addr: SocketAddr::V4(pending_client.base_app_addr),
                blowfish: Some(pending_client.blowfish),
            }))
        } else {
            warn!(%addr, "Rejected an unknown peer");
            Ok(None)
        }

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
                    warn!(addr = %peer.addr(), "-> Reply #{request_id}");
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
                info!(%addr, "-> Session key: 0x{:08X}", elt.element.session_key);
            }
            EnableEntities::ID => {
                let _ee = elt.read_simple::<EnableEntities>()?;
                info!(%addr, "-> Enable entities");
            }
            DisconnectClient::ID => {
                let dc = elt.read_simple::<DisconnectClient>()?;
                info!(%addr, "-> Disconnect: 0x{:02X}", dc.element.reason);
            }
            id if id::BASE_ENTITY_METHOD.contains(id) => {

                // Account::doCmdInt3 (AccountCommands.CMD_SYNC_DATA), exposed id: 0x0E, message id: 0x95

                if let Some(entity_id) = self.player_entity_id {
                    // Unwrap because selected entity should exist!
                    let entity_type = self.entities.get(&entity_id).unwrap();
                    return (entity_type.base_entity_method)(&mut *self, addr, entity_id, elt);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, "-> Base entity method (unknown selected entity): msg#{} {:?} (request: {:?})", id - id::BASE_ENTITY_METHOD.first, elt.element, elt.request_id);
                return Ok(false);

            }
            id => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                error!(%addr, "-> Element #{id} {:?} (request: {:?})", elt.element, elt.request_id);
                return Ok(false);
            }
        }

        Ok(true)

    }

    fn read_in_bundle(&mut self, mut peer: proxy::Peer, bundle: Bundle) -> io::Result<()> {

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
                    warn!(addr = %peer.addr(), "<- Reply #{request_id}");
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
                info!(%addr, "<- Update frequency: {} Hz, game time: {}", ufn.element.frequency, ufn.element.game_time);
            }
            TickSync::ID => {
                let ts = elt.read_simple::<TickSync>()?;
                if let Some(next_tick) = self.next_tick {
                    if next_tick != ts.element.tick {
                        trace!(%addr, "<- Tick missed, expected {next_tick}, got {}", ts.element.tick);
                    }
                }
                self.next_tick = Some(ts.element.tick.wrapping_add(1));
            }
            ResetEntities::ID => {

                let re = elt.read_simple::<ResetEntities>()?;

                info!(%addr, "<- Reset entities, keep player on base: {}, entities: {}", 
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
                info!(%addr, "<- Logged off: 0x{:02X}", lo.element.reason);
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
                warn!(%addr, "<- Create base player with invalid entity type id: 0x{:02X}, {:?}", 
                    cbp.element.entity_type_id, dbg.element);

            }
            CreateCellPlayer::ID => {
                let ccp = elt.read_simple::<CreateCellPlayer>()?;
                warn!(%addr, "<- Create cell player: {:?}", ccp.element);
            }
            SelectPlayerEntity::ID => {
                let _spe = elt.read_simple::<SelectPlayerEntity>()?;
                if let Some(player_entity_id) = self.player_entity_id {
                    info!(%addr, "<- Select player entity: {player_entity_id}");
                } else {
                    warn!(%addr, "<- Select player entity: no player entity")
                }
                self.selected_entity_id = self.player_entity_id;
            }
            SwitchBaseApp::ID => {

                let sba = elt.read_simple::<SwitchBaseApp>()?;
                info!(%addr, "<- Switch base app to: {:?} (reset entities: {})", sba.element.base_addr, sba.element.reset_entities);
                
                // Change the real base address for this peer.
                peer.set_real_addr(sba.element.base_addr.into());
                
                // Immediately send a new switch element to change back the client to use
                // this proxy instead of the forwarded one.
                let mut bundle = Bundle::new();
                bundle.element_writer().write_simple(SwitchBaseApp {
                    base_addr: self.shared.base_app_addr,
                    reset_entities: sba.element.reset_entities,
                });

                peer.channel(proxy::PacketDirection::In, None)
                    .prepare(&mut bundle, true);

                peer.send_bundle(proxy::PacketDirection::In, &bundle)?;

            }
            ResourceHeader::ID => {

                let rh = elt.read_simple::<ResourceHeader>()?;
                info!(%addr, "<- Resource header: {}", rh.element.id);

                // Intentionally overwrite any previous downloading resource!
                self.partial_resources.insert(rh.element.id, PartialResource {
                    description: rh.element.description,
                    sequence_num: 0,
                    data: Vec::new(),
                });

            }
            ResourceFragment::ID => {

                let rf = elt.read_simple::<ResourceFragment>()?;
                let res_id = rf.element.id;

                let Some(partial_resource) = self.partial_resources.get_mut(&res_id) else {
                    warn!(%addr, "<- Resource fragment: {res_id}, len: {}, missing header", rf.element.data.len());
                    return Ok(true);
                };

                if rf.element.sequence_num != partial_resource.sequence_num {
                    // Just forgetting about the resource!
                    warn!(%addr, "<- Resource fragment: {res_id}, len: {}, invalid sequence number, expected {}, got {}", 
                    rf.element.data.len(), partial_resource.sequence_num, rf.element.sequence_num);
                    let _ = self.partial_resources.remove(&res_id);
                    return Ok(true);
                }

                partial_resource.sequence_num += 1;
                partial_resource.data.extend_from_slice(&rf.element.data);
                info!(%addr, "<- Resource fragment: {res_id}, len: {}, sequence number: {}", 
                    rf.element.data.len(), partial_resource.sequence_num);
                
                // Process the finished fragment!
                if rf.element.last {

                    let resource = self.partial_resources.remove(&rf.element.id).unwrap();
                    
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

                    let actual_total_len = resource.data.len();
                    if actual_total_len != total_len as usize {
                        warn!(%addr, "<- Invalid resource length, expected: {total_len}, got: {actual_total_len}");
                        return Ok(true);
                    }

                    let actual_crc32 = crc32fast::hash(&resource.data);
                    if actual_crc32 != crc32 {
                        warn!(%addr, "<- Invalid resource crc32, expected: 0x{crc32:08X}, got: 0x{actual_crc32:08X}");
                        return Ok(true);
                    }

                    info!(%addr, "<- Resource completed: {res_id}, len: {actual_total_len}, crc32: 0x{crc32:08X}");

                    // TODO: The full data looks like to be a zlib-compressed pickle.
                    // TODO: onCmdResponse for requested SYNC use RES_SUCCESS=0, RES_STREAM=1, RES_CACHE=2 for result_id
                    //       When RES_STREAM is used, then a resource (header+fragment) is expected with the associated request_id.

                    match serde_pickle::value_from_reader(ZlibDecoder::new(&resource.data[..]), serde_pickle_de_options()) {
                        Ok(val) => {
                            
                            let dump_file = self.shared.dump_dir.join(format!("res_{crc32:08x}.txt"));
                            info!(%addr, "<- Saving resource to: {}", dump_file.display());

                            let mut dump_writer = File::create(dump_file).unwrap();
                            write!(dump_writer, "{val}").unwrap();

                        }
                        Err(e) => {

                            warn!(%addr, "<- Resource: python error: {e}");

                            // FIXME: It appears that the current serde-pickle impl doesn't
                            // support recursive structures, however the structure that is 
                            // initially requested with 'CMD_SYNC_DATA' contains some.
                            // FIXME: The resource that is received by the from the chat
                            // command contains a "deque" object, which cannot be parsed
                            // so we get a "unresolved global reference" error.

                            let raw_file = self.shared.dump_dir.join(format!("res_{crc32:08x}.raw"));
                            info!(%addr, "<- Saving resource to: {}", raw_file.display());

                            let mut raw_writer = File::create(raw_file).unwrap();
                            std::io::copy(&mut ZlibDecoder::new(&resource.data[..]), &mut raw_writer).unwrap();

                        }
                    }

                }

            }
            id if id::ENTITY_METHOD.contains(id) => {

                // Account::msg#37 = onClanInfoReceived
                // Account::msg#39 = showGUI

                if let Some(entity_id) = self.selected_entity_id {
                    // Unwrap because selected entity should exist!
                    let entity_type = self.entities.get(&entity_id).unwrap();
                    return (entity_type.entity_method)(&mut *self, addr, entity_id, elt);
                }

                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, "<- Entity method (unknown selected entity): msg#{} {:?} (request: {:?})", id - id::ENTITY_METHOD.first, elt.element, elt.request_id);
                return Ok(false);

            }
            id if id::ENTITY_PROPERTY.contains(id) => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                warn!(%addr, "<- Entity property: msg#{} {:?} (request: {:?})", id - id::ENTITY_PROPERTY.first, elt.element, elt.request_id);
                return Ok(false);
            }
            id => {
                let elt = elt.read_simple::<DebugElementUndefined<0>>()?;
                error!(%addr, "<- Element #{id} {:?} (request: {:?})", elt.element, elt.request_id);
                return Ok(false);
            }
        }

        Ok(true)

    }

    fn read_create_base_player<E>(&mut self, addr: SocketAddr, elt: ElementReader) -> io::Result<bool>
    where 
        E: Entity + fmt::Debug,
    {

        use client::element::CreateBasePlayer;

        let cbp = elt.read_simple::<CreateBasePlayer<E>>()?;

        let dump_file = self.shared.dump_dir.join(format!("entity_{}.txt", cbp.element.entity_id));
        let mut dump_writer = File::create(&dump_file)?;
        write!(dump_writer, "{:#?}", cbp.element.entity_data)?;

        info!(%addr, "<- Create base player: ({}) {}", cbp.element.entity_id, dump_file.display());

        Ok(true)

    }

    fn read_entity_method<E>(&mut self, addr: SocketAddr, entity_id: u32, elt: ElementReader) -> io::Result<bool>
    where 
        E: Entity,
        E::ClientMethod: fmt::Debug,
    {
        use client::element::EntityMethod;
        let em = elt.read_simple::<EntityMethod<E::ClientMethod>>()?;
        info!(%addr, "<- Entity method: ({entity_id}) {:?}", em.element.inner);
        Ok(true)
    }

    fn read_base_entity_method<E>(&mut self, addr: SocketAddr, entity_id: u32, elt: ElementReader) -> io::Result<bool>
    where 
        E: Entity,
        E::BaseMethod: fmt::Debug,
    {
        use base::element::BaseEntityMethod;
        let em = elt.read_simple::<BaseEntityMethod<E::BaseMethod>>()?;
        info!(%addr, "-> Base entity method: ({entity_id}) {:?}", em.element.inner);
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
