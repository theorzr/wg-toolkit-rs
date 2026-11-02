//! Base application where clients send all their requests.
//!
//! It also holds the cell part of entities: unlike BigWorld, with no cell app of its
//! own. An entity gets into a space with its cell part, its position and how clients
//! see it. A client whose player entity is in a space has a witness of what it knows of
//! that space: each tick, see [`App::end_tick`], it is told what entered or left its
//! area of interest, where the entities it knows moved, and what changed of them. An
//! app holds any number of spaces, a game in each say, which never see each other, and
//! games scale by running on apps of their own, which their clients are switched to.

pub mod element;

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};
use std::io::{self, Read, Write};
use std::num::Wrapping;
use std::borrow::Cow;
use std::sync::Arc;
use std::fmt;

use tracing::{debug, warn};
use blowfish::Blowfish;
use rand::rngs::OsRng;
use rand::RngCore;
use glam::Vec3;

use crate::net::bundle::{Bundle, NextElementReader, ElementReader};
use crate::net::element::{Element, ElementLength, SimpleElement};
use crate::script::{PropertyFlags, Script, Ty, Value};
use crate::net::socket::PacketSocket;
use crate::net::proto::Protocol;

use super::entity::{
    self, AnyEntity, BindError, Component, ComponentKey, ComponentSchema, ComponentState, Entity, EntityComponents,
    EntityEvent, EntityId, EntityKey, EntitySchema, EntitySink, Methods, NrlNodeIds, RawComponent, RawEntity, Registry, RemovedComponent, ScriptType,
};
use super::dispatch::{EntityDispatch, ScriptDispatch, MethodCall, MethodDef};
use super::client::element::ChangedProperties;
use super::outbox::{ClientOutbox, Outbox};
use super::math::PackedYawPitchRoll;
use super::slots::{SlotKey, Slots};
use super::io_invalid_data;
use super::client;

use element::{LoginKey, SessionKey};


// ========= //
//    APP    //
// ========= //

/// The base application.
///
/// Fully dynamic: driven by a loaded [`Script`] rather than a per-game generated
/// `AnyEntity` sum type -- entities, methods and properties are all resolved by name (or
/// exposed id) against that script model at runtime (see [`ScriptDispatch`]), so this
/// type itself never needs to know about any concrete game's entity types. Schemas
/// registered with [`Self::register`] give typed access on top, see
/// [`super::entity`].
///
/// Clients, entities and spaces are given as keys, [`ClientKey`], [`EntityKey`] and
/// [`SpaceKey`], which go stale once what they name is gone.
#[derive(Debug)]
pub struct App {
    /// The scripting model for this app, and its per entity-type dispatch tables
    /// computed from it at construction time.
    dispatch: ScriptDispatch,
    /// The registered entity and component schemas, see [`Self::register`] and
    /// [`Self::register_component`].
    entity_schemas: Registry,
    component_schemas: Registry,
    /// Internal socket for this application.
    socket: PacketSocket,
    /// The channel tracker.
    protocol: Protocol,
    /// Queue of events that are waiting to be returned.
    events: VecDeque<Event>,
    /// What waits to be sent to each client, until the next poll.
    outbox: Outbox,
    /// A bundle for what is sent right away rather than through the outbox: the login
    /// reply, before the client has a channel, and resource fragments.
    bundle: Bundle,
    /// Clients that have made an initial client connection, associated to the request id.
    pending_clients: HashMap<SocketAddr, u32>,
    /// The logged in clients, and the key of each by its address.
    clients: Slots<ClientState>,
    client_keys: HashMap<SocketAddr, ClientKey>,
    /// Every entity this app has created, and the key of each by its id.
    entities: Slots<EntityState>,
    entity_keys: HashMap<u32, EntityKey<AnyEntity>>,
    /// The next id for entities, this is wrapping around and we ensure that the same id
    /// isn't used twice!
    entities_next_id: Wrapping<u32>,
    /// The spaces, and the key of each by its id.
    spaces: Slots<SpaceState>,
    space_keys: HashMap<u32, SpaceKey>,
}

impl App {

    pub fn new(addr: SocketAddr, script: Script) -> io::Result<Self> {
        Ok(Self {
            dispatch: ScriptDispatch::new(script),
            entity_schemas: Registry::default(),
            component_schemas: Registry::default(),
            socket: PacketSocket::bind(addr)?,
            protocol: Protocol::new(),
            events: VecDeque::new(),
            outbox: Outbox::default(),
            bundle: Bundle::new(),
            pending_clients: HashMap::new(),
            clients: Slots::default(),
            client_keys: HashMap::new(),
            entities: Slots::default(),
            entity_keys: HashMap::new(),
            entities_next_id: Wrapping(OsRng.next_u32()),
            spaces: Slots::default(),
            space_keys: HashMap::new(),
        })
    }

    /// The dispatch tables of the script model this app was created with.
    pub fn dispatch(&self) -> &ScriptDispatch {
        &self.dispatch
    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.socket.addr()
    }

    /// Bind a schema to its entity type of the loaded script, checking every member it
    /// declares, see [`super::entity`]. Its entities can then be created, viewed
    /// through it with [`Self::entity`], and their calls decoded with [`Self::decode`].
    pub fn register<T: EntitySchema>(&mut self) -> Result<(), BindError> {
        self.entity_schemas.register::<T>(&self.dispatch)
    }

    /// Bind a component schema to its dynamic component type of the loaded script,
    /// checking every member it declares, see [`super::entity`]. Its components can
    /// then be added to entities, see [`Entity::add_component`], and viewed with
    /// [`Self::component`]. The client must have a class for it, for NRL to number it.
    pub fn register_component<C: ComponentSchema>(&mut self) -> Result<(), BindError> {

        let nrl_type_id = self.dispatch.entity_from_name(C::NAME)
            .and_then(|(type_id, _)| self.dispatch.component_nrl_id(type_id));
        if nrl_type_id.is_none() {
            return Err(BindError { schema: C::NAME, errors: vec!["no component type the client has a class for".to_string()] });
        }

        self.component_schemas.register::<C>(&self.dispatch)

    }

    /// Poll for the next event of this app, blocking until one comes.
    ///
    /// What was written to clients since the last poll is sent first, one bundle per
    /// client.
    pub fn poll(&mut self) -> Event {
        self.poll_inner(None)
    }

    /// Same as [`Self::poll`], but returns [`Event::Timeout`] once the deadline passes
    /// with no event, so the caller gets a chance to send periodic messages (ticks) to
    /// quiet clients. A deadline already passed still reads a packet waiting, if any.
    pub fn poll_until(&mut self, deadline: Instant) -> Event {
        self.poll_inner(Some(deadline))
    }

    fn poll_inner(&mut self, deadline: Option<Instant>) -> Event {

        for (addr, error) in self.outbox.flush_all(&mut self.protocol, &self.socket) {
            self.events.push_back(Event::IoError(IoErrorEvent { error, addr: Some(addr) }));
        }

        loop {

            // Empty the events before.
            if let Some(event) = self.events.pop_front() {
                return event;
            }

            // Reliable packets the clients haven't acknowledged yet, likely lost.
            let resends = self.protocol.take_resends(RESEND_TIMEOUT, RESEND_GIVE_UP);
            if !resends.is_empty() {
                debug!("Resending {} unacknowledged packets", resends.len());
            }
            for (addr, packet) in resends {
                if let Err(error) = self.socket.send(&packet, addr) {
                    return Event::IoError(IoErrorEvent { error, addr: Some(addr) });
                }
            }

            let timeout = deadline.map(|deadline| deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)));
            if let Err(error) = self.socket.set_recv_timeout(timeout) {
                return Event::IoError(IoErrorEvent { error, addr: None });
            }

            let (packet, addr) = match self.socket.recv() {
                Ok(ret) => ret,
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    return Event::Timeout;
                }
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

        let client = self.client_keys.get(&addr).copied();

        match reader.id() {
            LoginKey::ID => self.handle_client_auth(addr, reader),
            SessionKey::ID => self.handle_client_session_key(client, reader),
            element::id::ENABLE_ENTITIES => {
                reader.read_simple::<element::EnableEntities>()?;
                if let Some(client) = client {
                    self.events.push_back(Event::EnableEntities(EnableEntitiesEvent { client }));
                }
                Ok(())
            }
            element::id::DISCONNECT_CLIENT => self.skip::<element::DisconnectClient>(reader),
            element::id::PING_DATACENTER => self.skip::<element::PingDatacenter>(reader),
            element::id::AVATAR_UPDATE_IMPLICIT => {
                let update = reader.read_simple::<element::AvatarUpdateImplicit>()?.element;
                let (yaw, pitch, roll) = update.direction.unpack(true);
                if let Some(client) = client {
                    self.events.push_back(Event::AvatarUpdate(AvatarUpdateEvent {
                        client, vehicle_id: None, position: update.position, direction: Vec3::new(roll, pitch, yaw),
                    }));
                }
                Ok(())
            }
            element::id::AVATAR_UPDATE_EXPLICIT => {
                let update = reader.read_simple::<element::AvatarUpdateExplicit>()?.element;
                let (yaw, pitch, roll) = update.direction.unpack(true);
                if let Some(client) = client {
                    self.events.push_back(Event::AvatarUpdate(AvatarUpdateEvent {
                        client, vehicle_id: Some(update.vehicle_id), position: update.position, direction: Vec3::new(roll, pitch, yaw),
                    }));
                }
                Ok(())
            }
            element::id::ACK_PHYSICS_CORRECTION => self.skip::<element::AckPhysicsCorrection>(reader),
            element::id::REQUEST_ENTITY_UPDATE => {
                let request = reader.read_simple::<element::RequestEntityUpdate>()?.element;
                if let Some(client) = client {
                    self.request_entity(client, request.entity_id);
                }
                Ok(())
            }
            element::id::NRL_MSG_TO_CELL => self.skip::<element::NrlMsgToCell>(reader),
            element::id::AVATAR_UPDATE_WARD_IMPLICIT => self.skip::<element::AvatarUpdateWardImplicit>(reader),
            element::id::AVATAR_UPDATE_WARD_EXPLICIT => self.skip::<element::AvatarUpdateWardExplicit>(reader),
            element::id::ACK_WARD_PHYSICS_CORRECTION => self.skip::<element::AckWardPhysicsCorrection>(reader),
            element::id::RESTORE_CLIENT_ACK => self.skip::<element::RestoreClientAck>(reader),
            element::id::CLIENT_TO_SERVER_HEARTBEAT => self.skip::<element::ClientToServerHeartbeat>(reader),
            element::id::SEND_TO_CELL => self.skip::<element::SendToCell>(reader),
            id if element::id::BASE_ENTITY_METHOD.contains(id) => self.handle_base_entity_method(client, reader),
            id if element::id::CELL_ENTITY_METHOD.contains(id) => self.handle_cell_entity_method(client, reader),
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

    fn handle_client_session_key(&mut self, client: Option<ClientKey>, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let session = reader.read_simple::<SessionKey>()?;

        if let Some(key) = client {
            if let Some(client) = self.clients.get(key.slot) {
                if client.session_key != session.element.session_key {
                    warn!(addr = %key.addr, "Session key mismatch: expected 0x{:08X}, got 0x{:08X}",
                        client.session_key, session.element.session_key);
                }
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
    fn handle_base_entity_method(&mut self, client: Option<ClientKey>, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let Some((client, entity)) = client
            .and_then(|key| Some((key, self.clients.get(key.slot)?.player?))) else {
            return self.skip::<SkippedEntityMethod>(reader);
        };

        let Some(state) = self.entities.get(entity.slot) else {
            return self.skip::<SkippedEntityMethod>(reader);
        };

        let Some(dispatch) = self.dispatch.entity_from_id(state.type_id) else {
            return self.skip::<SkippedEntityMethod>(reader);
        };

        let call = reader.read::<element::BaseEntityMethod, _>(&dispatch.base_methods)?.element.call;
        self.events.push_back(Event::BaseMethod(BaseMethodEvent { client, entity, call }));

        Ok(())

    }

    /// Same as [`Self::handle_base_entity_method`], but for a cell-directed call. Unlike
    /// a base call, a cell call names its target, which is not necessarily the client's
    /// own entity (see [`element::CellMethodTables`]), so the table is resolved from that
    /// target. A client with no entity in a space has no business sending one.
    fn handle_cell_entity_method(&mut self, client: Option<ClientKey>, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let Some((client, cell_entity)) = client
            .and_then(|key| Some((key, self.clients.get(key.slot)?.witness.as_ref()?.entity))) else {
            return self.skip::<SkippedEntityMethod>(reader);
        };

        let tables = CellTables { entities: &self.entities, entity_keys: &self.entity_keys, dispatch: &self.dispatch, cell_entity };
        let method = reader.read::<element::CellEntityMethod, _>(&tables)?.element;
        let entity = match method.entity_id {
            0 => Some(cell_entity),
            id => self.entity_keys.get(&id).copied(),
        };

        match entity {
            Some(entity) => self.events.push_back(Event::CellMethod(CellMethodEvent { client, entity, call: method.call })),
            None => debug!(addr = %client.addr, "Cell method call on unknown entity {}", method.entity_id),
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

}

/// How long a reliable packet waits for its acknowledgment before being sent again, and
/// for how long it is sent again before its client is assumed gone. A poll timeout (see
/// [`App::poll_until`]) lets resends go out while no packet comes in.
const RESEND_TIMEOUT: Duration = Duration::from_millis(400);
const RESEND_GIVE_UP: Duration = Duration::from_secs(20);

/// Resolves a client-to-cell call's method table from the entity it names, for
/// [`element::CellEntityMethod`].
struct CellTables<'a> {
    entities: &'a Slots<EntityState>,
    entity_keys: &'a HashMap<u32, EntityKey<AnyEntity>>,
    dispatch: &'a ScriptDispatch,
    /// Substituted for a wire entity id of 0, exactly as the real base app does.
    cell_entity: EntityKey<AnyEntity>,
}

impl element::CellMethodTables for CellTables<'_> {
    fn cell_methods(&self, entity_id: u32) -> Option<&[MethodDef]> {
        let key = match entity_id {
            0 => self.cell_entity,
            id => *self.entity_keys.get(&id)?,
        };
        let state = self.entities.get(key.slot)?;
        Some(&self.dispatch.entity_from_id(state.type_id)?.cell_methods)
    }
}

/// A `BASE_ENTITY_METHOD` or `CELL_ENTITY_METHOD`-range element skipped without knowing
/// its method table: its framing is always `Variable16`, whatever the entity.
struct SkippedEntityMethod;

impl Element<()> for SkippedEntityMethod {

    fn write_length(&self, _config: &()) -> io::Result<ElementLength> {
        unreachable!("SkippedEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &()) -> io::Result<u8> {
        unreachable!("SkippedEntityMethod is read-only")
    }

    fn read_length(_config: &(), _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, _config: &(), _len: usize, _id: u8) -> io::Result<Self> {
        io::copy(read, &mut io::sink())?;
        Ok(Self)
    }

}


// ============= //
//    CLIENTS    //
// ============= //

impl App {

    /// Accept the login of the given user, in response to [`Event::Login`], registering
    /// the client (blowfish key included -- everything sent to this client from this
    /// point on, including this very reply, goes out encrypted) and replying with a
    /// freshly generated session key. A client logged in earlier from the same address
    /// is gone, with its entities.
    ///
    /// This returns the new client, if one was actually waiting for this reply.
    pub fn answer_login_success(&mut self, addr: SocketAddr, blowfish: Arc<Blowfish>) -> Option<ClientKey> {

        let request_id = self.pending_clients.remove(&addr)?;

        if let Some(previous) = self.client_keys.remove(&addr) {
            self.remove_client(previous);
        }

        let session_key = OsRng.next_u32();

        let slot = self.clients.insert(ClientState {
            session_key,
            blowfish: Arc::clone(&blowfish),
            player: None,
            witness: None,
        });
        let key = ClientKey { slot, addr };
        self.client_keys.insert(addr, key);

        self.socket.set_encryption(addr, blowfish);

        self.bundle.clear();
        let _ = self.bundle.element_writer().write_simple_reply(SessionKey { session_key }, request_id);
        self.protocol.off_channel(addr).prepare(&mut self.bundle, false);
        let _ = self.socket.send_bundle(&self.bundle, addr);

        Some(key)

    }

    /// Tell a client the server's update frequency and game time, which the real server
    /// does right after the login reply. `unknown` is a field of unknown meaning, see
    /// [`client::element::UpdateFrequencyNotification`].
    pub fn update_frequency(&mut self, client: ClientKey, frequency: u8, unknown: u16, game_time: u32) -> io::Result<()> {
        self.send_simple(client, client::element::UpdateFrequencyNotification { frequency, unknown, game_time })
    }

    /// Tell a client where its cell app is, before creating its cell player.
    pub fn set_cell_app_ext_address(&mut self, client: ClientKey, cell_app_addr: SocketAddrV4) -> io::Result<()> {
        self.send_simple(client, client::element::SetCellAppExtAddress(cell_app_addr.into()))
    }

    /// A logged in client's session key and blowfish key. A client told to switch base
    /// app logs in again with both: the session key as its login key, the blowfish key
    /// for the new channel.
    pub fn client_keys(&self, client: ClientKey) -> Option<(u32, Arc<Blowfish>)> {
        self.client(client).ok().map(|client| (client.session_key, Arc::clone(&client.blowfish)))
    }

    /// Tell the client to disconnect and reconnect to a (possibly different) base app
    /// address.
    pub fn switch_base_app(&mut self, client: ClientKey, base_addr: SocketAddrV4, reset_entities: bool) -> io::Result<()> {
        self.send_simple(client, client::element::SwitchBaseApp { base_addr: base_addr.into(), reset_entities })
    }

    /// Push a resource to the client (`ResourceHeader` followed by one or more
    /// `ResourceFragment`s), e.g. in response to `ClientCommandsPort.doCmdInt3` asking
    /// for a `RES_STREAM` reply. `data` is sent as-is, chunked into fragments -- any
    /// compression/framing (e.g. WoT's zlib+pickle blobs) is the caller's job, since
    /// this library has no opinion on it.
    ///
    /// Sent as one reliable bundle per fragment rather than one giant multi-packet
    /// bundle: `Protocol`'s own bundle-fragment reassembly has a hardcoded 10s timeout
    /// (`FRAGMENT_TIMEOUT`), and real captured traffic paces large resources over
    /// several seconds of separate fragments rather than one burst. Fragments are 8 KB,
    /// like the real server's (up to 9.8 KB for a 443 KB account cache), so that a
    /// resource that large fits the 256 fragments a one-byte sequence number can count.
    /// What waits in the client's outbox is sent before, to keep the order it was written
    /// in.
    pub fn push_resource(&mut self, client: ClientKey, res_id: u16, description: Vec<u8>, data: &[u8]) -> io::Result<()> {

        const CHUNK_SIZE: usize = 8192;

        self.client(client)?;
        let addr = client.addr;

        if data.len() > CHUNK_SIZE * 256 {
            return Err(io_invalid_data(format_args!("resource of {} bytes, more than 256 fragments", data.len())));
        }

        self.outbox.flush(addr, &mut self.protocol, &self.socket)?;

        self.bundle.clear();
        self.bundle.element_writer().write_simple(client::element::ResourceHeader { id: res_id, description })?;
        self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
        self.socket.send_bundle(&self.bundle, addr)?;

        let chunks: Vec<&[u8]> = if data.is_empty() {
            vec![&[]]
        } else {
            data.chunks(CHUNK_SIZE).collect()
        };

        let last_seq = (chunks.len() - 1) as u8;

        for (seq, chunk) in chunks.into_iter().enumerate() {
            self.bundle.clear();
            self.bundle.element_writer().write_simple(client::element::ResourceFragment {
                id: res_id,
                sequence_num: seq as u8,
                last: seq as u8 == last_seq,
                data: chunk.to_vec(),
            })?;
            self.protocol.channel(addr, None).prepare(&mut self.bundle, true);
            self.socket.send_bundle(&self.bundle, addr)?;
        }

        Ok(())

    }

    /// Reset all entities known to the given client, optionally keeping its current
    /// base-player entity alive (`keep_player_on_base`). This is the only entity
    /// *removal* mechanism the wire protocol has -- there's no way to remove a single
    /// arbitrary entity, only "reset everything" (see `re-work/doc/ENTITY.md`).
    pub fn reset_entities(&mut self, client: ClientKey, keep_player_on_base: bool) -> io::Result<()> {

        self.send_simple(client, client::element::ResetEntities { keep_player_on_base })?;

        let keep = keep_player_on_base
            .then(|| self.clients.get(client.slot).and_then(|c| c.player))
            .flatten();

        self.remove_entities(|entity, state| state.client == Some(client) && Some(entity) != keep);

        if let Some(keep) = keep {
            self.leave_space(keep);
        }
        if let Some(client) = self.clients.get_mut(client.slot) {
            if !keep_player_on_base {
                client.player = None;
            }
        }

        Ok(())

    }

    /// Forget a client and its entities.
    fn remove_client(&mut self, key: ClientKey) {
        self.clients.remove(key.slot);
        self.outbox.remove(key.addr);
        self.remove_entities(|_, state| state.client == Some(key));
    }

    /// A logged in client, if not gone.
    fn client(&self, key: ClientKey) -> io::Result<&ClientState> {
        self.clients.get(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such client: {}", key.addr)))
    }

    /// The outbox of a logged in client.
    fn out(&mut self, key: ClientKey) -> io::Result<&mut ClientOutbox> {
        self.client(key)?;
        self.outbox.client(key.addr, &mut self.protocol, &self.socket)
    }

    /// Write a simple element to a client's outbox.
    fn send_simple<E: SimpleElement>(&mut self, client: ClientKey, element: E) -> io::Result<()> {
        self.out(client)?.write_simple(element)
    }

}

/// A logged in client of the base app: its address and its place among the app's
/// clients, which no later client at the same address takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientKey {
    slot: SlotKey,
    addr: SocketAddr,
}

impl ClientKey {

    /// The address the client is at.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

}

impl fmt::Display for ClientKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.addr, f)
    }
}

/// A logged in client of the base app.
#[derive(Debug)]
struct ClientState {
    /// The session key for this client.
    session_key: u32,
    /// The blowfish key for encryption of this client's packets.
    blowfish: Arc<Blowfish>,
    /// This client's current base-player entity, if any created yet. There's only ever
    /// one at a time (real BigWorld: `Proxy` holds a single client channel, and a base
    /// method call carries no entity id of its own -- it always targets this one).
    player: Option<EntityKey<AnyEntity>>,
    /// What it knows of the space its cell player is in (e.g. its `Avatar` while in a
    /// battle), which its cell calls go to, see [`App::create_cell_player`].
    witness: Option<Witness>,
}


// ============== //
//    ENTITIES    //
// ============== //

impl App {

    /// View an entity through its schema. What its client methods and properties give
    /// goes to its client right away, or once in a space, to the clients knowing it at
    /// the end of the tick, see [`Self::end_tick`].
    pub fn entity<T: EntitySchema>(&mut self, key: EntityKey<T>) -> io::Result<Entity<'_, T>> {

        let state = self.entities.get_mut(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {key}")))?;
        let EntityState { client, type_id, properties, cell } = state;

        let bound = self.entity_schemas.bound(*type_id)
            .filter(|bound| bound.is::<T>())
            .ok_or_else(|| io_invalid_data(format_args!("entity {key} is not a registered {}", T::NAME)))?;
        let dispatch = self.dispatch.entity_from_id(*type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;

        // In a space, what it gives goes to the clients knowing it, and it may have
        // components. Otherwise, it goes to its own client.
        let (sink, components) = match (cell, *client) {
            (Some(cell), _) => {
                let space = self.spaces.get_mut(cell.space.slot)
                    .ok_or_else(|| io_invalid_data(format_args!("entity {key} is in no space")))?;
                let components = EntityComponents {
                    components: &mut cell.components,
                    nrl_node: &mut cell.nrl_node,
                    nrl_nodes: &mut space.nrl_nodes,
                    registry: &self.component_schemas,
                    script: &self.dispatch,
                };
                (EntitySink::Space(&mut space.events), Some(components))
            }
            (None, Some(client)) => {
                let is_player = self.clients.get(client.slot).and_then(|c| c.player) == Some(key.any());
                let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;
                (EntitySink::Client { out, is_player }, None)
            }
            (None, None) => return Err(io_invalid_data(format_args!("entity {key} has no client and is in no space"))),
        };

        Ok(Entity::new(RawEntity {
            slot: key.slot,
            id: key.id,
            properties,
            dispatch,
            bound,
            sink,
            components,
        }))

    }

    /// View a component of an entity through its schema, see [`Entity::add_component`].
    pub fn component<C: ComponentSchema>(&mut self, key: ComponentKey<C>) -> io::Result<Component<'_, C>> {

        let state = self.entities.get_mut(key.entity.slot)
            .and_then(|state| state.cell.as_mut())
            .and_then(|cell| cell.components.get_mut(key.slot))
            .ok_or_else(|| io_invalid_data(format_args!("no such component: {key:?}")))?;

        let type_id = state.type_id();
        let bound = self.component_schemas.bound(type_id)
            .filter(|bound| bound.is::<C>())
            .ok_or_else(|| io_invalid_data(format_args!("component {key:?} is not a registered {}", C::NAME)))?;
        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown component type: {type_id}")))?;

        Ok(Component::new(RawComponent { entity: key.entity, slot: key.slot, state, dispatch, bound }))

    }

    /// Remove a component of an entity, see [`Entity::add_component`]: the clients that
    /// have it remove it at the end of the tick.
    pub fn remove_component<C: ComponentSchema>(&mut self, key: ComponentKey<C>) -> io::Result<()> {

        let cell = self.entities.get_mut(key.entity.slot)
            .and_then(|state| state.cell.as_mut())
            .ok_or_else(|| io_invalid_data(format_args!("no such component: {key:?}")))?;

        let state = cell.components.remove(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such component: {key:?}")))?;
        cell.removed_components.push(state.removed(key.slot));

        Ok(())

    }

    /// The key of an entity of any type as one of the given schema, if it is one.
    pub fn cast<T: EntitySchema>(&self, key: EntityKey<AnyEntity>) -> Option<EntityKey<T>> {
        let state = self.entities.get(key.slot)?;
        self.entity_schemas.bound(state.type_id)?
            .is::<T>()
            .then(|| key.cast())
    }

    /// Decode a call a client made to one of an entity's base or cell methods, if the
    /// entity is of the given calls' schema and the method one they declare.
    pub fn decode<M: Methods>(&self, entity: EntityKey<AnyEntity>, call: &MethodCall) -> Option<(EntityKey<M::Schema>, M)> {
        let state = self.entities.get(entity.slot)?;
        let call = self.entity_schemas.bound(state.type_id)?.decode(call)?;
        Some((entity.cast(), call))
    }

    /// Create a new base-player entity of the given schema for a client. This becomes
    /// that client's current base-player entity, replacing any previous one (real
    /// BigWorld only ever has one at a time).
    ///
    /// `entity_data` is of the entity's client-visible data type (see
    /// [`crate::app::dispatch::EntityDispatch::data_ty`]), checked before anything is
    /// sent: a dict of its first property values. A [`Value`] passes unchecked.
    pub fn create_base_player<T: EntitySchema>(
        &mut self,
        client: ClientKey,
        entity_data: impl ScriptType,
        entity_components_count: u8,
    ) -> io::Result<EntityKey<T>> {

        self.client(client)?;
        let entity_type_id = self.entity_schemas.type_of::<T>()?;
        let entity_id = self.new_entity_id();

        let dispatch = self.dispatch.entity_from_id(entity_type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {entity_type_id}")))?;
        if !check_data(&entity_data, &dispatch.data_ty) {
            return Err(io_invalid_data(format_args!("{} is not the data of a {}", std::any::type_name_of_val(&entity_data), T::NAME)));
        }
        let entity_data = entity_data.into_value(&dispatch.data_ty);

        self.outbox.client(client.addr, &mut self.protocol, &self.socket)?.write(
            client::element::CreateBasePlayer {
                entity_id,
                entity_type_id,
                entity_data: Cow::Borrowed(&entity_data),
                entity_components_count,
            },
            dispatch,
        )?;

        let properties = match entity_data {
            Value::Dict(fields) => fields.into_iter()
                .filter_map(|(name, value)| Some((dispatch.properties.iter().position(|def| def.name == name)?, value)))
                .collect(),
            _ => Vec::new(),
        };
        let key = self.insert_entity(entity_id, Some(client), entity_type_id, None);
        self.store_properties(key, properties);

        if let Some(client) = self.clients.get_mut(client.slot) {
            client.player = Some(key);
        }

        Ok(key.cast())

    }

    /// Call a method, resolved by name against the target entity's client-method table,
    /// on the client owning the given entity, selecting it first if needed: for calls a
    /// schema doesn't declare, those replayed from a capture say.
    pub fn call_method<T>(&mut self, entity: EntityKey<T>, method_name: &str, args: Vec<Value>) -> io::Result<()> {

        let state = self.entity_state(entity)?;
        let client = state.client.ok_or_else(|| io_invalid_data(format_args!("entity {entity} has no client")))?;

        let dispatch = self.dispatch.entity_from_id(state.type_id)
            .ok_or_else(|| io_invalid_data(format_args!("no dispatch table for entity: {entity}")))?;

        let (exposed_id, name) = dispatch.client_methods.iter().enumerate()
            .find(|(_, m)| &*m.name == method_name)
            .map(|(exposed_id, m)| (exposed_id as u16, m.name.clone()))
            .ok_or_else(|| io_invalid_data(format_args!("unknown client method: {method_name}")))?;

        let is_player = self.client(client)?.player == Some(entity.any());
        let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;
        if is_player {
            out.select_player()?;
        } else {
            out.select_entity(entity.id)?;
        }

        out.write_selected(
            client::element::EntityMethod { call: MethodCall::Known { exposed_id, name, args } },
            &dispatch.client_methods,
        )

    }

    /// An entity, if not gone.
    fn entity_state<T>(&self, key: EntityKey<T>) -> io::Result<&EntityState> {
        self.entities.get(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {key}")))
    }

    /// A new unique entity id.
    fn new_entity_id(&mut self) -> u32 {
        loop {
            let id = self.entities_next_id.0;
            self.entities_next_id += 1;
            if !self.entity_keys.contains_key(&id) {
                break id;
            }
        }
    }

    /// Track a new entity, of a client or in a space, replacing any of the same id.
    fn insert_entity(&mut self, id: u32, client: Option<ClientKey>, type_id: u16, cell: Option<EntityCell>) -> EntityKey<AnyEntity> {

        if let Some(&previous) = self.entity_keys.get(&id) {
            self.remove_entity(previous);
        }

        let space = cell.as_ref().map(|cell| cell.space);
        let properties = self.dispatch.entity_from_id(type_id).map_or(0, |dispatch| dispatch.properties.len());
        let slot = self.entities.insert(EntityState { client, type_id, properties: vec![None; properties], cell });
        let key = EntityKey::new(slot, id);
        self.entity_keys.insert(id, key);

        if let Some(space) = space.and_then(|space| self.spaces.get_mut(space.slot)) {
            space.entities.push(key);
        }

        key

    }

    /// Forget an entity, and its cell part.
    fn remove_entity(&mut self, key: EntityKey<AnyEntity>) {
        self.leave_space(key);
        if self.entities.remove(key.slot).is_some() {
            self.entity_keys.remove(&key.id);
        }
    }

    /// Forget the entities the given predicate selects.
    fn remove_entities(&mut self, mut predicate: impl FnMut(EntityKey<AnyEntity>, &EntityState) -> bool) {

        let entities = &self.entities;
        let removed: Vec<_> = self.entity_keys.values()
            .copied()
            .filter(|&key| entities.get(key.slot).is_some_and(|state| predicate(key, state)))
            .collect();

        for key in removed {
            self.remove_entity(key);
        }

    }

    /// Keep the given property values of an entity, by exposed id.
    fn store_properties<T>(&mut self, key: EntityKey<T>, values: impl IntoIterator<Item = (usize, Value)>) {
        let Some(state) = self.entities.get_mut(key.slot) else { return };
        for (exposed_id, value) in values {
            if let Some(slot) = state.properties.get_mut(exposed_id) {
                *slot = Some(value);
            }
        }
    }

}

/// One entity this app has created: the client that owns it, if a base entity, its wire
/// type id, its client-visible property values, by exposed id, when known, and its cell
/// part, once in a space.
#[derive(Debug)]
struct EntityState {
    client: Option<ClientKey>,
    type_id: u16,
    properties: Vec<Option<Value>>,
    cell: Option<EntityCell>,
}

/// The cell part of an entity, in a space.
#[derive(Debug)]
struct EntityCell {
    space: SpaceKey,
    position: Vec3,
    /// Roll, pitch and yaw.
    direction: Vec3,
    /// Whether it moved during the tick.
    moved: bool,
    /// See [`EntityInit::alias`], [`EntityInit::controlled_by`] and
    /// [`EntityInit::visibility`].
    alias: bool,
    controlled_by: Option<EntityKey<AnyEntity>>,
    visibility: Visibility,
    /// Its dynamic components, those removed during the tick, and its NRL node once it
    /// has any.
    components: Slots<ComponentState>,
    removed_components: Vec<RemovedComponent>,
    nrl_node: Option<u16>,
}

impl EntityCell {

    fn new(space: SpaceKey, position: Vec3, direction: Vec3, alias: bool, controlled_by: Option<EntityKey<AnyEntity>>, visibility: Visibility) -> Self {
        Self { 
            space, 
            position, 
            direction, 
            moved: false, 
            alias, 
            controlled_by, 
            visibility, 
            components: Slots::default(), 
            removed_components: Vec::new(),
            nrl_node: None,
        }
    }

    /// Whether the given entity controls it, whose client is then its own.
    fn is_controlled_by(&self, entity: EntityKey<AnyEntity>) -> bool {
        self.controlled_by == Some(entity)
    }

}

/// Whether the given data is of the given type, see [`App::create_base_player`].
fn check_data<D: ScriptType>(_data: &D, ty: &Ty) -> bool {
    D::check(ty)
}

/// What of an entity's properties a client gets as it enters its area of interest: those
/// with a known value, but those for its own client only when it is.
fn visible_properties(state: &EntityState, dispatch: &EntityDispatch, own: bool) -> ChangedProperties {

    let values = state.properties.iter().enumerate()
        .filter_map(|(exposed_id, value)| {
            let def = &dispatch.properties[exposed_id];
            let own_client = matches!(def.flags, PropertyFlags::OwnClient | PropertyFlags::BaseAndClient);
            let value = value.as_ref().filter(|_| own || !own_client)?;
            Some((exposed_id as u8, Arc::clone(&def.name), value.clone()))
        })
        .collect();

    ChangedProperties { values, stopped: None }

}


// ============ //
//    SPACES    //
// ============ //

impl App {

    /// Create a space of the given id, which the protocol carries: one from a capture,
    /// whose space properties name it, say.
    pub fn create_space(&mut self, id: u32) -> io::Result<SpaceKey> {

        if self.space_keys.contains_key(&id) {
            return Err(io_invalid_data(format_args!("space {id} already exists")));
        }

        let slot = self.spaces.insert(SpaceState::default());
        let key = SpaceKey { slot, id };
        self.space_keys.insert(id, key);
        Ok(key)

    }

    /// Destroy a space: the entities in it without a client are gone, the others only
    /// leave it.
    pub fn destroy_space(&mut self, key: SpaceKey) {

        let Some(space) = self.spaces.get(key.slot) else { return };

        for entity in space.entities.clone() {
            match self.entities.get(entity.slot).and_then(|state| state.client) {
                Some(_) => self.leave_space(entity),
                None => self.remove_entity(entity),
            }
        }

        self.spaces.remove(key.slot);
        self.space_keys.remove(&key.id);

    }

    /// Give a space one of its properties, still raw, see [`client::element::SpaceProperty`]:
    /// clients get it once in the space, in the order given.
    pub fn set_space_property(&mut self, space: SpaceKey, data: Vec<u8>) -> io::Result<()> {
        self.add_space_data(space, SpaceData::Property(data))
    }

    /// Map a geometry (a map, `spaces/08_ruinberg` say) into a space, at the identity
    /// transform: clients get it once in the space, after what was given before.
    pub fn add_space_geometry_mapping(&mut self, space: SpaceKey, entry_id: SocketAddrV4, path: &str) -> io::Result<()> {
        self.add_space_data(space, SpaceData::GeometryMapping { entry_id, path: path.to_string() })
    }

    /// Get a client's base-player entity into a space, with a cell part: the client gets
    /// its cell player and the space's data right away, then what is in the space from
    /// the end of the tick on. Its cell method calls go to it.
    pub fn create_cell_player<T>(&mut self, player: EntityKey<T>, space: SpaceKey, init: CellPlayerInit) -> io::Result<()> {

        let state = self.entity_state(player)?;
        let client = state.client.ok_or_else(|| io_invalid_data(format_args!("entity {player} has no client")))?;
        let data = &self.spaces.get(space.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such space: {space}")))?
            .data;

        let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;
        out.write_simple(client::element::CreateCellPlayer {
            unk_flag: 0,
            space_id: space.id,
            unk_short: 0,
            vehicle_id: init.vehicle_id,
            position: init.position,
            packed_xz_scale: init.packed_xz_scale,
            direction: init.direction,
            cell_data: init.cell_data,
        })?;
        for data in data {
            data.write(out, space)?;
        }

        self.leave_space(player.any());
        if let Some(state) = self.entities.get_mut(player.slot) {
            state.cell = Some(EntityCell::new(space, init.position, init.direction, true, None, Visibility::Interest));
        }
        if let Some(space) = self.spaces.get_mut(space.slot) {
            space.entities.push(player.any());
        }
        if let Some(client) = self.clients.get_mut(client.slot) {
            client.witness = Some(Witness::new(player.any()));
        }

        Ok(())

    }

    /// Create an entity of the given schema in a space, see [`Self::create_entity_dynamic`].
    pub fn create_entity<T: EntitySchema>(&mut self, space: SpaceKey, init: EntityInit) -> io::Result<EntityKey<T>> {
        let type_id = self.entity_schemas.type_of::<T>()?;
        self.create_entity_dynamic(space, type_id, init).map(EntityKey::cast)
    }

    /// Create an entity of the given type in a space, which needs no schema. Its
    /// properties are its first property values: clients get those visible to them as
    /// it enters their area of interest, see [`EntityInit::alias`].
    pub fn create_entity_dynamic(&mut self, space: SpaceKey, type_id: u16, init: EntityInit) -> io::Result<EntityKey<AnyEntity>> {

        if self.dispatch.entity_from_id(type_id).is_none() {
            return Err(io_invalid_data(format_args!("unknown entity type: {type_id}")));
        }
        if self.spaces.get(space.slot).is_none() {
            return Err(io_invalid_data(format_args!("no such space: {space}")));
        }

        let id = match init.id {
            Some(EntityId(id)) => id,
            None => self.new_entity_id(),
        };

        let cell = EntityCell::new(space, init.position, init.direction, init.alias, init.controlled_by, init.visibility);
        let key = self.insert_entity(id, None, type_id, Some(cell));
        self.store_properties(key, init.properties.values.into_iter().map(|(exposed_id, _, value)| (exposed_id.into(), value)));
        Ok(key)

    }

    /// Destroy an entity: it leaves the area of interest of the clients knowing it.
    pub fn destroy_entity<T>(&mut self, key: EntityKey<T>) {
        self.remove_entity(key.any());
    }

    /// Take an entity out of its space, if in one: it leaves the area of interest of the
    /// clients knowing it, and its own client no longer knows that space.
    pub fn leave_space<T>(&mut self, entity: EntityKey<T>) {

        let Some(state) = self.entities.get_mut(entity.slot) else { return };
        let Some(cell) = state.cell.take() else { return };

        if let Some(space) = self.spaces.get_mut(cell.space.slot) {
            space.entities.retain(|&other| other != entity.any());
        }

        if let Some(client) = state.client.and_then(|client| self.clients.get_mut(client.slot)) {
            if client.witness.as_ref().is_some_and(|witness| witness.entity == entity.any()) {
                client.witness = None;
            }
        }

    }

    /// The space of an entity, if in one.
    pub fn space_of<T>(&self, entity: EntityKey<T>) -> Option<SpaceKey> {
        Some(self.entities.get(entity.slot)?.cell.as_ref()?.space)
    }

    /// Where an entity in a space is, and its direction (roll, pitch and yaw).
    pub fn position<T>(&self, entity: EntityKey<T>) -> Option<(Vec3, Vec3)> {
        let cell = self.entities.get(entity.slot)?.cell.as_ref()?;
        Some((cell.position, cell.direction))
    }

    /// Move an entity in a space, with its direction (roll, pitch and yaw): the clients
    /// knowing it get where it is at the end of the tick.
    pub fn set_position<T>(&mut self, entity: EntityKey<T>, position: Vec3, direction: Vec3) -> io::Result<()> {

        let cell = self.entities.get_mut(entity.slot)
            .and_then(|state| state.cell.as_mut())
            .ok_or_else(|| io_invalid_data(format_args!("entity {entity} is in no space")))?;

        cell.position = position;
        cell.direction = direction;
        cell.moved = true;
        Ok(())

    }

    /// Keep some data of a space for every client getting into it, and give it to those
    /// already in it.
    fn add_space_data(&mut self, key: SpaceKey, data: SpaceData) -> io::Result<()> {

        let space = self.spaces.get_mut(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such space: {key}")))?;

        let clients: Vec<_> = space.entities.iter()
            .filter_map(|entity| self.entities.get(entity.slot)?.client)
            .filter(|client| self.clients.get(client.slot).is_some_and(|c| c.witness.is_some()))
            .collect();
        for client in clients {
            data.write(self.outbox.client(client.addr, &mut self.protocol, &self.socket)?, key)?;
        }

        space.data.push(data);
        Ok(())

    }

}

/// A space of the app: its id and its place among the app's spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceKey {
    slot: SlotKey,
    id: u32,
}

impl SpaceKey {

    /// The space's id, as the protocol carries it.
    pub fn id(&self) -> u32 {
        self.id
    }

}

impl fmt::Display for SpaceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.id, f)
    }
}

/// A space: its entities, and what its clients get of it.
#[derive(Debug, Default)]
struct SpaceState {
    /// Its entities, in the order they got in it, which they enter areas of interest in.
    entities: Vec<EntityKey<AnyEntity>>,
    /// Its properties and geometry mappings, which clients get once in it.
    data: Vec<SpaceData>,
    /// What its entities gave their clients during the tick.
    events: Vec<EntityEvent>,
    /// The NRL node ids of its entities and their components.
    nrl_nodes: NrlNodeIds,
}

/// Something of a space its clients get once in it.
#[derive(Debug)]
enum SpaceData {
    /// See [`client::element::SpaceProperty`].
    Property(Vec<u8>),
    /// A geometry mapped at the identity transform.
    GeometryMapping { entry_id: SocketAddrV4, path: String },
}

impl SpaceData {

    fn write(&self, out: &mut ClientOutbox, space: SpaceKey) -> io::Result<()> {
        match self {
            Self::Property(data) => out.write_simple(client::element::SpaceProperty { data: data.clone() }),
            Self::GeometryMapping { entry_id, path } => {

                let mut transform = [0.0; 16];
                for i in 0..4 {
                    transform[i * 5] = 1.0;
                }

                out.write_simple(client::element::AddSpaceGeometryMapping {
                    space_id: space.id,
                    entry_id: (*entry_id).into(),
                    path: path.clone(),
                    transform,
                    unk: 1,
                    rest: Vec::new(),
                })

            }
        }
    }

}

/// The first state of an entity created in a space, see [`App::create_entity`].
#[derive(Debug, Clone, Default)]
pub struct EntityInit {
    /// Its id, a new one when none: one from a capture, say.
    pub id: Option<EntityId>,
    pub position: Vec3,
    /// Roll, pitch and yaw.
    pub direction: Vec3,
    /// Whether it enters areas of interest with an id alias, which its position updates
    /// use. Each client asks for one that has one before it is created; one without is
    /// created right away, an arena's info entity say.
    pub alias: bool,
    /// The entity whose client is its own, for those of its properties that only its own
    /// client gets: the avatar driving a vehicle, say. Its own client always knows it.
    pub controlled_by: Option<EntityKey<AnyEntity>>,
    pub visibility: Visibility,
    /// Its first property values, given in its type's property table.
    pub properties: ChangedProperties,
}

/// The cell part of a client's player entity, see [`App::create_cell_player`].
#[derive(Debug, Clone)]
pub struct CellPlayerInit {
    /// The entity the player is on, see [`client::element::CreateCellPlayer`].
    pub vehicle_id: u32,
    pub position: Vec3,
    /// Roll, pitch and yaw.
    pub direction: Vec3,
    pub packed_xz_scale: f32,
    /// The client's cell data, still encoded, given to its client as is.
    pub cell_data: Vec<u8>,
}


// =============== //
//    WITNESSES    //
// =============== //

impl App {

    /// Set what of its space a client knows, from the next tick on: [`Interest::WholeSpace`]
    /// until then. It must have a cell player, see [`Self::create_cell_player`].
    pub fn set_interest(&mut self, client: ClientKey, interest: Interest) -> io::Result<()> {
        self.witness_mut(client)?.interest = interest;
        Ok(())
    }

    /// Show an entity of its space to a client whose interest is [`Interest::Manual`]: it
    /// enters its area of interest with the next tick, until hidden.
    pub fn show<T>(&mut self, client: ClientKey, entity: EntityKey<T>) -> io::Result<()> {
        self.witness_mut(client)?.shown.insert(entity.any());
        Ok(())
    }

    /// Hide an entity shown to a client, see [`Self::show`]: it leaves its area of
    /// interest with the next tick.
    pub fn hide<T>(&mut self, client: ClientKey, entity: EntityKey<T>) -> io::Result<()> {
        self.witness_mut(client)?.shown.remove(&entity.any());
        Ok(())
    }

    /// How many ticks a client got in its space: its game time, in ticks.
    pub fn ticks(&self, client: ClientKey) -> Option<u32> {
        Some(self.client(client).ok()?.witness.as_ref()?.ticks)
    }

    /// End a tick: tell each client in a space what changed in its area of interest since
    /// the last one, in a bundle the tick opens, see [`Witness`].
    pub fn end_tick(&mut self) {

        let clients: Vec<_> = self.client_keys.values().copied().collect();
        for client in clients {
            if let Err(error) = self.tick_client(client) {
                self.events.push_back(Event::IoError(IoErrorEvent { error, addr: Some(client.addr) }));
            }
        }

        for (_, state) in self.entities.iter_mut() {
            if let Some(cell) = &mut state.cell {
                cell.moved = false;
                cell.removed_components.clear();
                for (_, component) in cell.components.iter_mut() {
                    component.end_tick();
                }
            }
        }

        for (_, space) in self.spaces.iter_mut() {
            space.events.clear();
        }

    }

    /// A client asks for an entity that entered its area of interest: it gets created
    /// with the next tick.
    fn request_entity(&mut self, client: ClientKey, entity_id: u32) {

        let known = self.clients.get_mut(client.slot)
            .and_then(|client| client.witness.as_mut())
            .and_then(|witness| witness.known.iter_mut().find(|known| known.entity.id == entity_id));

        match known {
            Some(known) => known.requested = true,
            None => debug!(addr = %client.addr, "Asked for entity {entity_id}, which it doesn't know"),
        }

    }

    /// Tell a client in a space what changed in its area of interest, see
    /// [`Witness::write_tick`].
    fn tick_client(&mut self, client: ClientKey) -> io::Result<()> {

        // Taken out of its client for the tick, which reads the rest of the app.
        let Some(mut witness) = self.clients.get_mut(client.slot).and_then(|c| c.witness.take()) else {
            return Ok(());
        };

        let result = self.write_tick(client, &mut witness);

        if let Some(state) = self.clients.get_mut(client.slot) {
            state.witness = Some(witness);
        }

        result

    }

    fn write_tick(&mut self, client: ClientKey, witness: &mut Witness) -> io::Result<()> {

        let Some(player) = self.entities.get(witness.entity.slot).and_then(|state| state.cell.as_ref()) else { return Ok(()) };
        let Some(space) = self.spaces.get(player.space.slot) else { return Ok(()) };
        let view = TickView { entities: &self.entities, space, dispatch: &self.dispatch, player };

        // What was written to it before goes first, the tick opens a bundle.
        self.outbox.flush(client.addr, &mut self.protocol, &self.socket)?;
        let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;

        let mut created = Vec::new();
        let result = witness.write_tick(out, &view, &mut created);

        for entity in created {
            self.events.push_back(Event::Entered(EnteredEvent { client, entity }));
        }

        result

    }

    /// What a client knows of its space, if it has a cell player.
    fn witness_mut(&mut self, key: ClientKey) -> io::Result<&mut Witness> {
        self.clients.get_mut(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such client: {}", key.addr)))?
            .witness.as_mut()
            .ok_or_else(|| io_invalid_data(format_args!("client {} has no cell player", key.addr)))
    }

}

/// What of its space a client knows, see [`App::set_interest`]: which entities are in its
/// area of interest, besides those it controls and those always visible (see
/// [`Visibility`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Interest {
    /// Every entity of its space.
    #[default]
    WholeSpace,
    /// The entities within `enter` metres of its player entity on the ground plane, until
    /// farther than `leave`, so that one on the edge doesn't come and go.
    Radius { enter: f32, leave: f32 },
    /// The entities shown to it, see [`App::show`]: those an enemy spotted, say.
    Manual,
}

/// Which clients of its space know an entity, see [`EntityInit::visibility`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Those whose interest has it, see [`Interest`].
    #[default]
    Interest,
    /// Every one of them, whatever their interest: an arena's info entity, say.
    Always,
}

/// What a client knows of the space its player entity is in.
///
/// Each tick sends it, in a bundle the tick opens: the entities entering and leaving its
/// area of interest, those it asked for, where it and those it knows are, what their
/// properties and client methods gave, then their dynamic components.
#[derive(Debug)]
struct Witness {
    /// Its player entity.
    entity: EntityKey<AnyEntity>,
    /// The entities in its area of interest, in the order they entered it.
    known: Vec<WitnessEntity>,
    /// Which id aliases are taken.
    aliases: Vec<bool>,
    /// The ticks sent to it, the first being tick 0.
    ticks: u32,
    /// What of its space it knows, and the entities shown to it, see [`App::show`].
    interest: Interest,
    shown: HashSet<EntityKey<AnyEntity>>,
    /// The entities whose NRL node it has, with their components it has.
    nrl: HashMap<EntityKey<AnyEntity>, Vec<SlotKey>>,
}

/// An entity in the area of interest of a witness.
#[derive(Debug)]
struct WitnessEntity {
    entity: EntityKey<AnyEntity>,
    alias: Option<u8>,
    /// Whether its client asked for it, and whether it was created on it since.
    requested: bool,
    created: bool,
}

/// What a witness's tick reads: the space its player entity is in, with the entities
/// and the tables.
struct TickView<'a> {
    entities: &'a Slots<EntityState>,
    space: &'a SpaceState,
    dispatch: &'a ScriptDispatch,
    /// The cell of its player entity.
    player: &'a EntityCell,
}

impl<'a> TickView<'a> {

    /// An entity in a space, with its cell.
    fn get(&self, key: EntityKey<AnyEntity>) -> Option<(&'a EntityState, &'a EntityCell)> {
        let state = self.entities.get(key.slot)?;
        Some((state, state.cell.as_ref()?))
    }

    /// The tables of an entity's type.
    fn dispatch(&self, state: &EntityState) -> io::Result<&'a EntityDispatch> {
        self.dispatch.entity_from_id(state.type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {}", state.type_id)))
    }

}

impl Witness {

    fn new(entity: EntityKey<AnyEntity>) -> Self {
        Self {
            entity,
            known: Vec::new(),
            aliases: vec![false; usize::from(NO_ALIAS)],
            ticks: 0,
            interest: Interest::WholeSpace,
            shown: HashSet::new(),
            nrl: HashMap::new(),
        }
    }

    /// Whether an entity of its space is in its area of interest, knowing it already or
    /// not, with its player entity at `from`.
    fn sees(&self, from: Vec3, key: EntityKey<AnyEntity>, cell: &EntityCell, known: bool) -> bool {

        if cell.visibility == Visibility::Always || cell.is_controlled_by(self.entity) {
            return true;
        }

        match self.interest {
            Interest::WholeSpace => true,
            Interest::Radius { enter, leave } => {
                let offset = cell.position - from;
                let distance = (offset.x * offset.x + offset.z * offset.z).sqrt();
                distance <= if known { leave.max(enter) } else { enter }
            }
            Interest::Manual => self.shown.contains(&key),
        }

    }

    /// Take the first free id alias.
    fn take_alias(&mut self) -> Option<u8> {
        let alias = self.aliases.iter().position(|&taken| !taken)?;
        self.aliases[alias] = true;
        Some(alias as u8)
    }

    /// Write what changed in its area of interest since the last tick, adding the
    /// entities created on its client to `created`.
    fn write_tick(&mut self, out: &mut ClientOutbox, view: &TickView, created: &mut Vec<EntityKey<AnyEntity>>) -> io::Result<()> {

        out.write_simple(client::element::TickSync { tick: self.ticks as u8 })?;
        self.ticks = self.ticks.wrapping_add(1);

        self.write_leaving(out, view)?;
        self.write_entering(out, view)?;
        self.write_created(out, view, created)?;
        self.write_positions(out, view)?;
        self.write_events(out, view)?;
        self.write_components(out, view)

    }

    /// The entities gone, no longer in its space, or out of its interest.
    fn write_leaving(&mut self, out: &mut ClientOutbox, view: &TickView) -> io::Result<()> {

        self.shown.retain(|key| view.space.entities.contains(key));

        let (known, left): (Vec<_>, Vec<_>) = std::mem::take(&mut self.known).into_iter()
            .partition(|known| view.space.entities.contains(&known.entity)
                && view.get(known.entity).is_some_and(|(_, cell)| self.sees(view.player.position, known.entity, cell, true)));
        self.known = known;

        for left in left {

            out.write_simple(client::element::LeaveAoi { entity_id: left.entity.id, last_event_numbers: Vec::new() })?;

            if let Some(alias) = left.alias {
                self.aliases[usize::from(alias)] = false;
            }

            // The client drops its NRL nodes with it: the captured server creates them
            // again, with the same ids, when it comes back.
            self.nrl.remove(&left.entity);

        }

        Ok(())

    }

    /// The entities new to its area of interest, in the order they got in the space.
    fn write_entering(&mut self, out: &mut ClientOutbox, view: &TickView) -> io::Result<()> {

        let entering: Vec<_> = view.space.entities.iter()
            .filter(|&&key| key != self.entity && !self.known.iter().any(|known| known.entity == key))
            .filter_map(|&key| Some((key, view.get(key)?.1)))
            .filter(|&(key, cell)| self.sees(view.player.position, key, cell, false))
            .collect();

        for (key, cell) in entering {

            let alias = match cell.alias {
                true => Some(self.take_alias().ok_or_else(|| io_invalid_data(format_args!("no alias left for {key}")))?),
                false => None,
            };

            out.write_simple(client::element::EnterAoi { entity_id: key.id, id_alias: alias.unwrap_or(NO_ALIAS) })?;

            // Without an alias, it is created right away rather than once asked for.
            self.known.push(WitnessEntity { entity: key, alias, requested: alias.is_none(), created: false });

        }

        Ok(())

    }

    /// The entities its client asked for, created with what of them is visible to it.
    fn write_created(&mut self, out: &mut ClientOutbox, view: &TickView, created: &mut Vec<EntityKey<AnyEntity>>) -> io::Result<()> {

        for known in &mut self.known {

            if !known.requested || known.created {
                continue;
            }

            let Some((state, cell)) = view.get(known.entity) else { continue };
            let dispatch = view.dispatch(state)?;
            let client_data = visible_properties(state, dispatch, cell.is_controlled_by(self.entity)).encode(&dispatch.properties)?;

            let (roll, pitch, yaw) = (cell.direction.x, cell.direction.y, cell.direction.z);
            match known.alias {
                Some(_) => out.write_simple(client::element::CreateEntity {
                    entity_id: known.entity.id,
                    entity_type_id: state.type_id,
                    unk_u16: 0,
                    resource_name: RESOURCE_NAME.to_string(),
                    position: cell.position,
                    direction: PackedYawPitchRoll::pack(yaw, pitch, roll, false),
                    direction_unk: CREATE_ENTITY_UNK,
                    client_data,
                })?,
                None => out.write_simple(client::element::CreateEntityDetailed {
                    entity_id: known.entity.id,
                    entity_type_id: state.type_id,
                    unk_u16: 0,
                    resource_name: RESOURCE_NAME.to_string(),
                    position: cell.position,
                    direction: cell.direction,
                    client_data,
                })?,
            }

            known.created = true;
            created.push(known.entity);

        }

        Ok(())

    }

    /// Where its player entity and the entities it knows moved.
    fn write_positions(&self, out: &mut ClientOutbox, view: &TickView) -> io::Result<()> {

        // Where it is, as the captured server places the player: a reference point at
        // its position rounded to the metre, then itself on that point.
        if view.player.moved {
            out.write_simple(client::element::RelativePosition { position: view.player.position.round() })?;
            out.write_simple(client::element::AvatarUpdatePlayerDetailed { ref_num: 0, position: Vec3::ZERO, direction: Vec3::ZERO })?;
        }

        // Each position update of the others selects its entity by alias.
        for known in &self.known {

            let (Some(id_alias), true) = (known.alias, known.created) else { continue };
            let Some((_, cell)) = view.get(known.entity).filter(|(_, cell)| cell.moved) else { continue };

            out.write_simple(client::element::AvatarUpdateAliasDetailed {
                ref_num: 0,
                id_alias,
                position: cell.position,
                direction: cell.direction,
            })?;

        }

        Ok(())

    }

    /// What the properties and client methods of its player entity and the entities it
    /// has gave, in the order given.
    fn write_events(&self, out: &mut ClientOutbox, view: &TickView) -> io::Result<()> {

        for event in &view.space.events {

            let entity = if event.entity_id == self.entity.id {
                out.select_player()?;
                self.entity
            } else {

                let Some(known) = self.known.iter().find(|known| known.entity.id == event.entity_id && known.created) else { continue };

                // Some go to its own client only.
                let own = view.get(known.entity).is_some_and(|(_, cell)| cell.is_controlled_by(self.entity));
                if event.own_client && !own {
                    continue;
                }

                out.select_entity(event.entity_id)?;
                known.entity

            };

            let Some((state, _)) = view.get(entity) else { continue };
            event.kind.write(out, view.dispatch(state)?)?;

        }

        Ok(())

    }

    /// The dynamic components of its player entity and the entities it has, those it
    /// gets: each removed one it has unlinked, each new one created, in an element of its own since the client reads no
    /// further than the first component of one, under its entity's node the first time,
    /// then what changed of the others.
    fn write_components(&mut self, out: &mut ClientOutbox, view: &TickView) -> io::Result<()> {

        let created = self.known.iter().filter(|known| known.created).map(|known| known.entity);
        let entities: Vec<_> = std::iter::once(self.entity).chain(created).collect();

        for entity in entities {

            let Some((_, cell)) = view.get(entity) else { continue };
            let Some(entity_node) = cell.nrl_node else { continue };
            let own = entity == self.entity || cell.is_controlled_by(self.entity);

            if let Some(slots) = self.nrl.get_mut(&entity) {
                for removed in &cell.removed_components {
                    if let Some(index) = slots.iter().position(|&slot| slot == removed.slot) {
                        slots.swap_remove(index);
                        removed.write(out)?;
                    }
                }
            }

            for (slot, component) in cell.components.iter() {

                if !component.is_for(own) {
                    continue;
                }

                let known = self.nrl.get(&entity);
                let new = !known.is_some_and(|slots| slots.contains(&slot));

                if new {

                    let mut messages = Vec::with_capacity(2);
                    if known.is_none() {
                        messages.push(entity::create_entity_node(entity.id, entity_node));
                    }
                    messages.push(component.create(entity_node, view.dispatch)?);

                    out.write_simple(client::element::NrlCreateNode(client::element::NrlStream { messages, rest: Vec::new() }))?;
                    self.nrl.entry(entity).or_default().push(slot);

                }

                component.write_events(out, view.dispatch, new)?;

            }

        }

        Ok(())

    }

}

/// The id alias of an entity entering an area of interest without one.
const NO_ALIAS: u8 = 255;

/// What entity creations carry before their position, always this in captures, see
/// [`client::element::CreateEntity::resource_name`].
const RESOURCE_NAME: &str = "bw.default";

/// The byte `CreateEntity` carries after the direction, of unknown meaning, see
/// [`client::element::CreateEntity::direction_unk`]: as captured for a vehicle, the
/// client crashing on a vehicle created with 0.
const CREATE_ENTITY_UNK: u8 = 5;


// ============ //
//    EVENTS    //
// ============ //

/// An event that happened in the base app.
#[derive(Debug)]
pub enum Event {
    IoError(IoErrorEvent),
    Login(LoginEvent),
    EnableEntities(EnableEntitiesEvent),
    /// No event came before the deadline, see [`App::poll_until`].
    Timeout,
    BaseMethod(BaseMethodEvent),
    CellMethod(CellMethodEvent),
    /// A client reports where its player entity is.
    AvatarUpdate(AvatarUpdateEvent),
    Entered(EnteredEvent),
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

/// A logged in client is ready for entities: the server may now create its player. The
/// client sends this once its logon is complete, and anything sent to it before is not
/// handled as it would be: a player created earlier is never read.
#[derive(Debug)]
pub struct EnableEntitiesEvent {
    pub client: ClientKey,
}

/// A client called a method on its current base-player entity.
#[derive(Debug)]
pub struct BaseMethodEvent {
    pub client: ClientKey,
    pub entity: EntityKey<AnyEntity>,
    /// The decoded call, resolved dynamically against the entity's own base-method
    /// table, see [`App::decode`] to decode it further.
    pub call: MethodCall,
}

/// A client called a cell method, on its own cell entity or on another one it targets.
#[derive(Debug)]
pub struct CellMethodEvent {
    pub client: ClientKey,
    /// The target entity, with the wire's `0` already resolved to the client's own.
    pub entity: EntityKey<AnyEntity>,
    /// The decoded call, resolved dynamically against the entity's own cell-method
    /// table, see [`App::decode`] to decode it further.
    pub call: MethodCall,
}

/// A client reports where its player entity is, see
/// [`element::AvatarUpdateImplicit`] and [`element::AvatarUpdateExplicit`].
#[derive(Debug)]
pub struct AvatarUpdateEvent {
    pub client: ClientKey,
    /// The vehicle it says it is on, when explicit.
    pub vehicle_id: Option<u32>,
    pub position: Vec3,
    /// Roll, pitch and yaw.
    pub direction: Vec3,
}

/// An entity in its space was created on a client, which from now on gets what it gives:
/// its properties set after its creation, say.
#[derive(Debug)]
pub struct EnteredEvent {
    pub client: ClientKey,
    pub entity: EntityKey<AnyEntity>,
}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn interest_keeps_entities_until_past_the_leave_radius_and_shows_others() {

        let mut slots = Slots::default();
        let space = SpaceKey { slot: slots.insert(()), id: 1 };
        let key = EntityKey::new(slots.insert(()), 2);
        let player = EntityKey::new(slots.insert(()), 3);
        let at = |x| EntityCell::new(space, Vec3::new(x, 5.0, 0.0), Vec3::ZERO, true, None, Visibility::Interest);

        let mut witness = Witness::new(player);
        witness.interest = Interest::Radius { enter: 10.0, leave: 15.0 };
        assert!(witness.sees(Vec3::ZERO, key, &at(9.0), false));
        assert!(!witness.sees(Vec3::ZERO, key, &at(12.0), false));
        assert!(witness.sees(Vec3::ZERO, key, &at(12.0), true));
        assert!(!witness.sees(Vec3::ZERO, key, &at(16.0), true));

        let mut controlled = at(1000.0);
        controlled.controlled_by = Some(player);
        assert!(witness.sees(Vec3::ZERO, key, &controlled, false));

        witness.interest = Interest::Manual;
        assert!(!witness.sees(Vec3::ZERO, key, &at(0.0), false));
        witness.shown.insert(key);
        assert!(witness.sees(Vec3::ZERO, key, &at(0.0), false));

        let mut always = at(1000.0);
        always.visibility = Visibility::Always;
        assert!(witness.sees(Vec3::ZERO, player, &always, false));

    }

}
