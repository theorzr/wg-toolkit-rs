//! The emulator implementation of the login + base applications, we are trying to
//! reproduce the official server implementation.

mod account;
mod capture;
mod bot;
mod gun;
mod physics;

use std::collections::{hash_map, BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::io::{self, Write};
use std::thread;

use blowfish::Blowfish;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use glam::Vec3;
use rsa::rand_core::{OsRng, RngCore};
use rsa::RsaPrivateKey;
use tracing::{debug, info, instrument, warn};

use wgtk::app::{login, base};
use wgtk::app::base::ClientKey;
use wgtk::app::client::element::{ChangedProperties, EntityProperty, NrlIdList, NrlUpdateNode, NrlBody, NrlChild, NrlComponentData, NrlMessage, NrlNode, NrlStream};
use wgtk::app::dispatch::{MethodCall, ScriptDispatch};
use wgtk::app::entity::{Bytes, Entity, EntityId, EntityKey};
use wgtk::res::fs::ResFilesystem;
use wgtk::script::{PythonValue, Script, StringValue, Value};
use wgtk::space::terrain::Terrain;
use wgtk::util::io::serde_pickle_ser_options;

use crate::CliResult;
use crate::wot::account_commands::{CMD_ENQUEUE_IN_BATTLE_QUEUE, CMD_REQ_QUEUE_INFO, CMD_SYNC_DATA};
use crate::wot::collision::{VehicleCollision, VehicleHit, VehiclePart};
use crate::wot::item_defs::{AimingDef, ShotDef, VehicleDef};
use crate::wot::items::{item_type, TankmanDescr, VehicleDescr};
use crate::wot::schema::{self, Account, Avatar, AvatarBase, AvatarCell, HitPoint, ShotResult, VehicleCell};

use account::AccountData;
use bot::Bot;
use capture::{Answer, CapturedSession};
use gun::{Gun, HitTarget, Landing, Shot};
use physics::{MotionLimits, Vehicle};


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

    let mut base_app = base::App::new(base_app_addr.into(), script)
        .map_err(|e| format!("Failed to bind base app: {e}"))?;
    schema::register(&mut base_app)
        .map_err(|e| format!("Failed to bind the entity schemas: {e}"))?;

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
        accounts: HashMap::new(),
        enqueued: HashMap::new(),
        pending_switches: HashMap::new(),
        pending_returns: HashMap::new(),
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
    /// Each client's account, outside of battles.
    accounts: HashMap<ClientKey, EntityKey<Account>>,
    /// Clients waiting in the battle queue, with the inventory id of their vehicle.
    enqueued: HashMap<ClientKey, i64>,
    /// Clients told to switch base app for their battle, by the session key they will
    /// log in again with, with the IP they will come from, their blowfish key and
    /// their vehicle.
    pending_switches: HashMap<u32, (IpAddr, Arc<Blowfish>, Option<VehicleConfig>)>,
    /// Clients told to switch base app back to their account, leaving their battle, by
    /// the session key they will log in with.
    pending_returns: HashMap<u32, (IpAddr, Arc<Blowfish>)>,
    /// Clients connected for a battle, which get an `Avatar` rather than an `Account`.
    battle_clients: HashMap<ClientKey, BattleClient>,
}

/// A client connected for a battle.
#[derive(Debug)]
struct BattleClient {
    /// Its vehicle's configuration, the captured vehicle's when `None`.
    config: Option<VehicleConfig>,
    /// Its avatar, once created.
    avatar: Option<EntityKey<Avatar>>,
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

/// An enemy vehicle the server places in front of the player and drives: one of the
/// captured arena's, with its entry in `vehiclesInfo`.
#[derive(Debug, Clone)]
struct Target {
    entity_id: u32,
    /// Its entity, once created.
    entity: Option<EntityKey<schema::Vehicle>>,
    /// Its `vehiclesInfo` entry's fields, which its `publicInfo` repeats.
    info: BTreeMap<Arc<str>, Value>,
    /// Its driver and its collision models, once created.
    bot: Option<Bot>,
    collision: Option<Arc<VehicleCollision>>,
    health: i64,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
}

/// A battle client's vehicle: the vehicle the server simulates, and how the client
/// knows it.
#[derive(Debug)]
struct DrivenVehicle {
    entity: EntityKey<schema::Vehicle>,
    id_alias: u8,
    vehicle: Vehicle,
    /// The node of its `GunMarkerComponent`'s `gunMarker` property, once created.
    gun_marker_node: Option<u16>,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// The avatar's `ownVehicleGear` last sent, likewise.
    engine_gear: Option<u8>,
    collision: Arc<VehicleCollision>,
    gun: Gun,
    /// The nodes of its `OwnVehicle`'s `vehicleAmmoList` and `vehicleGunReloadTime`.
    ammo_node: Option<u16>,
    reload_node: Option<u16>,
    /// Updates to its NRL nodes, for the next tick.
    nrl_updates: Vec<NrlUpdateNode>,
    /// The shells in flight, its and the target's, with the game time they land at and
    /// who fired them.
    shots: Vec<(f64, EntityId, Shot)>,
    health: i64,
    max_health: i64,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
    /// The node of its `OwnVehicle`'s `vehicleHealthInfo`.
    health_node: Option<u16>,
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
    fn enter_account(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(session) = &self.session else { return Ok(()) };

        let Some((data, components_count)) = &session.account else { return Ok(()) };
        let account = self.app.create_base_player::<Account>(client, data.clone(), *components_count)?;
        self.accounts.insert(client, account);
        info!(%client, "Created account {account}");

        if let Some(args) = &session.show_gui {
            self.app.call_method(account, "showGUI", args.clone())?;
        }

        Ok(())

    }

    /// The client's arena is ready: tell its account, then switch it to the base app
    /// holding its avatar, this one, as the captured server did.
    fn start_battle(&mut self, client: ClientKey, vehicle_id: i64) -> io::Result<()> {

        if self.session.as_ref().and_then(|s| s.battle.as_ref()).is_none() {
            warn!(%client, "The capture has no battle to enter");
            return Ok(());
        }

        let (Some(&account), Some((session_key, blowfish))) = (self.accounts.get(&client), self.app.client_keys(client)) else {
            return Ok(());
        };

        self.app.entity(account)?.onArenaCreated()?;

        let SocketAddr::V4(base_addr) = self.app.addr()? else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "base app not on IPv4"));
        };
        let config = self.vehicle_config(vehicle_id)
            .inspect_err(|e| warn!(%client, "Failed to read vehicle {vehicle_id}, the captured one is used: {e}"))
            .ok();
        self.pending_switches.insert(session_key, (client.addr().ip(), blowfish, config));
        self.app.switch_base_app(client, base_addr, true)?;
        self.accounts.remove(&client);
        info!(%client, "Arena created, switching to {base_addr}");

        Ok(())

    }

    /// Switch a battle client's arena to a period: the countdown, then the battle. The
    /// update is `(period, end time, length, extra)`, pickled then compressed.
    fn arena_period(&mut self, client: ClientKey, period: u8) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client) else { return Ok(()) };
        let Some(avatar) = battle.avatar else { return Ok(()) };
        let length = match period {
            ARENA_PERIOD_PREBATTLE => PREBATTLE_LENGTH,
            _ => BATTLE_LENGTH,
        };
        battle.next_period = (period == ARENA_PERIOD_PREBATTLE).then(|| (Instant::now() + length, ARENA_PERIOD_BATTLE));
        let end = battle.game_time() + length.as_secs_f64();

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

        self.app.entity(avatar)?.updateArena(ARENA_UPDATE_PERIOD, Bytes(data))?;
        info!(%client, "Arena period {period} until {end:.1}");

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
            collision: def.collision,
        })
    }

    /// Leave the client's battle, as the captured server does: switch it to the base
    /// app again, where it logs in back to its account.
    fn leave_battle(&mut self, client: ClientKey) -> io::Result<()> {
        let Some((session_key, blowfish)) = self.app.client_keys(client) else { return Ok(()) };
        let SocketAddr::V4(base_addr) = self.app.addr()? else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "base app not on IPv4"));
        };
        self.pending_returns.insert(session_key, (client.addr().ip(), blowfish));
        self.app.switch_base_app(client, base_addr, true)?;
        self.battle_clients.remove(&client);
        info!(%client, "Leaving the battle, switching to {base_addr}");
        Ok(())
    }

    /// Bring a client that reconnected for its battle to its avatar.
    fn enter_battle(&mut self, client: ClientKey) -> io::Result<()> {

        let Some((data, components_count)) = self.session.as_ref()
            .and_then(|s| s.battle.as_ref())
            .and_then(|b| b.avatar.as_ref()) else {
            warn!(%client, "The capture has no avatar to create");
            return Ok(());
        };

        let avatar = self.app.create_base_player::<Avatar>(client, data.clone(), *components_count)?;
        if let Some(battle) = self.battle_clients.get_mut(&client) {
            battle.avatar = Some(avatar);
        }
        info!(%client, "Created avatar {avatar}");

        // The battle's setup, as the captured server sent it: the arena, the avatar's
        // cell side in the space, the space and its map, then the player's vehicle
        // entering the area of interest.
        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        for (method, args) in &battle.calls {
            self.app.call_method(avatar, method, args.clone())?;
        }

        // No `SetCellAppExtAddress`, which the captured server sends: given one, the
        // client opens a direct connection to the cell app and waits for a cell login we
        // can't answer yet, holding back its cell traffic. Without one, everything
        // comes through this base app.

        let Some(cell) = &battle.cell_player else {
            warn!(%client, "The capture has no cell player");
            return Ok(());
        };
        self.app.create_cell_player(avatar, cell.space_id, cell.vehicle_id, cell.position, cell.direction,
            cell.packed_xz_scale, cell.cell_data.clone())?;

        for data in &battle.space_properties {
            self.app.space_property(client, data.clone())?;
        }
        if let Some(geometry) = &battle.geometry {
            self.app.add_space_geometry_mapping(client, geometry.space_id, geometry.entry_id.addr, &geometry.path)?;
            info!(%client, "Entering space {} with {}", geometry.space_id, geometry.path);
        }

        // Its position and properties follow its creation, see `create_vehicle`.
        if let Some(alias) = battle.vehicle_alias {
            self.app.enter_aoi(client, cell.vehicle_id, alias)?;
        }

        // An enemy to shoot at, created likewise once the client asks, see
        // `create_target`.
        let target = battle.arena_info.as_ref().and_then(|arena| enemy_of(&arena.nrl, cell.vehicle_id));
        if let Some(target) = &target {
            self.app.enter_aoi(client, target.entity_id, TARGET_ALIAS)?;
            info!(%client, "Target {} ({:?})", target.entity_id, target.info.get("name"));
        }
        if let Some(battle) = self.battle_clients.get_mut(&client) {
            battle.target = target.clone();
        }

        // The arena's info entity, created right away as entities without an alias are,
        // then given its vehicles: only the player's, the arena having no one else yet.
        if let Some(arena) = &battle.arena_info {
            let Some((type_id, properties)) = &arena.created else {
                warn!(%client, "The capture's arena info entity has no creation");
                return Ok(());
            };
            let mut properties = properties.clone();
            for (_, name, value) in &mut properties.values {
                if &**name == "avatarID" {
                    *value = with_int(value, avatar.id().0.into());
                }
            }
            let config = self.battle_clients.get(&client).and_then(|battle| battle.config.as_ref());
            let target_id = target.as_ref().map(|target| target.entity_id);
            let stream = arena_vehicles(self.app.dispatch(), &arena.nrl, cell.vehicle_id, target_id, config)?;
            self.app.enter_aoi(client, arena.entity_id, NO_ID_ALIAS)?;
            self.app.create_entity_detailed(client, arena.entity_id, *type_id, Vec3::ZERO, Vec3::ZERO, &properties)?;
            self.app.nrl_create_node(client, stream)?;
            info!(%client, "Created arena info {}", arena.entity_id);
        }

        Ok(())

    }

    /// Create the player's vehicle once the client asks for it, then place it and give
    /// it its properties: the client drops both for an entity it doesn't have yet.
    fn create_vehicle(&mut self, client: ClientKey, entity_id: u32) -> io::Result<()> {

        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let Some(entity) = battle.vehicle_entity.as_ref().filter(|e| e.entity_id == entity_id) else { return Ok(()) };

        // The vehicle belongs to our avatar, and is the one the client went to battle with.
        let config = self.battle_clients.get(&client).and_then(|battle| battle.config.clone());
        let config = config.as_ref();
        let avatar_id = self.battle_clients.get(&client).and_then(|battle| battle.avatar).map(|avatar| avatar.id().0);
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

        info!(%client, "Creating vehicle {entity_id}");
        let vehicle = self.app.create_entity::<schema::Vehicle>(client, entity)?;
        if let (Some(alias), Some(position)) = (battle.vehicle_alias, &battle.vehicle_position) {
            self.app.update_position_detailed(client, alias, position.position, position.direction)?;
            // From there on it moves as the client drives it.
            if let (Some(battle), Some(config)) = (self.battle_clients.get_mut(&client), config) {
                battle.driven = Some(DrivenVehicle {
                    entity: vehicle,
                    id_alias: alias,
                    vehicle: Vehicle::new(position.position, position.direction, config.limits, config.aiming.clone()),
                    gun_marker_node: None,
                    engine_mode: None,
                    engine_gear: None,
                    collision: config.collision.clone(),
                    gun: Gun::new(config.shots.iter().cloned().zip(config.ammo.iter().map(|&(_, count)| count)).collect(), config.aiming.reload_time),
                    ammo_node: None,
                    reload_node: None,
                    nrl_updates: Vec::new(),
                    shots: Vec::new(),
                    health: config.max_health.into(),
                    max_health: config.max_health.into(),
                    damage_stickers: Vec::new(),
                    health_node: None,
                });
            }
        }
        if let Some(update) = &battle.vehicle_update {
            let mut update = update.clone();
            if let Some(config) = config {
                config.apply_to_update(&mut update);
            }
            self.app.update_entity(vehicle, update)?;
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
            let health_node = property_node(self.app.dispatch(), &stream.messages[1], "vehicleHealthInfo");
            if let Some(driven) = self.battle_clients.get_mut(&client).and_then(|battle| battle.driven.as_mut()) {
                (driven.ammo_node, driven.reload_node, driven.health_node) = (ammo_node, reload_node, health_node);
            }
            self.app.nrl_create_node(client, stream)?;
            let (message, property_node) = gun_marker_component(self.app.dispatch(), entity_node)?;
            self.app.nrl_create_node(client, NrlStream { messages: vec![message], rest: Vec::new() })?;
            if let Some(driven) = self.battle_clients.get_mut(&client).and_then(|battle| battle.driven.as_mut()) {
                driven.gun_marker_node = Some(property_node);
            }
        }

        Ok(())

    }

    /// Create the target once the client asks for it, from the player's captured
    /// vehicle: its own id, descriptor, name, team and health, no avatar, then place it
    /// [`TARGET_DISTANCE`] in front of the player's start, facing it.
    fn create_target(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(target) = self.battle_clients.get(&client).and_then(|battle| battle.target.clone()) else { return Ok(()) };
        let Some(battle) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let (Some(entity), Some(properties), Some(start)) = (&battle.vehicle_entity, &battle.vehicle_properties, &battle.vehicle_position) else {
            warn!(%client, "The capture has no vehicle to make the target from");
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

        // Driven as its own vehicle, by its definition.
        let compact_descr = match target.info.get("compDescr") {
            Some(Value::String(StringValue::Raw(bytes))) => bytes.clone(),
            other => return Err(invalid_data(&format!("target has no vehicle descriptor: {other:?}"))),
        };
        let decoded = VehicleDescr::decode(&compact_descr).map_err(|e| invalid_data(&e.to_string()))?;
        let def = VehicleDef::load_by_id(&ResFilesystem::new(self.game_dir.join("res"))?, decoded.nation, decoded.type_id)?;
        let limits = MotionLimits { speed_forward: def.speed_forward, speed_backward: def.speed_backward, rotation_speed: def.rotation_speed };
        let ammo = def.shots.iter().cloned().zip(def.default_ammo().into_iter().map(|(_, count)| count)).collect();
        let gun = Gun::new(ammo, def.aiming.reload_time).with_shot_ids_from(TARGET_FIRST_SHOT_ID);
        let bot = Bot::new(Vehicle::new(position, direction, limits, def.aiming.clone()), gun, position);

        info!(%client, "Creating target {} ({}) at {position:?}", target.entity_id, def.name);
        let vehicle = self.app.create_entity::<schema::Vehicle>(client, entity)?;
        if let Some(target) = self.battle_clients.get_mut(&client).and_then(|battle| battle.target.as_mut()) {
            target.bot = Some(bot);
            target.collision = Some(def.collision.clone());
            target.entity = Some(vehicle);
        }
        self.app.update_position_detailed(client, TARGET_ALIAS, position, direction)?;
        Ok(())

    }

    /// Fire the client's gun, if loaded: the shell's tracer and the gun's effects, then
    /// one shell fewer and the reload. Where the shell lands follows with
    /// [`Self::land_shot`].
    fn shoot(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client) else { return Ok(()) };
        let Some(avatar) = battle.avatar else { return Ok(()) };
        let now = battle.game_time();
        let vehicles: Vec<_> = battle.target.iter()
            .filter(|target| target.health > 0)
            .filter_map(|target| Some(hit_target(target.entity_id, &target.bot.as_ref()?.vehicle, target.collision.as_ref()?)))
            .collect();
        let Some(driven) = battle.driven.as_mut() else { return Ok(()) };
        let (position, velocity, dispersion) = driven.vehicle.turret.gun_marker(&driven.vehicle.hull);
        let Some(shot) = driven.gun.fire(now, position, velocity, dispersion, self.terrain.as_ref(), &vehicles) else {
            debug!(%client, "Not loaded, not shooting");
            return Ok(());
        };
        info!(%client, "Shot {} from {:?} lands at {:?} in {:.2} s", shot.id, shot.start, shot.end, shot.flight_time);
        driven.shots.push((now + f64::from(shot.flight_time), driven.entity.id(), shot.clone()));

        let dispatch = self.app.dispatch();
        let ammo = ammo_list(driven.gun.ammo.iter().map(|(def, count)| (def.compact_descr.to_int(), *count)));
        let reload = gun_reload_time(driven.gun.reload_time(), driven.gun.ready_at as f32, driven.gun.reload_time());
        for (node, name, value) in [(driven.ammo_node, "vehicleAmmoList", ammo), (driven.reload_node, "vehicleGunReloadTime", reload)] {
            if let Some(node) = node {
                driven.nrl_updates.push(property_update(dispatch, "OwnVehicle", name, node, &value)?);
            }
        }
        let vehicle = driven.entity;

        show_shot(&mut self.app, avatar, vehicle, &shot)

    }

    /// A shell gets to its end: it damages the vehicle it hits, explodes on the
    /// terrain, or its tracer stops at its maximum distance.
    fn land_shot(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot) -> io::Result<()> {
        let Some(avatar) = self.battle_clients.get(&client).and_then(|battle| battle.avatar) else { return Ok(()) };
        let def = &shot.def;
        self.app.entity(avatar)?.stopTracer(shot.id, shot.end)?;
        match shot.landing {
            Landing::Vehicle(entity_id, hit) => {
                info!(%client, "Shot {} hits {entity_id}'s {:?}, material {}, at {:?}", shot.id, hit.part, hit.material_kind, hit.local_point);
                let player_vehicle = self.battle_clients.get(&client).and_then(|battle| battle.driven.as_ref()).map(|driven| driven.entity.id());
                if Some(EntityId(entity_id)) == player_vehicle {
                    self.damage_player(client, shooter, shot, &hit)
                } else {
                    self.damage_target(client, entity_id, shot, &hit)
                }
            }
            // EXPERIMENT: the explosion comes for a shot the client does not know, so that
            // it shows it from these arguments alone.
            Landing::Terrain => self.app.entity(avatar)?.explodeProjectile(
                shot.id | 0x4000_0000,
                def.effects_index,
                def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
                GROUND_EFFECT_MATERIAL,
                def.kind_index,
                def.caliber,
                shot.end,
                // Its last step: the client projects the crater along it, both ways.
                shot.end_velocity * EXPLOSION_STEP,
                shot.end_velocity.length(),
                Vec::new(),
            ),
            Landing::Range => Ok(()),
        }
    }

    /// The player's vehicle takes a shell's damage, as the target does: its health
    /// drops, in its `OwnVehicle` too.
    fn damage_player(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot, hit: &VehicleHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client) else { return Ok(()) };
        let Some(driven) = battle.driven.as_mut().filter(|driven| driven.health > 0) else { return Ok(()) };
        let amount = spread_damage(shot.def.damage);
        let old_health = driven.health;
        driven.health = (old_health - amount).max(0);
        let new_health = driven.health;
        let bounds = driven.collision.parts[hit.part as usize].bounds;
        info!(%client, "Player takes {amount} damage, {old_health} -> {new_health}{}", if new_health == 0 { ", destroyed" } else { "" });

        if let Some(node) = driven.health_node {
            let info = vehicle_health_info(driven.entity.id().0, new_health);
            driven.nrl_updates.push(property_update(self.app.dispatch(), "OwnVehicle", "vehicleHealthInfo", node, &info)?);
        }
        let stickers = add_sticker(&mut driven.damage_stickers, shot, hit, bounds);
        let damage = Damage { shooter, amount, max_health: driven.max_health, old_health, new_health };
        show_damage(&mut self.app.entity(driven.entity)?, shot, hit, bounds, &damage, stickers)

    }

    /// The target takes a shell's damage, varied by up to [`DAMAGE_SPREAD`] either way:
    /// its health drops, and the player sees the hit, as the captured server shows one.
    fn damage_target(&mut self, client: ClientKey, entity_id: u32, shot: &Shot, hit: &VehicleHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client) else { return Ok(()) };
        let (Some(avatar), Some(driven)) = (battle.avatar, battle.driven.as_ref()) else { return Ok(()) };
        let shooter = driven.entity.id();
        let Some(target) = battle.target.as_mut().filter(|target| target.entity_id == entity_id && target.health > 0) else {
            return Ok(());
        };
        let Some(vehicle) = target.entity else { return Ok(()) };
        let amount = spread_damage(shot.def.damage);
        let old_health = target.health;
        target.health = (old_health - amount).max(0);
        let new_health = target.health;
        let killed = new_health == 0;
        let bounds = target.collision.as_ref().map_or([Vec3::ZERO; 2], |collision| collision.parts[hit.part as usize].bounds);
        let max_health = target.info.get("maxHealth").and_then(Value::as_i64).unwrap_or(old_health);
        info!(%client, "Target {entity_id} takes {amount} damage, {old_health} -> {new_health}{}", if killed { ", destroyed" } else { "" });

        let stickers = add_sticker(&mut target.damage_stickers, shot, hit, bounds);
        let damage = Damage { shooter, amount, max_health, old_health, new_health };
        show_damage(&mut self.app.entity(vehicle)?, shot, hit, bounds, &damage, stickers)?;

        let mut flags = HIT_FLAGS_PIERCED;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        self.app.entity(avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.id(),
        }])

    }

    /// Answer a client request with what the captured server answered to the closest
    /// captured request, with the new request's id in place of the captured one.
    fn answer(&mut self, client: ClientKey, method: &str, args: &[Value]) -> io::Result<()> {

        let player = self.accounts.get(&client).map(|account| account.any())
            .or_else(|| self.battle_clients.get(&client)?.avatar.map(|avatar| avatar.any()));
        let (Some(session), Some(player)) = (&self.session, player) else { return Ok(()) };
        let Some(request_id) = args.first().and_then(Value::as_i64) else { return Ok(()) };

        let Some(captured) = session.find_request(method, args) else {
            warn!(%client, "No captured answer to {method} {args:?}");
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
                    info!(%client, "Answering with {method} {}", format!("{args:?}").chars().take(200).collect::<String>());
                    self.app.call_method(player, method, args)?;
                }
                Answer::Resource { description, data } => {
                    info!(%client, "Answering with resource {request_id}, {} bytes", data.len());
                    self.app.push_resource(client, request_id as u16, description.clone(), data)?;
                }
            }
        }

        Ok(())

    }

    /// Tell a client just logged in the update frequency, right after the login reply, as
    /// the captured server did.
    fn send_update_frequency(&mut self, client: ClientKey) {
        if let Some((frequency, unknown)) = self.session.as_ref().and_then(|s| s.update_frequency) {
            if let Err(e) = self.app.update_frequency(client, frequency, unknown, 0) {
                warn!(%client, "Failed to send the update frequency: {e}");
            }
        }
    }

    /// A request of the client's account, answered as captured. The arena is ready as
    /// soon as the client first asks how the queue is going.
    fn request(&mut self, client: ClientKey, name: &str, args: &[Value]) {
        if let Err(e) = self.answer(client, name, args) {
            warn!(%client, "Failed to answer {name}: {e}");
        }
        let command = args.get(1).and_then(Value::as_i64);
        if command == Some(CMD_ENQUEUE_IN_BATTLE_QUEUE.into()) {
            // `[queue type, vehicle inventory id, ...]`.
            let vehicle_id = match args.get(2) {
                Some(Value::Seq(values)) => values.get(1).and_then(Value::as_i64),
                _ => None,
            };
            self.enqueued.insert(client, vehicle_id.unwrap_or(0));
        } else if let Some(vehicle_id) = (command == Some(CMD_REQ_QUEUE_INFO.into()))
            .then(|| self.enqueued.remove(&client)).flatten() {
            if let Err(e) = self.start_battle(client, vehicle_id) {
                warn!(%client, "Failed to start the battle: {e}");
            }
        }
    }

    /// A cell method of its avatar the client called.
    fn avatar_cell_call(&mut self, client: ClientKey, call: AvatarCell) {
        if let AvatarCell::VehicleShoot {} = call {
            if let Err(e) = self.shoot(client) {
                warn!(%client, "Failed to shoot: {e}");
            }
            return;
        }
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.driven.as_mut()) else {
            warn!(%client, "{call:?}, but the client has no vehicle");
            return;
        };
        match call {
            AvatarCell::VehicleShoot {} => unreachable!(),
            AvatarCell::VehicleMoveWith { flags } => {
                info!(%client, "Moving with flags {flags:#04x}");
                driven.vehicle.hull.flags = flags;
            }
            AvatarCell::VehicleStopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }
    }

    /// A cell method of its vehicle the client called.
    fn vehicle_cell_call(&mut self, client: ClientKey, call: VehicleCell) {
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.driven.as_mut()) else {
            warn!(%client, "{call:?}, but the client has no vehicle");
            return;
        };
        match call {
            VehicleCell::TrackRelativePointWithGun { point } => {
                let Vehicle { hull, turret } = &mut driven.vehicle;
                debug!(%client, "Tracking {point:?} from {:?}", hull.position);
                turret.track(hull.position + point);
            }
            VehicleCell::StopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }
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
                let gun_marker_ty = self.app.dispatch().entity_from_name("GunMarkerComponent")
                    .and_then(|(_, component)| component.properties.first())
                    .map(|property| property.ty.clone());
                for (&client, battle) in &mut self.battle_clients {
                    let tick = battle.ticks as u8;
                    battle.ticks = battle.ticks.wrapping_add(1);
                    let now = battle.game_time();
                    let vehicle = battle.driven.as_mut().map(|driven| {
                        let DrivenVehicle { entity, id_alias, vehicle, gun_marker_node, engine_mode, engine_gear, gun, reload_node, nrl_updates: pending, .. } = driven;
                        vehicle.step(TICK.as_secs_f32(), self.terrain.as_ref());
                        let Vehicle { hull, turret } = vehicle;
                        let gun_marker = gun_marker_node.zip(gun_marker_ty.as_ref())
                            .map(|(node, ty)| gun_marker_update(ty, node, turret.gun_marker(hull)));
                        let mut nrl_updates = std::mem::take(pending);
                        if let (Some(node), true) = (*reload_node, gun.take_loaded(now)) {
                            let loaded = gun_reload_time(gun.reload_time(), 0.0, 0.0);
                            match property_update(self.app.dispatch(), "OwnVehicle", "vehicleGunReloadTime", node, &loaded) {
                                Ok(update) => nrl_updates.push(update),
                                Err(e) => warn!(%client, "Failed to encode the reload: {e}"),
                            }
                        }
                        match gun_marker {
                            Some(Ok(update)) => nrl_updates.push(update),
                            Some(Err(e)) => warn!(%client, "Failed to encode the gun marker: {e}"),
                            None => {}
                        }
                        // Every tick, to compare with what the client simulates.
                        debug!(%client, "Vehicle at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
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
                        // What the client aims from, so it must turn with the hull, and the
                        // engine's RPM, then its gear when it shifts.
                        let mut player_properties = vec![EntityProperty {
                            name: "ownVehicleAuxPhysicsData".into(),
                            value: Value::UInt64(hull.aux_physics_data()),
                        }];
                        let (gear, _) = hull.engine();
                        if *engine_gear != Some(gear) {
                            *engine_gear = Some(gear);
                            player_properties.push(EntityProperty { name: "ownVehicleGear".into(), value: Value::UInt8(gear) });
                        }
                        base::TickVehicle {
                            entity: entity.any(),
                            id_alias: *id_alias,
                            position: hull.position,
                            direction: hull.direction,
                            properties,
                            player_properties,
                            nrl_updates,
                        }
                    });
                    // The target drives, aims at the player and fires once on them.
                    let mut others = Vec::new();
                    let mut bot_shot = None;
                    if let (Some(target), Some(driven)) = (battle.target.as_mut(), battle.driven.as_mut()) {
                        if let (Some(bot), Some(target_entity)) = (target.bot.as_mut().filter(|_| target.health > 0), target.entity) {
                            let enemy = hit_target(driven.entity.id().0, &driven.vehicle, &driven.collision);
                            let enemy_hull = enemy.transforms[VehiclePart::Hull as usize].translation.into();
                            let on_target = bot.step(TICK.as_secs_f32(), self.terrain.as_ref(), enemy_hull, TARGET_DRIVES);
                            if TARGET_FIRES && on_target && driven.health > 0 {
                                let (position, velocity, dispersion) = bot.vehicle.turret.gun_marker(&bot.vehicle.hull);
                                if let Some(shot) = bot.gun.fire(now, position, velocity, dispersion, self.terrain.as_ref(), &[enemy]) {
                                    info!(%client, "Target shot {} lands at {:?} in {:.2} s", shot.id, shot.end, shot.flight_time);
                                    driven.shots.push((now + f64::from(shot.flight_time), target_entity.id(), shot.clone()));
                                    bot_shot = Some((target_entity, shot));
                                }
                            }
                            let Vehicle { hull, turret } = &bot.vehicle;
                            let mut properties = vec![EntityProperty { name: "gunAnglesPacked".into(), value: Value::UInt16(turret.gun_angles_packed()) }];
                            let mode = hull.engine_mode();
                            if target.engine_mode != Some(mode) {
                                target.engine_mode = Some(mode);
                                properties.push(EntityProperty {
                                    name: "engineMode".into(),
                                    value: Value::Seq(mode.iter().map(|&n| Value::UInt8(n)).collect()),
                                });
                            }
                            others.push(base::TickVehicle {
                                entity: target_entity.any(),
                                id_alias: TARGET_ALIAS,
                                position: hull.position,
                                direction: hull.direction,
                                properties,
                                player_properties: Vec::new(),
                                nrl_updates: Vec::new(),
                            });
                        }
                    }

                    if let Err(e) = self.app.tick(client, tick, vehicle.as_ref(), &others) {
                        warn!(%client, "Failed to tick: {e}");
                    }
                    if let (Some((shooter, shot)), Some(avatar)) = (bot_shot, battle.avatar) {
                        let result = show_shot(&mut self.app, avatar, shooter, &shot);
                        if let Err(e) = result {
                            warn!(%client, "Failed to show the target's shot: {e}");
                        }
                    }
                }
                let landed: Vec<_> = self.battle_clients.iter_mut()
                    .filter_map(|(&client, battle)| {
                        let now = battle.game_time();
                        let shots = &mut battle.driven.as_mut()?.shots;
                        let (landed, flying) = std::mem::take(shots).into_iter().partition(|&(at, _, _)| at <= now);
                        *shots = flying;
                        Some((client, landed))
                    })
                    .collect();
                for (client, landed) in landed {
                    for (_, shooter, shot) in landed {
                        if let Err(e) = self.land_shot(client, shooter, &shot) {
                            warn!(%client, "Failed to land shot {}: {e}", shot.id);
                        }
                    }
                }
                let due: Vec<_> = self.battle_clients.iter()
                    .filter_map(|(&client, battle)| battle.next_period.filter(|&(at, _)| at <= Instant::now()).map(|(_, period)| (client, period)))
                    .collect();
                for (client, period) in due {
                    if let Err(e) = self.arena_period(client, period) {
                        warn!(%client, "Failed to change the arena period: {e}");
                    }
                }
            }

            // Wait for packets until the next tick is due, no longer.
            match self.app.poll_until(next_tick) {
                base::Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Error: {}", error.error);
                    } else {
                        warn!("Error: {}", error.error);
                    }
                }
                base::Event::Login(login) => {

                    // A client coming back from its battle, see `leave_battle`: it gets its
                    // account again once it enables its entities.
                    if let Some((ip, blowfish)) = self.pending_returns.remove(&login.login_key) {
                        if ip != login.addr.ip() {
                            info!(addr = %login.addr, "Login #{}... Invalid address after switch", login.attempt_num);
                            continue;
                        }
                        info!(addr = %login.addr, "Login #{}... Success, back from the battle", login.attempt_num);
                        if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                            self.send_update_frequency(client);
                        }
                        continue;
                    }

                    // A client coming back for its battle, see `start_battle`.
                    if let Some((ip, blowfish, config)) = self.pending_switches.remove(&login.login_key) {
                        if ip != login.addr.ip() {
                            info!(addr = %login.addr, "Login #{}... Invalid address after switch", login.attempt_num);
                            continue;
                        }
                        info!(addr = %login.addr, "Login #{}... Success, for the battle", login.attempt_num);
                        if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                            self.send_update_frequency(client);
                            self.battle_clients.insert(client, BattleClient {
                                config, avatar: None, joined: Instant::now(), ticks: 0, next_period: None, driven: None, target: None,
                            });
                        }
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
                    let blowfish = client.blowfish;
                    drop(clients);
                    if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                        self.send_update_frequency(client);
                    }

                }
                base::Event::EnableEntities(event) => {
                    info!(client = %event.client, "Entities enabled");
                    if self.battle_clients.contains_key(&event.client) {
                        if let Err(e) = self.enter_battle(event.client) {
                            warn!(client = %event.client, "Failed to create the avatar: {e}");
                        }
                    } else if let Err(e) = self.enter_account(event.client) {
                        warn!(client = %event.client, "Failed to create the account: {e}");
                    }
                }
                base::Event::BaseMethod(event) => {
                    let MethodCall::Known { name, args, .. } = &event.call else {
                        warn!(client = %event.client, "Base method call on entity {}: {:?}", event.entity, event.call);
                        continue;
                    };
                    info!(client = %event.client, "Base method call on entity {}: {name} {args:?}", event.entity);
                    match self.app.decode::<AvatarBase>(event.entity, &event.call) {
                        Some((_, AvatarBase::SetClientReady {})) => {
                            if let Err(e) = self.arena_period(event.client, ARENA_PERIOD_PREBATTLE) {
                                warn!(client = %event.client, "Failed to start the countdown: {e}");
                            }
                        }
                        Some((_, AvatarBase::LeaveArena {})) => {
                            if let Err(e) = self.leave_battle(event.client) {
                                warn!(client = %event.client, "Failed to leave the battle: {e}");
                            }
                        }
                        None => self.request(event.client, name, args),
                    }
                }
                base::Event::CellMethod(event) => {
                    if let Some((_, call)) = self.app.decode::<AvatarCell>(event.entity, &event.call) {
                        self.avatar_cell_call(event.client, call);
                    } else if let Some((_, call)) = self.app.decode::<VehicleCell>(event.entity, &event.call) {
                        self.vehicle_cell_call(event.client, call);
                    } else {
                        match &event.call {
                            MethodCall::Known { name, .. } => debug!(client = %event.client, "Cell method call on entity {}: {name}", event.entity),
                            MethodCall::Unknown { .. } => debug!(client = %event.client, "Cell method call on entity {}: {:?}", event.entity, event.call),
                        }
                    }
                }
                base::Event::AvatarUpdate(event) => {
                    debug!(client = %event.client, "Avatar update on {:?}: {:?}, direction {:?}", event.vehicle_id, event.position, event.direction);
                }
                base::Event::RequestEntityUpdate(event) => {
                    let is_target = self.battle_clients.get(&event.client)
                        .and_then(|battle| battle.target.as_ref())
                        .is_some_and(|target| target.entity_id == event.entity_id);
                    if is_target {
                        if let Err(e) = self.create_target(event.client) {
                            warn!(client = %event.client, "Failed to create the target: {e}");
                        }
                    } else if let Err(e) = self.create_vehicle(event.client, event.entity_id) {
                        warn!(client = %event.client, "Failed to create vehicle {}: {e}", event.entity_id);
                    }
                }
                base::Event::Timeout => {}

            }

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
    collision: Arc<VehicleCollision>,
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
            entity: None,
            health: info.get("maxHealth")?.as_i64()?,
            info,
            bot: None,
            collision: None,
            engine_mode: None,
            damage_stickers: Vec::new(),
        }))
}

/// The target's id alias, apart from the one the captured server gave the player's
/// vehicle.
const TARGET_ALIAS: u8 = 200;


/// How much a shell's damage varies, either way, as a share of it.
const DAMAGE_SPREAD: f32 = 0.25;

/// `VEHICLE_HIT_FLAGS` of a shell piercing a vehicle, as captured: a direct projectile,
/// through armour and a module.
const HIT_FLAGS_PIERCED: u32 = 0x100000 | 0x100 | 0x10;
/// `VEHICLE_HIT_EFFECT.ARMOR_PIERCED`.
const HIT_EFFECT_ARMOR_PIERCED: u8 = 4;

/// `VEHICLE_HIT_FLAGS.VEHICLE_KILLED`.
const HIT_FLAG_VEHICLE_KILLED: u32 = 1;

/// The target's first shot id, apart from the player's.
const TARGET_FIRST_SHOT_ID: i32 = 1_000_000;

/// How far in front of the player's start the target stands, in metres.
const TARGET_DISTANCE: f32 = 40.0;

/// Whether the target fires back at the player.
const TARGET_FIRES: bool = false;
/// Whether the target drives around, else it stays where it is created.
const TARGET_DRIVES: bool = false;

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
        ("vehicleHealthInfo", vehicle_health_info(entity_id, config.max_health.into())),
    ]

}

/// Show a shot to the player: its tracer, and the gun firing on the shooter.
fn show_shot(app: &mut base::App, avatar: EntityKey<Avatar>, shooter: EntityKey<schema::Vehicle>, shot: &Shot) -> io::Result<()> {
    let def = &shot.def;
    app.entity(avatar)?.showTracer(
        shooter.id(),
        shot.id,
        0,
        def.effects_index,
        def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
        def.kind_index,
        def.caliber,
        shot.start,
        shot.velocity,
        def.gravity,
        def.max_distance,
        0,
        0,
    )?;
    // One shell from the only gun.
    app.entity(shooter)?.showShooting(1, 0, def.kind_index)
}

/// Stop the gun where our turret is rather than at the client's angles, which its own
/// rotator computes slightly apart: the gun would move as it locks.
fn hold_gun(client: ClientKey, driven: &mut DrivenVehicle, yaw: f32, pitch: f32) {
    let turret = &mut driven.vehicle.turret;
    debug!(%client, "Holding the gun at yaw {:.4}, pitch {:.4} (client: {yaw:.4}, {pitch:.4})", turret.yaw, turret.pitch);
    turret.hold(turret.yaw, turret.pitch);
}

/// A vehicle as shells hit it: its collision models where its parts are.
fn hit_target(entity_id: u32, vehicle: &Vehicle, collision: &Arc<VehicleCollision>) -> HitTarget {
    HitTarget { entity_id, collision: collision.clone(), transforms: vehicle.part_transforms(collision.hull_position) }
}

/// A shell's damage, varied by up to [`DAMAGE_SPREAD`] either way.
fn spread_damage(damage: f32) -> i64 {
    use rand::Rng;
    (damage * rand::thread_rng().gen_range(1.0 - DAMAGE_SPREAD..=1.0 + DAMAGE_SPREAD)).round() as i64
}

/// A shell's damage to a vehicle.
struct Damage {
    shooter: EntityId,
    amount: i64,
    max_health: i64,
    old_health: i64,
    new_health: i64,
}

/// Show a vehicle taking a shell's damage, pierced at `hit`: one hit point, its segment
/// encoded over the hit part's `bounds`, its params as `HitParamsEncoder` encodes them,
/// and the damage, also as a fraction of its maximum health out of 255. Then its new
/// health and its stickers, given when the shell added one, see [`add_sticker`].
fn show_damage(
    vehicle: &mut Entity<'_, schema::Vehicle>,
    shot: &Shot,
    hit: &VehicleHit,
    bounds: [Vec3; 2],
    damage: &Damage,
    stickers: Option<Vec<HitPoint>>,
) -> io::Result<()> {
    let def = &shot.def;
    let damage_factor = (damage.amount * 255 / damage.max_health.max(1)).clamp(0, 255) as u8;
    vehicle.showDamageFromShot(
        damage.shooter,
        vec![hit_point(shot, hit, bounds, HIT_EFFECT_ARMOR_PIERCED)],
        def.effects_index,
        def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
        damage.amount as i32,
        damage_factor,
        0, // The last material is not a shield.
        shot.end_velocity.length(),
        0,
    )?;
    vehicle.set_health(damage.new_health as i16)?;
    if let Some(stickers) = stickers {
        vehicle.set_damageStickers(stickers)?;
    }
    // `ATTACK_REASON.SHOT`, no extended reason.
    vehicle.onHealthChanged(damage.new_health as i16, damage.old_health as i16, damage.shooter, 0, -1)
}

/// A `VEHICLE_HIT_POINT` for a shell piercing at `hit`, its segment encoded over the hit
/// part's `bounds` with `context`: a `VEHICLE_HIT_EFFECT` for `showDamageFromShot`, a
/// sticker id for `damageStickers`. Its params are as `HitParamsEncoder` encodes them.
fn hit_point(shot: &Shot, hit: &VehicleHit, bounds: [Vec3; 2], context: u8) -> HitPoint {
    let def = &shot.def;
    let effect = HIT_EFFECT_ARMOR_PIERCED;
    let params = ((def.caliber * 10.0) as u32 & 0xFFFF) << 8 | u32::from(def.kind_index & 15) << 4 | u32::from(effect);
    HitPoint { networkID: 0, segment: hit.encode_segment(bounds, context), params }
}

/// Add the shell's sticker to a vehicle's `damageStickers`, if it leaves one, as
/// `encodeStickerIdData` encodes it (not parametrized: no prefab effects). Returns the
/// stickers to send when it did.
fn add_sticker(stickers: &mut Vec<HitPoint>, shot: &Shot, hit: &VehicleHit, bounds: [Vec3; 2]) -> Option<Vec<HitPoint>> {
    let sticker = shot.def.pierced_sticker?;
    stickers.push(hit_point(shot, hit, bounds, sticker & 0x7F));
    Some(stickers.clone())
}

/// The `OwnVehicle`'s `vehicleHealthInfo` of a vehicle with this health.
fn vehicle_health_info(entity_id: u32, health: i64) -> Value {
    Value::Dict(BTreeMap::from([
        (Arc::from("deathReasonExtID"), Value::Int8(-1)),
        (Arc::from("deathReasonID"), Value::Int8(-1)),
        (Arc::from("health"), Value::Int16(health as i16)),
        (Arc::from("id"), Value::Int32(entity_id as i32)),
        (Arc::from("isCrewActive"), Value::UInt8(1)),
        (Arc::from("isRespawnActive"), Value::UInt8(0)),
    ]))
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
/// its legacy shot effects.
const NO_PREFAB_EFFECTS: u8 = 255;

/// How long the last step `explodeProjectile` gives is, in seconds. The captured
/// server's are up to its 0.1 s ticks; the client projects the ground effects, the
/// crater with them, along that step, so it must reach the client's ground.
const EXPLOSION_STEP: f32 = 0.05;

/// The `ground` effect material, its index in `EFFECT_MATERIALS`: effect materials in the
/// order `system/data/material_kinds.xml` first names them.
const GROUND_EFFECT_MATERIAL: u8 = 0;

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
