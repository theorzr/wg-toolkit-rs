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

use std::any::TypeId;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::io::{self, Read, Write};
use std::num::Wrapping;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::fmt;

use blowfish::Blowfish;
use glam::Vec3;
use rand::rngs::OsRng;
use rand::RngCore;
use tracing::{debug, warn};

use crate::net::bundle::{Bundle, NextElementReader, ElementReader};
use crate::net::element::{Element, ElementLength, SimpleElement};
use crate::net::socket::PacketSocket;
use crate::script::{PropertyFlags, Script, Ty, Value};
use crate::net::proto::Protocol;

use super::client::element::{ChangedProperties, NrlStream, NrlUpdateNode};
use super::dispatch::{EntityDispatch, ScriptDispatch, MethodCall, MethodDef};
use super::math::PackedYawPitchRoll;
use super::outbox::{ClientOutbox, Outbox};
use super::slots::{SlotKey, Slots};
use super::entity::{self, AnyEntity, BindError, Bound, Calls, Entity, EntityEvent, EntityId, EntityKey, EntitySchema, EntitySink, RawEntity, ScriptType};
use super::io_invalid_data;
use super::client;

use element::{LoginKey, SessionKey};


/// The base application.
///
/// Fully dynamic: driven by a loaded [`Script`] rather than a per-game generated
/// `AnyEntity` sum type -- entities, methods and properties are all resolved by name (or
/// exposed id) against that script model at runtime (see [`ScriptDispatch`]), so this
/// type itself never needs to know about any concrete game's entity types. Schemas
/// registered with [`Self::register`] give typed access on top, see [`entity`].
///
/// Clients, entities and spaces are given as keys, [`ClientKey`], [`EntityKey`] and
/// [`SpaceKey`], which go stale once what they name is gone.
#[derive(Debug)]
pub struct App {
    /// The scripting model for this app, and its per entity-type dispatch tables
    /// computed from it at construction time.
    dispatch: ScriptDispatch,
    /// The schema registered for each entity type, by type id, see [`Self::register`].
    bindings: Vec<Option<Bound>>,
    /// The entity type of each registered schema.
    schema_types: HashMap<TypeId, u16>,
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
    clients: Slots<Client>,
    client_keys: HashMap<SocketAddr, ClientKey>,
    /// Every entity this app has created, and the key of each by its id.
    entities: Slots<EntityEntry>,
    entity_keys: HashMap<u32, EntityKey<AnyEntity>>,
    /// The next id for entities, this is wrapping around and we ensure that the same id
    /// isn't used twice!
    entities_next_id: Wrapping<u32>,
    /// The spaces, and the key of each by its id.
    spaces: Slots<Space>,
    space_keys: HashMap<u32, SpaceKey>,
}

impl App {

    pub fn new(addr: SocketAddr, script: Script) -> io::Result<Self> {
        let dispatch = ScriptDispatch::new(script);
        Ok(Self {
            bindings: (0..=dispatch.entities.len()).map(|_| None).collect(),
            schema_types: HashMap::new(),
            dispatch,
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
    /// declares, see [`entity`]. Its entities can then be created, viewed through it
    /// with [`Self::entity`], and their calls decoded with [`Self::decode`].
    pub fn register<T: EntitySchema>(&mut self) -> Result<(), BindError> {
        let Some((type_id, dispatch)) = self.dispatch.entity_from_name(T::NAME) else {
            return Err(BindError { schema: T::NAME, errors: vec![format!("no entity type {}", T::NAME)] });
        };
        self.bindings[usize::from(type_id)] = Some(entity::bind::<T>(dispatch)?);
        self.schema_types.insert(TypeId::of::<T>(), type_id);
        Ok(())
    }

    /// View an entity through its schema. What its client methods and properties give
    /// goes to its client right away, or once in a space, to the clients knowing it at
    /// the end of the tick, see [`Self::end_tick`].
    pub fn entity<T: EntitySchema>(&mut self, key: EntityKey<T>) -> io::Result<Entity<'_, T>> {
        let entry = self.entities.get_mut(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {key}")))?;
        let bound = self.bindings.get(usize::from(entry.type_id)).and_then(Option::as_ref)
            .filter(|bound| bound.is::<T>())
            .ok_or_else(|| io_invalid_data(format_args!("entity {key} is not a registered {}", T::NAME)))?;
        let dispatch = self.dispatch.entity_from_id(entry.type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {}", entry.type_id)))?;
        let sink = match (&entry.cell, entry.client) {
            (Some(cell), _) => {
                let space = self.spaces.get_mut(cell.space.slot)
                    .ok_or_else(|| io_invalid_data(format_args!("entity {key} is in no space")))?;
                EntitySink::Space(&mut space.events)
            }
            (None, Some(client)) => {
                let is_player = self.clients.get(client.slot).and_then(|c| c.player) == Some(key.any());
                let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;
                EntitySink::Client { out, is_player }
            }
            (None, None) => return Err(io_invalid_data(format_args!("entity {key} has no client and is in no space"))),
        };
        Ok(Entity::new(RawEntity {
            slot: key.slot,
            id: key.id,
            properties: &mut entry.properties,
            dispatch,
            bound,
            sink,
        }))
    }

    /// The key of an entity of any type as one of the given schema, if it is one.
    pub fn cast<T: EntitySchema>(&self, key: EntityKey<AnyEntity>) -> Option<EntityKey<T>> {
        let entry = self.entities.get(key.slot)?;
        self.bindings.get(usize::from(entry.type_id))?.as_ref()?
            .is::<T>()
            .then(|| key.cast())
    }

    /// Decode a call a client made to one of an entity's base or cell methods, if the
    /// entity is of the given calls' schema and the method one they declare.
    pub fn decode<C: Calls>(&self, entity: EntityKey<AnyEntity>, call: &MethodCall) -> Option<(EntityKey<C::Schema>, C)> {
        let entry = self.entities.get(entity.slot)?;
        let call = self.bindings.get(usize::from(entry.type_id))?.as_ref()?.decode(call)?;
        Some((entity.cast(), call))
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
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(entry) = self.entities.get(entity.slot) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let Some(dispatch) = self.dispatch.entity_from_id(entry.type_id) else {
            return self.skip::<RawEntityMethod>(reader);
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
            return self.skip::<RawEntityMethod>(reader);
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

        let slot = self.clients.insert(Client {
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

    /// Forget a client and its entities.
    fn remove_client(&mut self, key: ClientKey) {
        self.clients.remove(key.slot);
        self.outbox.remove(key.addr);
        self.remove_entities(|_, entry| entry.client == Some(key));
    }

    /// Forget the entities the given predicate selects.
    fn remove_entities(&mut self, mut predicate: impl FnMut(EntityKey<AnyEntity>, &EntityEntry) -> bool) {
        let entities = &self.entities;
        let removed: Vec<_> = self.entity_keys.values()
            .copied()
            .filter(|&key| entities.get(key.slot).is_some_and(|entry| predicate(key, entry)))
            .collect();
        for key in removed {
            self.remove_entity(key);
        }
    }

    /// A logged in client, if not gone.
    fn client(&self, key: ClientKey) -> io::Result<&Client> {
        self.clients.get(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such client: {}", key.addr)))
    }

    /// A logged in client, if not gone.
    fn client_mut(&mut self, key: ClientKey) -> io::Result<&mut Client> {
        self.clients.get_mut(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such client: {}", key.addr)))
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

    /// The outbox of a logged in client.
    fn out(&mut self, key: ClientKey) -> io::Result<&mut ClientOutbox> {
        self.client(key)?;
        self.outbox.client(key.addr, &mut self.protocol, &self.socket)
    }

    /// Write a simple element to a client's outbox.
    fn send_simple<E: SimpleElement>(&mut self, client: ClientKey, element: E) -> io::Result<()> {
        self.out(client)?.write_simple(element)
    }

    /// The entity type of a registered schema.
    fn schema_type<T: EntitySchema>(&self) -> io::Result<u16> {
        self.schema_types.get(&TypeId::of::<T>()).copied()
            .ok_or_else(|| io_invalid_data(format_args!("schema {} is not registered", T::NAME)))
    }

    /// Track a new entity, of a client or in a space, replacing any of the same id.
    fn insert_entity(&mut self, id: u32, client: Option<ClientKey>, type_id: u16, cell: Option<CellPart>) -> EntityKey<AnyEntity> {
        if let Some(&previous) = self.entity_keys.get(&id) {
            self.remove_entity(previous);
        }
        let space = cell.as_ref().map(|cell| cell.space);
        let properties = self.dispatch.entity_from_id(type_id).map_or(0, |dispatch| dispatch.properties.len());
        let slot = self.entities.insert(EntityEntry { client, type_id, properties: vec![None; properties], cell });
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

    /// Keep the given property values of an entity, by exposed id.
    fn store_properties<T>(&mut self, key: EntityKey<T>, values: impl IntoIterator<Item = (usize, Value)>) {
        if let Some(entry) = self.entities.get_mut(key.slot) {
            for (exposed_id, value) in values {
                if let Some(slot) = entry.properties.get_mut(exposed_id) {
                    *slot = Some(value);
                }
            }
        }
    }

    /// Tell a client the server's update frequency and game time, which the real server
    /// does right after the login reply. `unknown` is a field of unknown meaning, see
    /// [`client::element::UpdateFrequencyNotification`].
    pub fn update_frequency(&mut self, client: ClientKey, frequency: u8, unknown: u16, game_time: u32) -> io::Result<()> {
        self.send_simple(client, client::element::UpdateFrequencyNotification { frequency, unknown, game_time })
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
        let entity_type_id = self.schema_type::<T>()?;
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

    /// An entity, if not gone.
    fn entry<T>(&self, key: EntityKey<T>) -> io::Result<&EntityEntry> {
        self.entities.get(key.slot)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {key}")))
    }

    /// Call a method, resolved by name against the target entity's client-method table,
    /// on the client owning the given entity, selecting it first if needed: for calls a
    /// schema doesn't declare, those replayed from a capture say.
    pub fn call_method<T>(&mut self, entity: EntityKey<T>, method_name: &str, args: Vec<Value>) -> io::Result<()> {

        let entry = self.entry(entity)?;
        let client = entry.client.ok_or_else(|| io_invalid_data(format_args!("entity {entity} has no client")))?;

        let dispatch = self.dispatch.entity_from_id(entry.type_id)
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

    /// Reset all entities known to the given client, optionally keeping its current
    /// base-player entity alive (`keep_player_on_base`). This is the only entity
    /// *removal* mechanism the wire protocol has -- there's no way to remove a single
    /// arbitrary entity, only "reset everything" (see `re-work/doc/ENTITY.md`).
    pub fn reset_entities(&mut self, client: ClientKey, keep_player_on_base: bool) -> io::Result<()> {

        self.send_simple(client, client::element::ResetEntities { keep_player_on_base })?;

        let keep = keep_player_on_base
            .then(|| self.clients.get(client.slot).and_then(|c| c.player))
            .flatten();

        self.remove_entities(|entity, entry| entry.client == Some(client) && Some(entity) != keep);

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

        self.client(client)?;
        let addr = client.addr;

        const CHUNK_SIZE: usize = 8192;

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

    /// Read (and discard) a single already-typed element, without aborting the rest of
    /// the bundle -- used for ids whose framing is known but that this app doesn't act
    /// on yet.
    fn skip<T: Element<()>>(&mut self, reader: ElementReader<'_, '_>) -> io::Result<()> {
        reader.read_simple::<T>()?;
        Ok(())
    }

    /// Create a space of the given id, which the protocol carries: one from a capture,
    /// whose space properties name it, say.
    pub fn create_space(&mut self, id: u32) -> io::Result<SpaceKey> {
        if self.space_keys.contains_key(&id) {
            return Err(io_invalid_data(format_args!("space {id} already exists")));
        }
        let slot = self.spaces.insert(Space { entities: Vec::new(), data: Vec::new(), events: Vec::new() });
        let key = SpaceKey { slot, id };
        self.space_keys.insert(id, key);
        Ok(key)
    }

    /// Destroy a space: the entities in it without a client are gone, the others only
    /// leave it.
    pub fn destroy_space(&mut self, key: SpaceKey) {
        let Some(space) = self.spaces.get(key.slot) else { return };
        for entity in space.entities.clone() {
            match self.entities.get(entity.slot).and_then(|entry| entry.client) {
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

    /// Get a client's base-player entity into a space, with a cell part: the client gets
    /// its cell player and the space's data right away, then what is in the space from
    /// the end of the tick on. Its cell method calls go to it.
    pub fn create_cell_player<T>(&mut self, player: EntityKey<T>, space: SpaceKey, init: CellPlayerInit) -> io::Result<()> {

        let entry = self.entry(player)?;
        let client = entry.client.ok_or_else(|| io_invalid_data(format_args!("entity {player} has no client")))?;
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
        if let Some(entry) = self.entities.get_mut(player.slot) {
            entry.cell = Some(CellPart::new(space, init.position, init.direction, true, None));
        }
        if let Some(space) = self.spaces.get_mut(space.slot) {
            space.entities.push(player.any());
        }
        if let Some(client) = self.clients.get_mut(client.slot) {
            client.witness = Some(Witness::new(player.any()));
        }

        Ok(())

    }

    /// Take an entity out of its space, if in one: it leaves the area of interest of the
    /// clients knowing it, and its own client no longer knows that space.
    pub fn leave_space<T>(&mut self, entity: EntityKey<T>) {
        let Some(entry) = self.entities.get_mut(entity.slot) else { return };
        let Some(cell) = entry.cell.take() else { return };
        if let Some(space) = self.spaces.get_mut(cell.space.slot) {
            space.entities.retain(|&other| other != entity.any());
        }
        if let Some(client) = entry.client.and_then(|client| self.clients.get_mut(client.slot)) {
            if client.witness.as_ref().is_some_and(|witness| witness.entity == entity.any()) {
                client.witness = None;
            }
        }
    }

    /// The space of an entity, if in one.
    pub fn space_of<T>(&self, entity: EntityKey<T>) -> Option<SpaceKey> {
        Some(self.entities.get(entity.slot)?.cell.as_ref()?.space)
    }

    /// Create an entity of the given schema in a space, see [`Self::create_entity_dynamic`].
    pub fn create_entity<T: EntitySchema>(&mut self, space: SpaceKey, init: EntityInit) -> io::Result<EntityKey<T>> {
        let type_id = self.schema_type::<T>()?;
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

        let cell = CellPart::new(space, init.position, init.direction, init.alias, init.controlled_by);
        let key = self.insert_entity(id, None, type_id, Some(cell));
        self.store_properties(key, init.properties.values.into_iter().map(|(exposed_id, _, value)| (exposed_id.into(), value)));
        Ok(key)

    }

    /// Destroy an entity: it leaves the area of interest of the clients knowing it.
    pub fn destroy_entity<T>(&mut self, key: EntityKey<T>) {
        self.remove_entity(key.any());
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
            .and_then(|entry| entry.cell.as_mut())
            .ok_or_else(|| io_invalid_data(format_args!("entity {entity} is in no space")))?;
        cell.position = position;
        cell.direction = direction;
        cell.moved = true;
        Ok(())
    }

    /// Create NRL nodes on a client: an entity's node and its dynamic components, say.
    /// The stream's first message must create a node. In a space, sent with the tick,
    /// after what it says of the entities.
    pub fn nrl_create_node(&mut self, client: ClientKey, stream: NrlStream) -> io::Result<()> {
        self.write_queued(client, Queued::NrlCreateNode(stream))
    }

    /// Update an NRL node on a client, as the real server sends updates: the record goes
    /// to the client's NRL buffer, then a flag applies it. In a space, sent with the
    /// tick, after what it says of the entities.
    pub fn nrl_update_node(&mut self, client: ClientKey, update: &NrlUpdateNode) -> io::Result<()> {
        let mut fragment = update.node_id.to_be_bytes().to_vec();
        fragment.extend_from_slice(&update.payload);
        self.write_queued(client, Queued::NrlUpdate(fragment))
    }

    /// Write to a client, with its next tick if in a space.
    fn write_queued(&mut self, client: ClientKey, queued: Queued) -> io::Result<()> {
        match self.client_mut(client)?.witness.as_mut() {
            Some(witness) => {
                witness.queued.push(queued);
                Ok(())
            }
            None => queued.write(self.out(client)?),
        }
    }

    /// How many ticks a client got in its space: its game time, in ticks.
    pub fn ticks(&self, client: ClientKey) -> Option<u32> {
        Some(self.client(client).ok()?.witness.as_ref()?.ticks)
    }

    /// A client asks for an entity that entered its area of interest: it gets created
    /// with the next tick.
    pub(super) fn request_entity(&mut self, client: ClientKey, entity_id: u32) {
        let known = self.clients.get_mut(client.slot)
            .and_then(|client| client.witness.as_mut())
            .and_then(|witness| witness.known.iter_mut().find(|known| known.entity.id == entity_id));
        match known {
            Some(known) => known.requested = true,
            None => debug!(addr = %client.addr, "Asked for entity {entity_id}, which it doesn't know"),
        }
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

        for (_, entry) in self.entities.iter_mut() {
            if let Some(cell) = &mut entry.cell {
                cell.moved = false;
            }
        }
        for (_, space) in self.spaces.iter_mut() {
            space.events.clear();
        }

    }

    fn tick_client(&mut self, client: ClientKey) -> io::Result<()> {
        let Some(mut witness) = self.clients.get_mut(client.slot).and_then(|c| c.witness.take()) else {
            return Ok(());
        };
        let result = self.write_tick(client, &mut witness);
        if let Some(state) = self.clients.get_mut(client.slot) {
            state.witness = Some(witness);
        }
        result
    }

    /// Tell a client what changed in its area of interest, see [`Self::end_tick`]. Every
    /// entity of its space is of interest to it.
    fn write_tick(&mut self, client: ClientKey, witness: &mut Witness) -> io::Result<()> {

        let entities = &self.entities;
        let Some(own) = entities.get(witness.entity.slot).and_then(|entry| entry.cell.as_ref()) else { return Ok(()) };
        let Some(space) = self.spaces.get(own.space.slot) else { return Ok(()) };
        let own_id = witness.entity.id;
        let cell = |key: EntityKey<AnyEntity>| entities.get(key.slot).and_then(|entry| Some((entry, entry.cell.as_ref()?)));

        // What was written to it before goes first, the tick opens a bundle.
        self.outbox.flush(client.addr, &mut self.protocol, &self.socket)?;
        let out = self.outbox.client(client.addr, &mut self.protocol, &self.socket)?;
        out.write_simple(client::element::TickSync { tick: witness.ticks as u8 })?;
        witness.ticks = witness.ticks.wrapping_add(1);

        // Those gone, or no longer in its space.
        let mut known = Vec::with_capacity(witness.known.len());
        for entry in std::mem::take(&mut witness.known) {
            if space.entities.contains(&entry.entity) {
                known.push(entry);
            } else {
                out.write_simple(client::element::LeaveAoi { entity_id: entry.entity.id, last_event_numbers: Vec::new() })?;
                if let Some(alias) = entry.alias {
                    witness.aliases[usize::from(alias)] = false;
                }
            }
        }
        witness.known = known;

        // Those new to it, in the order they got in the space.
        for &key in &space.entities {
            if key.id == own_id || witness.known.iter().any(|known| known.entity == key) {
                continue;
            }
            let Some((_, other)) = cell(key) else { continue };
            let alias = match other.alias {
                true => Some(witness.take_alias().ok_or_else(|| io_invalid_data(format_args!("no alias left for {key}")))?),
                false => None,
            };
            out.write_simple(client::element::EnterAoi { entity_id: key.id, id_alias: alias.unwrap_or(NO_ALIAS) })?;
            // Without an alias, it is created right away rather than once asked for.
            witness.known.push(Known { entity: key, alias, requested: alias.is_none(), created: false });
        }

        // Those asked for: created with what of them is visible to it.
        for known in &mut witness.known {
            if !known.requested || known.created {
                continue;
            }
            let Some((entry, other)) = cell(known.entity) else { continue };
            let dispatch = self.dispatch.entity_from_id(entry.type_id)
                .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {}", entry.type_id)))?;
            let own = other.controlled_by.is_some_and(|controller| controller.id == own_id);
            let client_data = visible_properties(entry, dispatch, own).encode(&dispatch.properties)?;
            let (roll, pitch, yaw) = (other.direction.x, other.direction.y, other.direction.z);
            match known.alias {
                Some(_) => out.write_simple(client::element::CreateEntity {
                    entity_id: known.entity.id,
                    entity_type_id: entry.type_id,
                    unk_u16: 0,
                    resource_name: RESOURCE_NAME.to_string(),
                    position: other.position,
                    direction: PackedYawPitchRoll::pack(yaw, pitch, roll, false),
                    direction_unk: CREATE_ENTITY_UNK,
                    client_data,
                })?,
                None => out.write_simple(client::element::CreateEntityDetailed {
                    entity_id: known.entity.id,
                    entity_type_id: entry.type_id,
                    unk_u16: 0,
                    resource_name: RESOURCE_NAME.to_string(),
                    position: other.position,
                    direction: other.direction,
                    client_data,
                })?,
            }
            known.created = true;
            self.events.push_back(Event::Entered(EnteredEvent { client, entity: known.entity }));
        }

        // Where it is, as the captured server places the player: a reference point at
        // its position rounded to the metre, then itself on that point.
        if own.moved {
            out.write_simple(client::element::RelativePosition { position: own.position.round() })?;
            out.write_simple(client::element::AvatarUpdatePlayerDetailed { ref_num: 0, position: Vec3::ZERO, direction: Vec3::ZERO })?;
        }

        // Where those it knows moved, each position update selecting its entity.
        for known in &witness.known {
            let (Some(id_alias), true) = (known.alias, known.created) else { continue };
            let Some((_, other)) = cell(known.entity).filter(|(_, other)| other.moved) else { continue };
            out.write_simple(client::element::AvatarUpdateAliasDetailed {
                ref_num: 0,
                id_alias,
                position: other.position,
                direction: other.direction,
            })?;
        }

        // What their properties and client methods gave, in the order given.
        for event in &space.events {
            let entity = if event.entity_id == own_id {
                out.select_player()?;
                witness.entity
            } else {
                let Some(known) = witness.known.iter().find(|known| known.entity.id == event.entity_id && known.created) else { continue };
                let own = cell(known.entity)
                    .and_then(|(_, other)| other.controlled_by)
                    .is_some_and(|controller| controller.id == own_id);
                if event.own_client && !own {
                    continue;
                }
                out.select_entity(event.entity_id)?;
                known.entity
            };
            let Some((entry, _)) = cell(entity) else { continue };
            let dispatch = self.dispatch.entity_from_id(entry.type_id)
                .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {}", entry.type_id)))?;
            event.kind.write(out, dispatch)?;
        }

        for queued in witness.queued.drain(..) {
            queued.write(out)?;
        }

        Ok(())

    }

}

/// Whether the given data is of the given type, see [`App::create_base_player`].
fn check_data<D: ScriptType>(_data: &D, ty: &Ty) -> bool {
    D::check(ty)
}

/// Resolves a client-to-cell call's method table from the entity it names, for
/// [`element::CellEntityMethod`].
struct CellTables<'a> {
    entities: &'a Slots<EntityEntry>,
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
        let entry = self.entities.get(key.slot)?;
        Some(&self.dispatch.entity_from_id(entry.type_id)?.cell_methods)
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

/// How long a reliable packet waits for its acknowledgment before being sent again, and
/// for how long it is sent again before its client is assumed gone. A poll timeout (see
/// [`App::poll_until`]) lets resends go out while no packet comes in.
const RESEND_TIMEOUT: Duration = Duration::from_millis(400);
const RESEND_GIVE_UP: Duration = Duration::from_secs(20);

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
    player: Option<EntityKey<AnyEntity>>,
    /// What it knows of the space its cell player is in (e.g. its `Avatar` while in a
    /// battle), which its cell calls go to, see [`App::create_cell_player`].
    witness: Option<Witness>,
}

/// One entity this app has created: the client that owns it, if a base entity, its wire
/// type id, its client-visible property values, by exposed id, when known, and its cell
/// part, once in a space.
#[derive(Debug)]
struct EntityEntry {
    client: Option<ClientKey>,
    type_id: u16,
    properties: Vec<Option<Value>>,
    cell: Option<CellPart>,
}

/// What of an entity's properties a client gets as it enters its area of interest: those
/// with a known value, but those for its own client only when it is.
fn visible_properties(entry: &EntityEntry, dispatch: &EntityDispatch, own: bool) -> ChangedProperties {
    let values = entry.properties.iter().enumerate()
        .filter_map(|(exposed_id, value)| {
            let def = &dispatch.properties[exposed_id];
            let own_client = matches!(def.flags, PropertyFlags::OwnClient | PropertyFlags::BaseAndClient);
            let value = value.as_ref().filter(|_| own || !own_client)?;
            Some((exposed_id as u8, Arc::clone(&def.name), value.clone()))
        })
        .collect();
    ChangedProperties { values, stopped: None }
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
#[derive(Debug)]
struct Space {
    /// Its entities, in the order they got in it, which they enter areas of interest in.
    entities: Vec<EntityKey<AnyEntity>>,
    /// Its properties and geometry mappings, which clients get once in it.
    data: Vec<SpaceData>,
    /// What its entities gave their clients during the tick.
    events: Vec<EntityEvent>,
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
    /// client gets: the avatar driving a vehicle, say.
    pub controlled_by: Option<EntityKey<AnyEntity>>,
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

/// The cell part of an entity, in a space.
#[derive(Debug)]
struct CellPart {
    space: SpaceKey,
    position: Vec3,
    /// Roll, pitch and yaw.
    direction: Vec3,
    /// Whether it moved during the tick.
    moved: bool,
    /// See [`EntityInit::alias`] and [`EntityInit::controlled_by`].
    alias: bool,
    controlled_by: Option<EntityKey<AnyEntity>>,
}

impl CellPart {

    fn new(space: SpaceKey, position: Vec3, direction: Vec3, alias: bool, controlled_by: Option<EntityKey<AnyEntity>>) -> Self {
        Self { space, position, direction, moved: false, alias, controlled_by }
    }

}

/// What a client knows of the space its player entity is in.
///
/// Each tick sends it, in a bundle the tick opens: the entities entering and leaving its
/// area of interest, those it asked for, where it and those it knows are, what their
/// properties and client methods gave, then what was queued for it, see
/// [`App::nrl_create_node`].
#[derive(Debug)]
struct Witness {
    /// Its player entity.
    entity: EntityKey<AnyEntity>,
    /// The entities in its area of interest, in the order they entered it.
    known: Vec<Known>,
    /// Which id aliases are taken.
    aliases: Vec<bool>,
    /// The ticks sent to it, the first being tick 0.
    ticks: u32,
    /// What goes with its next tick.
    queued: Vec<Queued>,
}

impl Witness {

    fn new(entity: EntityKey<AnyEntity>) -> Self {
        Self { entity, known: Vec::new(), aliases: vec![false; usize::from(NO_ALIAS)], ticks: 0, queued: Vec::new() }
    }

    /// Take the first free id alias.
    fn take_alias(&mut self) -> Option<u8> {
        let alias = self.aliases.iter().position(|&taken| !taken)?;
        self.aliases[alias] = true;
        Some(alias as u8)
    }

}

/// An entity in the area of interest of a witness.
#[derive(Debug)]
struct Known {
    entity: EntityKey<AnyEntity>,
    alias: Option<u8>,
    /// Whether its client asked for it, and whether it was created on it since.
    requested: bool,
    created: bool,
}

/// What goes to a client with its next tick, see [`App::nrl_create_node`].
#[derive(Debug)]
enum Queued {
    NrlCreateNode(NrlStream),
    /// An NRL record, applied by the flag following it.
    NrlUpdate(Vec<u8>),
}

impl Queued {

    fn write(self, out: &mut ClientOutbox) -> io::Result<()> {
        match self {
            Self::NrlCreateNode(stream) => out.write_simple(client::element::NrlCreateNode(stream)),
            Self::NrlUpdate(fragment) => {
                out.write_simple(client::element::NrlData { fragment })?;
                out.write_simple(client::element::NrlUpdateNodeFlag)
            }
        }
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
