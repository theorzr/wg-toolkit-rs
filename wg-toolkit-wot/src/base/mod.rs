//! The server's base app, we are trying to reproduce the official server
//! implementation. Its battles are in [`battle`].

mod account;
mod advertise;
mod armor;
mod battle;
mod bot;
#[cfg(test)]
mod drive_replay;
mod gun;
mod physics;
mod player;
mod shop;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::io;

use tracing::{debug, info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use blowfish::cipher::KeyInit;
use blowfish::Blowfish;
use glam::Vec2;

use wgtk::app::entity::{Bytes, EntityId, EntityKey, Pickle};
use wgtk::pickle::{self, Value as PickleValue};
use wgtk::app::dispatch::MethodCall;
use wgtk::res::fs::ResFilesystem;
use wgtk::app::base::ClientKey;
use wgtk::script::Script;
use wgtk::space::Space;
use wgtk::app::base;
use wgtk::pxml;

use crate::wot::schema::{
    self, Account, AccountBase, AmmoViews, Avatar, AvatarBase, AvatarCellData, AvatarData, BattleChatRestriction,
    GunReloadTime, PlayLimits, RemoteCamera, ServerStats, TargetingInfo, VehicleAmmo, VehicleHealthInfo, VehicleSetting,
};
use crate::wot::account_commands::{
    token_type, CMD_ADD_INT_USER_SETTINGS, CMD_ENQUEUE_IN_BATTLE_QUEUE, CMD_LOG_CLIENT_XMPP_EVENTS, CMD_REQ_QUEUE_INFO,
    CMD_REQ_SERVER_STATS, CMD_RESET_SESSION_STAT, CMD_SET_ACTIVE_VEH_SEASON, CMD_SYNC_DATA, CMD_SYNC_DOSSIERS, CMD_SYNC_SHOP, REQUEST_ID_NO_RESPONSE, RES_CACHE, RES_STREAM, RES_SUCCESS,
};
use crate::wot::item_defs::{self, AimingDef, MagazineDef, MobilityDef, ShotDef, VehicleDef};
use crate::{init_tracing, load_script, AppBaseArgs, BaseArgs, CliResult};
use crate::link::{ClientHost, LinkEvent, LinkStream, Message};
use crate::wot::collision::VehicleCollision;
use crate::wot::arena_defs::{ArenaTypeDef, STANDARD_GAMEPLAY};
use crate::wot::item_prices::ItemPrices;
use crate::wot::items::TankmanDescr;

use gun::Gun;
pub use advertise::AdvertiseRule;

use battle::{Battle, BattleOptions};
use player::{AccountIds, Player};
use account::AccountData;
use shop::Shop;


pub fn run(args: AppBaseArgs, game_dir: &Path) -> CliResult<()> {

    init_tracing("base-trace.jsonl")?;

    let script = load_script(game_dir)?;
    let link = LinkStream::connect(args.login_link)
        .map_err(|e| format!("Failed to link with the login app: {e}"))?;
    BaseThread::new(args.bind, script, game_dir, &args.base, link)?.run();

    Ok(())

}


impl BaseThread {

    /// The base app, at this address, taking the clients of the login app it is
    /// linked with.
    pub fn new(
        addr: SocketAddrV4,
        script: Script,
        game_dir: &Path,
        args: &BaseArgs,
        link: LinkStream,
    ) -> CliResult<Self> {

        let mut app = base::App::new(addr.into(), script)
            .map_err(|e| format!("Failed to bind base app: {e}"))?;
        schema::register(&mut app)
            .map_err(|e| format!("Failed to bind the entity schemas: {e}"))?;

        // The battles' map, with its terrain for vehicles to rest on and its static
        // models to collide with.
        let start = Instant::now();
        let fs = ResFilesystem::new(game_dir.join("res"))
            .map_err(|e| format!("Failed to open resource filesystem: {e}"))?;
        let map = ArenaTypeDef::load(&fs, &args.map)
            .map_err(|e| format!("Failed to load the map {}: {e}", args.map))?;
        let space = Space::load(&fs, &map.geometry, STANDARD_GAMEPLAY)
            .map_err(|e| format!("Failed to load the space of {}: {e}", map.geometry))?;
        info!(triangles = space.statics.len(), "Loaded the map {} and its space in {:.1?}", map.name, start.elapsed());

        let load_vehicle = |vehicle: &String| {
            let (nation, name) = vehicle.split_once(':')
                .ok_or_else(|| format!("Expected <nation>:<name>, got {vehicle}"))?;
            VehicleDef::load(&fs, nation, name).map_err(|e| format!("Failed to load vehicle {vehicle}: {e}"))
        };
        let vehicles = args.give_vehicle.iter().map(load_vehicle).collect::<CliResult<Vec<_>>>()?;
        let battle_options = BattleOptions {
            allies: args.allies,
            enemies: args.enemies,
            bot_vehicles: args.bot_vehicle.iter().map(load_vehicle).collect::<CliResult<Vec<_>>>()?,
            bots_fire: args.bots_fire,
            bots_drive: args.bots_drive,
            enemy_distance: args.enemy_distance,
            spawn: args.spawn.map(|(x, z, yaw)| (Vec2::new(x, z), yaw.to_radians())),
        };
        info!(allies = args.allies, enemies = args.enemies, bot_vehicles = ?args.bot_vehicle, bots_fire = args.bots_fire,
            bots_drive = args.bots_drive, enemy_distance = ?args.enemy_distance, spawn = ?args.spawn, "Set up the battles");
        let account = AccountData::new(&fs, &vehicles)
            .map_err(|e| format!("Failed to create the account: {e}"))?;
        for vehicle in &vehicles {
            info!(descr = ?vehicle.descr, ammo = ?vehicle.default_ammo(), magazine = ?vehicle.magazine, "Gave the account a vehicle: {}", vehicle.name);
        }

        let required_version = required_version(&fs)
            .map_err(|e| format!("Failed to read the required client version: {e}"))?;

        let start = Instant::now();
        let prices = ItemPrices::load(&fs)
            .map_err(|e| format!("Failed to read the item prices: {e}"))?;
        info!("Read the prices of {} items, {} not in the shop, in {:.1?}", prices.prices.len(), prices.not_in_shop.len(), start.elapsed());
        let shop = Shop::new(&prices).map_err(|e| format!("Failed to encode the shop: {e}"))?;

        if !args.advertise.is_empty() {
            info!(rules = ?args.advertise, "Set up the addresses told to clients");
        }

        Ok(Self {
            app,
            addr,
            advertise: args.advertise.clone(),
            link,
            load: None,
            login_clients: HashMap::new(),
            account_ids: AccountIds::default(),
            account,
            shop,
            required_version,
            game_dir: game_dir.to_path_buf(),
            map,
            space: Arc::new(space),
            battle_options,
            players: HashMap::new(),
            accounts: HashMap::new(),
            enqueued: HashMap::new(),
            pending_switches: HashMap::new(),
            pending_returns: HashMap::new(),
            battle_clients: HashMap::new(),
        })

    }

}

#[derive(Debug)]
pub struct BaseThread {
    app: base::App,
    /// Where it is bound, and what address it tells its clients, see the `advertise`
    /// module.
    addr: SocketAddrV4,
    advertise: Vec<AdvertiseRule>,
    link: LinkStream,
    /// The load last told to the login app, none when it should be told again.
    load: Option<u32>,
    /// Clients the login app handed over, by the login key they will log in with.
    login_clients: HashMap<u32, LoginClient>,
    /// Each player name's account id, until accounts are stored.
    account_ids: AccountIds,
    /// Every player's garage, until accounts are stored.
    account: AccountData,
    shop: Shop,
    /// The client version required, the client's own.
    required_version: String,
    /// The game, whose vehicle definitions complete what the account says.
    game_dir: PathBuf,
    /// The battles' map, and its space.
    map: ArenaTypeDef,
    space: Arc<Space>,
    /// Who the battles have besides the player.
    battle_options: BattleOptions,
    /// Each logged in client's player.
    players: HashMap<ClientKey, Player>,
    /// Each client's account, outside of battles.
    accounts: HashMap<ClientKey, EntityKey<Account>>,
    /// Clients waiting in the battle queue, with the inventory id of their vehicle.
    enqueued: HashMap<ClientKey, i64>,
    /// Clients told to switch base app for their battle, by the session key they will
    /// log in again with, with the IP they will come from, their blowfish key and
    /// their player and vehicle.
    pending_switches: HashMap<u32, (IpAddr, Arc<Blowfish>, Player, VehicleConfig)>,
    /// Clients told to switch base app back to their account, leaving their battle, by
    /// the session key they will log in with, likewise.
    pending_returns: HashMap<u32, (IpAddr, Arc<Blowfish>, Player)>,
    /// Clients connected for a battle, which get an `Avatar` rather than an `Account`.
    battle_clients: HashMap<ClientKey, BattleClient>,
}

/// A client connected for a battle.
#[derive(Debug)]
struct BattleClient {
    /// Its vehicle's configuration.
    config: VehicleConfig,
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
/// The avatar's `customizationDisplayType`, as captured.
const CUSTOMIZATION_DISPLAY_TYPE: u8 = 2;
/// The account's sessions count, enough not to be a new player, until accounts are
/// stored.
const ACCOUNT_SESSIONS: u64 = 1000;
/// The version of the vehicle dossiers, any: the client drops its cached ones when
/// it differs from theirs.
const DOSSIERS_VERSION: i32 = 1;
/// Why a client is kicked once its player logs in again from another: what it shows,
/// and `ACCOUNT_KICK_REASONS.UNKNOWN`, the reason shown as is.
const KICKED_LOGGED_IN_AGAIN: &str = "Logged in again from another client";
const KICK_REASON_UNKNOWN: u8 = 0;
/// The `UpdateFrequencyNotification`'s field of unknown meaning, as captured.
const UPDATE_FREQUENCY_UNKNOWN: u16 = 1;

/// How often battles tick, 10 Hz as captured: the client times what it hears by the
/// tick it comes with.
const TICK: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct LoginClient {
    addr: SocketAddr,
    blowfish: Arc<Blowfish>,
    player: Player,
}

/// The client the login app handed over, if valid, logging in as the given player.
fn login_client(client: &ClientHost, player: Player) -> Option<LoginClient> {
    match Blowfish::new_from_slice(&client.blowfish_key) {
        Ok(blowfish) => Some(LoginClient {
            addr: client.addr,
            blowfish: Arc::new(blowfish),
            player,
        }),
        Err(_) => {
            warn!(addr = %client.addr, "Refused a client with an invalid blowfish key: {:?}", client.blowfish_key);
            None
        }
    }
}

impl BaseThread {

    /// The client the login app handed over with this login key.
    fn login_client(&mut self, login_key: u32) -> Option<LoginClient> {
        self.login_clients.remove(&login_key)
    }

    /// Handle the events of the link with the login app, and tell it the load if it
    /// changed, returning false once stopped.
    fn link_events(&mut self) -> bool {

        while let Some(event) = self.link.try_event() {
            match event {
                LinkEvent::Linked(()) | LinkEvent::Unlinked(()) => self.load = None,
                LinkEvent::Message((), Message::ClientHost { request_id, client }) => {
                    let player = self.account_ids.player(&client.name);
                    let hosted = login_client(&client, player).map(|login_client| self.login_clients.insert(client.login_key, login_client)).is_some();
                    let base_app = advertise::advertised(&self.advertise, self.addr, client.addr);
                    let message = Message::ClientHosted { request_id, base_app: hosted.then_some(base_app) };
                    if let Err(e) = self.link.send(&message) {
                        warn!(addr = %client.addr, "Failed to answer the login app: {e}");
                    }
                }
                LinkEvent::Message((), Message::BaseStop) => return false,
                LinkEvent::Message((), message) => warn!("Received an unexpected message from the login app: {message:?}"),
            }
        }

        let load = self.players.len() as u32;
        if self.load != Some(load) && self.link.send(&Message::BaseStatus { load }).is_ok() {
            self.load = Some(load);
        }

        true

    }

    /// Kick the clients playing as this player, logging in again from another: told why,
    /// then forgotten.
    fn kick(&mut self, name: &str) {
        let kicked = self.players.iter()
            .filter(|(_, player)| player.name == name)
            .map(|(&client, _)| client)
            .collect::<Vec<_>>();
        for client in kicked {
            if let Err(e) = self.tell_kicked(client) {
                warn!(%client, "Failed to tell a client it is kicked: {e}");
            }
            if let Err(e) = self.app.disconnect(client) {
                warn!(%client, "Failed to disconnect a client: {e}");
            }
            self.forget(client);
            info!(%client, name, "Kicked a client: its player logged in again");
        }
    }

    /// Tell a client it is kicked, through its account or its avatar.
    fn tell_kicked(&mut self, client: ClientKey) -> io::Result<()> {
        let reason = KICKED_LOGGED_IN_AGAIN.to_string();
        if let Some(&account) = self.accounts.get(&client) {
            self.app.entity(account)?.onKickedFromServer(reason, KICK_REASON_UNKNOWN, 0)?;
        } else if let Some(avatar) = self.battle_clients.get(&client).and_then(|battle| battle.avatar) {
            self.app.entity(avatar)?.onKickedFromServer(reason, KICK_REASON_UNKNOWN, 0)?;
        }
        Ok(())
    }

    /// Forget a client gone: its player, its account or its battle, its place in the
    /// queue.
    fn forget(&mut self, client: ClientKey) {
        self.players.remove(&client);
        self.accounts.remove(&client);
        self.enqueued.remove(&client);
        if let Some(battle) = self.battle_clients.remove(&client).and_then(|battle| battle.battle) {
            self.app.destroy_space(battle.space());
        }
    }

    /// Bring a client that enabled entities to its account, as the captured server did:
    /// the `Account` entity, then `showGUI`.
    fn enter_account(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(player) = self.players.get(&client) else { return Ok(()) };

        let id = player.id;
        let data = schema::AccountData {
            name: player.name.clone(),
            incarnationID: ACCOUNT_SESSIONS,
            requiredVersion_2410: self.required_version.clone(),
            initialServerSettings: server_settings(),
        };
        let account = self.app.create_base_player::<Account>(client, data)?;
        self.accounts.insert(client, account);
        info!(%client, %account, "Created the account");

        self.app.entity(account)?.showGUI(show_gui_ctx(id))?;

        Ok(())

    }

    /// The client's arena is ready: tell its account, then switch it to the base app
    /// holding its avatar, this one, as the captured server did.
    fn start_battle(&mut self, client: ClientKey, vehicle_id: i64) -> io::Result<()> {

        let (Some(&account), Some(player), Some((session_key, blowfish))) =
            (self.accounts.get(&client), self.players.get(&client), self.app.client_keys(client)) else {
            return Ok(());
        };
        let player = player.clone();
        let config = match self.vehicle_config(vehicle_id) {
            Ok(config) => config,
            Err(e) => {
                warn!(%client, vehicle_id, "Failed to read the vehicle: {e}");
                return Ok(());
            }
        };

        self.app.entity(account)?.onArenaCreated()?;

        let base_app = advertise::advertised(&self.advertise, self.addr, client.addr());
        self.pending_switches.insert(session_key, (client.addr().ip(), blowfish, player, config));
        self.app.switch_base_app(client, base_app, true)?;
        self.accounts.remove(&client);
        self.players.remove(&client);
        info!(%client, %base_app, "Created the arena, switching base app");

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
        if let (ARENA_PERIOD_BATTLE, Some(battle)) = (period, battle.battle.as_mut()) {
            battle.start();
        }
        // The client's game time is the ticks it got.
        let game_time = f64::from(self.app.ticks(client).unwrap_or(0)) * TICK.as_secs_f64();
        let end = game_time + length.as_secs_f64();

        let info = PickleValue::Tuple(vec![
            PickleValue::I64(period.into()),
            PickleValue::F64(end),
            PickleValue::I64(length.as_secs() as i64),
            PickleValue::List(Vec::new()),
        ]);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        pickle::to_writer(&mut encoder, &info)?;
        let data = encoder.finish()?;

        self.app.entity(avatar)?.updateArena(ARENA_UPDATE_PERIOD, Bytes(data))?;
        info!(%client, "Changed the arena period: {period}, until {end:.1}");

        Ok(())

    }

    /// The configuration of the vehicle of the given inventory id: its descriptor, crew
    /// and setups from the account, the rest from the game's definitions.
    fn vehicle_config(&self, vehicle_id: i64) -> io::Result<VehicleConfig> {
        let vehicle = self.account.vehicle(vehicle_id).ok_or_else(|| invalid_data("no such vehicle"))?;
        let fs = ResFilesystem::new(self.game_dir.join("res"))?;
        let def = VehicleDef::load_by_id(&fs, vehicle.descr.nation, vehicle.descr.type_id)?;
        info!(vehicle_id, "Read the battle vehicle: {}, {} health", def.name, def.max_health);
        Ok(VehicleConfig {
            descr: vehicle.descr.encode(),
            max_health: def.max_health,
            crew: vehicle.crew.iter().map(TankmanDescr::encode).collect(),
            setups: vehicle.setups(),
            setup_indexes: wgtk::pickle!({}),
            mobility: def.mobility,
            ammo: def.default_ammo().into_iter().map(|(cd, count)| (cd.to_int(), count)).collect(),
            shots: def.shots,
            aiming: def.aiming,
            magazine: def.magazine,
            collision: def.collision,
        })
    }

    /// Leave the client's battle, as the captured server does: switch it to the base
    /// app again, where it logs in back to its account.
    fn leave_battle(&mut self, client: ClientKey) -> io::Result<()> {
        let (Some((session_key, blowfish)), Some(player)) = (self.app.client_keys(client), self.players.remove(&client)) else {
            return Ok(());
        };
        let base_app = advertise::advertised(&self.advertise, self.addr, client.addr());
        self.pending_returns.insert(session_key, (client.addr().ip(), blowfish, player));
        if let Some(battle) = self.battle_clients.remove(&client).and_then(|c| c.battle) {
            self.app.destroy_space(battle.space());
        }
        self.app.switch_base_app(client, base_app, true)?;
        info!(%client, %base_app, "Left the battle, switching base app");
        Ok(())
    }

    /// Bring a client that reconnected for its battle to its avatar.
    fn enter_battle(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(player) = self.players.get(&client).cloned() else { return Ok(()) };

        let data = avatar_data(player.name.clone(), self.map.type_id);
        let avatar = self.app.create_base_player::<Avatar>(client, data)?;
        if let Some(battle) = self.battle_clients.get_mut(&client) {
            battle.avatar = Some(avatar);
        }
        info!(%client, %avatar, "Created the avatar");

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
        self.app.add_space_geometry_mapping(space, entry_id, &self.map.geometry)?;
        info!(%client, %space, "Entered the space: {}", self.map.geometry);
        self.enter_space(client, avatar, space, player)

    }

    /// Tell a client just logged in the update frequency, right after the login reply, as
    /// the captured server did.
    fn send_update_frequency(&mut self, client: ClientKey) {
        let frequency = (1.0 / TICK.as_secs_f32()) as u8;
        if let Err(e) = self.app.update_frequency(client, frequency, UPDATE_FREQUENCY_UNKNOWN, 0) {
            warn!(%client, "Failed to send the update frequency: {e}");
        }
    }

    /// A request of the client's account, those we don't know being left unanswered. The
    /// arena is ready as soon as the client first asks how the queue is going.
    fn account_request(&mut self, client: ClientKey, account: EntityKey<Account>, request: AccountBase, name: &str) -> io::Result<()> {
        match request {
            AccountBase::DoCmdInt3 { cmd: CMD_REQ_SERVER_STATS, .. } => {
                let online = self.players.len() as u32;
                self.app.entity(account)?.receiveServerStats(ServerStats { clusterCCU: online, regionCCU: online })?;
            }
            // `[revision, len, crc32]` of the client's cached shop.
            AccountBase::DoCmdInt3 { request_id, cmd: CMD_SYNC_SHOP, arg1: len, arg2: crc32, .. } => {
                if self.shop.is_cached(len, crc32) {
                    self.app.entity(account)?.onCmdResponse(request_id, RES_CACHE, String::new())?;
                } else {
                    let data = self.shop.data.clone();
                    self.stream(client, account, request_id, &data)?;
                }
            }
            // `[revision, crc32 of the cached data, 0]`: a full sync from revision 0, else
            // nothing new, the updates being sent as they happen.
            AccountBase::DoCmdInt3 { request_id, cmd: CMD_SYNC_DATA, arg0: revision, .. } => {
                let current = self.players.get(&client)
                    .ok_or_else(|| invalid_data("Syncing the account of no player"))?
                    .revision;
                if revision == 0 {
                    let data = account::compress(&self.account.data(current))?;
                    self.stream(client, account, request_id, &data)?;
                } else {
                    let ext = pickle_dict([("prevRev", PickleValue::from(revision)), ("rev", PickleValue::from(current))]);
                    self.app.entity(account)?.onCmdResponseExt(request_id, RES_SUCCESS, String::new(), Pickle(ext))?;
                }
            }
            // `[cache version, latest change time in cache, 0]`, answered with the
            // `(cache version, [(vehicle type, change time, dossier)])` changed since,
            // and there are no vehicle dossiers yet.
            AccountBase::DoCmdInt3 { request_id, cmd: CMD_SYNC_DOSSIERS, .. } => {
                let data = account::compress(&wgtk::pickle!((DOSSIERS_VERSION, [])))?;
                self.stream(client, account, request_id, &data)?;
            }
            // `[key, value, ...]` of the settings to set, kept by the client only for now.
            AccountBase::DoCmdIntArr { request_id, cmd: CMD_ADD_INT_USER_SETTINGS, args: settings, .. } => {
                let settings = settings.chunks_exact(2).map(|pair| (PickleValue::from(pair[0]), PickleValue::from(pair[1])));
                self.update(client, account, [("intUserSettings", PickleValue::Dict(settings.collect()))])?;
                self.app.entity(account)?.onCmdResponse(request_id, RES_SUCCESS, String::new())?;
            }
            AccountBase::DoCmdInt3 { cmd: CMD_REQ_QUEUE_INFO, .. } => {
                self.app.entity(account)?.receiveQueueInfo(queue_info())?;
                if let Some(vehicle_id) = self.enqueued.remove(&client) {
                    self.start_battle(client, vehicle_id)?;
                }
            }
            // `[queue type, vehicle inventory id, ...]`.
            AccountBase::DoCmdIntArr { cmd: CMD_ENQUEUE_IN_BATTLE_QUEUE, args: queue_args, .. } => {
                let (Some(&queue_type), Some(&vehicle_id)) = (queue_args.first(), queue_args.get(1)) else {
                    return Err(invalid_data("Missing the queue type or the vehicle"));
                };
                self.enqueued.insert(client, vehicle_id.into());
                self.app.entity(account)?.onEnqueued(queue_type as u8)?;
            }
            // Only logged by the real server, but answered when asked.
            AccountBase::DoCmdIntArrStrArr { request_id, cmd: CMD_LOG_CLIENT_XMPP_EVENTS, .. } => {
                if request_id != REQUEST_ID_NO_RESPONSE {
                    self.app.entity(account)?.onCmdResponse(request_id, RES_SUCCESS, String::new())?;
                }
            }
            AccountBase::RequestToken { request_id, token_type } => {
                let id = self.players.get(&client)
                    .ok_or_else(|| invalid_data("Asking a token for no player"))?
                    .id;
                self.app.entity(account)?.onTokenReceived(request_id, token_type, token_data(token_type, id))?;
            }
            // No chat yet, and the captured server answered nothing.
            AccountBase::ChatCommandFromClient { action, .. } => {
                debug!(%client, "Ignored a chat action: {action}");
            }
            // Accepted without effect, as nothing depends on them yet.
            AccountBase::DoCmdInt2 { request_id, cmd: CMD_SET_ACTIVE_VEH_SEASON, .. }
            | AccountBase::DoCmdStr { request_id, cmd: CMD_RESET_SESSION_STAT, .. } => {
                self.app.entity(account)?.onCmdResponse(request_id, RES_SUCCESS, String::new())?;
            }
            _ => warn!(%client, "Left a request unanswered: {name}"),
        }
        Ok(())
    }

    /// Update the client's account data with the given sections, to its next revision.
    fn update(&mut self, client: ClientKey, account: EntityKey<Account>, diff: impl IntoIterator<Item = (&'static str, PickleValue)>) -> io::Result<()> {
        let player = self.players.get_mut(&client)
            .ok_or_else(|| invalid_data("Updating the account of no player"))?;
        let prev_rev = player.revision;
        player.revision += 1;
        let revs = [("prevRev", PickleValue::from(prev_rev)), ("rev", PickleValue::from(player.revision))];
        self.app.entity(account)?.update(Pickle(pickle_dict(diff.into_iter().chain(revs))))
    }

    /// Answer an account command with a stream of its data, zlib-compressed.
    fn stream(&mut self, client: ClientKey, account: EntityKey<Account>, request_id: i16, data: &[u8]) -> io::Result<()> {
        self.app.entity(account)?.onCmdResponse(request_id, RES_STREAM, String::new())?;
        self.app.push_resource(client, request_id as u16, account::resource_description(data), data)
    }

    #[instrument(name = "base", skip_all)]
    pub fn run(mut self) {

        info!("Running on: {}", self.app.addr().unwrap());

        let mut next_tick = Instant::now() + TICK;

        loop {

            if !self.link_events() {
                info!("Stopped by the login app");
                return;
            }

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
                        warn!(%addr, "Failed to talk with a client: {}", error.error);
                    } else {
                        warn!("Failed to use the socket: {}", error.error);
                    }
                }
                base::Event::Login(login) => {

                    // A client coming back from its battle, see `leave_battle`: it gets its
                    // account again once it enables its entities.
                    if let Some((ip, blowfish, player)) = self.pending_returns.remove(&login.login_key) {
                        if ip != login.addr.ip() {
                            warn!(addr = %login.addr, attempt = login.attempt_num, "Refused a login: invalid address after the switch");
                            continue;
                        }
                        info!(addr = %login.addr, attempt = login.attempt_num, "Logged in, back from the battle");
                        if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                            self.send_update_frequency(client);
                            self.players.insert(client, player);
                        }
                        continue;
                    }

                    // A client coming back for its battle, see `start_battle`.
                    if let Some((ip, blowfish, player, config)) = self.pending_switches.remove(&login.login_key) {
                        if ip != login.addr.ip() {
                            warn!(addr = %login.addr, attempt = login.attempt_num, "Refused a login: invalid address after the switch");
                            continue;
                        }
                        info!(addr = %login.addr, attempt = login.attempt_num, "Logged in for the battle");
                        if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                            self.send_update_frequency(client);
                            self.players.insert(client, player);
                            self.battle_clients.insert(client, BattleClient { config, avatar: None, next_period: None, battle: None });
                        }
                        continue;
                    }

                    let Some(client) = self.login_client(login.login_key) else {
                        warn!(addr = %login.addr, attempt = login.attempt_num, "Refused a login: invalid key");
                        continue;
                    };

                    if client.addr != login.addr {
                        warn!(addr = %login.addr, attempt = login.attempt_num, "Refused a login: invalid address");
                        continue;
                    }
                    
                    info!(addr = %login.addr, attempt = login.attempt_num, name = %client.player.name, "Logged in");
                    let LoginClient { blowfish, player, .. } = client;
                    self.kick(&player.name);
                    if let Some(client) = self.app.answer_login_success(login.addr, blowfish) {
                        self.send_update_frequency(client);
                        self.players.insert(client, player);
                    }

                }
                base::Event::EnableEntities(event) => {
                    info!(client = %event.client, "Enabled entities");
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
                        warn!(client = %event.client, entity = %event.entity, "Received an unknown base method call: {:?}", event.call);
                        continue;
                    };
                    info!(client = %event.client, entity = %event.entity, "Received a base method call: {name} {args:?}");
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
                        None => match self.app.decode::<AccountBase>(event.entity, &event.call) {
                            Some((account, request)) => {
                                if let Err(e) = self.account_request(event.client, account, request, name) {
                                    warn!(client = %event.client, "Failed to answer {name}: {e}");
                                }
                            }
                            None => warn!(client = %event.client, "Left a request unanswered: {name}"),
                        }
                    }
                }
                base::Event::CellMethod(event) => self.cell_method(event.client, event.entity, &event.call),
                base::Event::Entered(event) => self.entity_entered(event.client, event.entity),
                base::Event::AvatarUpdate(event) => {
                    debug!(client = %event.client, vehicle_id = ?event.vehicle_id, "Received an avatar update: {:?}, direction {:?}", event.position, event.direction);
                }
                base::Event::Disconnected(event) => {
                    let name = self.players.get(&event.client).map(|player| player.name.clone());
                    info!(client = %event.client, reason = ?event.reason, ?name, "Disconnected a client");
                    self.forget(event.client);
                }
                base::Event::Timeout => {}

            }

        }

    }

}

/// The configuration of the vehicle a client went to battle with: what the battle tells
/// about it, and what the server simulates it with.
#[derive(Debug, Clone)]
pub struct VehicleConfig {
    descr: Vec<u8>,
    max_health: u32,
    /// Its crew's compact descriptors, in crew order.
    crew: Vec<Vec<u8>>,
    /// Its loadouts and the selected ones, see `account::Vehicle::setups`.
    setups: pickle::Value,
    setup_indexes: pickle::Value,
    mobility: MobilityDef,
    aiming: AimingDef,
    magazine: MagazineDef,
    /// Its shells, by integer descriptor, and how many of each.
    ammo: Vec<(i64, u32)>,
    /// How its gun fires each of its shells, in the order of `ammo`.
    shots: Vec<ShotDef>,
    collision: Arc<VehicleCollision>,
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// The client version the server requires, which the client checks against the
/// default of the same property in its own `AccountVersion.def`, so it is the game's.
fn required_version(fs: &ResFilesystem) -> io::Result<String> {
    let def = item_defs::read_xml(fs, "scripts/entity_defs/interfaces/AccountVersion.def")?;
    def.get_child_value_at(&["Properties", "requiredVersion_2410", "Default"])
        .and_then(pxml::Value::as_string)
        .map(str::to_string)
        .ok_or_else(|| invalid_data("No requiredVersion_2410 in AccountVersion.def, the game is another version"))
}

/// A pickled dict of the given entries, keyed by their names as bytes, as the game's
/// Python 2 strings.
fn pickle_dict(entries: impl IntoIterator<Item = (&'static str, PickleValue)>) -> PickleValue {
    PickleValue::Dict(entries.into_iter().map(|(key, value)| (PickleValue::Bytes(key.as_bytes().to_vec()), value)).collect())
}

/// The server's settings: only what the client reads without a default, every other
/// feature taking the client's own. Game days start at 4:00 UTC and weeks on Mondays,
/// as the captured server's, there is no web content and no ranked battles.
///
/// The configs that the client deserializes into models must be there, with their
/// required fields, else reading them raises: they are the captured server's, without
/// their optional fields and with empty lists. Story mode, fort rush and the newbie
/// hints (accounts have no battles yet) are disabled, and the dates are the captured
/// ones.
fn server_settings() -> Pickle {
    Pickle(wgtk::pickle!({
        "file_server": {},
        "ranked_config": {"isEnabled": false},
        "regional_settings": {
            "starting_day_of_a_new_week": 0,
            "starting_time_of_a_new_day": (4 * 3600),
            "starting_time_of_a_new_game_day": (4 * 3600),
        },
        "armor_flashlight_config": {"enabled": true},
        "armor_inspector_config": {"enabled": true},
        "commendations_config": {"isCommendationsEnabled": true, "isLiveTagsEnabled": true},
        "fort_rush_battles_config": {
            "isEnabled": false,
            "technicalPause": false,
            "isInfoPageEnabled": false,
            "startDatetime": "2024-06-06T02:00:00+00:00",
            "endDatetime": "2028-07-03T22:00:00+00:00",
            "eligibleVehicleTiers": "10",
            "baseProgressionToken": "frevent:progression_token",
            "completedProgressionToken": "frevent:progression_completed_token",
            "progressionQuestPrefix": "frevent:progression:",
            "maxRewardsAmountPerStage": 3,
            "progression": {"stage": []},
            "battleEconomy": {
                "creditsByPlace": {
                    "win": "25000 20000 15000 10000 10000 10000 5000",
                    "draw": "20000 15000 10000 5000 5000 5000 1000",
                    "loss": "20000 15000 10000 5000 5000 5000 1000",
                },
                "progressionPointsByPlace": {"win": "6 6 6 4 4 4 2", "draw": "4 4 4 2 2 2 0", "loss": "4 4 4 2 2 2 0"},
            },
            "dailyMissionsConfig": {
                "dailyGroupsCount": 5,
                "dailyMissionQuestPrefix": "fort_rush_daily_mission:",
                "dailyMissionTokenPostfix": "{group}:{num}",
                "dailyMissionTokenPrefix": "fort_rush_daily_token:",
                "dailyMissionTokenTtl": 24,
                "dailyQuestMaxDays": 12,
                "hardDailyMissionQuestPrefix": "fort_rush_hard_daily_mission:",
                "tokenLimit": 1,
            },
        },
        "halloween_anomalies_config": {"anomalies": {"anomaly": []}, "recipes": {"recipe": []}},
        "halloween_bestiary_config": {"enemies": {"enemy": []}},
        "halloween_gsw_config": {"cards": {"card": []}},
        "halloween_loot_config": {"loot": []},
        "newbie_battle_hints_config": {"enabled": false},
        "newbie_prebattle_hints_config": {"enabled": false, "hintDisplayCount": 2},
        "oit_availability_config": {},
        "player_satisfaction_config": {"enabled": true, "enabledInterfaces": {}},
        "prebattle_hints_config": {"enabled": true, "battleTimerThreshold": 5},
        "prefab_effects_availability_config": {},
        "story_mode_battle_mgr_quotums": {"quotums": []},
        "story_mode_missions": {"missions": [], "onboarding": {}},
        "story_mode_settings": {
            "enabled": false,
            "waitTimeQueue": 20,
            "hideGameLoadingTimeout": 20,
            "modeSelectorCardColumn": 3,
            "modeSelectorCardPriority": 50,
            "entryPoint": {"eventStartAt": "2024-06-06T02:00:00+00:00", "eventEndAt": "2024-07-03T22:00:00+00:00"},
            "newbieBannerEnabled": false,
            "newbieAdvertisingEnabled": false,
            "parallaxEnabled": true,
            "eventName": "story_mode_vday",
        },
        "umgConfig": {"enableAllDaily": 1, "enableAllWeekly": 1, "enableDailyWeeklyUI": 1, "enablePM3Banner": 1, "slides": []},
        "umgEvents": {"weights": []},
        "umgMissions": {"weights": []},
        "veh_playlists_config": {"isVehPlaylistsEnabled": true},
        "weekly_quests_config": {"enabled": true, "rerollTimeout": 14400},
    }))
}

/// The `showGUI` context of a player's account, of this id: the server's time and the
/// session's start, now, and no play time limits nor UI statistics.
fn show_gui_ctx(account_id: u64) -> Pickle {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    Pickle(pickle_dict([
        ("aogasStartedAt", PickleValue::F64(now.as_secs_f64())),
        ("collectUiStats", PickleValue::Bool(false)),
        ("databaseID", PickleValue::I64(account_id as i64)),
        ("isAogasEnabled", PickleValue::Bool(false)),
        ("logUXEvents", PickleValue::Bool(false)),
        ("serverUTC", PickleValue::F64(now.as_secs_f64())),
        ("sessionStartedAt", PickleValue::I64(now.as_secs() as i64)),
    ]))
}

/// The random battle queue as the player sees it: alone, the battle starting at once,
/// so no players counted by vehicle class (light, medium, heavy, SPG, AT-SPG).
fn queue_info() -> Pickle {
    Pickle(pickle_dict([
        ("classes", PickleValue::List(vec![PickleValue::I64(0); 5])),
        ("players", PickleValue::I64(1)),
    ]))
}

/// A token of the given type for the player of this account id, to log into a web
/// service: there are none, so it is empty, of the type the client expects, an int for
/// the chat's.
fn token_data(token_type: u8, account_id: u64) -> Pickle {
    let token = match token_type {
        token_type::XMPPCS => PickleValue::I64(0),
        _ => PickleValue::Bytes(Vec::new()),
    };
    Pickle(pickle_dict([
        ("databaseID", PickleValue::I64(account_id as i64)),
        ("token", token),
    ]))
}

/// The base data of a random battle's avatar, for the given player and map: no limits,
/// boosters nor modifiers, and no kill cam, whose data the server sends.
fn avatar_data(name: String, arena_type_id: i32) -> AvatarData {
    let extra_data = PickleValue::Dict([
        (PickleValue::Bytes(b"battleLevel".to_vec()), PickleValue::I64(BATTLE_LEVEL)),
        (PickleValue::Bytes(b"bonusCapsOverrides".to_vec()), PickleValue::Dict(Default::default())),
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

/// `VEHICLE_SETTING`: the shell loaded, the shell loaded after it, whether the hull
/// turns with the turret in sniper mode, reloading a magazine that is not full, and
/// emptying a full one for the mechanics that do.
const SETTING_CURRENT_SHELLS: u8 = 0;
const SETTING_NEXT_SHELLS: u8 = 1;
const SETTING_AUTOROTATION: u8 = 2;
const SETTING_RELOAD_PARTIAL_CLIP: u8 = 17;
const SETTING_RESET_FULL_CLIP: u8 = 18;

/// The `vehicleSettings` of a vehicle with this gun: its shell loaded, then the next one.
fn vehicle_settings(entity_id: EntityId, gun: &Gun) -> Vec<VehicleSetting> {
    let (loaded, next) = gun.shells();
    [(SETTING_CURRENT_SHELLS, loaded), (SETTING_NEXT_SHELLS, next)].into_iter()
        .map(|(code, shell)| VehicleSetting { code, value: shell.map_or(0, |def| def.compact_descr.to_int() as i32), vehicleID: entity_id })
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

/// The `vehicleAmmoList` of a vehicle with these shells, by integer descriptor, count
/// and count in the magazine, and the standard consumables.
fn ammo_list(shells: impl Iterator<Item = (i64, u32, u32)>) -> Vec<VehicleAmmo> {
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
        .map(|(cd, count, in_magazine)| ammo(cd, count as u16, in_magazine as u16, 0, 0))
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
