//! The emulator implementation of the login + base applications, we are trying to
//! reproduce the official server implementation.

use std::collections::{hash_map, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use wgtk::app::{login, base};
use wgtk::script::Script;
use rsa::RsaPrivateKey;
use blowfish::Blowfish;

mod account;
mod capture;

use std::path::Path;
use std::io;

use wgtk::app::dispatch::{MethodCall, ScriptDispatch};
use wgtk::app::client::element::{NrlBody, NrlChild, NrlComponentData, NrlMessage, NrlNode, NrlStream};
use glam::Vec3;
use wgtk::script::Value;

use account::AccountData;
use capture::{Answer, CapturedSession};
use crate::wot::account_commands::{CMD_ENQUEUE_IN_BATTLE_QUEUE, CMD_REQ_QUEUE_INFO, CMD_SYNC_DATA};
use crate::wot::items::{item_type, TankmanDescr, VehicleDescr};
use crate::wot::item_defs::VehicleDef;
use wgtk::res::fs::ResFilesystem;
use crate::CliResult;


pub fn run(
    login_app_addr: SocketAddrV4,
    base_app_addr: SocketAddrV4,
    encryption_key: Option<Arc<RsaPrivateKey>>,
    script: Script,
    from_capture: Option<&Path>,
    game_dir: &Path,
    give_vehicles: &[String],
) -> CliResult<()> {

    let mut login_app = login::App::new(login_app_addr.into())
        .map_err(|e| format!("Failed to bind login app: {e}"))?;

    if let Some(encryption_key) = encryption_key {
        login_app.set_encryption(encryption_key);
    }

    let base_app = base::App::new(base_app_addr.into(), script)
        .map_err(|e| format!("Failed to bind base app: {e}"))?;

    let session = match from_capture {
        Some(path) => {
            let mut session = CapturedSession::load(path, base_app.dispatch())
                .map_err(|e| format!("Failed to extract the session from {}: {e}", path.display()))?;
            let battle = match &session.battle {
                Some(battle) => format!("{} calls, cell player {}, {} space properties, map {:?}, vehicle alias {:?}, position {}, update {}, entity {}, arena info {:?}",
                    battle.calls.len(), battle.cell_player.is_some(), battle.space_properties.len(),
                    battle.geometry.as_ref().map(|g| &g.path), battle.vehicle_alias,
                    battle.vehicle_position.is_some(), battle.vehicle_update.is_some(), battle.vehicle_entity.is_some(),
                    battle.arena_info.as_ref().map(|a| (a.entity_id, a.nrl.len(), a.created.as_ref().map(|(t, p)| (*t, p.values.len()))))),
                None => "none".to_string(),
            };
            info!("Serving from capture {}: account {}, {} requests, battle: {battle}",
                path.display(), if session.account.is_some() { "found" } else { "missing" }, session.requests.len());
            for request in &session.requests {
                let answers: Vec<String> = request.answers.iter().map(|answer| match answer {
                    Answer::Call { method, .. } => method.to_string(),
                    Answer::Resource { data, .. } => format!("resource ({} bytes)", data.len()),
                }).collect();
                debug!("  {} {:?} -> {}", request.method, request.args.get(..2), answers.join(", "));
            }
            let mut vehicles = Vec::new();
            if !give_vehicles.is_empty() {
                let fs = ResFilesystem::new(game_dir.join("res"))
                    .map_err(|e| format!("Failed to open resource filesystem: {e}"))?;
                for vehicle in give_vehicles {
                    let (nation, name) = vehicle.split_once(':')
                        .ok_or_else(|| format!("Expected <nation>:<name>, got {vehicle}"))?;
                    vehicles.push(VehicleDef::load(&fs, nation, name)
                        .map_err(|e| format!("Failed to load vehicle {vehicle}: {e}"))?);
                }
            }
            reencode_account_data(&mut session, &vehicles)
                .map_err(|e| format!("Failed to re-encode the captured account data: {e}"))?;
            Some(session)
        }
        None => None,
    };

    let shared = Arc::new(Shared {
        login_clients: Mutex::new(HashMap::new()),
    });

    let login_thread = LoginThread {
        app: login_app,
        shared: Arc::clone(&shared),
        base_app_addr,
        login_challenges: HashMap::new(),
    };

    let base_thread = BaseThread {
        app: base_app,
        shared,
        session,
        players: HashMap::new(),
        enqueued: HashSet::new(),
        pending_switches: HashMap::new(),
        battle_clients: HashSet::new(),
        tick: 0,
    };

    thread::scope(move |scope| {
        scope.spawn(move || login_thread.run());
        scope.spawn(move || base_thread.run());
    });

    Ok(())

}

#[derive(Debug)]
struct LoginThread {
    app: login::App,
    shared: Arc<Shared>,
    base_app_addr: SocketAddrV4,
    login_challenges: HashMap<SocketAddr, bool>,
}

#[derive(Debug)]
struct BaseThread {
    app: base::App,
    shared: Arc<Shared>,
    /// What to serve, when running from a capture.
    session: Option<CapturedSession>,
    /// Each client's player entity.
    players: HashMap<SocketAddr, base::Handle>,
    /// Clients waiting in the battle queue.
    enqueued: HashSet<SocketAddr>,
    /// Clients told to switch base app for their battle, by the session key they will
    /// log in again with, with the IP they will come from and their blowfish key.
    pending_switches: HashMap<u32, (IpAddr, Arc<Blowfish>)>,
    /// Clients connected for a battle, which get an `Avatar` rather than an `Account`.
    battle_clients: HashSet<SocketAddr>,
    /// The game tick, sent to battle clients.
    tick: u8,
}

#[derive(Debug)]
struct Shared {
    login_clients: Mutex<HashMap<u32, LoginClient>>,
}

#[derive(Debug)]
struct LoginClient {
    addr: SocketAddr,
    blowfish: Arc<Blowfish>,
}

impl LoginThread {

    #[instrument(name = "login", skip_all)]
    fn run(mut self) {

        use login::Event;

        info!("Running on: {}", self.app.addr().unwrap());

        if self.app.has_encryption() {
            info!("Encryption enabled");
        }

        loop {
            match self.app.poll() {
                Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Error: {}", error.error);
                    } else {
                        warn!("Error: {}", error.error);
                    }
                }
                Event::Ping(ping) => {
                    info!(addr = %ping.addr, "Ping-Pong: {:?}", ping.latency);
                }
                Event::Login(login) => {

                    if !*self.login_challenges.entry(login.addr).or_default() {
                        info!(addr = %login.addr, "Login pending, sending challenge");
                        self.app.answer_login_challenge(login.addr);
                    } else {

                        info!(addr = %login.addr, "Login success");

                        let mut clients = self.shared.login_clients.lock().unwrap();
                        let (login_key, slot) = loop {
                            let login_key = OsRng.next_u32();
                            match clients.entry(login_key) {
                                hash_map::Entry::Occupied(_) => continue,
                                hash_map::Entry::Vacant(v) => break (login_key, v),
                            }
                        };

                        let blowfish = self.app.answer_login_success(login.addr, self.base_app_addr, login_key, String::new()).unwrap();

                        slot.insert(LoginClient {
                            addr: login.addr,
                            blowfish,
                        });

                        // app.answer_login_error(login.addr, LoginError::Banned, "{\"bans\":\"{\\\"expiryTime\\\":1726435530,\\\"reason\\\":\\\"It's the reason\\\"}\"}".to_string());
                        
                    }
                
                }
                Event::Challenge(challenge) => {
                    info!(addr = %challenge.addr, "Challenge successful...");
                    if let Some(completed) = self.login_challenges.get_mut(&challenge.addr) {
                        *completed = true;
                    }
                }
            }
        }

    }

}

impl BaseThread {

    /// Bring a client that enabled entities to its account, as the captured server did:
    /// the `Account` entity, then `showGUI`.
    fn enter_account(&mut self, addr: SocketAddr) -> io::Result<()> {

        let Some(session) = &self.session else { return Ok(()) };

        let Some((data, components_count)) = &session.account else { return Ok(()) };
        let handle = self.app.create_base_player(addr, "Account", data.clone(), *components_count)?;
        self.players.insert(addr, handle);
        info!(%addr, "Created account {}", handle.entity_id());

        if let Some(args) = &session.show_gui {
            self.app.select_player_entity(handle)?;
            self.app.call_method(handle, "showGUI", args.clone())?;
        }

        Ok(())

    }

    /// The client's arena is ready: tell its account, then switch it to the base app
    /// holding its avatar, this one, as the captured server did.
    fn start_battle(&mut self, addr: SocketAddr) -> io::Result<()> {

        if self.session.as_ref().and_then(|s| s.battle.as_ref()).is_none() {
            warn!(%addr, "The capture has no battle to enter");
            return Ok(());
        }

        let (Some(&handle), Some((session_key, blowfish))) = (self.players.get(&addr), self.app.client_keys(addr)) else {
            return Ok(());
        };

        self.app.select_player_entity(handle)?;
        self.app.call_method(handle, "onArenaCreated", Vec::new())?;

        let SocketAddr::V4(base_addr) = self.app.addr()? else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "base app not on IPv4"));
        };
        self.pending_switches.insert(session_key, (addr.ip(), blowfish));
        self.app.switch_base_app(addr, base_addr, true)?;
        self.players.remove(&addr);
        info!(%addr, "Arena created, switching to {base_addr}");

        Ok(())

    }

    /// Bring a client that reconnected for its battle to its avatar.
    fn enter_battle(&mut self, addr: SocketAddr) -> io::Result<()> {

        let Some((data, components_count)) = self.session.as_ref()
            .and_then(|s| s.battle.as_ref())
            .and_then(|b| b.avatar.as_ref()) else {
            warn!(%addr, "The capture has no avatar to create");
            return Ok(());
        };

        let handle = self.app.create_base_player(addr, "Avatar", data.clone(), *components_count)?;
        self.players.insert(addr, handle);
        info!(%addr, "Created avatar {}", handle.entity_id());

        // The battle's setup, as the captured server sent it, but for the cell app's
        // address, which is ours: the arena, the avatar's cell side in the space, the
        // space and its map, then the player's vehicle entering the area of interest.
        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        for (method, args) in &battle.calls {
            self.app.select_player_entity(handle)?;
            self.app.call_method(handle, method, args.clone())?;
        }

        let SocketAddr::V4(cell_app_addr) = self.app.addr()? else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "base app not on IPv4"));
        };
        self.app.set_cell_app_ext_address(addr, cell_app_addr)?;

        let Some(cell) = &battle.cell_player else {
            warn!(%addr, "The capture has no cell player");
            return Ok(());
        };
        self.app.create_cell_player(handle, cell.space_id, cell.vehicle_id, cell.position, cell.direction,
            cell.packed_xz_scale, cell.cell_data.clone())?;

        for data in &battle.space_properties {
            self.app.space_property(addr, data.clone())?;
        }
        if let Some(geometry) = &battle.geometry {
            self.app.add_space_geometry_mapping(addr, geometry.space_id, geometry.entry_id.addr, &geometry.path)?;
            info!(%addr, "Entering space {} with {}", geometry.space_id, geometry.path);
        }

        // Its position and properties follow its creation, see `create_vehicle`.
        if let Some(alias) = battle.vehicle_alias {
            self.app.enter_aoi(addr, cell.vehicle_id, alias)?;
        }

        // The arena's info entity, created right away as entities without an alias are,
        // then given its vehicles: only the player's, the arena having no one else yet.
        if let Some(arena) = &battle.arena_info {
            let Some((type_id, properties)) = &arena.created else {
                warn!(%addr, "The capture's arena info entity has no creation");
                return Ok(());
            };
            let mut properties = properties.clone();
            for (_, name, value) in &mut properties.values {
                if &**name == "avatarID" {
                    *value = with_int(value, handle.entity_id().into());
                }
            }
            let stream = arena_vehicles(self.app.dispatch(), &arena.nrl, cell.vehicle_id)?;
            self.app.enter_aoi(addr, arena.entity_id, NO_ID_ALIAS)?;
            self.app.create_entity_detailed(addr, arena.entity_id, *type_id, Vec3::ZERO, Vec3::ZERO, &properties)?;
            self.app.nrl_create_node(addr, stream)?;
            info!(%addr, "Created arena info {}", arena.entity_id);
        }

        Ok(())

    }

    /// Create the player's vehicle once the client asks for it, then place it and give
    /// it its properties: the client drops both for an entity it doesn't have yet.
    fn create_vehicle(&mut self, addr: SocketAddr, entity_id: u32) -> io::Result<()> {

        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let Some(entity) = battle.vehicle_entity.as_ref().filter(|e| e.entity_id == entity_id) else { return Ok(()) };

        info!(%addr, "Creating vehicle {entity_id}");
        self.app.create_entity(addr, entity.clone())?;
        if let (Some(alias), Some(position)) = (battle.vehicle_alias, &battle.vehicle_position) {
            self.app.update_position_detailed(addr, alias, position.position, position.direction)?;
        }
        if let Some(update) = &battle.vehicle_update {
            self.app.update_entity(addr, entity_id, entity.entity_type_id, update.clone())?;
        }

        Ok(())

    }

    /// Answer a client request with what the captured server answered to the closest
    /// captured request, with the new request's id in place of the captured one.
    fn answer(&mut self, addr: SocketAddr, method: &str, args: &[Value]) -> io::Result<()> {

        let (Some(session), Some(&handle)) = (&self.session, self.players.get(&addr)) else { return Ok(()) };
        let Some(request_id) = args.first().and_then(Value::as_i64) else { return Ok(()) };

        let Some(captured) = session.find_request(method, args) else {
            warn!(%addr, "No captured answer to {method} {args:?}");
            return Ok(());
        };

        for answer in &captured.answers {
            match answer {
                Answer::Call { method, args } => {
                    // Only an answer carrying the captured request's id gets the new
                    // one: an id-less answer's first argument is something else, the
                    // queue type of `onEnqueued` say.
                    let mut args = args.clone();
                    if let Some(first) = args.first_mut().filter(|first| Some(&**first) == captured.args.first()) {
                        *first = with_int(first, request_id);
                    }
                    info!(%addr, "Answering with {method} {}", format!("{args:?}").chars().take(200).collect::<String>());
                    self.app.select_player_entity(handle)?;
                    self.app.call_method(handle, method, args)?;
                }
                Answer::Resource { description, data } => {
                    info!(%addr, "Answering with resource {request_id}, {} bytes", data.len());
                    self.app.push_resource(addr, request_id as u16, description.clone(), data)?;
                }
            }
        }

        Ok(())

    }

    #[instrument(name = "base", skip_all)]
    fn run(mut self) {

        info!("Running on: {}", self.app.addr().unwrap());

        // Battle clients need to hear from the server every tick, 10 Hz as captured.
        const TICK: Duration = Duration::from_millis(100);
        if let Err(e) = self.app.set_poll_timeout(Some(TICK)) {
            warn!("Failed to set the poll timeout, no ticks: {e}");
        }
        let mut next_tick = Instant::now() + TICK;

        loop {

            if Instant::now() >= next_tick {
                next_tick += TICK;
                self.tick = self.tick.wrapping_add(1);
                for &addr in &self.battle_clients {
                    if let Err(e) = self.app.tick(addr, self.tick) {
                        warn!(%addr, "Failed to tick: {e}");
                    }
                }
            }

            match self.app.poll() {
                base::Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Error: {}", error.error);
                    } else {
                        warn!("Error: {}", error.error);
                    }
                }
                base::Event::Login(login) => {

                    // A client coming back for its battle, see `start_battle`.
                    if let Some((ip, blowfish)) = self.pending_switches.remove(&login.login_key) {
                        if ip != login.addr.ip() {
                            info!(addr = %login.addr, "Login #{}... Invalid address after switch", login.attempt_num);
                            continue;
                        }
                        info!(addr = %login.addr, "Login #{}... Success, for the battle", login.attempt_num);
                        self.app.answer_login_success(login.addr, blowfish);
                        if let Some((frequency, unknown)) = self.session.as_ref().and_then(|s| s.update_frequency) {
                            if let Err(e) = self.app.update_frequency(login.addr, frequency, unknown, 0) {
                                warn!(addr = %login.addr, "Failed to send the update frequency: {e}");
                            }
                        }
                        self.battle_clients.insert(login.addr);
                        continue;
                    }

                    let mut clients = self.shared.login_clients.lock().unwrap();
                    let client = match clients.remove(&login.login_key) {
                        Some(client) => client,
                        None => {
                            info!(addr = %login.addr, "Login #{}... Invalid key", login.attempt_num);
                            continue;
                        }
                    };

                    if client.addr != login.addr {
                        info!(addr = %login.addr, "Login #{}... Invalid address", login.attempt_num);
                        continue;
                    }
                    
                    info!(addr = %login.addr, "Login #{}... Success", login.attempt_num);
                    self.app.answer_login_success(login.addr, client.blowfish);

                    // Right after the login reply, as the captured server did.
                    if let Some((frequency, unknown)) = self.session.as_ref().and_then(|s| s.update_frequency) {
                        if let Err(e) = self.app.update_frequency(login.addr, frequency, unknown, 0) {
                            warn!(addr = %login.addr, "Failed to send the update frequency: {e}");
                        }
                    }

                }
                base::Event::EnableEntities(event) => {
                    info!(addr = %event.addr, "Entities enabled");
                    if self.battle_clients.contains(&event.addr) {
                        if let Err(e) = self.enter_battle(event.addr) {
                            warn!(addr = %event.addr, "Failed to create the avatar: {e}");
                        }
                    } else if let Err(e) = self.enter_account(event.addr) {
                        warn!(addr = %event.addr, "Failed to create the account: {e}");
                    }
                }
                base::Event::BaseMethod(event) => {
                    match &event.call {
                        MethodCall::Known { name, args } => {
                            info!(addr = %event.addr, "Base method call on entity {}: {name} {args:?}", event.entity_id);
                            if let Err(e) = self.answer(event.addr, name, args) {
                                warn!(addr = %event.addr, "Failed to answer {name}: {e}");
                            }
                            // The arena is ready as soon as the client first asks how the
                            // queue is going.
                            let command = args.get(1).and_then(Value::as_i64);
                            if command == Some(CMD_ENQUEUE_IN_BATTLE_QUEUE.into()) {
                                self.enqueued.insert(event.addr);
                            } else if command == Some(CMD_REQ_QUEUE_INFO.into()) && self.enqueued.remove(&event.addr) {
                                if let Err(e) = self.start_battle(event.addr) {
                                    warn!(addr = %event.addr, "Failed to start the battle: {e}");
                                }
                            }
                        }
                        MethodCall::Unknown { .. } =>
                            warn!(addr = %event.addr, "Base method call on entity {}: {:?}", event.entity_id, event.call),
                    }
                }
                base::Event::CellMethod(event) => {
                    info!(addr = %event.addr, "Cell method call on entity {}", event.entity_id);
                }
                base::Event::RequestEntityUpdate(event) => {
                    // Only the player's vehicle enters the area of interest for now.
                    if let Err(e) = self.create_vehicle(event.addr, event.entity_id) {
                        warn!(addr = %event.addr, "Failed to create vehicle {}: {e}", event.entity_id);
                    }
                }
                base::Event::Timeout => {}

            }

            // let entity: Handle<entity::Login> = self.app.create_base_player(addr, entity::Login {
            //     accountDBID_s: "09518858105".to_string(),
            //     loginPriority: 0,
            // });

            // self.app.call_method(entity, entity::Login_Client::setPeripheryRoutingGroup());
            // self.app.reset_entities(addr);

        }

    }

}

/// Serve the captured account data as we encode it rather than as captured: decode
/// each full sync answer, check that its vehicles and tankmen decode and encode back
/// the same, give it the vehicles asked for, and encode it back.
fn reencode_account_data(session: &mut CapturedSession, give_vehicles: &[VehicleDef]) -> io::Result<()> {

    for request in &mut session.requests {

        let is_full_sync = &*request.method == "doCmdInt3"
            && request.args.get(1).and_then(Value::as_i64) == Some(CMD_SYNC_DATA.into())
            && request.args.get(2).and_then(Value::as_i64) == Some(0);
        if !is_full_sync {
            continue;
        }

        for answer in &mut request.answers {

            let Answer::Resource { description, data } = answer else { continue };

            if account::resource_description(data)? != *description {
                warn!("The captured account data description differs from ours: {description:02X?}");
            }

            let mut account = AccountData::decode(data)?;
            let invalid = |what: &str, id: i64, e: &dyn std::fmt::Display| {
                io::Error::new(io::ErrorKind::InvalidData, format!("{what} {id}: {e}"))
            };
            let mut vehicles = 0;
            for (id, cd) in account.compact_descrs(item_type::VEHICLE) {
                let descr = VehicleDescr::decode(cd).map_err(|e| invalid("vehicle", id, &e))?;
                if descr.encode() != cd {
                    return Err(invalid("vehicle", id, &"encodes differently"));
                }
                vehicles += 1;
            }
            let mut tankmen = 0;
            for (id, cd) in account.compact_descrs(item_type::TANKMAN) {
                let descr = TankmanDescr::decode(cd).map_err(|e| invalid("tankman", id, &e))?;
                if descr.encode() != cd {
                    return Err(invalid("tankman", id, &"encodes differently"));
                }
                tankmen += 1;
            }

            for vehicle in give_vehicles {
                let id = account.add_vehicle(vehicle)?;
                info!("Gave vehicle {} as {id}: {:?}, ammunition {:?}", vehicle.name, vehicle.descr, vehicle.default_ammo());
            }

            let (new_description, new_data) = account.encode()?;
            info!("Account data: revision {:?}, {} sections, {vehicles} vehicles, {tankmen} tankmen, {} bytes re-encoded to {}",
                account.revision(), account.dict().map_or(0, |d| d.len()), data.len(), new_data.len());
            *description = new_description;
            *data = new_data;

        }

    }

    Ok(())

}

/// The arena info entity's NRL tree, its `vehiclesInfo` reduced to the given vehicle's,
/// encoded again.
fn arena_vehicles(dispatch: &ScriptDispatch, captured: &[NrlMessage], vehicle_id: u32) -> io::Result<NrlStream> {
    let mut messages = captured.to_vec();
    for message in &mut messages {
        let NrlBody::CreateNode { node: NrlNode::DynComponent { component_type_id, data: Some(data), .. }, .. } = &mut message.body else {
            continue;
        };
        let children = data.children.iter().cloned().map(|(id, child)| match child {
            NrlChild::Property { name, value: Value::Seq(entries) } if &*name == "vehiclesInfo" => {
                let entries = entries.into_iter().filter(|entry| match entry {
                    Value::Dict(fields) => fields.get("vehicleID").and_then(Value::as_i64) == Some(vehicle_id.into()),
                    _ => false,
                }).collect();
                (id, NrlChild::Property { name, value: Value::Seq(entries) })
            }
            other => (id, other),
        }).collect();
        *data = NrlComponentData::encode(dispatch, *component_type_id, children)?;
    }
    Ok(NrlStream { messages, rest: Vec::new() })
}

/// The id alias given to an entity entering the area of interest without one: entities
/// that don't move, which no position update needs a short alias for.
const NO_ID_ALIAS: u8 = 255;

/// The same integer value type as `value`, holding `n`: a request id or an entity id
/// must keep the type its method or property declares.
fn with_int(value: &Value, n: i64) -> Value {
    match value {
        Value::Int8(_) => Value::Int8(n as i8),
        Value::Int16(_) => Value::Int16(n as i16),
        Value::Int32(_) => Value::Int32(n as i32),
        Value::Int64(_) => Value::Int64(n),
        Value::UInt8(_) => Value::UInt8(n as u8),
        Value::UInt16(_) => Value::UInt16(n as u16),
        Value::UInt32(_) => Value::UInt32(n as u32),
        Value::UInt64(_) => Value::UInt64(n as u64),
        other => other.clone(),
    }
}
