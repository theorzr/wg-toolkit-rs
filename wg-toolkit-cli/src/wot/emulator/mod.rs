//! The emulator implementation of the login + base applications, we are trying to
//! reproduce the official server implementation.

use std::collections::{hash_map, BTreeMap, HashMap};
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
mod gun;
mod physics;

use std::path::{Path, PathBuf};
use std::io;

use wgtk::app::dispatch::{MethodCall, ScriptDispatch};
use wgtk::app::client::element::{ChangedProperties, EntityProperty, NrlIdList, NrlUpdateNode, NrlBody, NrlChild, NrlComponentData, NrlMessage, NrlNode, NrlStream};
use wgtk::script::{PythonValue, StringValue};
use wgtk::util::io::serde_pickle_ser_options;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;
use glam::Vec3;
use wgtk::script::Value;

use account::AccountData;
use capture::{Answer, CapturedSession};
use gun::{Gun, Landing, Shot, VehicleBox};
use physics::{MotionLimits, Vehicle};
use crate::wot::account_commands::{CMD_ENQUEUE_IN_BATTLE_QUEUE, CMD_REQ_QUEUE_INFO, CMD_SYNC_DATA};
use crate::wot::items::{item_type, TankmanDescr, VehicleDescr};
use crate::wot::item_defs::{AimingDef, ShotDef, VehicleDef};
use wgtk::res::fs::ResFilesystem;
use wgtk::space::terrain::Terrain;
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
                Some(battle) => format!("{} calls, cell player {}, {} space properties, map {:?}, vehicle alias {:?}, position {}, update {}, entity {}, arena info {:?}, own vehicle {}",
                    battle.calls.len(), battle.cell_player.is_some(), battle.space_properties.len(),
                    battle.geometry.as_ref().map(|g| &g.path), battle.vehicle_alias,
                    battle.vehicle_position.is_some(), battle.vehicle_update.is_some(), battle.vehicle_entity.is_some(),
                    battle.arena_info.as_ref().map(|a| (a.entity_id, a.nrl.len(), a.created.as_ref().map(|(t, p)| (*t, p.values.len())))),
                    battle.own_vehicle.is_some()),
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
            let account = reencode_account_data(&mut session, &vehicles)
                .map_err(|e| format!("Failed to re-encode the captured account data: {e}"))?;
            (Some(session), account)
        }
        None => (None, None),
    };
    let (session, account) = session;

    // The battle's map, for vehicles to rest on.
    let geometry = session.as_ref().and_then(|s| s.battle.as_ref()).and_then(|b| b.geometry.as_ref());
    let terrain = match geometry {
        Some(geometry) => {
            let start = Instant::now();
            let fs = ResFilesystem::new(game_dir.join("res"))
                .map_err(|e| format!("Failed to open resource filesystem: {e}"))?;
            let terrain = Terrain::load(&fs, &geometry.path)
                .map_err(|e| format!("Failed to load the terrain of {}: {e}", geometry.path))?;
            info!("Loaded the terrain of {} in {:.1?}", geometry.path, start.elapsed());
            Some(terrain)
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
        account,
        game_dir: game_dir.to_path_buf(),
        terrain,
        players: HashMap::new(),
        enqueued: HashMap::new(),
        pending_switches: HashMap::new(),
        battle_clients: HashMap::new(),
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
    /// The account's data as served, which the vehicles going to battle are read from.
    account: Option<AccountData>,
    /// The game, whose vehicle definitions complete what the account says.
    game_dir: PathBuf,
    /// The battle map's terrain.
    terrain: Option<Terrain>,
    /// Each client's player entity.
    players: HashMap<SocketAddr, base::Handle>,
    /// Clients waiting in the battle queue, with the inventory id of their vehicle.
    enqueued: HashMap<SocketAddr, i64>,
    /// Clients told to switch base app for their battle, by the session key they will
    /// log in again with, with the IP they will come from, their blowfish key and
    /// their vehicle.
    pending_switches: HashMap<u32, (IpAddr, Arc<Blowfish>, Option<VehicleConfig>)>,
    /// Clients connected for a battle, which get an `Avatar` rather than an `Account`.
    battle_clients: HashMap<SocketAddr, BattleClient>,
}

/// A client connected for a battle.
#[derive(Debug)]
struct BattleClient {
    /// Its vehicle's configuration, the captured vehicle's when `None`.
    config: Option<VehicleConfig>,
    /// When its game time started, at 0: the battle's clock, which arena periods end on.
    joined: Instant,
    /// The ticks sent to it, the first being tick 0.
    ticks: u32,
    /// The arena period to switch to, and when.
    next_period: Option<(Instant, u8)>,
    /// Its vehicle once created, moving as the client drives it.
    driven: Option<DrivenVehicle>,
    /// The enemy vehicle standing in front of it, to shoot at.
    target: Option<Target>,
}

/// An enemy vehicle the server places in front of the player, standing still: one of
/// the captured arena's, with its entry in `vehiclesInfo`.
#[derive(Debug, Clone)]
struct Target {
    entity_id: u32,
    /// Its `vehiclesInfo` entry's fields, which its `publicInfo` repeats.
    info: BTreeMap<Arc<str>, Value>,
    /// Where it stands and its yaw, once created.
    placed: Option<(Vec3, f32)>,
    health: i64,
}

/// A battle client's vehicle: the vehicle the server simulates, and how the client
/// knows it.
#[derive(Debug)]
struct DrivenVehicle {
    id_alias: u8,
    entity_type_id: u16,
    vehicle: Vehicle,
    /// The node of its `GunMarkerComponent`'s `gunMarker` property, once created.
    gun_marker_node: Option<u16>,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    entity_id: u32,
    gun: Gun,
    /// The nodes of its `OwnVehicle`'s `vehicleAmmoList` and `vehicleGunReloadTime`.
    ammo_node: Option<u16>,
    reload_node: Option<u16>,
    /// Updates to its NRL nodes, for the next tick.
    nrl_updates: Vec<NrlUpdateNode>,
    /// The shells in flight, with the game time they land at.
    shots: Vec<(f64, Shot)>,
}

impl BattleClient {

    /// The client's game time, in seconds.
    fn game_time(&self) -> f64 {
        self.joined.elapsed().as_secs_f64()
    }

}

/// The arena periods, `ARENA_PERIOD`.
const ARENA_PERIOD_PREBATTLE: u8 = 2;
const ARENA_PERIOD_BATTLE: u8 = 3;
/// The countdown before the battle, and the battle's length.
const PREBATTLE_LENGTH: Duration = Duration::from_secs(10);
const BATTLE_LENGTH: Duration = Duration::from_secs(900);
/// `ARENA_UPDATE.PERIOD`.
const ARENA_UPDATE_PERIOD: u8 = 3;

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
    fn start_battle(&mut self, addr: SocketAddr, vehicle_id: i64) -> io::Result<()> {

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
        let config = self.vehicle_config(vehicle_id)
            .inspect_err(|e| warn!(%addr, "Failed to read vehicle {vehicle_id}, the captured one is used: {e}"))
            .ok();
        self.pending_switches.insert(session_key, (addr.ip(), blowfish, config));
        self.app.switch_base_app(addr, base_addr, true)?;
        self.players.remove(&addr);
        info!(%addr, "Arena created, switching to {base_addr}");

        Ok(())

    }

    /// Switch a battle client's arena to a period: the countdown, then the battle. The
    /// update is `(period, end time, length, extra)`, pickled then compressed.
    fn arena_period(&mut self, addr: SocketAddr, period: u8) -> io::Result<()> {

        let (Some(client), Some(&handle)) = (self.battle_clients.get_mut(&addr), self.players.get(&addr)) else { return Ok(()) };
        let length = match period {
            ARENA_PERIOD_PREBATTLE => PREBATTLE_LENGTH,
            _ => BATTLE_LENGTH,
        };
        client.next_period = (period == ARENA_PERIOD_PREBATTLE).then(|| (Instant::now() + length, ARENA_PERIOD_BATTLE));
        let end = client.game_time() + length.as_secs_f64();

        let info = serde_pickle::Value::Tuple(vec![
            serde_pickle::Value::I64(period.into()),
            serde_pickle::Value::F64(end),
            serde_pickle::Value::I64(length.as_secs() as i64),
            serde_pickle::Value::List(Vec::new()),
        ]);
        let pickled = serde_pickle::value_to_vec(&info, serde_pickle_ser_options())
            .map_err(|e| invalid_data(&e.to_string()))?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&pickled)?;
        let data = encoder.finish()?;

        // As the captured server's own period update carries it.
        let like = self.session.as_ref().and_then(|s| s.battle.as_ref())
            .and_then(|b| b.calls.iter().find(|(method, args)| &**method == "updateArena" && args.first().and_then(Value::as_i64) == Some(ARENA_UPDATE_PERIOD.into())))
            .and_then(|(_, args)| args.get(1).cloned())
            .unwrap_or(Value::String(StringValue::Raw(Vec::new())));
        self.app.select_player_entity(handle)?;
        self.app.call_method(handle, "updateArena", vec![Value::UInt8(ARENA_UPDATE_PERIOD), with_bytes(&like, data)])?;
        info!(%addr, "Arena period {period} until {end:.1}");

        Ok(())

    }

    /// The configuration of the vehicle of the given inventory id: its descriptor, crew
    /// and setups from the account, the rest from the game's definitions.
    fn vehicle_config(&self, vehicle_id: i64) -> io::Result<VehicleConfig> {
        let account = self.account.as_ref().ok_or_else(|| invalid_data("no account data"))?;
        let descr = account.vehicle_descr(vehicle_id).ok_or_else(|| invalid_data("no such vehicle"))?.to_vec();
        let decoded = VehicleDescr::decode(&descr).map_err(|e| invalid_data(&e.to_string()))?;
        let fs = ResFilesystem::new(self.game_dir.join("res"))?;
        let def = VehicleDef::load_by_id(&fs, decoded.nation, decoded.type_id)?;
        info!("Battle vehicle {vehicle_id}: {}, {} health", def.name, def.max_health);
        Ok(VehicleConfig {
            descr,
            max_health: def.max_health,
            crew: account.crew_descrs(vehicle_id).into_iter().map(<[u8]>::to_vec).collect(),
            setups: account.vehicle_setups(vehicle_id),
            setup_indexes: account.vehicle_setup_indexes(vehicle_id),
            limits: MotionLimits {
                speed_forward: def.speed_forward,
                speed_backward: def.speed_backward,
                rotation_speed: def.rotation_speed,
            },
            ammo: def.default_ammo().into_iter().map(|(cd, count)| (cd.to_int(), count)).collect(),
            shots: def.shots,
            aiming: def.aiming,
        })
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

        // The battle's setup, as the captured server sent it: the arena, the avatar's
        // cell side in the space, the space and its map, then the player's vehicle
        // entering the area of interest.
        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        for (method, args) in &battle.calls {
            self.app.select_player_entity(handle)?;
            self.app.call_method(handle, method, args.clone())?;
        }

        // No `SetCellAppExtAddress`, which the captured server sends: given one, the
        // client opens a direct connection to the cell app and waits for a cell login we
        // can't answer yet, holding back its cell traffic. Without one, everything
        // comes through this base app.

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

        // An enemy to shoot at, created likewise once the client asks, see
        // `create_target`.
        let target = battle.arena_info.as_ref().and_then(|arena| enemy_of(&arena.nrl, cell.vehicle_id));
        if let Some(target) = &target {
            self.app.enter_aoi(addr, target.entity_id, TARGET_ALIAS)?;
            info!(%addr, "Target {} ({:?})", target.entity_id, target.info.get("name"));
        }
        if let Some(client) = self.battle_clients.get_mut(&addr) {
            client.target = target.clone();
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
            let config = self.battle_clients.get(&addr).and_then(|client| client.config.as_ref());
            let target_id = target.as_ref().map(|target| target.entity_id);
            let stream = arena_vehicles(self.app.dispatch(), &arena.nrl, cell.vehicle_id, target_id, config)?;
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

        // The vehicle belongs to our avatar, and is the one the client went to battle with.
        let config = self.battle_clients.get(&addr).and_then(|client| client.config.clone());
        let config = config.as_ref();
        let avatar_id = self.players.get(&addr).map(|handle| handle.entity_id());
        let mut entity = entity.clone();
        if let Some(properties) = &battle.vehicle_properties {
            let mut properties = properties.clone();
            for (_, name, value) in &mut properties.values {
                match (&**name, avatar_id) {
                    ("avatarID", Some(avatar_id)) => *value = with_int(value, avatar_id.into()),
                    _ => {}
                }
            }
            if let Some(config) = config {
                config.apply_to_entity(&mut properties);
            }
            let dispatch = self.app.dispatch().entity_from_id(entity.entity_type_id)
                .ok_or_else(|| invalid_data("unknown vehicle entity type"))?;
            entity.client_data = properties.encode(&dispatch.properties)?;
        }

        info!(%addr, "Creating vehicle {entity_id}");
        self.app.create_entity(addr, entity.clone())?;
        if let (Some(alias), Some(position)) = (battle.vehicle_alias, &battle.vehicle_position) {
            self.app.update_position_detailed(addr, alias, position.position, position.direction)?;
            // From there on it moves as the client drives it.
            if let (Some(client), Some(config)) = (self.battle_clients.get_mut(&addr), config) {
                client.driven = Some(DrivenVehicle {
                    id_alias: alias,
                    entity_type_id: entity.entity_type_id,
                    vehicle: Vehicle::new(position.position, position.direction, config.limits, config.aiming.clone()),
                    gun_marker_node: None,
                    engine_mode: None,
                    entity_id,
                    gun: Gun::new(config.shots.iter().cloned().zip(config.ammo.iter().map(|&(_, count)| count)).collect(), config.aiming.reload_time),
                    ammo_node: None,
                    reload_node: None,
                    nrl_updates: Vec::new(),
                    shots: Vec::new(),
                });
            }
        }
        if let Some(update) = &battle.vehicle_update {
            let mut update = update.clone();
            if let Some(config) = config {
                config.apply_to_update(&mut update);
            }
            self.app.update_entity(addr, entity_id, entity.entity_type_id, update)?;
        }

        // Its `OwnVehicle` component, under an entity node of its own: the client only
        // turns its turret once it knows how fast it can.
        // Its `GunMarkerComponent` too, beside it, for the client's server reticle: in
        // an element of its own, the client reading no further than the first component
        // of one.
        if let (Some(component), Some(config)) = (&battle.own_vehicle, config) {
            let stream = own_vehicle(self.app.dispatch(), component, entity_id, config)?;
            let entity_node = stream.messages[0].node_id;
            let ammo_node = property_node(self.app.dispatch(), &stream.messages[1], "vehicleAmmoList");
            let reload_node = property_node(self.app.dispatch(), &stream.messages[1], "vehicleGunReloadTime");
            if let Some(driven) = self.battle_clients.get_mut(&addr).and_then(|client| client.driven.as_mut()) {
                (driven.ammo_node, driven.reload_node) = (ammo_node, reload_node);
            }
            self.app.nrl_create_node(addr, stream)?;
            let (message, property_node) = gun_marker_component(self.app.dispatch(), entity_node)?;
            self.app.nrl_create_node(addr, NrlStream { messages: vec![message], rest: Vec::new() })?;
            if let Some(driven) = self.battle_clients.get_mut(&addr).and_then(|client| client.driven.as_mut()) {
                driven.gun_marker_node = Some(property_node);
            }
        }

        Ok(())

    }

    /// Create the target once the client asks for it, from the player's captured
    /// vehicle: its own id, descriptor, name, team and health, no avatar, then place it
    /// [`TARGET_DISTANCE`] in front of the player's start, facing it.
    fn create_target(&mut self, addr: SocketAddr) -> io::Result<()> {

        let Some(target) = self.battle_clients.get(&addr).and_then(|client| client.target.clone()) else { return Ok(()) };
        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let (Some(entity), Some(properties), Some(start)) = (&battle.vehicle_entity, &battle.vehicle_properties, &battle.vehicle_position) else {
            warn!(%addr, "The capture has no vehicle to make the target from");
            return Ok(());
        };

        let max_health = target.info.get("maxHealth").and_then(Value::as_i64).unwrap_or(1000);
        let mut properties = properties.clone();
        for (_, name, value) in &mut properties.values {
            match (&**name, &mut *value) {
                ("health", value) => *value = with_int(value, max_health),
                ("avatarID", value) => *value = with_int(value, 0),
                ("vehPostProgression", value) => *value = Value::Seq(Vec::new()),
                ("publicInfo", Value::Dict(public)) => {
                    for (field, value) in public.iter_mut() {
                        match &**field {
                            "maxHealth" => *value = with_int(value, max_health),
                            // The player's camouflage and marks are not the target's.
                            "outfit" => *value = with_bytes(value, Vec::new()),
                            "marksOnGun" => *value = with_int(value, 0),
                            "compDescr" | "name" | "team" => if let Some(own) = target.info.get(field) {
                                *value = match (own, &*value) {
                                    (Value::Int8(n), _) => with_int(value, (*n).into()),
                                    _ => own.clone(),
                                };
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let dispatch = self.app.dispatch().entity_from_id(entity.entity_type_id)
            .ok_or_else(|| invalid_data("unknown vehicle entity type"))?;
        let yaw = start.direction.z;
        let mut position = start.position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * TARGET_DISTANCE;
        if let Some(height) = self.terrain.as_ref().and_then(|terrain| terrain.height_at(position.x, position.z)) {
            position.y = height;
        }
        // Facing the player's start, in [-pi, pi) as angles are packed.
        let facing = (yaw + 2.0 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let direction = Vec3::new(0.0, 0.0, facing);

        // The client places it where its creation says, not where the position update
        // that follows does.
        let mut entity = entity.clone();
        entity.entity_id = target.entity_id;
        entity.client_data = properties.encode(&dispatch.properties)?;
        entity.position = position;
        entity.direction = wgtk::app::math::PackedYawPitchRoll::pack(direction.z, direction.y, direction.x, false);

        info!(%addr, "Creating target {} at {position:?}", target.entity_id);
        if let Some(target) = self.battle_clients.get_mut(&addr).and_then(|client| client.target.as_mut()) {
            target.placed = Some((position, direction.z));
        }
        self.app.create_entity(addr, entity)?;
        self.app.update_position_detailed(addr, TARGET_ALIAS, position, direction)?;
        Ok(())

    }

    /// Fire the client's gun, if loaded: the shell's tracer and the gun's effects, then
    /// one shell fewer and the reload. Where the shell lands follows with
    /// [`Self::land_shot`].
    fn shoot(&mut self, addr: SocketAddr) -> io::Result<()> {

        let Some(&avatar) = self.players.get(&addr) else { return Ok(()) };
        let Some(client) = self.battle_clients.get_mut(&addr) else { return Ok(()) };
        let now = client.game_time();
        let vehicles: Vec<_> = client.target.iter()
            .filter(|target| target.health > 0)
            .filter_map(|target| target.placed.map(|(position, yaw)| VehicleBox {
                entity_id: target.entity_id,
                center: position + Vec3::Y * TARGET_HALF_EXTENTS.y,
                yaw,
                half_extents: TARGET_HALF_EXTENTS,
            }))
            .collect();
        let Some(driven) = client.driven.as_mut() else { return Ok(()) };
        let (position, velocity, dispersion) = driven.vehicle.turret.gun_marker(&driven.vehicle.hull);
        let Some(shot) = driven.gun.fire(now, position, velocity, dispersion, self.terrain.as_ref(), &vehicles) else {
            debug!(%addr, "Not loaded, not shooting");
            return Ok(());
        };
        info!(%addr, "Shot {} from {:?} lands at {:?} in {:.2} s", shot.id, shot.start, shot.end, shot.flight_time);
        driven.shots.push((now + f64::from(shot.flight_time), shot.clone()));

        let dispatch = self.app.dispatch();
        let ammo = ammo_list(driven.gun.ammo.iter().map(|(def, count)| (def.compact_descr.to_int(), *count)));
        let reload = gun_reload_time(driven.gun.reload_time(), driven.gun.ready_at as f32, driven.gun.reload_time());
        for (node, name, value) in [(driven.ammo_node, "vehicleAmmoList", ammo), (driven.reload_node, "vehicleGunReloadTime", reload)] {
            if let Some(node) = node {
                driven.nrl_updates.push(property_update(dispatch, "OwnVehicle", name, node, &value)?);
            }
        }
        let (vehicle_id, vehicle_type_id) = (driven.entity_id, driven.entity_type_id);

        let def = &shot.def;
        self.app.call_method(avatar, "showTracer", vec![
            Value::Int32(vehicle_id as i32),
            Value::Int32(shot.id),
            Value::UInt8(0),
            Value::UInt8(def.effects_index),
            Value::UInt8(NO_PREFAB_EFFECTS),
            Value::UInt8(def.kind_index),
            Value::Float32(def.caliber),
            Value::Vector3(shot.start),
            Value::Vector3(shot.velocity),
            Value::Float32(def.gravity),
            Value::Float32(def.max_distance),
            Value::UInt8(0),
            Value::UInt8(0),
        ])?;
        // One shell from the only gun.
        self.app.call_entity_method(addr, vehicle_id, vehicle_type_id, "showShooting", vec![
            Value::UInt8(1),
            Value::Int8(0),
            Value::UInt8(def.kind_index),
        ])?;

        Ok(())

    }

    /// A shell gets to its end: it damages the vehicle it hits, explodes on the
    /// terrain, or its tracer stops at its maximum distance.
    fn land_shot(&mut self, addr: SocketAddr, shot: &Shot) -> io::Result<()> {
        let Some(&avatar) = self.players.get(&addr) else { return Ok(()) };
        let def = &shot.def;
        match shot.landing {
            Landing::Vehicle(entity_id) => {
                self.app.call_method(avatar, "stopTracer", vec![Value::Int32(shot.id), Value::Vector3(shot.end)])?;
                self.damage_target(addr, entity_id, def.damage)
            }
            Landing::Terrain => self.app.call_method(avatar, "explodeProjectile", vec![
                Value::Int32(shot.id),
                Value::UInt8(def.effects_index),
                Value::UInt8(NO_PREFAB_EFFECTS),
                Value::UInt8(GROUND_EFFECT_MATERIAL),
                Value::UInt8(def.kind_index),
                Value::Float32(def.caliber),
                Value::Vector3(shot.end),
                Value::Vector3(shot.end_velocity.normalize()),
                Value::Float32(shot.end_velocity.length()),
                Value::Seq(Vec::new()),
            ]),
            Landing::Range => self.app.call_method(avatar, "stopTracer", vec![Value::Int32(shot.id), Value::Vector3(shot.end)]),
        }
    }

    /// The target takes a shell's damage, varied by up to [`DAMAGE_SPREAD`] either way:
    /// its health drops, and the player sees the hit, as the captured server shows one.
    fn damage_target(&mut self, addr: SocketAddr, entity_id: u32, damage: f32) -> io::Result<()> {

        use rand::Rng;

        let Some(&avatar) = self.players.get(&addr) else { return Ok(()) };
        let Some(client) = self.battle_clients.get_mut(&addr) else { return Ok(()) };
        let shooter_id = client.driven.as_ref().map_or(0, |driven| driven.entity_id);
        let Some(target) = client.target.as_mut().filter(|target| target.entity_id == entity_id && target.health > 0) else {
            return Ok(());
        };
        let damage = (damage * rand::thread_rng().gen_range(1.0 - DAMAGE_SPREAD..=1.0 + DAMAGE_SPREAD)).round() as i64;
        let old_health = target.health;
        target.health = (old_health - damage).max(0);
        let new_health = target.health;
        let killed = new_health == 0;
        info!(%addr, "Target {entity_id} takes {damage} damage, {old_health} -> {new_health}{}", if killed { ", destroyed" } else { "" });

        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let (Some(entity), Some(properties)) = (&battle.vehicle_entity, &battle.vehicle_properties) else { return Ok(()) };
        let type_id = entity.entity_type_id;
        let health = properties.values.iter()
            .find(|(_, name, _)| &**name == "health")
            .map(|(id, name, value)| (*id, name.clone(), with_int(value, new_health)));

        let mut flags = HIT_FLAGS_PIERCED;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        self.app.call_method(avatar, "showShotResults", vec![Value::Seq(vec![Value::Dict(BTreeMap::from([
            (Arc::from("gunInstallationIndex"), Value::UInt8(0)),
            (Arc::from("hitFlags"), Value::UInt32(flags)),
            (Arc::from("vehicleID"), Value::UInt32(entity_id)),
        ]))])])?;
        if let Some(health) = health {
            self.app.update_entity(addr, entity_id, type_id, ChangedProperties { values: vec![health], stopped: None })?;
        }
        // `ATTACK_REASON.SHOT`, no extended reason.
        self.app.call_entity_method(addr, entity_id, type_id, "onHealthChanged", vec![
            Value::Int16(new_health as i16),
            Value::Int16(old_health as i16),
            Value::Int32(shooter_id as i32),
            Value::UInt8(0),
            Value::Int8(-1),
        ])?;
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

        // Battle clients need to hear from the server every tick, 10 Hz as captured, and
        // regularly: the client times what it hears by the tick it comes with.
        const TICK: Duration = Duration::from_millis(100);
        let mut next_tick = Instant::now() + TICK;

        loop {

            // Every tick due, late ones included, so that no tick number is skipped.
            while Instant::now() >= next_tick {
                next_tick += TICK;
                let avatar_type_id = self.app.dispatch().entity_from_name("Avatar").map(|(type_id, _)| type_id);
                let gun_marker_ty = self.app.dispatch().entity_from_name("GunMarkerComponent")
                    .and_then(|(_, component)| component.properties.first())
                    .map(|property| property.ty.clone());
                for (&addr, client) in &mut self.battle_clients {
                    let tick = client.ticks as u8;
                    client.ticks = client.ticks.wrapping_add(1);
                    let now = client.game_time();
                    let vehicle = client.driven.as_mut().map(|driven| {
                        let DrivenVehicle { id_alias, entity_type_id, vehicle, gun_marker_node, engine_mode, gun, reload_node, nrl_updates: pending, .. } = driven;
                        vehicle.step(TICK.as_secs_f32(), self.terrain.as_ref());
                        let Vehicle { hull, turret } = vehicle;
                        let gun_marker = gun_marker_node.zip(gun_marker_ty.as_ref())
                            .map(|(node, ty)| gun_marker_update(ty, node, turret.gun_marker(hull)));
                        let mut nrl_updates = std::mem::take(pending);
                        if let (Some(node), true) = (*reload_node, gun.take_loaded(now)) {
                            let loaded = gun_reload_time(gun.reload_time(), 0.0, 0.0);
                            match property_update(self.app.dispatch(), "OwnVehicle", "vehicleGunReloadTime", node, &loaded) {
                                Ok(update) => nrl_updates.push(update),
                                Err(e) => warn!(%addr, "Failed to encode the reload: {e}"),
                            }
                        }
                        match gun_marker {
                            Some(Ok(update)) => nrl_updates.push(update),
                            Some(Err(e)) => warn!(%addr, "Failed to encode the gun marker: {e}"),
                            None => {}
                        }
                        // Every tick, to compare with what the client simulates.
                        debug!(%addr, "Vehicle at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
                            hull.position, hull.direction, hull.speed, turret.yaw, turret.pitch);
                        let mut properties = vec![EntityProperty { name: "gunAnglesPacked".into(), value: Value::UInt16(turret.gun_angles_packed()) }];
                        let mode = hull.engine_mode();
                        if *engine_mode != Some(mode) {
                            *engine_mode = Some(mode);
                            properties.push(EntityProperty {
                                name: "engineMode".into(),
                                value: Value::Seq(mode.iter().map(|&n| Value::UInt8(n)).collect()),
                            });
                        }
                        base::TickVehicle {
                            id_alias: *id_alias,
                            position: hull.position,
                            direction: hull.direction,
                            properties: Some((*entity_type_id, properties)),
                            // What the client aims from, so it must turn with the hull.
                            player_properties: avatar_type_id.map(|type_id| (type_id, vec![EntityProperty {
                                name: "ownVehicleAuxPhysicsData".into(),
                                value: Value::UInt64(hull.aux_physics_data()),
                            }])),
                            nrl_updates,
                        }
                    });
                    if let Err(e) = self.app.tick(addr, tick, vehicle.as_ref()) {
                        warn!(%addr, "Failed to tick: {e}");
                    }
                }
                let landed: Vec<_> = self.battle_clients.iter_mut()
                    .filter_map(|(&addr, client)| {
                        let now = client.game_time();
                        let shots = &mut client.driven.as_mut()?.shots;
                        let (landed, flying) = std::mem::take(shots).into_iter().partition(|&(at, _)| at <= now);
                        *shots = flying;
                        Some((addr, landed))
                    })
                    .collect();
                for (addr, landed) in landed {
                    for (_, shot) in landed {
                        if let Err(e) = self.land_shot(addr, &shot) {
                            warn!(%addr, "Failed to land shot {}: {e}", shot.id);
                        }
                    }
                }
                let due: Vec<_> = self.battle_clients.iter()
                    .filter_map(|(&addr, client)| client.next_period.filter(|&(at, _)| at <= Instant::now()).map(|(_, period)| (addr, period)))
                    .collect();
                for (addr, period) in due {
                    if let Err(e) = self.arena_period(addr, period) {
                        warn!(%addr, "Failed to change the arena period: {e}");
                    }
                }
            }

            // Wait for packets until the next tick is due, no longer.
            let wait = next_tick.saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
            if let Err(e) = self.app.set_poll_timeout(Some(wait)) {
                warn!("Failed to set the poll timeout: {e}");
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
                    if let Some((ip, blowfish, config)) = self.pending_switches.remove(&login.login_key) {
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
                        self.battle_clients.insert(login.addr, BattleClient { config, joined: Instant::now(), ticks: 0, next_period: None, driven: None, target: None });
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
                    if self.battle_clients.contains_key(&event.addr) {
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
                            if &**name == "setClientReady" {
                                if let Err(e) = self.arena_period(event.addr, ARENA_PERIOD_PREBATTLE) {
                                    warn!(addr = %event.addr, "Failed to start the countdown: {e}");
                                }
                            }
                            if let Err(e) = self.answer(event.addr, name, args) {
                                warn!(addr = %event.addr, "Failed to answer {name}: {e}");
                            }
                            // The arena is ready as soon as the client first asks how the
                            // queue is going.
                            let command = args.get(1).and_then(Value::as_i64);
                            if command == Some(CMD_ENQUEUE_IN_BATTLE_QUEUE.into()) {
                                // `[queue type, vehicle inventory id, ...]`.
                                let vehicle_id = match args.get(2) {
                                    Some(Value::Seq(values)) => values.get(1).and_then(Value::as_i64),
                                    _ => None,
                                };
                                self.enqueued.insert(event.addr, vehicle_id.unwrap_or(0));
                            } else if let Some(vehicle_id) = (command == Some(CMD_REQ_QUEUE_INFO.into()))
                                .then(|| self.enqueued.remove(&event.addr)).flatten() {
                                if let Err(e) = self.start_battle(event.addr, vehicle_id) {
                                    warn!(addr = %event.addr, "Failed to start the battle: {e}");
                                }
                            }
                        }
                        MethodCall::Unknown { .. } =>
                            warn!(addr = %event.addr, "Base method call on entity {}: {:?}", event.entity_id, event.call),
                    }
                }
                base::Event::CellMethod(event) => {
                    match &event.call {
                        MethodCall::Known { name, .. } if &**name == "vehicle_shoot" => {
                            if let Err(e) = self.shoot(event.addr) {
                                warn!(addr = %event.addr, "Failed to shoot: {e}");
                            }
                        }
                        MethodCall::Known { name, args } if &**name == "vehicle_moveWith" => {
                            let flags = args.first().and_then(Value::as_i64).unwrap_or(0) as u8;
                            match self.battle_clients.get_mut(&event.addr).and_then(|c| c.driven.as_mut()) {
                                Some(driven) => {
                                    info!(addr = %event.addr, "Moving with flags {flags:#04x}");
                                    driven.vehicle.hull.flags = flags;
                                }
                                None => warn!(addr = %event.addr, "Moving with flags {flags:#04x}, but the vehicle can't move"),
                            }
                        }
                        MethodCall::Known { name, args } if matches!(&**name, "trackRelativePointWithGun" | "stopTrackingWithGun" | "vehicle_stopTrackingWithGun") => {
                            if let Some(driven) = self.battle_clients.get_mut(&event.addr).and_then(|c| c.driven.as_mut()) {
                                match (&**name, &args[..]) {
                                    ("trackRelativePointWithGun", [Value::Vector3(point)]) => {
                                        let Vehicle { hull, turret } = &mut driven.vehicle;
                                        debug!(addr = %event.addr, "Tracking {point:?} from {:?}", hull.position);
                                        turret.track(hull.position + *point);
                                    }
                                    // Where our turret is rather than the client's angles,
                                    // which its own rotator computes slightly apart: the
                                    // gun would move as it locks.
                                    (_, [Value::Float32(yaw), Value::Float32(pitch)]) => {
                                        let turret = &mut driven.vehicle.turret;
                                        debug!(addr = %event.addr, "Holding the gun at yaw {:.4}, pitch {:.4} (client: {yaw:.4}, {pitch:.4})",
                                            turret.yaw, turret.pitch);
                                        turret.hold(turret.yaw, turret.pitch);
                                    }
                                    _ => warn!(addr = %event.addr, "Unexpected {name} arguments: {args:?}"),
                                }
                            }
                        }
                        MethodCall::Known { name, .. } => debug!(addr = %event.addr, "Cell method call on entity {}: {name}", event.entity_id),
                        MethodCall::Unknown { .. } => debug!(addr = %event.addr, "Cell method call on entity {}: {:?}", event.entity_id, event.call),
                    }
                }
                base::Event::AvatarUpdate(event) => {
                    debug!(addr = %event.addr, "Avatar update on {:?}: {:?}, direction {:?}", event.vehicle_id, event.position, event.direction);
                }
                base::Event::RequestEntityUpdate(event) => {
                    let is_target = self.battle_clients.get(&event.addr)
                        .and_then(|client| client.target.as_ref())
                        .is_some_and(|target| target.entity_id == event.entity_id);
                    if is_target {
                        if let Err(e) = self.create_target(event.addr) {
                            warn!(addr = %event.addr, "Failed to create the target: {e}");
                        }
                    } else if let Err(e) = self.create_vehicle(event.addr, event.entity_id) {
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
/// the same, give it the vehicles asked for, and encode it back. Returns the account
/// data served.
fn reencode_account_data(session: &mut CapturedSession, give_vehicles: &[VehicleDef]) -> io::Result<Option<AccountData>> {

    let mut served = None;
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
            served = Some(account);

        }

    }

    Ok(served)

}

/// The configuration of the vehicle a client went to battle with: what the battle tells
/// about it, and what the server simulates it with.
#[derive(Debug, Clone)]
struct VehicleConfig {
    descr: Vec<u8>,
    max_health: u32,
    /// Its crew's compact descriptors, in crew order.
    crew: Vec<Vec<u8>>,
    /// Its loadouts and the selected ones, see `AccountData::vehicle_setups`.
    setups: serde_pickle::Value,
    setup_indexes: serde_pickle::Value,
    limits: MotionLimits,
    aiming: AimingDef,
    /// Its shells, by integer descriptor, and how many of each.
    ammo: Vec<(i64, u32)>,
    /// How its gun fires each of its shells, in the order of `ammo`.
    shots: Vec<ShotDef>,
}

impl VehicleConfig {

    /// Its entry in the arena's `vehiclesInfo`.
    fn apply_to_arena_entry(&self, fields: &mut BTreeMap<Arc<str>, Value>) {
        for (name, value) in fields.iter_mut() {
            match &**name {
                "compDescr" => *value = with_bytes(value, self.descr.clone()),
                "maxHealth" => *value = with_int(value, self.max_health.into()),
                "vehPostProgression" => *value = Value::Seq(Vec::new()),
                _ => {}
            }
        }
    }

    /// Its vehicle entity's creation properties.
    fn apply_to_entity(&self, properties: &mut ChangedProperties) {
        for (_, name, value) in &mut properties.values {
            match (&**name, &mut *value) {
                ("health", value) => *value = with_int(value, self.max_health.into()),
                ("publicInfo", Value::Dict(info)) => {
                    for (name, value) in info.iter_mut() {
                        match &**name {
                            "compDescr" => *value = with_bytes(value, self.descr.clone()),
                            "maxHealth" => *value = with_int(value, self.max_health.into()),
                            _ => {}
                        }
                    }
                }
                ("vehPostProgression", value) => *value = Value::Seq(Vec::new()),
                _ => {}
            }
        }
    }

    /// The properties its vehicle entity gets once created.
    fn apply_to_update(&self, update: &mut ChangedProperties) {
        for (_, name, value) in &mut update.values {
            match (&**name, &mut *value) {
                ("crewCompactDescrs", Value::Seq(crew)) => {
                    let like = crew.first().cloned().unwrap_or(Value::String(StringValue::Raw(Vec::new())));
                    *crew = self.crew.iter().map(|cd| with_bytes(&like, cd.clone())).collect();
                }
                ("setups", value) => *value = with_python(value, self.setups.clone()),
                ("setupsIndexes", value) => *value = with_python(value, self.setup_indexes.clone()),
                _ => {}
            }
        }
    }

}

/// The same kind of byte string as `value`, holding `bytes`.
fn with_bytes(value: &Value, bytes: Vec<u8>) -> Value {
    match value {
        Value::Python(_) => Value::Python(PythonValue::Raw(bytes)),
        _ => Value::String(StringValue::Raw(bytes)),
    }
}

/// The same kind of Python value as `value`, holding `python`.
fn with_python(value: &Value, python: serde_pickle::Value) -> Value {
    match value {
        Value::String(_) => Value::String(StringValue::Python(python)),
        _ => Value::Python(PythonValue::Decoded(python)),
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// The arena info entity's NRL tree, its `vehiclesInfo` reduced to the given vehicle's
/// and the target's, encoded again.
fn arena_vehicles(dispatch: &ScriptDispatch, captured: &[NrlMessage], vehicle_id: u32, target_id: Option<u32>, config: Option<&VehicleConfig>) -> io::Result<NrlStream> {
    let mut messages = captured.to_vec();
    for message in &mut messages {
        let NrlBody::CreateNode { node: NrlNode::DynComponent { component_type_id, data: Some(data), .. }, .. } = &mut message.body else {
            continue;
        };
        let children = data.children.iter().cloned().map(|(id, child)| match child {
            NrlChild::Property { name, value: Value::Seq(entries) } if &*name == "vehiclesInfo" => {
                let entries = entries.into_iter().filter_map(|entry| match entry {
                    Value::Dict(mut fields) if fields.get("vehicleID").and_then(Value::as_i64) == Some(vehicle_id.into()) => {
                        if let Some(config) = config {
                            config.apply_to_arena_entry(&mut fields);
                        }
                        Some(Value::Dict(fields))
                    }
                    Value::Dict(fields) if target_id.is_some() && fields.get("vehicleID").and_then(Value::as_i64) == target_id.map(i64::from) => {
                        Some(Value::Dict(fields))
                    }
                    _ => None,
                }).collect();
                (id, NrlChild::Property { name, value: Value::Seq(entries) })
            }
            other => (id, other),
        }).collect();
        *data = NrlComponentData::encode(dispatch, *component_type_id, children)?;
    }
    Ok(NrlStream { messages, rest: Vec::new() })
}

/// The first vehicle of the captured arena not in the given vehicle's team, as a
/// target.
fn enemy_of(arena: &[NrlMessage], vehicle_id: u32) -> Option<Target> {
    let entries = arena.iter().find_map(|message| match &message.body {
        NrlBody::CreateNode { node: NrlNode::DynComponent { data: Some(data), .. }, .. } => data.children.iter().find_map(|(_, child)| match child {
            NrlChild::Property { name, value: Value::Seq(entries) } if &**name == "vehiclesInfo" => Some(entries),
            _ => None,
        }),
        _ => None,
    })?;
    let fields = |entry: &Value| match entry {
        Value::Dict(fields) => Some(fields.clone()),
        _ => None,
    };
    let team = |fields: &BTreeMap<Arc<str>, Value>| fields.get("team").and_then(Value::as_i64);
    let own_team = entries.iter().filter_map(fields)
        .find(|fields| fields.get("vehicleID").and_then(Value::as_i64) == Some(vehicle_id.into()))
        .and_then(|fields| team(&fields))?;
    entries.iter().filter_map(fields)
        .find(|fields| team(fields).is_some_and(|team| team != own_team))
        .and_then(|info| Some(Target {
            entity_id: info.get("vehicleID")?.as_i64()? as u32,
            health: info.get("maxHealth")?.as_i64()?,
            info,
            placed: None,
        }))
}

/// The target's id alias, apart from the one the captured server gave the player's
/// vehicle.
const TARGET_ALIAS: u8 = 200;

/// Half the target's width, height and length: a typical tank's hull, in metres.
const TARGET_HALF_EXTENTS: Vec3 = Vec3::new(1.7, 1.2, 3.5);

/// How much a shell's damage varies, either way, as a share of it.
const DAMAGE_SPREAD: f32 = 0.25;

/// `VEHICLE_HIT_FLAGS` of a shell piercing a vehicle, as captured: a direct projectile,
/// through armour and a module.
const HIT_FLAGS_PIERCED: u32 = 0x100000 | 0x100 | 0x10;
/// `VEHICLE_HIT_FLAGS.VEHICLE_KILLED`.
const HIT_FLAG_VEHICLE_KILLED: u32 = 1;

/// How far in front of the player's start the target stands, in metres.
const TARGET_DISTANCE: f32 = 40.0;

/// The NRL node every entity node goes under.
const NRL_ROOT_NODE: u16 = 1;

/// The vehicle's entity node, then its captured `OwnVehicle` component with the
/// vehicle's own `targetingInfo`: turret and gun at rest, their speeds and dispersion
/// factors from its definitions, without the crew's and equipment's bonuses.
fn own_vehicle(dispatch: &ScriptDispatch, captured: &NrlMessage, entity_id: u32, config: &VehicleConfig) -> io::Result<NrlStream> {

    let NrlBody::CreateNode { parent_id, node: NrlNode::DynComponent { component_type_id, data: Some(data), .. } } = &captured.body else {
        return Err(invalid_data("the captured OwnVehicle message creates no component"));
    };
    let entity = NrlMessage {
        node_id: *parent_id,
        body: NrlBody::CreateNode { parent_id: NRL_ROOT_NODE, node: NrlNode::Entity { entity_id } },
    };

    let aiming = &config.aiming;
    let mut children: Vec<_> = data.children.iter().cloned().map(|(id, child)| match child {
        NrlChild::Property { name, value: Value::Dict(mut fields) } if &*name == "targetingInfo" => {
            for (field, value) in fields.iter_mut() {
                let float = match &**field {
                    "turretYaw" | "gunPitch" => 0.0,
                    "maxTurretRotationSpeed" => aiming.turret_rotation_speed,
                    "maxGunRotationSpeed" => aiming.gun_rotation_speed,
                    "shotDispMultiplierFactor" => 1.0,
                    "gunShotDispersionFactorsTurretRotation" => aiming.dispersion_turret_rotation,
                    "chassisShotDispersionFactorsMovement" => aiming.dispersion_movement,
                    "chassisShotDispersionFactorsRotation" => aiming.dispersion_rotation,
                    "gunShotDispersionFactorsAfterShot" => aiming.dispersion_after_shot,
                    "aimingTime" => aiming.aiming_time,
                    _ => continue,
                };
                *value = Value::Float32(float);
            }
            (id, NrlChild::Property { name, value: Value::Dict(fields) })
        }
        other => (id, other),
    }).collect();

    // What the real server sends right after, as updates: the ammunition, the shells
    // loaded, the reload and the health, given here as the properties' first values.
    let NrlBody::CreateNode { node: NrlNode::DynComponent { children: ids, .. }, .. } = &captured.body else {
        unreachable!()
    };
    let (_, schema) = dispatch.component_from_nrl_id(*component_type_id)
        .ok_or_else(|| invalid_data("unknown OwnVehicle component type"))?;
    let mut present = ids.present.clone().unwrap_or_else(|| vec![0xFF; (ids.ids.len() + 7) / 8]);
    for (name, value) in own_vehicle_state(entity_id, config) {
        let Some(index) = schema.properties.iter().position(|property| &*property.name == name) else { continue };
        let id = ids.ids[index];
        present[index / 8] |= 1 << (index % 8);
        children.retain(|&(child_id, _)| child_id != id);
        children.push((id, NrlChild::Property { name: Arc::from(name), value }));
    }
    // In the order of the ids, as the client reads them.
    children.sort_by_key(|&(id, _)| ids.ids.iter().position(|&i| i == id));
    let values = children;
    let ids = NrlIdList::new(captured.node_id.into(), ids.ids.clone(), Some(present));

    let mut component = captured.clone();
    if let NrlBody::CreateNode { node: NrlNode::DynComponent { children, data, .. }, .. } = &mut component.body {
        *data = Some(NrlComponentData::encode(dispatch, *component_type_id, values)?);
        *children = ids;
    }

    Ok(NrlStream { messages: vec![entity, component], rest: Vec::new() })

}

/// The `OwnVehicle` properties the real server fills in after creating the component,
/// for a vehicle just entering the battle: its shells, then the standard consumables of
/// the captured battle (repair kit, first aid kit, fire extinguisher), the first shell
/// loaded and next, the gun to reload and full health.
fn own_vehicle_state(entity_id: u32, config: &VehicleConfig) -> Vec<(&'static str, Value)> {

    let shell = config.ammo.first().map_or(0, |&(cd, _)| cd as i32);
    let setting = |code: u8| Value::Dict(BTreeMap::from([
        (Arc::from("code"), Value::UInt8(code)),
        (Arc::from("value"), Value::Int32(shell)),
        (Arc::from("vehicleID"), Value::Int32(entity_id as i32)),
    ]));

    vec![
        ("vehicleAmmoList", ammo_list(config.ammo.iter().copied())),
        // `VEHICLE_SETTING`: the current shell, then the next one.
        ("vehicleSettings", Value::Seq(vec![setting(0), setting(1)])),
        ("vehicleGunReloadTime", gun_reload_time(config.aiming.reload_time, 0.0, 0.0)),
        ("vehicleHealthInfo", Value::Dict(BTreeMap::from([
            (Arc::from("deathReasonExtID"), Value::Int8(-1)),
            (Arc::from("deathReasonID"), Value::Int8(-1)),
            (Arc::from("health"), Value::Int16(config.max_health as i16)),
            (Arc::from("id"), Value::Int32(entity_id as i32)),
            (Arc::from("isCrewActive"), Value::UInt8(1)),
            (Arc::from("isRespawnActive"), Value::UInt8(0)),
        ]))),
    ]

}

/// The `vehicleAmmoList` of a vehicle with these shells, by integer descriptor and
/// count, and the standard consumables.
fn ammo_list(shells: impl Iterator<Item = (i64, u32)>) -> Value {
    let ammo = |compact_descr: i64, quantity: u16, in_clip: u16, stage: u16, total_time: i16| Value::Dict(BTreeMap::from([
        (Arc::from("compactDescr"), Value::Int32(compact_descr as i32)),
        (Arc::from("endTime"), Value::Float32(0.0)),
        (Arc::from("index"), Value::UInt8(0)),
        (Arc::from("previousStage"), Value::UInt16(stage)),
        (Arc::from("quantity"), Value::UInt16(quantity)),
        (Arc::from("quantityInClip"), Value::UInt16(in_clip)),
        (Arc::from("totalTime"), Value::Int16(total_time)),
    ]));
    Value::Seq(shells
        .map(|(cd, count)| ammo(cd, count as u16, 0, 0, 0))
        .chain(CONSUMABLES.iter().map(|&cd| ammo(cd, u16::MAX, 3, 3, 60)))
        .collect())
}

/// The `vehicleGunReloadTime` of a gun reloading in `base_time` seconds, done at game
/// time `end_time` with `time_left` seconds to go: both 0 once loaded, -1 before the
/// battle.
fn gun_reload_time(base_time: f32, end_time: f32, time_left: f32) -> Value {
    Value::Dict(BTreeMap::from([
        (Arc::from("baseTime"), Value::Float32(base_time)),
        (Arc::from("clipTime"), Value::Float32(0.0)),
        (Arc::from("endTime"), Value::Float32(end_time)),
        (Arc::from("timeLeft"), Value::Float32(time_left)),
    ]))
}

/// The node of a property of the component a message creates.
fn property_node(dispatch: &ScriptDispatch, message: &NrlMessage, name: &str) -> Option<u16> {
    let NrlBody::CreateNode { node: NrlNode::DynComponent { component_type_id, children, .. }, .. } = &message.body else {
        return None;
    };
    let (_, schema) = dispatch.component_from_nrl_id(*component_type_id)?;
    let index = schema.properties.iter().position(|property| &*property.name == name)?;
    children.ids.get(index).map(|&id| id as u16)
}

/// Set a property of an `OwnVehicle`, as the real server does: `[0][packed length]`
/// then the value, see `NrlTree::decode_update`.
fn property_update(dispatch: &ScriptDispatch, component: &str, name: &str, node_id: u16, value: &Value) -> io::Result<NrlUpdateNode> {
    use wgtk::net::codec::Codec;
    use wgtk::util::io::WgWriteExt;
    let (_, schema) = dispatch.entity_from_name(component)
        .ok_or_else(|| invalid_data(&format!("no {component}")))?;
    let property = schema.properties.iter().find(|property| &*property.name == name)
        .ok_or_else(|| invalid_data(&format!("{component} has no {name}")))?;
    let mut data = Vec::new();
    value.write(&mut data, &property.ty)?;
    let mut payload = vec![0];
    payload.write_packed_u24(data.len() as u32)?;
    payload.extend_from_slice(&data);
    Ok(NrlUpdateNode { node_id, payload, python_value: None })
}

/// `INVALID_EFFECT_INDEX`: no prefab effects for a shot, the client then only playing
/// its shot effects.
const NO_PREFAB_EFFECTS: u8 = 255;

/// The effect material the captured server gave shells exploding on the ground.
const GROUND_EFFECT_MATERIAL: u8 = 2;

/// The consumables of the captured battle, by integer descriptor, which every vehicle
/// gets until accounts have their own.
const CONSUMABLES: [i64; 3] = [1531, 1019, 507];

/// The NRL node of the vehicle's `GunMarkerComponent`, its children following: above
/// the ids the captured trees use, up to about 2200.
const GUN_MARKER_NODE: u16 = 0x2000;

/// Create the vehicle's `GunMarkerComponent` under its entity node, its `gunMarker`
/// None: its children are that property then the trailing child, both present, as
/// the components whose properties the real server updates start. The client has no
/// node for an absent property, and misreads an update to it. Returns the message and
/// the property's node, which updates go to.
fn gun_marker_component(dispatch: &ScriptDispatch, entity_node: u16) -> io::Result<(NrlMessage, u16)> {
    let (type_id, _) = dispatch.entity_from_name("GunMarkerComponent")
        .ok_or_else(|| invalid_data("no GunMarkerComponent"))?;
    let nrl_type_id = dispatch.component_nrl_id(type_id)
        .ok_or_else(|| invalid_data("GunMarkerComponent is not a network component"))?;
    let base = u32::from(GUN_MARKER_NODE);
    let (property, trailing) = (base + 1, base + 2);
    let message = NrlMessage {
        node_id: GUN_MARKER_NODE,
        body: NrlBody::CreateNode {
            parent_id: entity_node,
            node: NrlNode::DynComponent {
                component_type_id: nrl_type_id,
                name: String::new(),
                children: NrlIdList::new(base, vec![property, trailing], Some(vec![0b11])),
                data: Some(NrlComponentData::encode(dispatch, nrl_type_id, vec![
                    (property, NrlChild::Property { name: Arc::from("gunMarker"), value: Value::None }),
                    (trailing, NrlChild::Trailing),
                ])?),
            },
        },
    };
    Ok((message, property as u16))
}

/// Set the `gunMarker` property at the given node: where the gun fires from, the
/// shell's velocity and the dispersion angle. Unlike the properties the captured server
/// updates, `[kind][packed length][value]` (see `NrlTree::decode_update`), this one is
/// the value alone: its `ClientCompositePropertyNode::update` has the node's flag at
/// `+0x5c` set and streams the whole value instead.
fn gun_marker_update(ty: &wgtk::script::Ty, node_id: u16, (position, velocity, dispersion): (Vec3, Vec3, f32)) -> io::Result<NrlUpdateNode> {
    use wgtk::net::codec::Codec;
    let value = Value::Dict(BTreeMap::from([
        (Arc::from("gunPosition"), Value::Vector3(position)),
        (Arc::from("shotVector"), Value::Vector3(velocity)),
        (Arc::from("dispersion"), Value::Float32(dispersion)),
    ]));
    let mut payload = Vec::new();
    value.write(&mut payload, ty)?;
    Ok(NrlUpdateNode { node_id, payload, python_value: None })
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
