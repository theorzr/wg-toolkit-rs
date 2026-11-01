//! Base application where clients send all their requests.

pub mod element;

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::io::{self, Read, Write};
use std::num::Wrapping;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use blowfish::Blowfish;
use rand::rngs::OsRng;
use rand::RngCore;
use tracing::{debug, warn};

use crate::net::bundle::{Bundle, NextElementReader, ElementReader};
use crate::net::element::{Element, ElementLength, SimpleElement};
use crate::net::socket::PacketSocket;
use crate::script::{Script, Value};
use crate::net::proto::Protocol;

use super::dispatch::{ScriptDispatch, PropertyDef, MethodCall, MethodDef};
use super::outbox::Outbox;
use super::schema::{self, BindError, Bound, Calls, Entity, EntitySchema, RawEntity};
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
    /// The schema registered for each entity type, by type id, see [`Self::register`].
    bindings: Vec<Option<Bound>>,
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
        let dispatch = ScriptDispatch::new(script);
        Ok(Self {
            bindings: (0..=dispatch.entities.len()).map(|_| None).collect(),
            dispatch,
            socket: PacketSocket::bind(addr)?,
            protocol: Protocol::new(),
            events: VecDeque::new(),
            outbox: Outbox::default(),
            bundle: Bundle::new(),
            pending_clients: HashMap::new(),
            clients: HashMap::new(),
            entities: HashMap::new(),
            entities_next_id: Wrapping(OsRng.next_u32()),
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
    /// declares, see [`schema`]. Its entities can then be viewed through it with
    /// [`Self::entity`], and their calls decoded with [`Self::decode`].
    pub fn register<T: EntitySchema>(&mut self) -> Result<(), BindError> {
        let Some((type_id, dispatch)) = self.dispatch.entity_from_name(T::NAME) else {
            return Err(BindError { schema: T::NAME, errors: vec![format!("no entity type {}", T::NAME)] });
        };
        self.bindings[usize::from(type_id)] = Some(schema::bind::<T>(dispatch)?);
        Ok(())
    }

    /// View an entity of a client through its type's registered schema, which must be
    /// the given one.
    pub fn entity<T: EntitySchema>(&mut self, entity_id: u32) -> io::Result<Entity<'_, T>> {
        let entry = self.entities.get(&entity_id)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {entity_id}")))?;
        let (addr, type_id) = (entry.addr, entry.type_id);
        let bound = self.bindings.get(usize::from(type_id)).and_then(Option::as_ref)
            .filter(|bound| bound.is::<T>())
            .ok_or_else(|| io_invalid_data(format_args!("entity {entity_id} is not a registered {}", T::NAME)))?;
        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
        let is_player = self.clients.get(&addr).and_then(|c| c.base_entity_id) == Some(entity_id);
        let out = self.outbox.client(addr, &mut self.protocol, &self.socket)?;
        Ok(Entity::new(RawEntity { out, dispatch, bound, entity_id, is_player }))
    }

    /// Decode a call a client made to one of an entity's base or cell methods, if the
    /// entity is of the given calls' schema and the method one they declare.
    pub fn decode<C: Calls>(&self, entity_id: u32, call: &MethodCall) -> Option<C> {
        let type_id = self.entities.get(&entity_id)?.type_id;
        self.bindings.get(usize::from(type_id))?.as_ref()?.decode(call)
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
        match reader.id() {
            LoginKey::ID => self.handle_client_auth(addr, reader),
            SessionKey::ID => self.handle_client_session_key(addr, reader),
            element::id::ENABLE_ENTITIES => {
                reader.read_simple::<element::EnableEntities>()?;
                self.events.push_back(Event::EnableEntities(EnableEntitiesEvent { addr }));
                Ok(())
            }
            element::id::DISCONNECT_CLIENT => self.skip::<element::DisconnectClient>(reader),
            element::id::PING_DATACENTER => self.skip::<element::PingDatacenter>(reader),
            element::id::AVATAR_UPDATE_IMPLICIT => {
                let update = reader.read_simple::<element::AvatarUpdateImplicit>()?.element;
                let (yaw, pitch, roll) = update.direction.unpack(true);
                self.events.push_back(Event::AvatarUpdate(AvatarUpdateEvent {
                    addr, vehicle_id: None, position: update.position, direction: glam::Vec3::new(roll, pitch, yaw),
                }));
                Ok(())
            }
            element::id::AVATAR_UPDATE_EXPLICIT => {
                let update = reader.read_simple::<element::AvatarUpdateExplicit>()?.element;
                let (yaw, pitch, roll) = update.direction.unpack(true);
                self.events.push_back(Event::AvatarUpdate(AvatarUpdateEvent {
                    addr, vehicle_id: Some(update.vehicle_id), position: update.position, direction: glam::Vec3::new(roll, pitch, yaw),
                }));
                Ok(())
            }
            element::id::ACK_PHYSICS_CORRECTION => self.skip::<element::AckPhysicsCorrection>(reader),
            element::id::REQUEST_ENTITY_UPDATE => {
                let request = reader.read_simple::<element::RequestEntityUpdate>()?.element;
                self.events.push_back(Event::RequestEntityUpdate(RequestEntityUpdateEvent { addr, entity_id: request.entity_id }));
                Ok(())
            }
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

    /// Same as [`Self::handle_base_entity_method`], but for a cell-directed call. The
    /// client has no direct connection to a cell app, so these calls arrive here to be
    /// forwarded in real BigWorld; this project has no `cell::App` to forward to yet, but
    /// decoding still works today, dispatched via the same stored entity data (an
    /// entity's base and cell slices share one id).
    ///
    /// Unlike a base call, a cell call names its target, which is not necessarily the
    /// client's own entity (see [`element::CellMethodTables`]), so the table is resolved
    /// from that target. A client with no cell entity yet has no business sending one.
    fn handle_cell_entity_method(&mut self, addr: SocketAddr, reader: ElementReader<'_, '_>) -> io::Result<()> {

        let Some(cell_entity_id) = self.clients.get(&addr).and_then(|c| c.cell_entity_id) else {
            return self.skip::<RawEntityMethod>(reader);
        };

        let tables = CellTables { entities: &self.entities, dispatch: &self.dispatch, cell_entity_id };
        let method = reader.read::<element::CellEntityMethod, _>(&tables)?.element;
        let entity_id = if method.entity_id == 0 { cell_entity_id } else { method.entity_id };
        self.events.push_back(Event::CellMethod(CellMethodEvent { addr, entity_id, call: method.call }));

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
        let _ = self.bundle.element_writer().write_simple_reply(SessionKey { session_key }, request_id);
        self.protocol.off_channel(addr).prepare(&mut self.bundle, false);
        let _ = self.socket.send_bundle(&self.bundle, addr);

        true

    }

    /// Tell a client the server's update frequency and game time, which the real server
    /// does right after the login reply. `unknown` is a field of unknown meaning, see
    /// [`client::element::UpdateFrequencyNotification`].
    pub fn update_frequency(&mut self, addr: SocketAddr, frequency: u8, unknown: u16, game_time: u32) -> io::Result<()> {
        self.outbox.client(addr, &mut self.protocol, &self.socket)?.write_simple(client::element::UpdateFrequencyNotification { frequency, unknown, game_time })
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

        self.outbox.client(addr, &mut self.protocol, &self.socket)?.write(
            client::element::CreateBasePlayer {
                entity_id,
                entity_type_id,
                entity_data: Cow::Borrowed(&entity_data),
                entity_components_count,
            },
            dispatch,
        )?;

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

        self.outbox.client(addr, &mut self.protocol, &self.socket)?.write_simple(client::element::CreateCellPlayer {
            unk_flag: 0,
            space_id,
            unk_short: 0,
            vehicle_id,
            position,
            packed_xz_scale,
            direction,
            cell_data,
        })?;

        if let Some(client) = self.clients.get_mut(&addr) {
            client.cell_entity_id = Some(handle.entity_id);
        }

        Ok(())

    }

    /// Call a method, resolved by name against the target entity's client-method table,
    /// on the client owning the given entity, selecting it first if needed.
    pub fn call_method(&mut self, handle: Handle, method_name: &str, args: Vec<Value>) -> io::Result<()> {

        let entry = self.entities.get(&handle.entity_id)
            .ok_or_else(|| io_invalid_data(format_args!("no such entity: {}", handle.entity_id)))?;
        let addr = entry.addr;
        let type_id = entry.type_id;

        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("no dispatch table for entity: {}", handle.entity_id)))?;

        let (exposed_id, name) = dispatch.client_methods.iter().enumerate()
            .find(|(_, m)| &*m.name == method_name)
            .map(|(exposed_id, m)| (exposed_id as u16, m.name.clone()))
            .ok_or_else(|| io_invalid_data(format_args!("unknown client method: {method_name}")))?;

        let is_player = self.clients.get(&addr).and_then(|c| c.base_entity_id) == Some(handle.entity_id);
        let out = self.outbox.client(addr, &mut self.protocol, &self.socket)?;
        if is_player {
            out.select_player()?;
        } else {
            out.select_entity(handle.entity_id)?;
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
    pub fn reset_entities(&mut self, addr: SocketAddr, keep_player_on_base: bool) -> io::Result<()> {

        self.outbox.client(addr, &mut self.protocol, &self.socket)?.write_simple(client::element::ResetEntities { keep_player_on_base })?;

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

    /// Write a simple element to a client's outbox.
    fn send_simple<E: SimpleElement>(&mut self, addr: SocketAddr, element: E) -> io::Result<()> {
        self.outbox.client(addr, &mut self.protocol, &self.socket)?.write_simple(element)
    }

    /// Give a client the server's game time: sent each tick, it also keeps the channel
    /// alive, a client that hears nothing for a while dropping it.
    ///
    /// With the client's vehicle, the same bundle places it, as the client times the
    /// position updates in a bundle by the tick opening it: a reference point at the
    /// vehicle's position rounded to the metre, the player entity on that point, then
    /// the vehicle's exact position. Then the other vehicles, by their alias, with their
    /// properties; their `player_properties` and `nrl_updates` are not sent.
    ///
    /// What was written to the client before is sent first, so that the tick opens a
    /// bundle of its own; what is written after it, until the next poll, follows it in
    /// that bundle.
    pub fn tick(&mut self, addr: SocketAddr, tick: u8, vehicle: Option<&TickVehicle>, others: &[TickVehicle]) -> io::Result<()> {
        self.outbox.flush(addr, &mut self.protocol, &self.socket)?;
        let writer = self.outbox.client(addr, &mut self.protocol, &self.socket)?;
        writer.write_simple(client::element::TickSync { tick })?;
        if let Some(vehicle) = vehicle {
            writer.write_simple(client::element::RelativePosition { position: vehicle.position.round() })?;
            writer.write_simple(client::element::AvatarUpdatePlayerDetailed {
                ref_num: 0,
                position: glam::Vec3::ZERO,
                direction: glam::Vec3::ZERO,
            })?;
            writer.write_simple(client::element::AvatarUpdateAliasDetailed {
                ref_num: 0,
                id_alias: vehicle.id_alias,
                position: vehicle.position,
                direction: vehicle.direction,
            })?;
            // The position update selected the vehicle, its properties follow.
            if let Some((type_id, properties)) = &vehicle.properties {
                let dispatch = self.dispatch.entity_from_id(*type_id)
                    .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
                for property in properties {
                    writer.write_selected(property.clone(), &dispatch.properties[..])?;
                }
            }
            // As the real server sends updates: the record goes to the client's NRL buffer,
            // then a flag applies it. The client rejects them as `NrlUpdateNode` elements.
            for update in &vehicle.nrl_updates {
                let mut fragment = update.node_id.to_be_bytes().to_vec();
                fragment.extend_from_slice(&update.payload);
                writer.write_simple(client::element::NrlData { fragment })?;
                writer.write_simple(client::element::NrlUpdateNodeFlag)?;
            }
            if let Some((type_id, properties)) = &vehicle.player_properties {
                let dispatch = self.dispatch.entity_from_id(*type_id)
                    .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
                writer.select_player()?;
                for property in properties {
                    writer.write_selected(property.clone(), &dispatch.properties[..])?;
                }
            }
        }
        // Other vehicles: each position update selects its vehicle for its properties.
        for other in others {
            writer.write_simple(client::element::AvatarUpdateAliasDetailed {
                ref_num: 0,
                id_alias: other.id_alias,
                position: other.position,
                direction: other.direction,
            })?;
            if let Some((type_id, properties)) = &other.properties {
                let dispatch = self.dispatch.entity_from_id(*type_id)
                    .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
                for property in properties {
                    writer.write_selected(property.clone(), &dispatch.properties[..])?;
                }
            }
        }
        Ok(())
    }

    /// Tell a client where its cell app is, before creating its cell player.
    pub fn set_cell_app_ext_address(&mut self, addr: SocketAddr, cell_app_addr: SocketAddrV4) -> io::Result<()> {
        self.send_simple(addr, client::element::SetCellAppExtAddress(cell_app_addr.into()))
    }

    /// Send a client one of its space's properties, still raw, see
    /// [`client::element::SpaceProperty`].
    pub fn space_property(&mut self, addr: SocketAddr, data: Vec<u8>) -> io::Result<()> {
        self.send_simple(addr, client::element::SpaceProperty { data })
    }

    /// Map a geometry (a map, `spaces/08_ruinberg` say) into a client's space, at the
    /// identity transform.
    pub fn add_space_geometry_mapping(&mut self, addr: SocketAddr, space_id: u32, entry_id: SocketAddrV4, path: &str) -> io::Result<()> {
        let mut transform = [0.0; 16];
        for i in 0..4 {
            transform[i * 5] = 1.0;
        }
        self.send_simple(addr, client::element::AddSpaceGeometryMapping {
            space_id,
            entry_id: entry_id.into(),
            path: path.to_string(),
            transform,
            unk: 1,
            rest: Vec::new(),
        })
    }

    /// An entity enters a client's area of interest, under an id alias the position
    /// updates use. The client asks for its state next, see
    /// [`Event::RequestEntityUpdate`], answered with [`Self::create_entity`].
    pub fn enter_aoi(&mut self, addr: SocketAddr, entity_id: u32, id_alias: u8) -> io::Result<()> {
        self.send_simple(addr, client::element::EnterAoi { entity_id, id_alias })
    }

    /// Place an entity of a client's area of interest, by its id alias, with a full
    /// position and direction (yaw, pitch, roll). Like every position update, it also
    /// selects the entity for the property updates that follow.
    pub fn update_position_detailed(&mut self, addr: SocketAddr, id_alias: u8, position: glam::Vec3, direction: glam::Vec3) -> io::Result<()> {
        self.send_simple(addr, client::element::AvatarUpdateAliasDetailed { ref_num: 0, id_alias, position, direction })
    }

    /// Create an entity on a client, from its state, and keep track of its type so its
    /// properties can be updated and calls targeting it decoded.
    pub fn create_entity(&mut self, addr: SocketAddr, element: client::element::CreateEntity) -> io::Result<()> {
        let entity_id = element.entity_id;
        let type_id = element.entity_type_id;
        self.send_simple(addr, element)?;
        self.entities.insert(entity_id, EntityEntry { addr, type_id, data: Value::None });
        Ok(())
    }

    /// Create an entity on a client with a full direction (yaw, pitch, roll) rather than
    /// a packed one, and its initial properties, given in the entity type's property
    /// table. The entity is tracked like [`Self::create_entity`]'s.
    pub fn create_entity_detailed(
        &mut self,
        addr: SocketAddr,
        entity_id: u32,
        type_id: u16,
        position: glam::Vec3,
        direction: glam::Vec3,
        properties: &client::element::ChangedProperties,
    ) -> io::Result<()> {
        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
        let client_data = properties.encode(&dispatch.properties)?;
        self.send_simple(addr, client::element::CreateEntityDetailed {
            entity_id,
            entity_type_id: type_id,
            unk_u16: 0,
            resource_name: "bw.default".to_string(),
            position,
            direction,
            client_data,
        })?;
        self.entities.insert(entity_id, EntityEntry { addr, type_id, data: Value::None });
        Ok(())
    }

    /// Create NRL nodes on a client: an entity's node and its dynamic components, say.
    /// The stream's first message must create a node.
    pub fn nrl_create_node(&mut self, addr: SocketAddr, stream: client::element::NrlStream) -> io::Result<()> {
        self.send_simple(addr, client::element::NrlCreateNode(stream))
    }

    /// Change some of an entity's client properties on the client owning it, given in
    /// that entity type's property table.
    pub fn update_entity(&mut self, addr: SocketAddr, entity_id: u32, type_id: u16, properties: client::element::ChangedProperties) -> io::Result<()> {
        let dispatch = self.dispatch.entity_from_id(type_id)
            .ok_or_else(|| io_invalid_data(format_args!("unknown entity type: {type_id}")))?;
        let out = self.outbox.client(addr, &mut self.protocol, &self.socket)?;
        out.select_entity(entity_id)?;
        out.write_selected(client::element::UpdateEntity(properties), &dispatch.properties[..])
    }

    /// A logged in client's session key and blowfish key. A client told to switch base
    /// app logs in again with both: the session key as its login key, the blowfish key
    /// for the new channel.
    pub fn client_keys(&self, addr: SocketAddr) -> Option<(u32, Arc<Blowfish>)> {
        self.clients.get(&addr).map(|client| (client.session_key, Arc::clone(&client.blowfish)))
    }

    /// Tell the client to disconnect and reconnect to a (possibly different) base app
    /// address.
    pub fn switch_base_app(&mut self, addr: SocketAddr, base_addr: SocketAddrV4, reset_entities: bool) -> io::Result<()> {
        self.send_simple(addr, client::element::SwitchBaseApp { base_addr: base_addr.into(), reset_entities })
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
    pub fn push_resource(&mut self, addr: SocketAddr, res_id: u16, description: Vec<u8>, data: &[u8]) -> io::Result<()> {

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

/// Resolves a client-to-cell call's method table from the entity it names, for
/// [`element::CellEntityMethod`].
struct CellTables<'a> {
    entities: &'a HashMap<u32, EntityEntry>,
    dispatch: &'a ScriptDispatch,
    /// Substituted for a wire entity id of 0, exactly as the real base app does.
    cell_entity_id: u32,
}

impl element::CellMethodTables for CellTables<'_> {
    fn cell_methods(&self, entity_id: u32) -> Option<&[MethodDef]> {
        let entity_id = if entity_id == 0 { self.cell_entity_id } else { entity_id };
        let entry = self.entities.get(&entity_id)?;
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

/// A client's vehicle, as each tick places it, see [`App::tick`].
#[derive(Debug, Clone)]
pub struct TickVehicle {
    pub id_alias: u8,
    pub position: glam::Vec3,
    /// Roll, pitch and yaw.
    pub direction: glam::Vec3,
    /// Properties changing every tick, `gunAnglesPacked` say, with the vehicle's entity
    /// type.
    pub properties: Option<(u16, Vec<client::element::EntityProperty>)>,
    /// The player entity's properties about its vehicle, `ownVehicleAuxPhysicsData`
    /// say, with the player entity's type.
    pub player_properties: Option<(u16, Vec<client::element::EntityProperty>)>,
    /// Updates to the vehicle's components, its server gun marker say.
    pub nrl_updates: Vec<client::element::NrlUpdateNode>,
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
    EnableEntities(EnableEntitiesEvent),
    /// A client asks for the state of an entity that entered its area of interest.
    RequestEntityUpdate(RequestEntityUpdateEvent),
    /// No event came before the deadline, see [`App::poll_until`].
    Timeout,
    BaseMethod(BaseMethodEvent),
    CellMethod(CellMethodEvent),
    /// A client reports where its player entity is.
    AvatarUpdate(AvatarUpdateEvent),
}

/// A client reports where its player entity is, see
/// [`element::AvatarUpdateImplicit`] and [`element::AvatarUpdateExplicit`].
#[derive(Debug)]
pub struct AvatarUpdateEvent {
    pub addr: SocketAddr,
    /// The vehicle it says it is on, when explicit.
    pub vehicle_id: Option<u32>,
    pub position: glam::Vec3,
    /// Roll, pitch and yaw.
    pub direction: glam::Vec3,
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
    pub addr: SocketAddr,
}

/// A client asks for an entity's state, after it entered the client's area of interest.
#[derive(Debug)]
pub struct RequestEntityUpdateEvent {
    pub addr: SocketAddr,
    pub entity_id: u32,
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

/// A client called a cell method, on its own cell entity or on another one it targets.
#[derive(Debug)]
pub struct CellMethodEvent {
    pub addr: SocketAddr,
    /// The target entity, with the wire's `0` already resolved to the client's own.
    pub entity_id: u32,
    /// The decoded call, resolved dynamically against the entity's own cell-method
    /// table (see [`crate::app::script::EntityDispatch`]).
    pub call: MethodCall,
}
