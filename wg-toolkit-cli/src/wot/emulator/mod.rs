//! The emulator implementation of the login + base applications, we are trying to
//! reproduce the official server implementation. Its battles are in [`battle`].

mod account;
mod battle;
mod capture;
mod bot;
mod gun;
mod physics;

use std::collections::{hash_map, HashMap};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::io::{self, Write};
use std::thread;

use tracing::{debug, info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use blowfish::Blowfish;
use rsa::RsaPrivateKey;

use wgtk::app::client::element::{NrlBody, NrlChild, NrlMessage, NrlNode};
use wgtk::app::entity::{Bytes, EntityId, EntityKey, Pickle, ScriptType};
use wgtk::util::io::serde_pickle_ser_options;
use wgtk::app::dispatch::MethodCall;
use wgtk::script::{Script, Value};
use wgtk::space::terrain::Terrain;
use wgtk::res::fs::ResFilesystem;
use wgtk::app::base::ClientKey;
use wgtk::app::{login, base};

use crate::wot::schema::{
    self, Account, AmmoViews, Avatar, AvatarBase, AvatarCellData, AvatarData, BattleChatRestriction, GunReloadTime,
    PlayLimits, RemoteCamera, TargetingInfo, VehicleAmmo, VehicleHealthInfo, VehicleSetting,
};
use crate::wot::account_commands::{CMD_ENQUEUE_IN_BATTLE_QUEUE, CMD_REQ_QUEUE_INFO, CMD_SYNC_DATA};
use crate::wot::items::{item_type, TankmanDescr, VehicleDescr};
use crate::wot::item_defs::{AimingDef, ShotDef, VehicleDef};
use crate::wot::arena_defs::ArenaTypeDef;
use crate::wot::collision::VehicleCollision;
use crate::CliResult;

use capture::{Answer, CapturedSession};
use physics::MotionLimits;
use account::AccountData;
use battle::Battle;


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
                Some(battle) => format!("cell player {}, vehicle alias {:?}, position {}, entity {}, arena info {:?}",
                    battle.cell_player.is_some(), battle.vehicle_alias,
                    battle.vehicle_position.is_some(), battle.vehicle_entity.is_some(),
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
            let account = reencode_account_data(&mut session, &vehicles)
                .map_err(|e| format!("Failed to re-encode the captured account data: {e}"))?;
            (Some(session), account)
        }
        None => (None, None),
    };
    let (session, account) = session;
    let session = session.map(Arc::new);

    // The battle's map, the captured avatar's, with its terrain for vehicles to rest on.
    let arena_type_id = session.as_ref()
        .and_then(|s| s.battle.as_ref()?.avatar.clone())
        .and_then(AvatarData::from_value)
        .map(|avatar| avatar.arenaTypeID);
    let (map, terrain) = match arena_type_id {
        Some(arena_type_id) => {
            let start = Instant::now();
            let fs = ResFilesystem::new(game_dir.join("res"))
                .map_err(|e| format!("Failed to open resource filesystem: {e}"))?;
            let def = ArenaTypeDef::load_by_type_id(&fs, arena_type_id)
                .map_err(|e| format!("Failed to load the arena type {arena_type_id}: {e}"))?;
            let terrain = Terrain::load(&fs, &def.geometry)
                .map_err(|e| format!("Failed to load the terrain of {}: {e}", def.geometry))?;
            info!("Loaded the map {} and its terrain in {:.1?}", def.name, start.elapsed());
            (Some((arena_type_id, def)), Some(terrain))
        }
        None => (None, None),
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
        map,
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
    session: Option<Arc<CapturedSession>>,
    /// The account's data as served, which the vehicles going to battle are read from.
    account: Option<AccountData>,
    /// The game, whose vehicle definitions complete what the account says.
    game_dir: PathBuf,
    /// The battle's map, by its arena type id.
    map: Option<(i32, ArenaTypeDef)>,
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
    /// The arena period to switch to, and when.
    next_period: Option<(Instant, u8)>,
    /// Its battle, once its avatar is in its space.
    battle: Option<Battle>,
}

/// The arena periods, `ARENA_PERIOD`.
const ARENA_PERIOD_PREBATTLE: u8 = 2;
const ARENA_PERIOD_BATTLE: u8 = 3;
/// The countdown before the battle, and the battle's length.
const PREBATTLE_LENGTH: Duration = Duration::from_secs(10);
const BATTLE_LENGTH: Duration = Duration::from_secs(900);
/// `ARENA_UPDATE.PERIOD`.
const ARENA_UPDATE_PERIOD: u8 = 3;
/// `ARENA_BONUS_TYPE.REGULAR` and `ARENA_GUI_TYPE.RANDOM`: a random battle.
const ARENA_BONUS_TYPE_REGULAR: u8 = 1;
const ARENA_GUI_TYPE_RANDOM: u8 = 1;
/// The tier of the battle.
const BATTLE_LEVEL: i64 = 10;
/// How many players a player may report each day.
const DENUNCIATIONS: i16 = 10;
/// The avatar's dynamic components count, as captured, see [`CreateBasePlayer`].
///
/// [`CreateBasePlayer`]: wgtk::app::client::element::CreateBasePlayer
const AVATAR_COMPONENTS_COUNT: u8 = 2;
/// The avatar's `customizationDisplayType`, as captured.
const CUSTOMIZATION_DISPLAY_TYPE: u8 = 2;

#[derive(Debug)]
struct Shared {
    login_clients: Mutex<HashMap<u32, LoginClient>>,
}

/// How often battles tick, 10 Hz as captured: the client times what it hears by the
/// tick it comes with.
const TICK: Duration = Duration::from_millis(100);

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
        // The client's game time is the ticks it got.
        let game_time = f64::from(self.app.ticks(client).unwrap_or(0)) * TICK.as_secs_f64();
        let end = game_time + length.as_secs_f64();

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
        if let Some(battle) = self.battle_clients.remove(&client).and_then(|c| c.battle) {
            self.app.destroy_space(battle.space());
        }
        self.app.switch_base_app(client, base_addr, true)?;
        info!(%client, "Leaving the battle, switching to {base_addr}");
        Ok(())
    }

    /// Bring a client that reconnected for its battle to its avatar.
    fn enter_battle(&mut self, client: ClientKey) -> io::Result<()> {

        // The captured avatar names the player and the map, until the account and the
        // battle do.
        let Some(captured) = self.session.as_ref()
            .and_then(|s| s.battle.as_ref())
            .and_then(|b| b.avatar.as_ref())
            .and_then(|data| AvatarData::from_value(data.clone())) else {
            warn!(%client, "The capture has no avatar to create");
            return Ok(());
        };

        let Some((arena_type_id, map)) = &self.map else {
            warn!(%client, "No map to battle on");
            return Ok(());
        };
        let data = avatar_data(captured.name, *arena_type_id);
        let avatar = self.app.create_base_player::<Avatar>(client, data, AVATAR_COMPONENTS_COUNT)?;
        if let Some(battle) = self.battle_clients.get_mut(&client) {
            battle.avatar = Some(avatar);
        }
        info!(%client, "Created avatar {avatar}");

        // No `SetCellAppExtAddress`, which the captured server sends: given one, the
        // client opens a direct connection to the cell app and waits for a cell login we
        // can't answer yet, holding back its cell traffic. Without one, everything
        // comes through this base app. No arena updates either, which the client
        // defaults to, see the capture.

        // Then its cell side, in a space of its own with the map, see [`Self::enter_space`].
        let space = self.app.create_space();
        for data in space_properties(space.id()) {
            self.app.set_space_property(space, data)?;
        }
        let SocketAddr::V4(entry_id) = self.app.addr()? else {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "base app not on IPv4"));
        };
        self.app.add_space_geometry_mapping(space, entry_id, &map.geometry)?;
        info!(%client, "Entering space {space} with {}", map.geometry);
        self.enter_space(client, avatar, space)

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

    #[instrument(name = "base", skip_all)]
    fn run(mut self) {

        info!("Running on: {}", self.app.addr().unwrap());

        let mut next_tick = Instant::now() + TICK;

        loop {

            // Every tick due, late ones included, so that no tick number is skipped.
            while Instant::now() >= next_tick {
                next_tick += TICK;
                self.battle_tick();
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
                            self.battle_clients.insert(client, BattleClient { config, avatar: None, next_period: None, battle: None });
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
                base::Event::CellMethod(event) => self.cell_method(event.client, event.entity, &event.call),
                base::Event::Entered(event) => self.entity_entered(event.client, event.entity),
                base::Event::AvatarUpdate(event) => {
                    debug!(client = %event.client, "Avatar update on {:?}: {:?}, direction {:?}", event.vehicle_id, event.position, event.direction);
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

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// A property of the component a captured NRL message creates, if given.
fn captured_property<'m>(message: &'m NrlMessage, name: &str) -> Option<&'m Value> {
    let NrlBody::CreateNode { node: NrlNode::DynComponent { data: Some(data), .. }, .. } = &message.body else {
        return None;
    };
    data.children.iter().find_map(|(_, child)| match child {
        NrlChild::Property { name: child_name, value } if &**child_name == name => Some(value),
        _ => None,
    })
}

/// The base data of a random battle's avatar, for the given player and map: no limits,
/// boosters nor modifiers, and no kill cam, whose data the server sends.
fn avatar_data(name: String, arena_type_id: i32) -> AvatarData {
    use serde_pickle::{HashableValue, Value as PickleValue};
    let extra_data = PickleValue::Dict([
        (HashableValue::Bytes(b"battleLevel".to_vec()), PickleValue::I64(BATTLE_LEVEL)),
        (HashableValue::Bytes(b"bonusCapsOverrides".to_vec()), PickleValue::Dict(Default::default())),
    ].into_iter().collect());
    AvatarData {
        name,
        sessionID: String::new(),
        arenaUniqueID: OsRng.next_u64(),
        arenaTypeID: arena_type_id,
        arenaBonusType: ARENA_BONUS_TYPE_REGULAR,
        arenaGuiType: ARENA_GUI_TYPE_RANDOM,
        arenaExtraData: Pickle(extra_data),
        weatherPresetID: 0,
        denunciationsLeft: DENUNCIATIONS,
        clientCtx: String::new(),
        tkillIsSuspected: false,
        playLimits: PlayLimits { curfew: -1, weeklyPlayLimit: -1, dailyPlayLimit: -1, sessionLimit: -1 },
        battleChatRestriction: BattleChatRestriction { isBattleChatDisabled: false, restrictionReasonID: -1 },
        goodiesSnapshot: Vec::new(),
    }
}

/// The cell data of a random battle's avatar, in the given team on the given vehicle:
/// not observing nor observed, its vehicle at rest until it moves.
fn avatar_cell_data(team: u8, vehicle_id: EntityId) -> AvatarCellData {
    AvatarCellData {
        team,
        playerVehicleID: vehicle_id,
        isGunLocked: false,
        ownVehicleGear: 0,
        ownVehicleAuxPhysicsData: 0,
        ownVehicleHullAimingPitchPacked: 0,
        ammoViews: AmmoViews::default(),
        customizationDisplayType: CUSTOMIZATION_DISPLAY_TYPE,
        isObserverBothTeams: false,
        isObserverFPV: false,
        numOfObservers: 0,
        observableTeamID: 0,
        remoteCamera: RemoteCamera::default(),
        shouldSendKillcamSimulationData: false,
    }
}

/// The properties of a battle's space, as captured: three records the client expects,
/// the space id then fixed bytes, see [`SpaceProperty`].
///
/// [`SpaceProperty`]: wgtk::app::client::element::SpaceProperty
fn space_properties(space_id: u32) -> impl Iterator<Item = Vec<u8>> {
    const RECORDS: [&[u8]; 3] = [
        &[0, 0, 0, 0, 0, 1, 0, 0, 0],
        &[2, 0, 0, 0, 0, 0],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    RECORDS.into_iter().map(move |record| [&space_id.to_le_bytes()[..], record].concat())
}

/// The `targetingInfo` of a vehicle: turret and gun at rest, their speeds and
/// dispersion factors from its definitions, without the crew's and equipment's bonuses.
fn targeting_info(config: &VehicleConfig) -> TargetingInfo {
    let aiming = &config.aiming;
    TargetingInfo {
        turretYaw: 0.0,
        gunPitch: 0.0,
        maxTurretRotationSpeed: aiming.turret_rotation_speed,
        maxGunRotationSpeed: aiming.gun_rotation_speed,
        shotDispMultiplierFactor: 1.0,
        gunShotDispersionFactorsTurretRotation: aiming.dispersion_turret_rotation,
        chassisShotDispersionFactorsMovement: aiming.dispersion_movement,
        chassisShotDispersionFactorsRotation: aiming.dispersion_rotation,
        aimingTime: aiming.aiming_time,
        gunShotDispersionFactorsAfterShot: aiming.dispersion_after_shot,
    }
}

/// The `vehicleSettings` of a vehicle just entering the battle: its first shell, loaded
/// and next.
fn vehicle_settings(entity_id: EntityId, config: &VehicleConfig) -> Vec<VehicleSetting> {
    let shell = config.ammo.first().map_or(0, |&(cd, _)| cd as i32);
    // `VEHICLE_SETTING`: the current shell, then the next one.
    [0, 1].into_iter()
        .map(|code| VehicleSetting { code, value: shell, vehicleID: entity_id })
        .collect()
}

/// The `vehicleHealthInfo` of a vehicle with this health.
fn vehicle_health_info(entity_id: EntityId, health: i64) -> VehicleHealthInfo {
    VehicleHealthInfo {
        deathReasonExtID: -1,
        deathReasonID: -1,
        health: health as i16,
        id: entity_id,
        isCrewActive: true,
        isRespawnActive: false,
    }
}

/// The `vehicleAmmoList` of a vehicle with these shells, by integer descriptor and
/// count, and the standard consumables.
fn ammo_list(shells: impl Iterator<Item = (i64, u32)>) -> Vec<VehicleAmmo> {
    let ammo = |compact_descr: i64, quantity: u16, in_clip: u16, stage: u16, total_time: i16| VehicleAmmo {
        compactDescr: compact_descr as i32,
        endTime: 0.0,
        index: 0,
        previousStage: stage,
        quantity,
        quantityInClip: in_clip,
        totalTime: total_time,
    };
    shells
        .map(|(cd, count)| ammo(cd, count as u16, 0, 0, 0))
        .chain(CONSUMABLES.iter().map(|&cd| ammo(cd, u16::MAX, 3, 3, 60)))
        .collect()
}

/// The `vehicleGunReloadTime` of a gun reloading in `base_time` seconds, done at game
/// time `end_time` with `time_left` seconds to go.
fn gun_reload_time(base_time: f32, end_time: f32, time_left: f32) -> GunReloadTime {
    GunReloadTime { baseTime: base_time, clipTime: 0.0, endTime: end_time, timeLeft: time_left }
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

/// The consumables of the captured battle (repair kit, first aid kit, fire
/// extinguisher), by integer descriptor, which every vehicle gets until accounts have
/// their own.
const CONSUMABLES: [i64; 3] = [1531, 1019, 507];

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
