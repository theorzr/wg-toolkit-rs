//! The battles of the server: each in a space of its own, with two teams of its clients'
//! vehicles and bots, see [`BattleOptions`]: every vehicle the same, but who drives it,
//! see [`Driver`].
//!
//! A battle gathers the clients the queue switched into it, waiting for them to enter,
//! see [`Gathering`], then is made with them all at once: their vehicles, each one's own
//! arena info, and the bots. It counts down once they are all ready, done loading, and
//! goes on while any client is in it, a vehicle whose client left standing where it
//! stopped, until a team has no vehicle left alive or its time is up. Over, it tells its
//! clients who won, then is done with them a little later: they go back to their garage,
//! see [`period`].
//!
//! A battle holds all it needs, and only tells its clients what happens through the app
//! it is given, so that an app may run several, a thread of its own each, say. The base
//! app only finds a client's battle, see [`crate::base`].
//!
//! ```text
//!   (here)       a battle: made, its tick, its shells landing, its clients' calls
//!   participant  a client in it: its avatar, its vehicle, what it sees
//!   spotting     what each team spots of the other's vehicles
//!   period       waiting, counting down, fighting
//!   vehicle      a vehicle of it, a player's or a bot's: shown, its gun, damaged
//!   bot          the server driving a vehicle as a dev orders it: where, at what
//!   shells       its shells in flight, and their tracers
//!   hit          what a shell does to a vehicle, as the clients are told
//!   areas        what broke of its destructibles, by chunk
//!   spawn        where its vehicles start
//!   chat         its clients' messages, and the server's commands
//! ```

mod participant;
mod spotting;
mod period;
mod vehicle;
mod bot;
mod shells;
mod hit;
mod areas;
mod spawn;
mod chat;

use std::time::{Duration, Instant};
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::{Vec2, Vec3, Vec4};
use clap::ValueEnum;

use wgtk::app::base::{self, CellPlayerInit, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::app::entity::{AnyEntity, Audience, EntityId, EntityKey};
use wgtk::space::Space;

use crate::game::schema::{
    Avatar, AvatarCell, AvatarComponentsData, AvatarInfo, AvatarVehiclesInfo, ChatArgs, ShotResult, VehicleCell,
};
use crate::base::{
    avatar_cell_data, VehicleLoadout, EXPLOSION_STEP, GROUND_EFFECT_MATERIAL, NO_PREFAB_EFFECTS, SETTING_SIEGE_MODE,
    TICK,
};
use crate::base::ballistics::shot::{Shot, ShotLanding, ShotObstacles, ShotTarget, DEBUG_SHOT_SECONDS};
use crate::base::debug::{self, DebugChannel, DebugChannels, DebugFrame};
use crate::base::physics::{ground_at, Vehicle, VehicleId, World};
use crate::base::ballistics::ricochet::{self, GroundMaterial};
use crate::base::ballistics::impact::{self, ImpactOutcome};
use crate::game::item_defs::{ShellKind, VehicleDef};
use crate::game::collision::VehicleIntersection;
use crate::game::arena_defs::ArenaTypeDef;
use crate::base::garage::player::Player;

use bot::{Bot, BotAlly, BotFire, BotFireMode, BotRoute, BotTravel};
use areas::{explosion_obstacle, DestructibleAreas};
use vehicle::{hold_gun, BattleVehicle, Driver};
use shells::{show_shot, show_tracer, Shells};
use hit::{ShellHit, HIT_FLAG_VEHICLE_KILLED};
use period::{ArenaPeriod, NO_WINNER};
use participant::Participant;
use spotting::Spotting;
use spawn::TeamStart;



/// The two teams, as the clients know them.
const TEAMS: [u8; 2] = [1, 2];

/// The scale of the packed horizontal offsets in position updates, as captured: we only
/// send detailed ones.
const PACKED_XZ_SCALE: f32 = 0.007644442;

/// The farthest a vehicle spots others, in metres on the ground plane, whatever its own
/// view range: `VISIBILITY.MAX_RADIUS`. A bot fires at those it sees itself as far.
pub const VIEW_RANGE: f32 = 445.0;

/// How long a battle waits for the clients switched into it to enter before it is made
/// with those that did. Our choice: a client logs in again within a second or two.
const GATHER_WAIT: Duration = Duration::from_secs(20);

/// How far past the point under a participant's gun marker its gun's line goes to find
/// the vehicle there, in metres, see [`Battle::marked_spot`]: the client gives a point
/// on its surface.
const MARKED_DEPTH: f32 = 2.0;

/// How a shell's hit on a vehicle is drawn to be debugged, in [`DebugChannel::Shells`],
/// see [`debug_hit`]: a text this many metres above where it hit, white.
const DEBUG_HIT_RISE: f32 = 1.0;
const DEBUG_HIT_TEXT: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);


/// A battle, with its clients, see the module's documentation.
#[derive(Debug)]
pub struct Battle {
    /// Its id, which its clients are switched into it with.
    id: u32,
    /// The app's space it is in, its entities' and its clients' cells'.
    space: SpaceKey,
    /// When it was made, at 0: the battle's clock.
    clock: Instant,
    /// Its clients, each driving one of its vehicles.
    participants: Vec<Participant>,
    /// Its period: waiting for its clients, counting down, fighting or over, see
    /// [`ArenaPeriod::next`].
    period: ArenaPeriod,
    /// Its vehicles, its clients' first, then the bots on either side: their index here
    /// is their `vehiclesInfo` entry's.
    vehicles: Vec<BattleVehicle>,
    /// The world its vehicles move in, on its map's space: their hulls and turrets, and
    /// what broke of the map.
    world: World,
    /// What each team spots of the other's vehicles, which its clients see.
    spotting: Spotting,
    /// The route its bots follow, which its devs trace with `/route`.
    route: BotRoute,
    /// The shells in flight, until they land.
    shells: Shells,
    /// The destructibles' entities of its map's chunks, telling the clients what broke.
    areas: DestructibleAreas,
    /// What it draws in its clients' world to be debugged, if its options say so.
    debug: Option<DebugFrame>,
    /// What shells hit, refilled as they fly, kept to reuse its memory.
    targets: Vec<ShotTarget>,
    /// Whether a client has its vehicle: it runs from then on.
    running: bool,
}

/// How big a battle's teams are, and what it draws to be debugged, as the server is
/// launched with.
#[derive(Debug, Clone)]
pub struct BattleOptions {
    /// How many vehicles each team has, its clients' then bots.
    pub team_size: u32,
    /// The vehicles bots drive, each the next, round: the first client's own when none.
    pub bot_vehicles: Vec<VehicleDef>,
    /// What it draws to be debugged in its clients' world as it starts, see
    /// [`DebugFrame`]: its clients' `/debug` changes it, see [`Battle::command`].
    pub debug_draw: DebugChannels,
    /// How far in front of the first team the second starts, if not at its spawn.
    pub enemy_distance: Option<f32>,
    /// Where the first team starts, if not at its spawn, and its yaw in radians.
    pub spawn: Option<(Vec2, f32)>,
}

/// What a battle is made of, its clients aside.
#[derive(Debug)]
pub struct BattleSetup<'a> {
    /// Its map: its bounds, its bases and spawn points.
    pub map: &'a ArenaTypeDef,
    /// The map's space, which its vehicles move in: its terrain and static models.
    pub space: Arc<Space>,
    /// How big its teams are, and what it draws to be debugged.
    pub options: BattleOptions,
    /// The vehicle bots drive when the options give none: the first client's.
    pub own_def: Option<VehicleDef>,
}

/// A client entering a battle, with its avatar.
#[derive(Debug)]
pub struct Arrival {
    /// The client, connected again to this base app for the battle.
    pub client: ClientKey,
    /// Its avatar, created for it as it entered, base side only until the battle is made.
    pub avatar: EntityKey<Avatar>,
    /// Who plays it: the name and id the others see.
    pub player: Player,
    /// The vehicle it goes to battle with.
    pub loadout: VehicleLoadout,
}

/// A battle waiting for its clients to enter before it is made, as the base app gathers
/// them, see [`crate::base`].
#[derive(Debug)]
pub struct Gathering {
    /// How many clients were switched into it.
    pub expected: usize,
    /// Those that entered so far, in the order they did.
    pub arrivals: Vec<Arrival>,
    /// When it started gathering, which [`GATHER_WAIT`] counts from.
    since: Instant,
}

impl Gathering {

    /// A battle's gathering, for this many clients switched into it.
    pub fn new(expected: usize) -> Self {
        Self {
            expected,
            arrivals: Vec::new(),
            since: Instant::now(),
        }
    }

    /// Whether the battle is made now: all its clients entered, or those that did after
    /// [`GATHER_WAIT`].
    pub fn due(&self) -> bool {
        self.arrivals.len() >= self.expected || self.since.elapsed() >= GATHER_WAIT
    }

}

impl Battle {

    /// A battle in its own space, with its map, made for these clients entering it with
    /// their avatars: their vehicles, on either team in turn, in rows from their team's
    /// start, their avatars' cell sides on them, then the bots filling each team, and each
    /// client's arena info entity, which each client gets in that order. Each sees its
    /// allies and the enemies its team spots, see [`Participant::spot`].
    pub fn new(app: &mut base::App, setup: BattleSetup, space: SpaceKey, id: u32, arrivals: Vec<Arrival>) -> io::Result<Self> {

        let BattleSetup { map, space: map_space, options, own_def } = setup;
        let terrain = &map_space.terrain;
        let mut world = World::new(map_space.clone(), Some(map.bounds));

        // The clients' vehicles, on either team in turn, from their team's start, where the
        // first of the team stands then: the first team's where the options spawn it, the
        // second's in front of it if the options say so.
        let first_start = match options.spawn {
            Some((at, yaw)) => TeamStart::at(at, yaw, terrain),
            None => TeamStart::team(map, terrain, TEAMS[0]),
        };
        let second_start = match options.enemy_distance {
            Some(distance) => first_start.facing(distance, terrain),
            None => TeamStart::team(map, terrain, TEAMS[1]),
        };
        let mut starts = [first_start, second_start];
        let mut team_sizes = [0; 2];
        let mut vehicles = Vec::new();
        for (index, arrival) in arrivals.iter().enumerate() {

            let Arrival { client, avatar, player, loadout } = arrival;
            let side = index % 2;
            let slot = team_sizes[side];
            let at = starts[side].slot(slot, terrain);
            let info_index = vehicles.len() as u32;
            let vehicle = BattleVehicle::player(app, &mut world, *client, *avatar, space, loadout, player.clone(), TEAMS[side], at, info_index)?;

            if slot == 0 {
                let hull = &world.vehicle(vehicle.id).hull;
                starts[side] = TeamStart {
                    position: hull.position,
                    direction: hull.direction(),
                };
                if let Some(distance) = options.enemy_distance.filter(|_| side == 0) {
                    starts[1] = starts[0].facing(distance, terrain);
                }
            }

            team_sizes[side] += 1;
            vehicles.push(vehicle);

        }

        for (vehicle, arrival) in vehicles.iter().zip(&arrivals) {
            let hull = &world.vehicle(vehicle.id).hull;
            app.create_cell_player(arrival.avatar, space, CellPlayerInit {
                vehicle_id: vehicle.entity.id().0,
                position: hull.position,
                direction: hull.direction(),
                packed_xz_scale: PACKED_XZ_SCALE,
                data: avatar_cell_data(vehicle.team, vehicle.entity.id()),
                components_data: AvatarComponentsData::default(),
            })?;
            app.set_interest(arrival.client, Interest::Manual)?;
        }

        // The bots filling each team, the first's then the second's, in rows from its
        // start after its clients. They drive the given vehicles in turn, else the first
        // client's.
        let bots = (0..2).flat_map(|side| (team_sizes[side]..options.team_size).map(move |slot| (side, slot)));
        for (index, (side, slot)) in bots.enumerate() {
            let (def, descr) = match (&own_def, arrivals.first()) {
                (Some(def), Some(first)) => (def, first.loadout.descr.clone()),
                _ => {
                    let def = &options.bot_vehicles[index % options.bot_vehicles.len()];
                    (def, def.descr.encode())
                }
            };
            let team = TEAMS[side];
            let at = starts[side].slot(slot, terrain);
            let name = format!("Bot {}", index + 1);
            let info_index = vehicles.len() as u32;
            let bot = BattleVehicle::bot(
                app,
                &mut world,
                space,
                def,
                &descr,
                team,
                name,
                at,
                info_index,
                index as i32,
            )?;
            info!(battle = id, entity_id = %bot.entity.id(), team, "Added a bot to the arena: {}, {}", bot.info.name, def.name);
            vehicles.push(bot);
        }

        // Each client's arena info entity, its own only, created right away as entities
        // without an alias are, with the vehicles.
        let mut participants = Vec::with_capacity(arrivals.len());
        for (index, Arrival { client, avatar, player, loadout }) in arrivals.into_iter().enumerate() {
            let info = app.create_entity::<AvatarInfo>(space, EntityInit {
                controlled_by: Some(avatar.any()),
                visibility: Visibility::Interest,
                ..EntityInit::default()
            })?;
            let mut entity = app.entity(info)?;
            entity.set_avatarID(avatar.id())?;
            let mut vehicles_info = entity.add_component::<AvatarVehiclesInfo>(Audience::AllClients)?;
            vehicles_info.set_vehiclesInfo(vehicles.iter().map(|vehicle| vehicle.info.clone()).collect())?;
            info!(battle = id, %client, entity_id = %info.id(), "Created the arena info");
            participants.push(Participant::new(
                client,
                avatar,
                loadout,
                index,
                vehicles_info.key(),
                vehicles.len(),
                player.dev,
            ));
        }

        let areas = DestructibleAreas::new(app, space, &world.space().statics)?;
        info!(battle = id, chunks = areas.len(), "Created the destructibles' areas");
        
        let center = (map.bounds.0 + map.bounds.1) / 2.0;
        let debug = DebugFrame::new(app, space, Vec3::new(center.x, 0.0, center.y), options.debug_draw)?;

        Ok(Self {
            id,
            space,
            clock: Instant::now(),
            participants,
            period: ArenaPeriod::new(),
            spotting: Spotting::new(vehicles.len()),
            route: BotRoute::default(),
            vehicles,
            world,
            shells: Shells::default(),
            areas,
            debug: Some(debug),
            targets: Vec::new(),
            running: false,
        })

    }

    /// The space it is in.
    pub fn space(&self) -> SpaceKey {
        self.space
    }

    /// Whether no client is left in it.
    pub fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }

    /// Whether it has been over long enough for its clients to go back to their garage,
    /// see [`ArenaPeriod::done`].
    pub fn is_done(&self) -> bool {
        self.period.done()
    }

    /// Its clients.
    pub fn clients(&self) -> impl Iterator<Item = ClientKey> + '_ {
        self.participants.iter().map(|participant| participant.client)
    }

    /// The winner once a team has no vehicle left alive: the other team, or nobody
    /// ([`NO_WINNER`]) if neither has.
    fn exterminated(&self) -> Option<u8> {
        let [first, second] = TEAMS.map(|team| self.vehicles.iter().any(|vehicle| vehicle.team == team && vehicle.alive()));
        if first && second {
            None
        } else if first {
            Some(TEAMS[0])
        } else if second {
            Some(TEAMS[1])
        } else {
            Some(NO_WINNER)
        }
    }

    /// Its game time, in seconds.
    fn game_time(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// Its clients' avatars, which are shown what happens.
    fn avatars(&self) -> Vec<EntityKey<Avatar>> {
        self.participants.iter().map(|participant| participant.avatar).collect()
    }

    /// The simulated vehicle a client drives.
    fn own_vehicle_mut(&mut self, client: ClientKey) -> Option<&mut Vehicle> {
        let id = self.vehicles[self.own(client)?].id;
        Some(self.world.vehicle_mut(id))
    }

    /// The index of the vehicle a client drives.
    fn own(&self, client: ClientKey) -> Option<usize> {
        self.participants.iter()
            .find(|participant| participant.client == client)
            .map(|participant| participant.vehicle)
    }

    /// The vehicles the shells of `shooter` can hit: every one alive but its own.
    fn shell_targets(&self, shooter: EntityId) -> Vec<ShotTarget> {
        self.vehicles.iter()
            .filter(|vehicle| vehicle.alive() && vehicle.entity.id() != shooter)
            .map(|vehicle| vehicle.shot_target(&self.world))
            .collect()
    }

    // -- Its clients --

    /// A client is ready, done loading: once all are, it counts down, see [`Self::tick`].
    /// One ready once it counted down already is told where it is.
    pub fn ready(&mut self, app: &mut base::App, client: ClientKey) -> io::Result<()> {
        let period = self.period;
        let Some(participant) = self.participants.iter_mut().find(|participant| participant.client == client) else { return Ok(()) };
        participant.ready = true;
        participant.show_period(app, period)?;
        chat::init(app, participant.avatar)
    }

    /// A client leaves it: its vehicle stops, and stands there, nobody driving it.
    pub fn leave(&mut self, client: ClientKey) {
        let Some(at) = self.participants.iter().position(|participant| participant.client == client) else { return };
        let participant = self.participants.remove(at);
        let vehicle = &mut self.vehicles[participant.vehicle];
        vehicle.driver = Driver::Nobody;
        self.world.vehicle_mut(vehicle.id).hull.flags = 0;
        info!(battle = self.id, %client, left = self.participants.len(), "Left a battle");
    }

    /// An entity of it was created on a client: its own vehicle, the client drives it
    /// from there on, see [`BattleVehicle::enter`], and it runs.
    pub fn entity_entered(&mut self, app: &mut base::App, client: ClientKey, entity: EntityKey<AnyEntity>) -> io::Result<()> {
        debug!(battle = self.id, %client, %entity, "Created an entity on the client");
        let Some(participant) = self.participants.iter().find(|participant| participant.client == client) else { return Ok(()) };
        let own = &mut self.vehicles[participant.vehicle];
        if own.entity.any() != entity {
            return Ok(());
        }
        own.enter(app, &participant.loadout)?;
        self.running = true;
        Ok(())
    }

    /// A cell method of its avatar a client called.
    pub fn avatar_call(&mut self, app: &mut base::App, client: ClientKey, call: AvatarCell) {

        match call {
            AvatarCell::VehicleShoot {} => {
                if let Err(e) = self.shoot(app, client) {
                    warn!(%client, "Failed to shoot: {e}");
                }
                return;
            }
            AvatarCell::VehicleChangeSetting { code, value } => {
                if let Err(e) = self.change_setting(app, client, code, value) {
                    warn!(%client, "Failed to change setting {code} to {value}: {e}");
                }
                return;
            }
            _ => {}
        }

        let Some(driven) = self.own_vehicle_mut(client) else {
            warn!(%client, "Ignored a cell method call without a vehicle: {call:?}");
            return;
        };

        match call {
            AvatarCell::VehicleShoot {} |
            AvatarCell::VehicleChangeSetting { .. } => unreachable!(),
            AvatarCell::VehicleMoveWith { flags } => {
                info!(%client, "Changed the movement flags: {flags:#04x}");
                driven.hull.flags = flags;
            }
            AvatarCell::VehicleStopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }

    }

    /// Fire a client's gun, if loaded and the battle is on: the shell's tracer and the
    /// gun's effects, of its whole burst, then one shell fewer, the shell it loads next
    /// and the reload, once the burst's other shells are fired too, see [`Self::step`].
    /// Where the shell lands follows with [`Self::land`].
    fn shoot(&mut self, app: &mut base::App, client: ClientKey) -> io::Result<()> {

        let now = self.game_time();
        if !self.period.fighting() {
            debug!(%client, "Refused a shot: the battle is not on");
            return Ok(());
        }
        let Some(own) = self.own(client).filter(|&own| self.vehicles[own].alive()) else { return Ok(()) };
        let own_id = self.vehicles[own].entity.id();
        let avatars = self.avatars();
        let Self { vehicles, world, shells, .. } = self;
        let own = &mut vehicles[own];
        let driven = world.vehicle(own.id);
        let (position, velocity, dispersion) = driven.turret.gun_marker(&driven.hull);
        let Some(shot) = own.gun.fire(now, position, velocity, dispersion) else {
            debug!(%client, "Refused a shot: not loaded, or out of shells");
            return Ok(());
        };
        info!(%client, shot = shot.id, burst = shot.burst, "Fired a shell from {:?} at {:?}", shot.start, shot.start_velocity);
        own.fired(now);
        own.show_gun(app, now)?;
        show_shot(app, &avatars, own.entity, &shot)?;
        shells.fly(now, own_id, shot);
        Ok(())

    }

    /// A client changes one of its vehicle's settings, see
    /// [`BattleVehicle::change_setting`], or switches its siege mode, see
    /// [`BattleVehicle::switch_siege`].
    fn change_setting(&mut self, app: &mut base::App, client: ClientKey, code: u8, value: i32) -> io::Result<()> {
        
        let now = self.game_time();
        let Some(own) = self.own(client) else {
            warn!(%client, "Ignored a setting without a vehicle: {code} = {value}");
            return Ok(());
        };

        let vehicle = &mut self.vehicles[own];
        if code == SETTING_SIEGE_MODE {
            let driven = self.world.vehicle_mut(vehicle.id);
            return vehicle.switch_siege(app, driven, client, now, value != 0);
        }

        vehicle.change_setting(app, client, now, code, value)
        
    }

    /// A cell method of its vehicle a client called.
    pub fn vehicle_call(&mut self, client: ClientKey, call: VehicleCell) {

        let Some(driven) = self.own_vehicle_mut(client) else {
            warn!(%client, "Ignored a cell method call without a vehicle: {call:?}");
            return;
        };

        match call {
            VehicleCell::TrackRelativePointWithGun { point } => {
                let Vehicle { hull, turret } = driven;
                debug!(%client, "Tracked a point with the gun: {point:?}, from {:?}", hull.position);
                turret.track(hull.position + point);
            }
            VehicleCell::StopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }

    }

    // -- Its chat --

    /// A chat action of a client, see the `chat` module: its battle chat set up again, or
    /// a message, to its team or to all, shown to each it is for, or a command to the
    /// server, a line starting with `/`, see [`Self::command`], its request answered.
    pub fn chat(
        &mut self,
        app: &mut base::App,
        client: ClientKey,
        action_id: i16,
        request_id: u16,
        args: ChatArgs,
    ) -> io::Result<()> {

        let Some(from) = self.participants.iter().position(|participant| participant.client == client) else {
            return Ok(());
        };
        let avatar = self.participants[from].avatar;

        match action_id {
            chat::REINIT_BATTLE_CHAT => chat::init(app, avatar),
            chat::BROADCAST_BATTLE_MESSAGE => {

                let text = String::from_utf8_lossy(&args.strArg1.0);
                let common = args.int32Arg1 != 0;
                match text.strip_prefix('/') {
                    Some(line) => self.command(app, from, line)?,
                    None => self.broadcast(app, from, &text, common)?,
                }

                chat::answer(app, avatar, request_id, action_id)

            }
            _ => {
                debug!(battle = self.id, %client, action_id, ?args, "Ignored a chat action");
                Ok(())
            }
        }

    }

    /// Show a participant's message to its team, or to all if `common`, as from its
    /// vehicle.
    fn broadcast(&self, app: &mut base::App, from: usize, text: &str, common: bool) -> io::Result<()> {

        let sender = &self.vehicles[self.participants[from].vehicle];
        info!(battle = self.id, entity_id = %sender.entity.id(), common, "Sent a chat message: {text}");

        let shown = self.participants.iter()
            .filter(|participant| common || self.vehicles[participant.vehicle].team == sender.team);
        for participant in shown {
            chat::message(app, participant.avatar, sender.entity.id(), common, text)?;
        }
        Ok(())

    }

    /// A participant's command to the server, its chat line after its `/`, answered to it
    /// alone, as a message of the server, see `chat::server_message`:
    ///
    /// ```text
    ///   /debug                 the debug drawing's channels, see `debug_command`
    ///   /debug <channel>...    each of these channels drawn if it wasn't, else no more
    ///   /debug off             none drawn
    ///   /bot ...               orders to bots, see `bot_command`
    ///   /route ...             the route bots follow, see `route_command`
    /// ```
    fn command(&mut self, app: &mut base::App, from: usize, line: &str) -> io::Result<()> {

        let mut words = line.split_whitespace();
        let reply = match words.next() {
            Some("debug") => self.debug_command(from, words),
            Some("bot") => self.bot_command(from, words),
            Some("route") => self.route_command(from, words),
            Some(name) => Err(format!("Unknown command: /{name}")),
            None => Err("No command after /".to_string()),
        };

        let avatar = self.participants[from].avatar;
        match reply {
            Ok(text) => chat::server_message(app, avatar, &text, false),
            Err(text) => chat::server_message(app, avatar, &text, true),
        }

    }

    /// The `/debug` command of a participant, with the words after it, see
    /// [`Self::command`], for devs only: the channels drawn after it, of all, else what
    /// is wrong. None changes if one of its words is no channel.
    fn debug_command<'a>(&mut self, from: usize, words: impl Iterator<Item = &'a str>) -> Result<String, String> {

        if !self.participants[from].dev {
            return Err("Only devs may use /debug".to_string());
        }
        let Some(frame) = &mut self.debug else {
            return Err("No debug drawing in this battle".to_string());
        };

        let mut channels = frame.channels();
        for word in words {
            if word == "off" {
                channels = DebugChannels::default();
                continue;
            }
            match DebugChannel::from_str(word, true) {
                Ok(channel) => channels.toggle(channel),
                Err(_) => return Err(format!("Unknown debug channel: {word}, of {}", DebugChannels::all())),
            }
        }

        frame.set_channels(channels);
        info!(battle = self.id, %channels, "Changed the debug drawing's channels");
        Ok(format!("Debug drawing: {channels}, of {}", DebugChannels::all()))

    }

    /// The `/bot` command of a participant, with the words after it, see
    /// [`Self::command`], for devs only: the orders of the bots it names after it, else
    /// what is wrong.
    ///
    /// ```text
    ///   /bot <who> <order>
    ///     who: a bot's number, as the players' list names it, `aimed` for the one under
    ///       its gun marker, `all`, `allies` or `enemies`, see `bots_named`
    ///     order:
    ///       goto [here] [<km/h>]   drive where its gun marker is, or to its vehicle, and
    ///                              stay there
    ///       follow [loop] [<km/h>] drive along the route, see `route_command`, once or
    ///                              round
    ///       speed <km/h>|off       drive at most this fast, from now on, or as fast as
    ///                              it can: a speed after goto or follow sets it too
    ///       fire [once|hold|off]   fire at the spot under its gun marker, on the vehicle
    ///                              there, see `marked_spot`: each time loaded, once,
    ///                              only aim at it, or no more
    ///       shell <n>              load its gun's nth shell, from 1
    ///       stop                   stand still holding fire, its orders kept
    ///       start                  carry them out again
    ///       clear                  no orders
    /// ```
    fn bot_command<'a>(&mut self, from: usize, mut words: impl Iterator<Item = &'a str>) -> Result<String, String> {

        if !self.participants[from].dev {
            return Err("Only devs may use /bot".to_string());
        }
        let Some(who) = words.next() else {
            return Err(
                "/bot <number|aimed|all|allies|enemies> <goto [here] [km/h]|follow [loop] [km/h]|\
                speed <km/h|off>|fire [once|hold|off]|shell <n>|stop|start|clear>".to_string(),
            );
        };
        let bots = self.bots_named(from, who)?;
        let order = words.next().ok_or("No order after the bots")?;
        let now = self.game_time();

        match order {
            "goto" | "follow" => {

                // Its options, in any order: a speed, and `here` or `loop`.
                let mut option = None;
                let mut speed = None;
                for word in words {
                    match word.parse::<f32>() {
                        Ok(kmh) if kmh > 0.0 => speed = Some(kmh / 3.6),
                        Ok(_) => return Err(format!("Not a speed: {word} km/h")),
                        Err(_) => option = Some(word),
                    }
                }

                let driven = self.world.vehicle(self.vehicles[self.participants[from].vehicle].id);
                let travel = if order == "goto" {
                    match option {
                        Some("here") => BotTravel::Goto(driven.hull.position),
                        None => BotTravel::Goto(driven.turret.tracked().ok_or("No point under your gun marker")?),
                        Some(word) => return Err(format!("Unknown goto option: {word}, of here")),
                    }
                } else {
                    match option {
                        _ if self.route.points.is_empty() => {
                            return Err("No route: /route start, drive, then /route stop".to_string());
                        }
                        None | Some("loop") => BotTravel::Follow {
                            points: self.route.points.clone(),
                            next: 0,
                            looped: option.is_some(),
                        },
                        Some(word) => return Err(format!("Unknown follow option: {word}, of loop")),
                    }
                };

                for &index in &bots {
                    if let Some(bot) = self.vehicles[index].as_bot_mut() {
                        bot.set_travel(Some(travel.clone()));
                        if speed.is_some() {
                            bot.set_speed(speed);
                        }
                    }
                }

            }
            "speed" => {
                let speed = match words.next() {
                    Some("off") => None,
                    Some(word) => {
                        let kmh = word.parse::<f32>()
                            .ok()
                            .filter(|&kmh| kmh > 0.0)
                            .ok_or_else(|| format!("Not a speed: {word} km/h"))?;
                        Some(kmh / 3.6)
                    }
                    None => return Err("No speed after speed: km/h, or off".to_string()),
                };
                for &index in &bots {
                    if let Some(bot) = self.vehicles[index].as_bot_mut() {
                        bot.set_speed(speed);
                    }
                }
            }
            "fire" => {
                let mode = match words.next() {
                    None => Some(BotFireMode::Repeat),
                    Some("once") => Some(BotFireMode::Once),
                    Some("hold") => Some(BotFireMode::Hold),
                    Some("off") => None,
                    Some(word) => return Err(format!("Unknown fire mode: {word}, of once, hold, off")),
                };
                let fire = match mode {
                    Some(mode) => {
                        let (target, hit, hull_position) = self.marked_spot(from)
                            .ok_or("No vehicle under your gun marker")?;
                        let target = self.vehicles[target].id;
                        Some(BotFire::new(target, hit.part, hit.local_point, hull_position, mode))
                    }
                    None => None,
                };
                for &index in &bots {
                    let vehicle = &mut self.vehicles[index];
                    let own = fire.is_some_and(|fire| fire.vehicle == vehicle.id);
                    if let (Some(bot), false) = (vehicle.as_bot_mut(), own) {
                        bot.set_fire(fire);
                    }
                }
            }
            "shell" => {
                let number = words.next()
                    .and_then(|number| number.parse::<usize>().ok())
                    .filter(|&number| number >= 1)
                    .ok_or("No shell number after shell, from 1")?;
                for &index in &bots {
                    let vehicle = &mut self.vehicles[index];
                    let Some((def, _)) = vehicle.gun.ammo.get(number - 1) else {
                        return Err(format!("No shell {number}: {} has {}", vehicle.info.name, vehicle.gun.ammo.len()));
                    };
                    let compact_descr = def.compact_descr.to_int();
                    vehicle.gun.change_loaded(now, compact_descr);
                }
            }
            "stop" | "start" => {
                for &index in &bots {
                    if let Some(bot) = self.vehicles[index].as_bot_mut() {
                        bot.set_stopped(order == "stop");
                    }
                }
            }
            "clear" => {
                for &index in &bots {
                    if let Some(bot) = self.vehicles[index].as_bot_mut() {
                        *bot = Bot::default();
                    }
                }
            }
            _ => return Err(format!("Unknown bot order: {order}, of goto, follow, speed, fire, shell, stop, start, clear")),
        }

        let orders: Vec<_> = bots.iter()
            .map(|&index| {
                let vehicle = &self.vehicles[index];
                let orders = vehicle.as_bot().map(|bot| bot.to_string()).unwrap_or_default();
                format!("{}: {orders}", vehicle.info.name)
            })
            .collect();
        info!(battle = self.id, order, bots = bots.len(), "Ordered bots");
        Ok(orders.join("; "))

    }

    /// The indices of the bots alive `who` names, for the `from`th participant, see
    /// [`Self::bot_command`]: by its number in the players' list, `Bot <number>`, the one
    /// under its gun marker, see [`Self::marked_spot`], all, its team's or the other's.
    fn bots_named(&self, from: usize, who: &str) -> Result<Vec<usize>, String> {

        let team = self.vehicles[self.participants[from].vehicle].team;
        let aimed = match who {
            "aimed" => {
                let (index, _, _) = self.marked_spot(from).ok_or("No vehicle under your gun marker")?;
                Some(index)
            }
            _ => None,
        };
        let name = match who {
            "aimed" | "all" | "allies" | "enemies" => None,
            number => {
                let number = number.parse::<u32>()
                    .map_err(|_| format!("Unknown bots: {who}, of a number, aimed, all, allies, enemies"))?;
                Some(format!("Bot {number}"))
            }
        };

        let named: Vec<_> = self.vehicles.iter()
            .enumerate()
            .filter(|(_, vehicle)| matches!(vehicle.driver, Driver::Bot(_)) && vehicle.alive())
            .filter(|&(index, vehicle)| match who {
                "aimed" => aimed == Some(index),
                "all" => true,
                "allies" => vehicle.team == team,
                "enemies" => vehicle.team != team,
                _ => name.as_ref() == Some(&vehicle.info.name),
            })
            .map(|(index, _)| index)
            .collect();
        if named.is_empty() {
            return Err(format!("No bot alive: {who}"));
        }
        Ok(named)

    }

    /// The vehicle alive under the gun marker of the `from`th participant, by index, if
    /// any, where on it, see [`VehicleIntersection`], and where its hull sits on its
    /// chassis: the first its gun's line to the point its client tracks crosses, that
    /// line going [`MARKED_DEPTH`] past it.
    fn marked_spot(&self, from: usize) -> Option<(usize, VehicleIntersection, Vec3)> {

        let own = self.participants[from].vehicle;
        let driven = self.world.vehicle(self.vehicles[own].id);
        let marked = driven.turret.tracked()?;
        let (eye, _, _) = driven.turret.gun_marker(&driven.hull);
        let to = marked + (marked - eye).normalize_or_zero() * MARKED_DEPTH;

        self.vehicles.iter()
            .enumerate()
            .filter(|&(index, vehicle)| index != own && vehicle.alive())
            .filter_map(|(index, vehicle)| {
                let target = vehicle.shot_target(&self.world);
                let hit = target.collision.first_hit(&target.transforms, eye, to)?;
                Some((index, hit, target.collision.hull_position))
            })
            .min_by(|(_, a, _), (_, b, _)| a.distance.total_cmp(&b.distance))

    }

    /// The `/route` command of a participant, with the words after it, see
    /// [`Self::command`], for devs only: what its bots' route is after it, else what is
    /// wrong.
    ///
    /// ```text
    ///   /route                 how many points it has
    ///   /route start           record its vehicle's way as it drives, see BotRoute::record
    ///   /route stop            no more
    ///   /route add             add the point under its gun marker
    ///   /route clear           no points, and no more recording
    /// ```
    fn route_command<'a>(&mut self, from: usize, mut words: impl Iterator<Item = &'a str>) -> Result<String, String> {

        if !self.participants[from].dev {
            return Err("Only devs may use /route".to_string());
        }

        let route = &mut self.route;
        match words.next() {
            None => {
                let count = route.points.len();
                return Ok(format!("{count} points on the route. /route start|stop|add|clear"));
            }
            Some("start") => route.recorder = Some(from),
            Some("stop") => route.recorder = None,
            Some("add") => {
                let driven = self.world.vehicle(self.vehicles[self.participants[from].vehicle].id);
                let point = driven.turret.tracked().ok_or("No point under your gun marker")?;
                route.points.push(point);
            }
            Some("clear") => *route = BotRoute::default(),
            Some(word) => return Err(format!("Unknown route command: {word}, of start, stop, add, clear")),
        }

        let recording = if route.recorder.is_some() { ", recording" } else { "" };
        info!(
            battle = self.id,
            points = route.points.len(),
            recording = route.recorder.is_some(),
            "Changed the bots' route",
        );
        Ok(format!("{} points on the route{recording}", route.points.len()))

    }

    // -- Its tick --

    /// End a tick: go to its next period when due, over once a team is exterminated, see
    /// [`Self::exterminated`], step it, once a client has its vehicle, fly its shells on
    /// against its vehicles where they are now, see [`Shells::step`], breaking what they
    /// get through and landing those that hit something, then tell what broke. All
    /// drawing to be debugged, see the `debug` module.
    pub fn tick(&mut self, app: &mut base::App) {

        let id = self.id;
        if let Some(frame) = self.debug.take() {
            frame.begin(self.game_time());
        }

        let ready = self.participants.iter().all(|participant| participant.ready);
        if let Some(period) = self.period.next(ready, self.exterminated()) {
            self.period = period;
            info!(battle = id, ?period, "Changed the battle's period");
            for participant in self.participants.iter().filter(|participant| participant.ready) {
                if let Err(e) = participant.show_period(app, period) {
                    warn!(battle = id, client = %participant.client, "Failed to tell the battle's period: {e}");
                }
            }
        }

        if self.running {
            if let Err(e) = self.step(app) {
                warn!(battle = id, "Failed to step the battle: {e}");
            }
        }

        // Ricochets fly on from where they start in the same tick, against the same
        // vehicles: those they ricocheted off as they were.
        let now = self.game_time();
        loop {

            let Self { shells, world, vehicles, targets, .. } = self;
            fill_targets(targets, vehicles, world);
            let obstacles = ShotObstacles {
                space: world.space(),
                rubble: world.rubble(),
            };
            let (through, landed) = shells.step(now, obstacles, targets);

            for (piece, yaw) in through {
                self.world.break_shot(piece.instance, piece.piece, yaw);
            }
            if landed.is_empty() {
                break;
            }

            for (at, shooter, mut shot, landing) in landed {
                if let Err(e) = self.land(app, at, shooter, &mut shot, landing) {
                    warn!(battle = id, shot = shot.id, "Failed to land a shot: {e}");
                }
            }

        }
        let breakages = self.world.take_breakages().collect::<Vec<_>>();
        if let Err(e) = self.areas.show(app, id, breakages, &self.world) {
            warn!(battle = id, "Failed to tell the destructibles broken: {e}");
        }

        if let Some(frame) = DebugFrame::end() {
            let vehicles = &self.vehicles;
            let shown = self.debug.insert(frame).show(app, |vehicle| debug_entity_id(vehicles, vehicle));
            if let Err(e) = shown {
                warn!(battle = id, "Failed to draw to be debugged: {e}");
            }
        }

    }

    /// Step it by a tick: every vehicle moves as its driver drives it, a bot as it is
    /// ordered, see [`Bot::drive`], its fire order dropped once its target is destroyed,
    /// a wreck or one nobody drives stopping where it was shoved, and those that ran into
    /// each other are pushed apart, see [`World::step`]. Then the bots' route gets where
    /// its recorder is, see [`BotRoute::record`], each team spots the other's vehicles,
    /// see [`Spotting`], each client is shown those its team spots, and each vehicle
    /// fires the rest of its burst, a bot as it is ordered too, while the battle fights,
    /// see [`Bot::fires`], and the clients see each where it is, see
    /// [`BattleVehicle::show`].
    fn step(&mut self, app: &mut base::App) -> io::Result<()> {

        let id = self.id;
        let now = self.game_time();
        let fighting = self.period.fighting();
        let avatars = self.avatars();
        let Self { participants, vehicles, world, spotting, route, shells, .. } = self;

        for vehicle in vehicles.iter_mut() {
            vehicle.finish_switching(app, world.vehicle_mut(vehicle.id), now)?;
        }
        for index in 0..vehicles.len() {

            if !vehicles[index].alive() {
                world.vehicle_mut(vehicles[index].id).hull.flags = 0;
                continue;
            }
            if !matches!(vehicles[index].driver, Driver::Bot(_)) {
                continue;
            }

            let allies = allies_of(vehicles, world, index);
            let lost = vehicles[index].as_bot()
                .and_then(|bot| bot.target())
                .is_some_and(|target| {
                    !vehicles.iter().any(|vehicle| vehicle.id == target && vehicle.alive())
                });
            let vehicle_id = vehicles[index].id;
            if let Some(bot) = vehicles[index].as_bot_mut() {
                if lost {
                    bot.set_fire(None);
                }
                bot.drive(world, vehicle_id, &allies, fighting);
            }

        }
        world.step(TICK.as_secs_f32());

        if let Some(recorder) = route.recorder {
            let recording = &vehicles[participants[recorder].vehicle];
            route.record(world.vehicle(recording.id).hull.position);
        }
        if debug::draws(DebugChannel::Bots) {
            route.debug_draw();
        }

        spotting.update(now, vehicles, world);
        for participant in participants.iter_mut() {
            participant.spot(app, vehicles, spotting, now)?;
        }
        
        spotting.debug_draw(vehicles, world);

        for index in 0..vehicles.len() {

            let shoved = &world.vehicle(vehicles[index].id).hull;
            if matches!(vehicles[index].driver, Driver::Bot(_)) && (shoved.speed != 0.0 || shoved.slide != 0.0) {
                debug!(
                    battle = id,
                    entity_id = %vehicles[index].entity.id(),
                    position = ?shoved.position,
                    yaw = shoved.yaw(),
                    speed = shoved.speed,
                    slide = shoved.slide,
                    "Shoved a bot",
                );
            }

            if !vehicles[index].alive() {
                vehicles[index].show_wreck(app, world.vehicle(vehicles[index].id))?;
                continue;
            }

            let allies = allies_of(vehicles, world, index);
            let vehicle = &mut vehicles[index];
            let driven = world.vehicle(vehicle.id);

            vehicle.put_out_fire(app, id, now)?;
            vehicle.show_loaded(app, now)?;

            let marker = driven.turret.gun_marker(&driven.hull);
            let (position, velocity, dispersion) = marker;
            let mut shot = vehicle.gun.fire_burst(now, position, velocity, dispersion);
            if let (None, Driver::Bot(bot), true) = (&shot, &mut vehicle.driver, fighting) {
                if bot.fires(driven, &allies) {
                    shot = vehicle.gun.fire(now, position, velocity, dispersion);
                    if shot.is_some() {
                        bot.fired();
                    }
                }
            }

            if let Some(shot) = shot {
                info!(battle = id, entity_id = %vehicle.entity.id(), shot = shot.id, "Fired a shell from {:?} at {:?}", shot.start, shot.start_velocity);
                vehicle.fired(now);
                vehicle.show_burst(app)?;
                show_shot(app, &avatars, vehicle.entity, &shot)?;
                shells.fly(now, vehicle.entity.id(), shot);
            }

            vehicle.show(app, driven)?;
            vehicle.show_own(app, driven, marker)?;

        }
        Ok(())

    }

    // -- Its shells --

    /// A shell gets to its end, at game time `at` (what it flew through broke as it got
    /// there, see [`Shot::through`]): what it does to the vehicle it hits, by its armour,
    /// see [`impact::hit`], or it explodes on the ground or a static model, a destructible
    /// one taking its damage, see [`World::shoot`], and an HE shell damaging the vehicles
    /// in its explosion's reach, see [`impact::explosion`]. A shell ricocheting off a
    /// vehicle or the ground goes on, see [`Self::ricochet`]; a static model stops it
    /// whole.
    fn land(
        &mut self,
        app: &mut base::App,
        at: f64,
        shooter: EntityId,
        shot: &mut Shot,
        landing: ShotLanding,
    ) -> io::Result<()> {

        // What it lost on the way it loses now, see `Shot::impact`.
        shot.impact();
        let shot = &*shot;
        let id = self.id;
        let avatars = self.avatars();
        debug!(battle = id, shot = shot.id, "Landed a shell at {:?} after {:.2} s", shot.end, shot.flight_time);

        // What a destructible hit breaks the clients break as the shell explodes.
        let mut damaged = Vec::new();
        if let ShotLanding::Static { piece: Some(hit), .. } = landing {
            let yaw = shot.end_velocity.x.atan2(shot.end_velocity.z);
            let damage = shot.def.damage_at(shot.distance());
            let broken = self.world.shoot(hit.instance, hit.piece, damage, yaw);
            info!(
                battle = id,
                shot = shot.id,
                chunk_id = hit.piece.chunk_id,
                index = hit.piece.index,
                module = ?hit.piece.module,
                "Hit a destructible for {damage:.0} damage{}",
                if broken.is_empty() { "" } else { ", broke it" },
            );
            damaged.extend(broken.iter().filter_map(|breakage| explosion_obstacle(breakage.piece)));
        }

        let def = &shot.def;
        for &avatar in &avatars {
            app.entity(avatar)?.stopTracer(shot.id, shot.end)?;
        }

        match landing {
            ShotLanding::Vehicle { entity_id, hit, transforms } => {
                
                let vehicle = self.vehicles.iter().find(|vehicle| vehicle.entity.id().0 == entity_id);
                let Some(collision) = vehicle.map(|vehicle| vehicle.collision.clone()) else { return Ok(()) };
                let Some(impact) = impact::hit(def, shot.distance(), shot.speed_factor, shot.end, shot.end_velocity, &collision, &transforms, &mut rand::thread_rng()) else {
                    debug!(battle = id, shot = shot.id, entity_id, "Hit a vehicle where it has no armour: {:?}", hit.part);
                    return Ok(());
                };

                info!(battle = id, shot = shot.id, entity_id, "Hit a vehicle: {:?}, {:?} {} mm at {:?}, {:.0} damage{}",
                    impact.outcome, impact.hit.part, impact.hit.material.map_or(0.0, |m| m.armor), impact.hit.local_point,
                    impact.damage, if impact.exploded { ", exploded" } else { "" });
                
                let dealt = self.hit(app, shooter, entity_id, shot, &ShellHit::of_impact(&impact))?;
                debug_hit(shot.end, &impact.trace, debug_outcome(&impact), impact.damage, dealt);
                
                if impact.outcome == ImpactOutcome::Ricochet {
                    let velocity = ricochet::off_armor(shot.end_velocity, impact.hit.normal);
                    self.ricochet(app, at, shooter, shot, velocity)?;
                }
                
                Ok(())

            }
            ShotLanding::Terrain |
            ShotLanding::Static { .. } => {
                
                if let ShotLanding::Static { normal, .. } = landing {
                    debug!(battle = id, shot = shot.id, "Hit a static model at {:?}, facing {normal:?}", shot.end);
                }
                
                // EXPERIMENT: the explosion comes for a shot the client does not know, so
                // that it shows it from these arguments alone.
                for &avatar in &avatars {
                    app.entity(avatar)?.explodeProjectile(
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
                        damaged.clone(),
                    )?;
                }
                
                // Off the ground only: a static model stops a shell whole.
                let skims = match landing {
                    ShotLanding::Terrain => {
                        let terrain = &self.world.space().terrain;
                        let normal = terrain.normal_at(shot.end.x, shot.end.z).unwrap_or(Vec3::Y);
                        let ground = GroundMaterial::of(ground_at(terrain, shot.end));
                        ricochet::off_ground(def, shot.end_velocity, normal, &ground, &mut rand::thread_rng())
                    }
                    _ => None,
                };
                
                if let Some(velocity) = skims {
                    self.ricochet(app, at, shooter, shot, velocity)?;
                } else if def.kind() == ShellKind::HighExplosive {
                    self.splash(app, shooter, shot)?;
                }
                
                Ok(())

            }
            ShotLanding::Range => Ok(()),
        }

    }

    /// A vehicle of it alive takes a shell's damage, see [`BattleVehicle::take_damage`],
    /// every client listing it destroyed once killed, and the client that fired it, if
    /// any, learns the hit's result. The damage it took, if it was alive.
    fn hit(
        &mut self,
        app: &mut base::App,
        shooter: EntityId,
        entity_id: u32,
        shot: &Shot,
        shell_hit: &ShellHit,
    ) -> io::Result<Option<i64>> {

        let id = self.id;
        let now = self.game_time();
        let vehicles_infos: Vec<_> = self.participants.iter().map(|participant| participant.vehicles_info).collect();
        let shooter_avatar = self.vehicles.iter()
            .find(|vehicle| vehicle.entity.id() == shooter)
            .and_then(|vehicle| vehicle.client_driver())
            .map(|driver| driver.avatar());
        let Some(vehicle) = self.vehicles.iter_mut().find(|vehicle| vehicle.entity.id().0 == entity_id && vehicle.alive()) else {
            return Ok(None);
        };

        let dealt = vehicle.take_damage(app, id, shooter, shot, shell_hit, now)?;
        let killed = !vehicle.alive();
        if killed {
            vehicle.mark_killed(app, &vehicles_infos, shooter)?;
        }

        let Some(shooter_avatar) = shooter_avatar else { return Ok(Some(dealt)) };
        let mut flags = shell_hit.flags;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        app.entity(shooter_avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.entity.id(),
        }])?;
        Ok(Some(dealt))

    }

    /// A shell ricochets off what it landed on, at game time `at`, at `velocity`,
    /// slower than it hit, its penetration following its speed, see [`Shot::ricochet`]:
    /// it flies on from there, and the players see it go on, as the server shows it,
    /// with a tracer of the same shot.
    fn ricochet(
        &mut self,
        app: &mut base::App,
        at: f64,
        shooter: EntityId,
        shot: &Shot,
        velocity: Vec3,
    ) -> io::Result<()> {

        let id = self.id;
        let Some(next) = shot.ricochet(velocity) else {
            debug!(battle = id, shot = shot.id, ricochets = shot.ricochets, "Stopped a shell's ricochets: too many, or out of range");
            return Ok(());
        };

        info!(battle = id, shot = next.id, ricochets = next.ricochets,
            "Ricocheted a shell from {:?} at {:.0}% of its speed", next.start, next.speed_factor * 100.0);
        show_tracer(app, &self.avatars(), shooter, &next, true)?;
        self.shells.fly(at, shooter, next);
        Ok(())

    }

    /// An HE shell explodes on the ground: the vehicles in its reach take its damage,
    /// but the shooter's own.
    fn splash(&mut self, app: &mut base::App, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        let explosions: Vec<_> = self.shell_targets(shooter).into_iter()
            .filter_map(|target| {
                let explosion = impact::explosion(&shot.def, shot.end, &target.collision, &target.transforms)?;
                let reached = target.transforms[explosion.hit.part as usize].transform_point3(explosion.hit.local_point);
                Some((target.entity_id, explosion, reached))
            })
            .collect();

        for (entity_id, explosion, reached) in explosions {
            info!(battle = self.id, shot = shot.id, entity_id, "Exploded beside a vehicle: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            let dealt = self.hit(app, shooter, entity_id, shot, &ShellHit::of_explosion(&explosion))?;
            debug_hit(reached, &explosion.trace, "Splashed", explosion.damage, dealt);
        }
        Ok(())

    }

}

/// Draw a shell's hit on a vehicle to be debugged, above `at`, for as long as its
/// flight: how it went, see [`impact::Impact::trace`], then its outcome, the damage the
/// vehicle took, after its spread, unless it was destroyed already, and its alpha,
/// `damage`, before it.
fn debug_hit(at: Vec3, trace: &[String], outcome: &str, damage: f32, dealt: Option<i64>) {

    if !debug::draws(DebugChannel::Shells) {
        return;
    }

    let result = match dealt {
        _ if damage <= 0.0 => format!("{outcome}: no damage"),
        Some(dealt) => format!("{outcome}: {dealt} damage ({damage:.0} alpha)"),
        None => format!("{outcome}: already destroyed ({damage:.0} alpha)"),
    };
    let text = format!("{}\n{result}", trace.join("\n"));
    debug::lasting(DEBUG_SHOT_SECONDS, || {
        debug::text(DebugChannel::Shells, at + Vec3::Y * DEBUG_HIT_RISE, text, DEBUG_HIT_TEXT);
    });

}

/// What a shell did to a vehicle, to be debugged, see [`debug_hit`].
fn debug_outcome(impact: &impact::Impact) -> &'static str {
    match (impact.outcome, impact.exploded) {
        (ImpactOutcome::Pierced, _) => "Pierced",
        (ImpactOutcome::NotPierced, true) => "Not pierced, exploded",
        (ImpactOutcome::NotPierced, false) => "Not pierced",
        (ImpactOutcome::Ricochet, _) => "Ricocheted",
        (ImpactOutcome::Screens, _) => "Through screens only",
    }
}

/// The allies alive of the `index`th vehicle, as a bot drives round them, gives way to
/// them and doesn't fire through them, see [`BotAlly`]. None for a vehicle no bot drives.
fn allies_of(vehicles: &[BattleVehicle], world: &World, index: usize) -> Vec<BotAlly> {
    let vehicle = &vehicles[index];
    if !matches!(vehicle.driver, Driver::Bot(_)) {
        return Vec::new();
    }
    vehicles.iter()
        .enumerate()
        .filter(|&(other, ally)| other != index && ally.alive() && ally.team == vehicle.team)
        .map(|(_, ally)| {
            let hull = &world.vehicle(ally.id).hull;
            let [min, max] = hull.mobility.bounds;
            BotAlly {
                center: ally.hull_center(world),
                radius: (max - min).length() / 2.0,
                velocity: hull.velocity(),
            }
        })
        .collect()
}

/// Replace `targets` with the vehicles shells can hit: every one alive, reusing its
/// memory.
fn fill_targets(targets: &mut Vec<ShotTarget>, vehicles: &[BattleVehicle], world: &World) {
    targets.clear();
    targets.extend(vehicles.iter()
        .filter(|vehicle| vehicle.alive())
        .map(|vehicle| vehicle.shot_target(world)));
}

/// The entity id of the battle's vehicle of this id, if any, for its debug drawing.
fn debug_entity_id(vehicles: &[BattleVehicle], id: VehicleId) -> Option<u32> {
    vehicles.iter()
        .find(|vehicle| vehicle.id == id)
        .map(|vehicle| vehicle.entity.id().0)
}
