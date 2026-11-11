//! The battles of the server: each in a space of its own, with two teams of its clients'
//! vehicles and bots, see [`BattleOptions`]: every vehicle the same, but who drives it,
//! see [`Controller`].
//!
//! A battle gathers the clients the queue switched into it, waiting for them to enter,
//! see [`Gathering`], then is made with them all at once: their vehicles, each one's own
//! arena info, and the bots. It counts down once they are all ready, done loading, and
//! goes on while any client is in it, a vehicle whose client left standing where it
//! stopped.
//!
//! A battle holds all it needs, and only tells its clients what happens through the app
//! it is given, so that an app may run several, a thread of its own each, say. The base
//! app only finds a client's battle, see [`crate::base`].
//!
//! ```text
//!   (here)       a battle: made, its tick, its shells landing, its clients' calls
//!   participant  a client in it: its avatar, its vehicle, what it sees
//!   period       waiting, counting down, fighting
//!   vehicle      a vehicle of it, a player's or a bot's: shown, its gun, damaged
//!   bot          the server driving a vehicle: its way, its aim, when it fires
//!   shells       its shells in flight, and their tracers
//!   hit          what a shell does to a vehicle, as the clients are told
//!   areas        what broke of its destructibles, by chunk
//!   spawn        where its vehicles start
//! ```

mod participant;
mod period;
mod vehicle;
mod bot;
mod shells;
mod hit;
mod areas;
mod spawn;

use std::time::{Duration, Instant};
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::{Affine3A, Vec2, Vec3};

use wgtk::app::base::{self, CellPlayerInit, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::app::entity::{AnyEntity, Audience, EntityId, EntityKey};
use wgtk::space::Space;

use crate::game::schema::{
    Avatar, AvatarCell, AvatarComponentsData, AvatarInfo, AvatarVehiclesInfo, ShotResult, VehicleCell,
};
use crate::base::{
    avatar_cell_data, VehicleConfig, EXPLOSION_STEP, GROUND_EFFECT_MATERIAL, NO_PREFAB_EFFECTS, TICK,
};
use crate::base::ballistics::gun::{HitTarget, Landing, Obstacles, Shot};
use crate::game::item_defs::{ShellKind, VehicleDef};
use crate::base::ballistics::armor::{self, Outcome};
use crate::base::physics::{Vehicle, World};
use crate::game::arena_defs::ArenaTypeDef;
use crate::base::garage::player::Player;

use vehicle::{hold_gun, BattleVehicle, Controller};
use shells::{show_shot, show_tracer, Shells};
use hit::{ShellHit, HIT_FLAG_VEHICLE_KILLED};
use areas::{explosion_obstacle, Areas};
use participant::Participant;
use period::Period;
use spawn::Start;


/// A battle, with its clients, see the module's documentation.
#[derive(Debug)]
pub struct Battle {
    /// Its id, which its clients are switched into it with.
    id: u32,
    space: SpaceKey,
    /// When it was made, at 0: the battle's clock.
    clock: Instant,
    /// How big its teams are, and what bots do.
    options: BattleOptions,
    /// Its clients, each driving one of its vehicles.
    participants: Vec<Participant>,
    period: Period,
    /// Its vehicles, its clients' first, then the bots on either side, and the world
    /// they move in.
    vehicles: Vec<BattleVehicle>,
    world: World,
    shells: Shells,
    areas: Areas,
    /// What shells hit, refilled by each shooter, kept to reuse its memory.
    targets: Vec<HitTarget>,
    /// Whether a client has its vehicle: it runs from then on.
    running: bool,
}

/// How big a battle's teams are, and what its bots do, as the server is launched with.
#[derive(Debug, Clone)]
pub struct BattleOptions {
    /// How many vehicles each team has, its clients' then bots.
    pub team_size: u32,
    /// The vehicles bots drive, each the next, round: the first client's own when none.
    pub bot_vehicles: Vec<VehicleDef>,
    pub bots_fire: bool,
    pub bots_drive: bool,
    /// How far in front of the first team the second starts, if not at its spawn.
    pub enemy_distance: Option<f32>,
    /// Where the first team starts, if not at its spawn, and its yaw in radians.
    pub spawn: Option<(Vec2, f32)>,
}

/// What a battle is made of, its clients aside.
#[derive(Debug)]
pub struct BattleSetup<'a> {
    /// Its map, and the map's space, which its vehicles move in.
    pub map: &'a ArenaTypeDef,
    pub space: Arc<Space>,
    pub options: BattleOptions,
    /// The vehicle bots drive when the options give none: the first client's.
    pub own_def: Option<VehicleDef>,
}

/// A client entering a battle, with its avatar.
#[derive(Debug)]
pub struct Arrival {
    pub client: ClientKey,
    pub avatar: EntityKey<Avatar>,
    pub player: Player,
    pub config: VehicleConfig,
}

/// A battle waiting for its clients to enter before it is made, as the base app gathers
/// them, see [`crate::base`].
#[derive(Debug)]
pub struct Gathering {
    /// How many clients were switched into it, and those that entered.
    pub expected: usize,
    pub arrivals: Vec<Arrival>,
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
    /// allies and the enemies its vehicle spots, see [`Participant::spot`].
    pub fn new(app: &mut base::App, setup: BattleSetup, space: SpaceKey, id: u32, arrivals: Vec<Arrival>) -> io::Result<Self> {

        let BattleSetup { map, space: map_space, options, own_def } = setup;
        let terrain = &map_space.terrain;
        let mut world = World::new(map_space.clone(), Some(map.bounds));

        // The clients' vehicles, on either team in turn, from their team's start, where the
        // first of the team stands then: the first team's where the options spawn it, the
        // second's in front of it if the options say so.
        let first_start = match options.spawn {
            Some((at, yaw)) => Start::at(at, yaw, terrain),
            None => Start::team(map, terrain, TEAMS[0]),
        };
        let second_start = match options.enemy_distance {
            Some(distance) => first_start.facing(distance, terrain),
            None => Start::team(map, terrain, TEAMS[1]),
        };
        let mut starts = [first_start, second_start];
        let mut team_sizes = [0; 2];
        let mut vehicles = Vec::new();
        for (index, arrival) in arrivals.iter().enumerate() {

            let Arrival { client, avatar, player, config } = arrival;
            let side = index % 2;
            let slot = team_sizes[side];
            let at = starts[side].slot(slot, terrain);
            let info_index = vehicles.len() as u32;
            let vehicle = BattleVehicle::player(app, &mut world, *client, *avatar, space, config, player.clone(), TEAMS[side], at, info_index)?;

            if slot == 0 {
                let hull = &world.vehicle(vehicle.id).hull;
                starts[side] = Start {
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
                (Some(def), Some(first)) => (def, first.config.descr.clone()),
                _ => {
                    let def = &options.bot_vehicles[index % options.bot_vehicles.len()];
                    (def, def.descr.encode())
                }
            };
            let team = TEAMS[side];
            let at = starts[side].slot(slot, terrain);
            let name = format!("Bot {}", index + 1);
            let bot = BattleVehicle::bot(app, &mut world, space, def, &descr, team, name, at, vehicles.len() as u32, index as i32)?;
            info!(battle = id, entity_id = %bot.entity.id(), team, "Added a bot to the arena: {}, {}", bot.info.name, def.name);
            vehicles.push(bot);
        }

        // Each client's arena info entity, its own only, created right away as entities
        // without an alias are, with the vehicles.
        let mut participants = Vec::with_capacity(arrivals.len());
        for (index, Arrival { client, avatar, config, .. }) in arrivals.into_iter().enumerate() {
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
            participants.push(Participant::new(client, avatar, config, index, vehicles_info.key(), vehicles.len()));
        }

        let areas = Areas::new(app, space, &world.space().statics)?;
        info!(battle = id, chunks = areas.len(), "Created the destructibles' areas");

        Ok(Self {
            id,
            space,
            clock: Instant::now(),
            options,
            participants,
            period: Period::new(),
            vehicles,
            world,
            shells: Shells::default(),
            areas,
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

    /// Its game time, in seconds.
    fn game_time(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// Its clients' avatars, which are shown what happens.
    fn avatars(&self) -> Vec<EntityKey<Avatar>> {
        self.participants.iter().map(|participant| participant.avatar).collect()
    }

    /// The index of the vehicle a client drives.
    fn own(&self, client: ClientKey) -> Option<usize> {
        self.participants.iter()
            .find(|participant| participant.client == client)
            .map(|participant| participant.vehicle)
    }

    /// The simulated vehicle a client drives.
    fn own_vehicle_mut(&mut self, client: ClientKey) -> Option<&mut Vehicle> {
        let id = self.vehicles[self.own(client)?].id;
        Some(self.world.vehicle_mut(id))
    }

    /// The vehicles the shells of `shooter` can hit: every one alive but its own.
    fn shell_targets(&self, shooter: EntityId) -> Vec<HitTarget> {
        let mut targets = Vec::with_capacity(self.vehicles.len());
        fill_targets(&mut targets, &self.vehicles, &self.world, shooter);
        targets
    }

    // -- Its clients --

    /// A client is ready, done loading: once all are, it counts down, see [`Self::tick`].
    /// One ready once it counted down already is told where it is.
    pub fn ready(&mut self, app: &mut base::App, client: ClientKey) -> io::Result<()> {
        let period = self.period;
        let Some(participant) = self.participants.iter_mut().find(|participant| participant.client == client) else { return Ok(()) };
        participant.ready = true;
        participant.show_period(app, period)
    }

    /// A client leaves it: its vehicle stops, and stands there, nobody driving it.
    pub fn leave(&mut self, client: ClientKey) {
        let Some(at) = self.participants.iter().position(|participant| participant.client == client) else { return };
        let participant = self.participants.remove(at);
        let vehicle = &mut self.vehicles[participant.vehicle];
        vehicle.controller = Controller::Nobody;
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
        own.enter(app, &participant.config)?;
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

    /// Fire a client's gun, if loaded: the shell's tracer and the gun's effects, of its
    /// whole burst, then one shell fewer, the shell it loads next and the reload, once
    /// the burst's other shells are fired too, see [`Self::step`]. Where the shell lands
    /// follows with [`Self::land`].
    fn shoot(&mut self, app: &mut base::App, client: ClientKey) -> io::Result<()> {

        let now = self.game_time();
        let Some(own) = self.own(client).filter(|&own| self.vehicles[own].alive()) else { return Ok(()) };
        let own_id = self.vehicles[own].entity.id();
        let targets = self.shell_targets(own_id);
        let avatars = self.avatars();
        let Self { vehicles, world, shells, .. } = self;
        let own = &mut vehicles[own];
        let driven = world.vehicle(own.id);
        let (position, velocity, dispersion) = driven.turret.gun_marker(&driven.hull);
        let obstacles = Obstacles {
            space: world.space(),
            rubble: world.rubble(),
        };
        let Some(shot) = own.gun.fire(now, position, velocity, dispersion, Some(obstacles), &targets) else {
            debug!(%client, "Refused a shot: not loaded, or out of shells");
            return Ok(());
        };
        info!(%client, shot = shot.id, burst = shot.burst, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
        own.show_gun(app, now)?;
        show_shot(app, &avatars, own.entity, &shot)?;
        shells.fly(now, own_id, shot);
        Ok(())

    }

    /// A client changes one of its vehicle's settings, see
    /// [`BattleVehicle::change_setting`].
    fn change_setting(&mut self, app: &mut base::App, client: ClientKey, code: u8, value: i32) -> io::Result<()> {
        let now = self.game_time();
        let Some(own) = self.own(client) else {
            warn!(%client, "Ignored a setting without a vehicle: {code} = {value}");
            return Ok(());
        };
        self.vehicles[own].change_setting(app, client, now, code, value)
    }

    // -- Its tick --

    /// End a tick: go to its next period when due, step it, once a client has its
    /// vehicle, break what shells got through, land the shells due, then tell what broke.
    pub fn tick(&mut self, app: &mut base::App) {

        let id = self.id;
        let ready = self.participants.iter().all(|participant| participant.ready);
        if let Some(period) = self.period.next(ready) {
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

        let now = self.game_time();
        for (through, yaw) in self.shells.take_through(now) {
            self.world.break_shot(through.instance, through.piece, yaw);
        }
        for (at, shooter, shot) in self.shells.take_landed(now) {
            if let Err(e) = self.land(app, at, shooter, &shot) {
                warn!(battle = id, shot = shot.id, "Failed to land a shot: {e}");
            }
        }
        if let Err(e) = self.areas.show(app, id, self.world.take_breakages()) {
            warn!(battle = id, "Failed to tell the destructibles broken: {e}");
        }

    }

    /// Step it by a tick: every vehicle moves as its controller drives it, a wreck or one
    /// nobody drives stopping where it was shoved, and those that ran into each other are
    /// pushed apart, see [`World::step`]. Then each client is shown the vehicles its own
    /// spots, and each vehicle fires the rest of its burst, a bot once its gun is on its
    /// enemy too, as its options say, and the clients see each where it is, see
    /// [`BattleVehicle::show`].
    fn step(&mut self, app: &mut base::App) -> io::Result<()> {

        let id = self.id;
        let now = self.game_time();
        let fighting = self.period.fighting();
        let drives = fighting && self.options.bots_drive;
        let bots_fire = fighting && self.options.bots_fire;
        let avatars = self.avatars();
        let Self { participants, vehicles, world, shells, targets, .. } = self;

        for index in 0..vehicles.len() {
            let enemy = nearest_enemy(vehicles, world, index);
            let vehicle = &mut vehicles[index];
            let alive = vehicle.alive();
            match (&mut vehicle.controller, alive) {
                (_, false) => world.vehicle_mut(vehicle.id).hull.flags = 0,
                (Controller::Bot(bot), true) => bot.drive(world, vehicle.id, enemy, drives),
                (Controller::Client(_) | Controller::Nobody, true) => {}
            }
        }
        world.step(TICK.as_secs_f32());

        for participant in participants.iter_mut() {
            participant.spot(app, vehicles, world)?;
        }

        for index in 0..vehicles.len() {

            let enemy = nearest_enemy(vehicles, world, index);

            let shoved = &world.vehicle(vehicles[index].id).hull;
            if matches!(vehicles[index].controller, Controller::Bot(_)) && (shoved.speed != 0.0 || shoved.slide != 0.0) {
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
            fill_targets(targets, vehicles, world, vehicles[index].entity.id());
            let vehicle = &mut vehicles[index];
            let driven = world.vehicle(vehicle.id);

            vehicle.put_out_fire(app, id, now)?;
            vehicle.show_loaded(app, now)?;

            let obstacles = Some(Obstacles {
                space: world.space(),
                rubble: world.rubble(),
            });
            let marker = driven.turret.gun_marker(&driven.hull);
            let (position, velocity, dispersion) = marker;
            let mut shot = vehicle.gun.fire_burst(now, position, velocity, dispersion, obstacles, targets);
            if let (None, Controller::Bot(bot), Some(enemy), true) = (&shot, &vehicle.controller, enemy, bots_fire) {
                if bot.on_target(driven, enemy) {
                    shot = vehicle.gun.fire(now, position, velocity, dispersion, obstacles, targets);
                }
            }
            if let Some(shot) = shot {
                info!(battle = id, entity_id = %vehicle.entity.id(), shot = shot.id, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
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
    /// see [`armor::hit`], or it explodes on the ground or a static model, a destructible
    /// one taking its damage, see [`World::shoot`], and an HE shell damaging the vehicles
    /// in its explosion's reach, see [`armor::explosion`]. A shell ricocheting off a
    /// vehicle or the ground goes on, see [`Self::ricochet`]; a static model stops it
    /// whole.
    fn land(&mut self, app: &mut base::App, at: f64, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        let id = self.id;
        let avatars = self.avatars();

        // What a destructible hit breaks the clients break as the shell explodes.
        let mut damaged = Vec::new();
        if let Landing::Static { piece: Some(hit), .. } = shot.landing {
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

        match shot.landing {
            Landing::Vehicle { entity_id, hit, transforms } => {
                let vehicle = self.vehicles.iter().find(|vehicle| vehicle.entity.id().0 == entity_id);
                let Some(collision) = vehicle.map(|vehicle| vehicle.collision.clone()) else { return Ok(()) };
                let Some(impact) = armor::hit(def, shot.distance(), shot.pen_factor, shot.end, shot.end_velocity, &collision, &transforms, &mut rand::thread_rng()) else {
                    debug!(battle = id, shot = shot.id, entity_id, "Hit a vehicle where it has no armour: {:?}", hit.part);
                    return Ok(());
                };
                info!(battle = id, shot = shot.id, entity_id, "Hit a vehicle: {:?}, {:?} {} mm at {:?}, {:.0} damage{}",
                    impact.outcome, impact.hit.part, impact.hit.material.map_or(0.0, |m| m.armor), impact.hit.local_point,
                    impact.damage, if impact.exploded { ", exploded" } else { "" });
                self.hit(app, shooter, entity_id, shot, &ShellHit::of_impact(&impact))?;
                if impact.outcome == Outcome::Ricochet {
                    let direction = armor::reflect(shot.end_velocity, impact.hit.normal);
                    self.ricochet(app, at, shooter, shot, direction, Some((entity_id, transforms)))?;
                }
                Ok(())
            }
            Landing::Terrain | Landing::Static { .. } => {
                if let Landing::Static { normal, .. } = shot.landing {
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
                let skims = match shot.landing {
                    Landing::Terrain => {
                        let normal = self.world.space().terrain.normal_at(shot.end.x, shot.end.z).unwrap_or(Vec3::Y);
                        armor::ricochets_off_ground(def, shot.end_velocity, normal, &armor::GROUND).then_some(normal)
                    }
                    _ => None,
                };
                if let Some(normal) = skims {
                    let direction = armor::reflect(shot.end_velocity, normal);
                    self.ricochet(app, at, shooter, shot, direction, None)?;
                } else if def.kind() == ShellKind::HighExplosive {
                    self.splash(app, shooter, shot)?;
                }
                Ok(())
            }
            Landing::Range => Ok(()),
        }

    }

    /// A shell ricochets off what it landed on, at game time `at`, along `direction`,
    /// losing penetration as [`armor::ricochet_pen`] says, see [`Shot::ricochet`]: the
    /// players see it go on, as the server shows it, with a tracer of the same shot.
    /// `landed` is the vehicle it ricochets off, placed as it hit it, which it leaves
    /// from.
    fn ricochet(
        &mut self,
        app: &mut base::App,
        at: f64,
        shooter: EntityId,
        shot: &Shot,
        direction: Vec3,
        landed: Option<(u32, [Affine3A; 4])>,
    ) -> io::Result<()> {

        let id = self.id;
        let mut vehicles = self.shell_targets(shooter);
        if let Some((entity_id, transforms)) = landed {
            for vehicle in vehicles.iter_mut().filter(|vehicle| vehicle.entity_id == entity_id) {
                vehicle.transforms = transforms;
            }
        }
        let obstacles = Obstacles {
            space: self.world.space(),
            rubble: self.world.rubble(),
        };
        let Some(next) = shot.ricochet(direction, armor::ricochet_pen_factor(&shot.def), Some(obstacles), &vehicles) else {
            debug!(battle = id, shot = shot.id, ricochets = shot.ricochets, "Stopped a shell's ricochets: too many, or out of range");
            return Ok(());
        };
        info!(battle = id, shot = next.id, ricochets = next.ricochets,
            "Ricocheted a shell off {}, {:.0}% of its penetration left, landing at {:?} in {:.2} s",
            if landed.is_some() { "a vehicle" } else { "the ground" }, next.pen_factor * 100.0, next.end, next.flight_time);
        show_tracer(app, &self.avatars(), shooter, &next, true)?;
        self.shells.fly(at, shooter, next);
        Ok(())

    }

    /// An HE shell explodes on the ground: the vehicles in its reach take its damage,
    /// but the shooter's own.
    fn splash(&mut self, app: &mut base::App, shooter: EntityId, shot: &Shot) -> io::Result<()> {
        let explosions: Vec<_> = self.shell_targets(shooter).into_iter()
            .filter_map(|target| Some((target.entity_id, armor::explosion(&shot.def, shot.end, &target.collision, &target.transforms)?)))
            .collect();
        for (entity_id, explosion) in explosions {
            info!(battle = self.id, shot = shot.id, entity_id, "Exploded beside a vehicle: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            self.hit(app, shooter, entity_id, shot, &ShellHit::of_explosion(&explosion))?;
        }
        Ok(())
    }

    /// A vehicle of it alive takes a shell's damage, see [`BattleVehicle::take_damage`],
    /// every client listing it destroyed once killed, and the client that fired it, if
    /// any, learns the hit's result.
    fn hit(&mut self, app: &mut base::App, shooter: EntityId, entity_id: u32, shot: &Shot, shell_hit: &ShellHit) -> io::Result<()> {

        let id = self.id;
        let now = self.game_time();
        let vehicles_infos: Vec<_> = self.participants.iter().map(|participant| participant.vehicles_info).collect();
        let shooter_avatar = self.vehicles.iter()
            .find(|vehicle| vehicle.entity.id() == shooter)
            .and_then(|vehicle| vehicle.driver())
            .map(|driver| driver.avatar());
        let Some(vehicle) = self.vehicles.iter_mut().find(|vehicle| vehicle.entity.id().0 == entity_id && vehicle.alive()) else {
            return Ok(());
        };

        let killed = vehicle.take_damage(app, id, shooter, shot, shell_hit, now)?;
        if killed {
            vehicle.mark_killed(app, &vehicles_infos, shooter)?;
        }

        let Some(shooter_avatar) = shooter_avatar else { return Ok(()) };
        let mut flags = shell_hit.flags;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        app.entity(shooter_avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.entity.id(),
        }])

    }

}

/// Where the nearest enemy alive of the `index`th vehicle is, its hull's: what a bot
/// fights. None for a vehicle no bot drives.
fn nearest_enemy(vehicles: &[BattleVehicle], world: &World, index: usize) -> Option<Vec3> {
    let vehicle = &vehicles[index];
    if !matches!(vehicle.controller, Controller::Bot(_)) {
        return None;
    }
    let position = world.vehicle(vehicle.id).hull.position;
    vehicles.iter()
        .filter(|other| other.alive() && other.team != vehicle.team)
        .map(|other| other.hull_center(world))
        .min_by(|a, b| a.distance_squared(position).total_cmp(&b.distance_squared(position)))
}

/// Replace `targets` with the vehicles the shells of `shooter` can hit, as
/// [`Battle::shell_targets`], reusing its memory.
fn fill_targets(targets: &mut Vec<HitTarget>, vehicles: &[BattleVehicle], world: &World, shooter: EntityId) {
    targets.clear();
    targets.extend(vehicles.iter()
        .filter(|vehicle| vehicle.alive() && vehicle.entity.id() != shooter)
        .map(|vehicle| vehicle.hit_target(world)));
}


/// The two teams, as the clients know them.
const TEAMS: [u8; 2] = [1, 2];

/// The scale of the packed horizontal offsets in position updates, as captured: we only
/// send detailed ones.
const PACKED_XZ_SCALE: f32 = 0.007644442;

/// How far a vehicle spots others, in metres on the ground plane: the most any vehicle
/// can see, until vehicles have their own.
const VIEW_RANGE: f32 = 445.0;

/// How long a battle waits for the clients switched into it to enter before it is made
/// with those that did. Our choice: a client logs in again within a second or two.
const GATHER_WAIT: Duration = Duration::from_secs(20);
