//! Base application where clients send all their requests.

pub mod element;

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::io::{self, Read, Write};
use std::num::Wrapping;
use std::borrow::Cow;
use std::sync::Arc;

use blowfish::Blowfish;

use rand::rngs::OsRng;
use rand::RngCore;

use tracing::warn;

use crate::net::bundle::{Bundle, NextElementReader, ElementReader};
use crate::net::element::{Element, ElementLength, SimpleElement};
use crate::net::socket::PacketSocket;
use crate::script::{Script, Value};
use crate::net::proto::Protocol;

use super::dispatch::{ScriptDispatch, PropertyDef, MethodCall};
use super::io_invalid_data;
use super::client;

use element::{LoginKey, SessionKey};


/// The base application.
///
/// Fully dynamic: driven by a loaded [`Script`] rather than a per-game generated
/// `AnyEntity` sum type -- entities, methods and properties are all resolved by name (or
/// exposed id) against that script model at runtime (see [`ScriptDispatch`]), so this
/// type itself never needs to know about any concrete game's entity types.
#[derive(Debug)]
pub struct App {
    /// The scripting model for this app, and its per entity-type dispatch tables
    /// computed from it at construction time.
    dispatch: ScriptDispatch,
    /// Internal socket for this application.
    socket: PacketSocket,
    /// The channel tracker.
    protocol: Protocol,
    /// Queue of events that are waiting to be returned.
    events: VecDeque<Event>,
    /// A temporary bundle for sending.
    bundle: Bundle,
    /// Clients that have made an initial client connection, associated to the request id.
    pending_clients: HashMap<SocketAddr, u32>,
    /// Map of clients.
    clients: HashMap<SocketAddr, Client>,
    /// Every entity this app has created, keyed by its globally-unique entity id and
    /// pointing to both its data and the address of the one client that owns it. This
    /// is a single flat/global map (not nested per-client) matching real BigWorld's own
    /// design (`BaseApp`'s `container_: BW::map<EntityID, Base*>`) -- entity ids are a
    /// process-wide namespace, even though in practice a base-player entity is only
    /// ever relevant to the one client that owns it (BigWorld's `Proxy` only ever holds
    /// a single client channel, never a set of subscribers).
    entities: HashMap<u32, EntityEntry>,
    /// The next id for entities, this is wrapping around and we ensure that the same id
    /// isn't used twice!
    entities_next_id: Wrapping<u32>,
}

impl App {

    pub fn new(addr: SocketAddr, script: Script) -> io::Result<Self> {
        Ok(Self {
            dispatch: ScriptDispatch::new(script),
            socket: PacketSocket::bind(addr)?,
            protocol: Protocol::new(),
            events: VecDeque::new(),
            bundle: Bundle::new(),
            pending_clients: HashMap::new(),
            clients: HashMap::new(),
            entities: HashMap::new(),
            entities_next_id: Wrapping(OsRng.next_u32()),
        })
    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.socket.addr()
    }

    /// Poll for the next event of this login app, blocking.
    pub fn poll(&mut self) -> Event {
        loop {

            // Empty the events before.
            while let Some(event) = self.events.pop_front() {
                return event;
            }

            let (packet, addr) = match self.socket.recv() {
                Ok(ret) => ret,
                Err(error) => return Event::IoError(IoErrorEvent { error, addr: None }),
            };

            let Ok(mut channel) = self.protocol.accept(packet, addr) else {
                continue;
            };

            let Some(bundle) = channel.next_bundle() else {
                continue;
            };

            // Fully read the bundle to determine how to handle that client.
            let mut reader = bundle.element_reader();
            while let Some(reader) = reader.next() {
                match reader {
                    NextElementReader::Element(elt) => {
                        if let Err(error) = self.handle_element(addr, elt) {
                            return Event::IoError(IoErrorEvent { error, addr: Some(addr) });
                        }
                    }
                    NextElementReader::Reply(reply) => {
                        return Event::IoError(IoErrorEvent {
                            error: io_invalid_data(format_args!("unexpected reply #{}", reply.request_id())),
                            addr: Some(addr),
                        });
                    }
                }
            }

        }
    }

    /// Handle an element read from the given address.
    fn handle_element(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {
        match reader.id() {
            LoginKey::ID => self.handle_client_auth(addr, reader),
            SessionKey::ID => self.handle_client_session_key(addr, reader),
            element::id::ENABLE_ENTITIES => self.skip::<element::EnableEntities>(reader),
            element::id::DISCONNECT_CLIENT => self.skip::<element::DisconnectClient>(reader),
            element::id::PING_DATACENTER => self.skip::<element::PingDatacenter>(reader),
            element::id::AVATAR_UPDATE_IMPLICIT => self.skip::<element::AvatarUpdateImplicit>(reader),
            element::id::AVATAR_UPDATE_EXPLICIT => self.skip::<element::AvatarUpdateExplicit>(reader),
            element::id::ACK_PHYSICS_CORRECTION => self.skip::<element::AckPhysicsCorrection>(reader),
            element::id::REQUEST_ENTITY_UPDATE => self.skip::<element::RequestEntityUpdate>(reader),
            element::id::NRL_MSG_TO_CELL => self.skip::<element::NrlMsgToCell>(reader),
            element::id::AVATAR_UPDATE_WARD_IMPLICIT => self.skip::<element::AvatarUpdateWardImplicit>(reader),
            element::id::AVATAR_UPDATE_WARD_EXPLICIT => self.skip::<element::AvatarUpdateWardExplicit>(reader),
            element::id::ACK_WARD_PHYSICS_CORRECTION => self.skip::<element::AckWardPhysicsCorrection>(reader),
            element::id::RESTORE_CLIENT_ACK => self.skip::<element::RestoreClientAck>(reader),
            element::id::CLIENT_TO_SERVER_HEARTBEAT => self.skip::<element::ClientToServerHeartbeat>(reader),
            element::id::SEND_TO_CELL => self.skip::<element::SendToCell>(reader),
            id if element::id::BASE_ENTITY_METHOD.contains(id) => self.handle_base_entity_method(addr, reader),
            id if element::id::CELL_ENTITY_METHOD.contains(id) => self.handle_cell_entity_method(addr, reader),
            id => Err(io_invalid_data(format_args!("unexpected element #{id}"))),
        }
    }

    fn handle_client_auth(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let auth = reader.read_simple::<LoginKey>()?;
        let request_id = auth.request_id
            .ok_or_else(|| io_invalid_data(format_args!("auth should be a request")))?;

        self.events.push_back(Event::Login(LoginEvent {
            addr,
            login_key: auth.element.login_key,
            attempt_num: auth.element.attempt_num,
        }));

        self.pending_clients.insert(addr, request_id);

        Ok(())

    }

    fn handle_client_session_key(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let session = reader.read_simple::<SessionKey>()?;

        if let Some(client) = self.clients.get(&addr) {
            if client.session_key != session.element.session_key {
                warn!(%addr, "Session key mismatch: expected 0x{:08X}, got 0x{:08X}",
                    client.session_key, session.element.session_key);
            }
        }

        Ok(())

    }

    /// Decode and dispatch an incoming base method call, targeting the client's current
    /// base-player entity (there's only ever one -- the wire call itself carries no
    /// entity id). Decode happens right here, eagerly, dispatched dynamically against
    /// the entity's own type (resolved from `dispatch`, computed from the script model
    /// at construction time) -- not deferred to the caller.
    ///
    /// Propagates (rather than swallows) a genuine decode error from the dynamic method
    /// read: per its doc comment, that means the bundle reader rolled back to before this
    /// element, so returning `Ok(())` here (as if this element had been consumed) would
    /// make the next `poll()` iteration re-read the exact same unconsumed element
    /// forever.
    fn handle_base_entity_method(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let Some(entity_id) = self.clients.get(&addr).and_then(|c| c.base_entity_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(entry) = self.entities.get(&entity_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(dispatch) = self.dispatch.entity_from_id(entry.type_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let call = reader.read::<element::BaseEntityMethod, _>(&dispatch.base_methods)?.element.call;
        self.events.push_back(Event::BaseMethod(BaseMethodEvent { addr, entity_id, call }));

        Ok(())

    }

    /// Same as [`Self::handle_base_entity_method`], but for a call targeting the
    /// client's current cell-side entity. The client has no direct connection to a cell
    /// app, so these calls arrive here to be forwarded in real BigWorld; this project has
    /// no `cell::App` to forward to yet, but decoding still works today, dispatched via
    /// the same stored entity data (an entity's base and cell slices share one id).
    /// `Client::cell_entity_id` isn't populated by anything yet, so until it is, this
    /// always takes the skip path below.
    fn handle_cell_entity_method(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let Some(entity_id) = self.clients.get(&addr).and_then(|c| c.cell_entity_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(entry) = self.entities.get(&entity_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(dispatch) = self.dispatch.entity_from_id(entry.type_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let call = reader.read::<element::CellEntityMethod, _>(&dispatch.cell_methods)?.element.call;
        self.events.push_back(Event::CellMethod(CellMethodEvent { addr, entity_id, call }));

        Ok(())

    }

    /// Accept the login of the given user, in response to [`Event::Login`], registering
    /// the client (blowfish key included -- everything sent to this client from this
    /// point on, including this very reply, goes out encrypted) and replying with a
    /// freshly generated session key.
    ///
    /// This returns true if a client was actually waiting for this reply.
    pub fn answer_login_success(&mut self, addr: SocketAddr, blowfish: Arc<Blowfish>) -> bool {

        let Some(request_id) = self.pending_clients.remove(&addr) else {
            return false;
        };

        let session_key = OsRng.next_u32();

        self.clients.insert(addr, Client {
            session_key,
            blowfish: Arc::clone(&blowfish),
            base_entity_id: None,
            cell_entity_id: None,
        });

        self.socket.set_encryption(addr, blowfish);

        self.bundle.clear();
        self.bundle.element_writer().write_simple_reply(SessionKey { session_key }, request_id);
        self.protocol.off_channel(addr).prepare(&mut self.bundle, false);
        let _ = self.socket.send_bundle(&self.bundle, addr);

        true

    }

    /// Create a new base-player entity for the given (already logged in) client, and
    /// return a handle to it. This becomes that client's current base-player entity,
    /// replacing any previous one in `Client::base_entity_id` (real BigWorld only ever
    /// has one at a time).
    ///
    /// `entity_name` is resolved against the loaded [`Script`]'s entities by name, and
    /// `entity_data` must be a [`Value::Dict`] matching that entity's client-visible
    /// property layout (see [`crate::app::script::EntityDispatch::data_ty`]) -- a
    /// mismatch surfaces as an `io::Error` from the underlying codec rather than being
    /// validated up front.
    pub fn create_base_player(
        &mut self,
        addr: SocketAddr,
        entity_name: &str,
        entity_data: Value,
        entity_components_count: u8,
    ) -> io::Result<Handle> {

        if !self.clients.contains_key(&addr) {
            return Err(io_invalid_data(format_args!("no such client: {addr}")));
        }

        let (entity_type_id, dispatch) = self.dispatch.entity_from_name(entity_name)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity: {entity_name}")))?;

        // Generate a new unique entity id.
        let entity_id = loop {
            let id = self.entities_next_id.0;
            self.entities_next_id += 1;
            if !self.entities.contains_key(&id) {
                break id;
            }
        };

        self.bundle.clear();
        self.bundle.element_writer().write(
            client::element::CreateBasePlayer {
                entity_id,
                entity_type_id,
                entity_data: Cow::Borrowed(&entity_data),
                entity_components_count,
            },
            dispatch,
        );
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        self.entities.insert(entity_id, EntityEntry { addr, type_id: entity_type_id, data: entity_data });

        if let Some(client) = self.clients.get_mut(&addr) {
            client.base_entity_id = Some(entity_id);
        }

        Ok(Handle { entity_id })

    }

    /// Give the client's already-created base-player entity a cell-side presence too
    /// (e.g. it has entered a space/battle), becoming its current cell entity in
    /// `Client::cell_entity_id`. Unlike `create_base_player`, this doesn't mint a new
    /// entity id: base and cell slices of an entity share the same id in real BigWorld,
    /// and the wire message itself carries none of its own (see
    /// [`client::element::CreateCellPlayer`]'s doc comment for why, and for the parts of
    /// this message that aren't confirmed against a live capture).
    pub fn create_cell_player(
        &mut self,
        handle: Handle,
        space_id: u32,
        vehicle_id: u32,
        position: glam::Vec3,
        direction: glam::Vec3,
        packed_xz_scale: f32,
        cell_data: Vec<u8>,
    ) -> io::Result<()> {

        let addr = self.entities.get(&handle.entity_id)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {}", handle.entity_id)))?
            .addr;

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::CreateCellPlayer {
            unk_flag: 0,
            space_id,
            unk_short: 0,
            vehicle_id,
            position,
            packed_xz_scale,
            direction,
            cell_data,
        });
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        if let Some(client) = self.clients.get_mut(&addr) {
            client.cell_entity_id = Some(handle.entity_id);
        }

        Ok(())

    }

    /// Call a method, resolved by name against the target entity's client-method table,
    /// on the client owning the given entity.
    pub fn call_method(&mut self, handle: Handle, method_name: &str, args: Vec<Value>) -> io::Result<()> {

        let entry = self.entities.get(&handle.entity_id)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {}", handle.entity_id)))?;
        let addr = entry.addr;
        let type_id = entry.type_id;

        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("no dispatch table for entity: {}", handle.entity_id)))?;

        let name = dispatch.client_methods.iter().find(|m| &*m.name == method_name)
            .ok_or_else(|| io_invalid_data(format_args!("unknown client method: {method_name}")))?
            .name.clone();

        self.bundle.clear();
        self.bundle.element_writer().write(
            client::element::EntityMethod { call: MethodCall::Known { name, args } },
            &dispatch.client_methods,
        );
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        Ok(())

    }

    /// Tell the client that subsequent elements target the given entity's player slot.
    pub fn select_player_entity(&mut self, handle: Handle) -> io::Result<()> {

        let addr = self.entities.get(&handle.entity_id)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {}", handle.entity_id)))?
            .addr;

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::SelectPlayerEntity {});
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        Ok(())

    }

    /// Reset all entities known to the given client, optionally keeping its current
    /// base-player entity alive (`keep_player_on_base`). This is the only entity
    /// *removal* mechanism the wire protocol has -- there's no way to remove a single
    /// arbitrary entity, only "reset everything" (see `re-work/doc/ENTITY.md`).
    pub fn reset_entities(&mut self, addr: SocketAddr, keep_player_on_base: bool) -> io::Result<()> {

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::ResetEntities { keep_player_on_base });
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        let keep_entity_id = keep_player_on_base
            .then(|| self.clients.get(&addr).and_then(|c| c.base_entity_id))
            .flatten();

        self.entities.retain(|&id, entry| entry.addr != addr || Some(id) == keep_entity_id);

        if let Some(client) = self.clients.get_mut(&addr) {
            if !keep_player_on_base {
                client.base_entity_id = None;
            }
            client.cell_entity_id = None;
        }

        Ok(())

    }

    /// Tell the client to disconnect and reconnect to a (possibly different) base app
    /// address.
    pub fn switch_base_app(&mut self, addr: SocketAddr, base_addr: SocketAddrV4, reset_entities: bool) -> io::Result<()> {

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::SwitchBaseApp { base_addr: base_addr.into(), reset_entities });
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        Ok(())

    }

    /// Push a resource to the client (`ResourceHeader` followed by one or more
    /// `ResourceFragment`s), e.g. in response to `ClientCommandsPort.doCmdInt3` asking
    /// for a `RES_STREAM` reply. `data` is sent as-is, chunked into fragments -- any
    /// compression/framing (e.g. WoT's zlib+pickle blobs) is the caller's job, since
    /// this library has no opinion on it.
    ///
    /// Sent as many small reliable bundles rather than one giant multi-packet bundle:
    /// `Protocol`'s own bundle-fragment reassembly has a hardcoded 10s timeout
    /// (`FRAGMENT_TIMEOUT`), and real captured traffic paces large resources over
    /// several seconds of separate fragments rather than one burst.
    pub fn push_resource(&mut self, addr: SocketAddr, res_id: u16, description: Vec<u8>, data: &[u8]) -> io::Result<()> {

        const CHUNK_SIZE: usize = 1024;

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::ResourceHeader { id: res_id, description });
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        let chunks: Vec<&[u8]> = if data.is_empty() {
            vec![&[]]
        } else {
            data.chunks(CHUNK_SIZE).collect()
        };

        // NOTE: `ResourceFragment::sequence_num` is a plain `u8` on the wire, so this
        // can't correctly address more than 256 fragments (~256KB at this chunk size) --
        // fine for the small fabricated payloads this is used for today, but a real
        // multi-hundred-KB resource (like WoT's ~368KB account cache) would need a
        // larger chunk size or isn't representable as-is.
        let last_seq = (chunks.len() - 1) as u8;

        for (seq, chunk) in chunks.into_iter().enumerate() {
            self.bundle.clear();
            self.bundle.element_writer().write_simple(client::element::ResourceFragment {
                id: res_id,
                sequence_num: seq as u8,
                last: seq as u8 == last_seq,
                data: chunk.to_vec(),
            });
            self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
            self.socket.send_bundle(&self.bundle, addr)?;
        }

        Ok(())

    }

    /// Read (and discard) a single already-typed element, without aborting the rest of
    /// the bundle -- used for ids whose framing is known but that this app doesn't act
    /// on yet.
    fn skip<T: Element<()>>(&mut self, reader: ElementReader<'_, '_>) -> io::Result<()> {
        reader.read_simple::<T>()?;
        Ok(())
    }

    /// Get an entity's current data, as last set by [`Self::create_base_player`] -- a
    /// [`Value::Dict`] matching its client-visible property layout (see
    /// [`crate::app::script::EntityDispatch::data_ty`]).
    pub fn entity_data(&self, handle: Handle) -> Option<&Value> {
        self.entities.get(&handle.entity_id).map(|entry| &entry.data)
    }

    /// Get an entity's client-visible property table (covering either its base or cell
    /// slice, both share one id space on the wire), resolved dynamically from the loaded
    /// script model -- see [`crate::app::script::EntityDispatch::properties`].
    pub fn entity_properties(&self, handle: Handle) -> Option<&[PropertyDef]> {
        let entry = self.entities.get(&handle.entity_id)?;
        self.dispatch.entity_from_id(entry.type_id).map(|dispatch| dispatch.properties.as_slice())
    }

}

/// Read the raw wire payload of a `CELL_ENTITY_METHOD`/`BASE_ENTITY_METHOD`-range
/// element without needing to know its method table ahead of time -- its framing is
/// always `Variable16` regardless of the target entity, so this can be read generically
/// and decoded later once the target entity's dispatch table is known.
struct RawEntityMethod {
    element_id: u8,
    data: Vec<u8>,
}

impl Element<()> for RawEntityMethod {

    fn write_length(&self, _config: &()) -> io::Result<ElementLength> {
        unreachable!("RawEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &()) -> io::Result<u8> {
        unreachable!("RawEntityMethod is read-only")
    }

    fn read_length(_config: &(), _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, _config: &(), _len: usize, id: u8) -> io::Result<Self> {
        let mut data = Vec::new();
        read.read_to_end(&mut data)?;
        Ok(Self { element_id: id, data })
    }

}

/// An active logged in client in the base application.
#[derive(Debug)]
struct Client {
    /// The session key for this client.
    session_key: u32,
    /// The blowfish key for encryption of this client's packets.
    blowfish: Arc<Blowfish>,
    /// This client's current base-player entity, if any created yet. There's only ever
    /// one at a time (real BigWorld: `Proxy` holds a single client channel, and a base
    /// method call carries no entity id of its own -- it always targets this one).
    base_entity_id: Option<u32>,
    /// This client's current cell-side entity (e.g. its `Avatar`/`Vehicle` while in a
    /// battle), if any. Tracked separately from `base_entity_id` since a client can have
    /// both a base and a cell entity live at once -- not populated by anything yet since
    /// no `cell::App` exists in this project.
    cell_entity_id: Option<u32>,
}

/// One entity this app has created: its wire type id, its data (a [`Value::Dict`]
/// matching that type's client-visible property layout, see
/// [`crate::app::script::EntityDispatch::data_ty`]), and the address of the one client
/// that owns it.
#[derive(Debug)]
struct EntityEntry {
    addr: SocketAddr,
    type_id: u16,
    data: Value,
}

/// A handle to an entity in the base app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    entity_id: u32,
}

impl Handle {

    /// This entity's globally-unique id.
    pub fn entity_id(&self) -> u32 {
        self.entity_id
    }

}

// ============ //
//    EVENTS    //
// ============ //

/// An event that happened in the base app.
#[derive(Debug)]
pub enum Event {
    IoError(IoErrorEvent),
    Login(LoginEvent),
    BaseMethod(BaseMethodEvent),
    CellMethod(CellMethodEvent),
}

/// Some IO error happened internally and optionally related to a client.
#[derive(Debug)]
pub struct IoErrorEvent {
    /// The IO error.
    pub error: io::Error,
    /// An optional client address related to the error.
    pub addr: Option<SocketAddr>,
}

/// A client is trying to connect.
#[derive(Debug)]
pub struct LoginEvent {
    /// The address of the client that pinged the login app.
    pub addr: SocketAddr,
    /// The given client from the given address
    pub login_key: u32,
    /// The attempt number.
    pub attempt_num: u8,
}

/// A client called a method on its current base-player entity.
#[derive(Debug)]
pub struct BaseMethodEvent {
    pub addr: SocketAddr,
    pub entity_id: u32,
    /// The decoded call, resolved dynamically against the entity's own base-method
    /// table (see [`crate::app::script::EntityDispatch`]).
    pub call: MethodCall,
}

/// A client called a method on its current cell-side entity.
#[derive(Debug)]
pub struct CellMethodEvent {
    pub addr: SocketAddr,
    pub entity_id: u32,
    /// The decoded call, resolved dynamically against the entity's own cell-method
    /// table (see [`crate::app::script::EntityDispatch`]).
    pub call: MethodCall,
}
