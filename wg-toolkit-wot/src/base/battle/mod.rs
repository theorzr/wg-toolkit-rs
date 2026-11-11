//! The battles of the server: each in a space of its own, with the player's vehicle,
//! which the client drives, and bots on either side, see [`BattleOptions`]: every vehicle
//! the same, but who drives it, see [`Controller`].
//!
//! A battle holds all it needs, and only tells its clients what happens through the app
//! it is given, so that an app may run several, a thread of its own each, say. The base
//! app only finds a client's battle, see [`BaseThread::battle_tick`].
//!
//! ```text
//!   (here)    a battle: set up, its tick, its shells landing, its client's calls
//!   vehicle   a vehicle of it, a player's or a bot's: shown, spotted, its gun, damaged
//!   shells    its shells in flight, and their tracers
//!   hit       what a shell does to a vehicle, as the clients are told
//!   areas     what broke of its destructibles, by chunk
//!   spawn     where its vehicles start
//! ```

mod vehicle;
mod shells;
mod hit;
mod areas;
mod spawn;

use std::time::Instant;
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::{Affine3A, Vec2, Vec3};

use wgtk::app::base::{self, CellPlayerInit, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::app::entity::{AnyEntity, Audience, ComponentKey, EntityId, EntityKey};
use wgtk::app::dispatch::MethodCall;
use wgtk::res::fs::ResFilesystem;
use wgtk::space::Space;

use crate::wot::schema::{
    Avatar, AvatarCell, AvatarComponentsData, AvatarInfo, AvatarVehiclesInfo, ShotResult, VehicleCell,
};
use crate::wot::item_defs::{ShellKind, VehicleDef};
use crate::wot::arena_defs::ArenaTypeDef;
use crate::wot::items::VehicleDescr;

use super::{
    avatar_cell_data, invalid_data, BaseThread, Player, VehicleConfig, EXPLOSION_STEP, GROUND_EFFECT_MATERIAL,
    NO_PREFAB_EFFECTS, TICK,
};
use super::gun::{HitTarget, Landing, Obstacles, Shot};
use super::physics::{Vehicle, World};
use super::armor::{self, Outcome};

use vehicle::{hold_gun, BattleVehicle, Controller};
use shells::{show_shot, show_tracer, Shells};
use hit::{ShellHit, HIT_FLAG_VEHICLE_KILLED};
use areas::{explosion_obstacle, Areas};
use spawn::Start;


/// A battle, with its client, see the module's documentation.
#[derive(Debug)]
pub struct Battle {
    space: SpaceKey,
    /// When it started, at 0: the battle's clock.
    clock: Instant,
    /// Who it has besides its client, and what bots do.
    options: BattleOptions,
    /// Its vehicles, its client's first, then the bots on either side, and the world
    /// they move in.
    vehicles: Vec<BattleVehicle>,
    world: World,
    shells: Shells,
    areas: Areas,
    /// What shells hit, refilled by each shooter, kept to reuse its memory.
    targets: Vec<HitTarget>,
    /// Whether its client has its vehicle: it runs from then on.
    running: bool,
    /// Whether its countdown is over: bots only aim until then.
    started: bool,
    /// Its client, its avatar, its vehicle's configuration, and the arena's vehicles on
    /// its info entity.
    client: ClientKey,
    avatar: EntityKey<Avatar>,
    config: VehicleConfig,
    vehicles_info: Option<ComponentKey<AvatarVehiclesInfo>>,
}

/// Who a battle has besides the player, as the server is launched with.
#[derive(Debug, Clone)]
pub struct BattleOptions {
    /// The bots on the player's side, and on the other.
    pub allies: u32,
    pub enemies: u32,
    /// The vehicles bots drive, each the next, round: the player's own when none.
    pub bot_vehicles: Vec<VehicleDef>,
    pub bots_fire: bool,
    pub bots_drive: bool,
    /// How far in front of the player the enemies start, if not at their spawn.
    pub enemy_distance: Option<f32>,
    /// Where the player starts, if not at its team's start, and its yaw in radians.
    pub spawn: Option<(Vec2, f32)>,
}

/// What a battle is made of, its client aside.
#[derive(Debug)]
pub struct BattleSetup<'a> {
    /// Its map, and the map's space, which its vehicles move in.
    pub map: &'a ArenaTypeDef,
    pub space: Arc<Space>,
    pub options: BattleOptions,
    /// The vehicle bots drive when the options give none: the player's.
    pub own_def: Option<VehicleDef>,
}

impl Battle {

    /// A battle in its own space, with its map, for a client entering it with its avatar:
    /// the vehicle of its player, the avatar's cell side on it, then the bots and the
    /// arena's info entity, which the client gets in that order. It sees its allies and
    /// the enemies its vehicle spots, see [`Self::tick`].
    pub fn new(
        app: &mut base::App,
        setup: BattleSetup,
        space: SpaceKey,
        client: ClientKey,
        avatar: EntityKey<Avatar>,
        player: Player,
        config: VehicleConfig,
    ) -> io::Result<Self> {

        let BattleSetup { map, space: map_space, options, own_def } = setup;
        let terrain = &map_space.terrain;
        let mut world = World::new(map_space.clone(), Some(map.bounds));

        let start = match options.spawn {
            Some((at, yaw)) => Start::at(at, yaw, terrain),
            None => Start::team(map, terrain, PLAYER_TEAM),
        };
        let own = BattleVehicle::player(app, &mut world, avatar, space, &config, player, PLAYER_TEAM, start)?;
        let own_id = own.entity.id();
        let hull = &world.vehicle(own.id).hull;
        let start = Start {
            position: hull.position,
            direction: hull.direction(),
        };

        app.create_cell_player(avatar, space, CellPlayerInit {
            vehicle_id: own_id.0,
            position: start.position,
            direction: start.direction,
            packed_xz_scale: PACKED_XZ_SCALE,
            data: avatar_cell_data(PLAYER_TEAM, own_id),
            components_data: AvatarComponentsData::default(),
        })?;
        app.set_interest(client, Interest::Manual)?;

        // The bots, allies then enemies, each team in rows from its start, the player
        // first in its own. They drive the given vehicles in turn, else the player's.
        let enemy_start = match options.enemy_distance {
            Some(distance) => start.facing(distance, terrain),
            None => Start::team(map, terrain, ENEMY_TEAM),
        };
        let sides = (1..=options.allies).map(|slot| (PLAYER_TEAM, start, slot))
            .chain((0..options.enemies).map(|slot| (ENEMY_TEAM, enemy_start, slot)));
        let mut vehicles = vec![own];
        for (index, (team, team_start, slot)) in sides.enumerate() {
            let (def, descr) = match &own_def {
                Some(def) => (def, config.descr.clone()),
                None => {
                    let def = &options.bot_vehicles[index % options.bot_vehicles.len()];
                    (def, def.descr.encode())
                }
            };
            let start = team_start.slot(slot, terrain);
            let name = format!("Bot {}", index + 1);
            let bot = BattleVehicle::bot(app, &mut world, space, def, &descr, team, name, start, vehicles.len() as u32, index as i32)?;
            info!(%client, entity_id = %bot.entity.id(), team, "Added a bot to the arena: {}, {}", bot.info.name, def.name);
            vehicles.push(bot);
        }

        // The arena's info entity, created right away as entities without an alias are,
        // with its vehicles.
        let info = app.create_entity::<AvatarInfo>(space, EntityInit {
            visibility: Visibility::Always,
            ..EntityInit::default()
        })?;
        let mut entity = app.entity(info)?;
        entity.set_avatarID(avatar.id())?;
        let mut vehicles_info = entity.add_component::<AvatarVehiclesInfo>(Audience::AllClients)?;
        vehicles_info.set_vehiclesInfo(vehicles.iter().map(|vehicle| vehicle.info.clone()).collect())?;
        let vehicles_info = Some(vehicles_info.key());
        info!(%client, entity_id = %info.id(), "Created the arena info");

        let areas = Areas::new(app, space, &world.space().statics)?;
        info!(%client, chunks = areas.len(), "Created the destructibles' areas");

        Ok(Self {
            space,
            clock: Instant::now(),
            options,
            vehicles,
            world,
            shells: Shells::default(),
            areas,
            targets: Vec::new(),
            running: false,
            started: false,
            client,
            avatar,
            config,
            vehicles_info,
        })

    }

    /// The space it is in.
    pub fn space(&self) -> SpaceKey {
        self.space
    }

    /// Its countdown is over: the bots drive and fire from now on.
    pub fn start(&mut self) {
        self.started = true;
    }

    /// Its game time, in seconds.
    fn game_time(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// The vehicle its client drives.
    fn own(&self) -> Option<&BattleVehicle> {
        self.vehicles.iter().find(|vehicle| vehicle.controller.is_client())
    }

    fn own_mut(&mut self) -> Option<&mut BattleVehicle> {
        self.vehicles.iter_mut().find(|vehicle| vehicle.controller.is_client())
    }

    /// The simulated vehicle its client drives.
    fn own_vehicle_mut(&mut self) -> Option<&mut Vehicle> {
        let id = self.own()?.id;
        Some(self.world.vehicle_mut(id))
    }

    /// The vehicles the shells of `shooter` can hit: every one alive but its own.
    fn shell_targets(&self, shooter: EntityId) -> Vec<HitTarget> {
        let mut targets = Vec::with_capacity(self.vehicles.len());
        fill_targets(&mut targets, &self.vehicles, &self.world, shooter);
        targets
    }

    // -- Its tick --

    /// End a tick: step it, once its client has its vehicle, break what shells got
    /// through, land the shells due, then tell what broke.
    pub fn tick(&mut self, app: &mut base::App) {

        let client = self.client;
        if self.running {
            if let Err(e) = self.step(app) {
                warn!(%client, "Failed to step the battle: {e}");
            }
        }

        let now = self.game_time();
        for (through, yaw) in self.shells.take_through(now) {
            self.world.break_shot(through.instance, through.piece, yaw);
        }
        for (at, shooter, shot) in self.shells.take_landed(now) {
            if let Err(e) = self.land(app, at, shooter, &shot) {
                warn!(%client, shot = shot.id, "Failed to land a shot: {e}");
            }
        }
        if let Err(e) = self.areas.show(app, client, self.world.take_breakages()) {
            warn!(%client, "Failed to tell the destructibles broken: {e}");
        }

    }

    /// Step it by a tick: every vehicle moves as its controller drives it, a wreck
    /// stopping where it was shoved, and those that ran into each other are pushed apart,
    /// see [`World::step`]. Then each fires the rest of its burst, a bot once its gun is
    /// on its enemy too, as its options say, and the client sees each where it is, see
    /// [`BattleVehicle::show`].
    fn step(&mut self, app: &mut base::App) -> io::Result<()> {

        let client = self.client;
        let now = self.game_time();
        let drives = self.started && self.options.bots_drive;
        let bots_fire = self.started && self.options.bots_fire;
        let own = self.own().map(|own| (own.team, self.world.vehicle(own.id).hull.position));
        let Self { avatar, vehicles, world, shells, targets, .. } = self;

        for index in 0..vehicles.len() {
            let enemy = nearest_enemy(vehicles, world, index);
            let vehicle = &mut vehicles[index];
            let alive = vehicle.alive();
            match (&mut vehicle.controller, alive) {
                (_, false) => world.vehicle_mut(vehicle.id).hull.flags = 0,
                (Controller::Bot(bot), true) => bot.drive(world, vehicle.id, enemy, drives),
                (Controller::Client(_), true) => {}
            }
        }
        world.step(TICK.as_secs_f32());

        for index in 0..vehicles.len() {

            let enemy = nearest_enemy(vehicles, world, index);

            let shoved = &world.vehicle(vehicles[index].id).hull;
            if matches!(vehicles[index].controller, Controller::Bot(_)) && (shoved.speed != 0.0 || shoved.slide != 0.0) {
                debug!(
                    %client,
                    entity_id = %vehicles[index].entity.id(),
                    position = ?shoved.position,
                    yaw = shoved.yaw(),
                    speed = shoved.speed,
                    slide = shoved.slide,
                    "Shoved a bot",
                );
            }

            vehicles[index].spot(app, client, world, own)?;
            if !vehicles[index].alive() {
                vehicles[index].show_wreck(app, world.vehicle(vehicles[index].id))?;
                continue;
            }
            fill_targets(targets, vehicles, world, vehicles[index].entity.id());
            let vehicle = &mut vehicles[index];
            let driven = world.vehicle(vehicle.id);

            vehicle.put_out_fire(app, client, now)?;
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
                info!(%client, entity_id = %vehicle.entity.id(), shot = shot.id, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
                vehicle.show_burst(app)?;
                show_shot(app, *avatar, vehicle.entity, &shot)?;
                shells.fly(now, vehicle.entity.id(), shot);
            }

            vehicle.show(app, driven)?;
            vehicle.show_own(app, client, *avatar, driven, marker)?;

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

        let client = self.client;

        // What a destructible hit breaks the client breaks as the shell explodes.
        let mut damaged = Vec::new();
        if let Landing::Static { piece: Some(hit), .. } = shot.landing {
            let yaw = shot.end_velocity.x.atan2(shot.end_velocity.z);
            let damage = shot.def.damage_at(shot.distance());
            let broken = self.world.shoot(hit.instance, hit.piece, damage, yaw);
            info!(
                %client,
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
        app.entity(self.avatar)?.stopTracer(shot.id, shot.end)?;

        match shot.landing {
            Landing::Vehicle { entity_id, hit, transforms } => {
                let vehicle = self.vehicles.iter().find(|vehicle| vehicle.entity.id().0 == entity_id);
                let Some(collision) = vehicle.map(|vehicle| vehicle.collision.clone()) else { return Ok(()) };
                let Some(impact) = armor::hit(def, shot.distance(), shot.pen_factor, shot.end, shot.end_velocity, &collision, &transforms, &mut rand::thread_rng()) else {
                    debug!(%client, shot = shot.id, entity_id, "Hit a vehicle where it has no armour: {:?}", hit.part);
                    return Ok(());
                };
                info!(%client, shot = shot.id, entity_id, "Hit a vehicle: {:?}, {:?} {} mm at {:?}, {:.0} damage{}",
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
                    debug!(%client, shot = shot.id, "Hit a static model at {:?}, facing {normal:?}", shot.end);
                }
                // EXPERIMENT: the explosion comes for a shot the client does not know, so
                // that it shows it from these arguments alone.
                app.entity(self.avatar)?.explodeProjectile(
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
                    damaged,
                )?;
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
    /// player sees it go on, as the server shows it, with a tracer of the same shot.
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

        let client = self.client;
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
            debug!(%client, shot = shot.id, ricochets = shot.ricochets, "Stopped a shell's ricochets: too many, or out of range");
            return Ok(());
        };
        info!(%client, shot = next.id, ricochets = next.ricochets,
            "Ricocheted a shell off {}, {:.0}% of its penetration left, landing at {:?} in {:.2} s",
            if landed.is_some() { "a vehicle" } else { "the ground" }, next.pen_factor * 100.0, next.end, next.flight_time);
        show_tracer(app, self.avatar, shooter, &next, true)?;
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
            info!(client = %self.client, shot = shot.id, entity_id, "Exploded beside a vehicle: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            self.hit(app, shooter, entity_id, shot, &ShellHit::of_explosion(&explosion))?;
        }
        Ok(())
    }

    /// A vehicle of it alive takes a shell's damage, see [`BattleVehicle::take_damage`],
    /// every client listing it destroyed once killed, and the client learns the hit's
    /// result when it fired it.
    fn hit(&mut self, app: &mut base::App, shooter: EntityId, entity_id: u32, shot: &Shot, shell_hit: &ShellHit) -> io::Result<()> {

        let (client, avatar, vehicles_info) = (self.client, self.avatar, self.vehicles_info);
        let now = self.game_time();
        let by_client = self.vehicles.iter().any(|vehicle| vehicle.entity.id() == shooter && vehicle.controller.is_client());
        let Some(vehicle) = self.vehicles.iter_mut().find(|vehicle| vehicle.entity.id().0 == entity_id && vehicle.alive()) else {
            return Ok(());
        };

        let killed = vehicle.take_damage(app, client, shooter, shot, shell_hit, now)?;
        if killed {
            vehicle.mark_killed(app, vehicles_info, shooter)?;
        }

        if !by_client {
            return Ok(());
        }
        let mut flags = shell_hit.flags;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        app.entity(avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.entity.id(),
        }])

    }

    // -- Its client --

    /// An entity of it was created on its client: its own vehicle, the client drives it
    /// from there on, see [`BattleVehicle::enter`], and it runs.
    pub fn entity_entered(&mut self, app: &mut base::App, entity: EntityKey<AnyEntity>) -> io::Result<()> {
        debug!(client = %self.client, %entity, "Created an entity on the client");
        let Self { vehicles, config, running, .. } = self;
        let Some(own) = vehicles.iter_mut().find(|vehicle| vehicle.controller.is_client()) else { return Ok(()) };
        if own.entity.any() != entity {
            return Ok(());
        }
        own.enter(app, config)?;
        *running = true;
        Ok(())
    }

    /// A cell method of its avatar its client called.
    pub fn avatar_call(&mut self, app: &mut base::App, call: AvatarCell) {

        let client = self.client;
        match call {
            AvatarCell::VehicleShoot {} => {
                if let Err(e) = self.shoot(app) {
                    warn!(%client, "Failed to shoot: {e}");
                }
                return;
            }
            AvatarCell::VehicleChangeSetting { code, value } => {
                if let Err(e) = self.change_setting(app, code, value) {
                    warn!(%client, "Failed to change setting {code} to {value}: {e}");
                }
                return;
            }
            _ => {}
        }

        let Some(driven) = self.own_vehicle_mut() else {
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

    /// A cell method of its vehicle its client called.
    pub fn vehicle_call(&mut self, call: VehicleCell) {

        let client = self.client;
        let Some(driven) = self.own_vehicle_mut() else {
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

    /// Fire its client's gun, if loaded: the shell's tracer and the gun's effects, of its
    /// whole burst, then one shell fewer, the shell it loads next and the reload, once
    /// the burst's other shells are fired too, see [`Self::step`]. Where the shell lands
    /// follows with [`Self::land`].
    fn shoot(&mut self, app: &mut base::App) -> io::Result<()> {

        let client = self.client;
        let now = self.game_time();
        let Some(own_id) = self.own().filter(|own| own.alive()).map(|own| own.entity.id()) else { return Ok(()) };
        let targets = self.shell_targets(own_id);
        let Self { avatar, vehicles, world, shells, .. } = self;
        let Some(own) = vehicles.iter_mut().find(|vehicle| vehicle.controller.is_client()) else { return Ok(()) };
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
        show_shot(app, *avatar, own.entity, &shot)?;
        shells.fly(now, own_id, shot);
        Ok(())

    }

    /// Its client changes one of its vehicle's settings, see
    /// [`BattleVehicle::change_setting`].
    fn change_setting(&mut self, app: &mut base::App, code: u8, value: i32) -> io::Result<()> {
        let client = self.client;
        let now = self.game_time();
        let Some(own) = self.own_mut() else {
            warn!(%client, "Ignored a setting without a vehicle: {code} = {value}");
            return Ok(());
        };
        own.change_setting(app, client, now, code, value)
    }

}

/// Where the nearest enemy alive of the `index`th vehicle is, its hull's: what a bot
/// fights. None for a vehicle a client drives.
fn nearest_enemy(vehicles: &[BattleVehicle], world: &World, index: usize) -> Option<Vec3> {
    let vehicle = &vehicles[index];
    if vehicle.controller.is_client() {
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

impl BaseThread {

    /// Get a client's avatar into its battle's space, with its map, see [`Battle::new`].
    pub fn enter_space(&mut self, client: ClientKey, avatar: EntityKey<Avatar>, space: SpaceKey, player: Player) -> io::Result<()> {

        let Some(config) = self.battle_clients.get(&client).map(|c| c.config.clone()) else { return Ok(()) };
        let own_def = if self.battle_options.bot_vehicles.is_empty() {
            let decoded = VehicleDescr::decode(&config.descr).map_err(|e| invalid_data(&e.to_string()))?;
            Some(VehicleDef::load_by_id(&ResFilesystem::new(self.game_dir.join("res"))?, decoded.nation, decoded.type_id)?)
        } else {
            None
        };
        let setup = BattleSetup {
            map: &self.map,
            space: self.space.clone(),
            options: self.battle_options.clone(),
            own_def,
        };
        let battle = Battle::new(&mut self.app, setup, space, client, avatar, player, config)?;
        if let Some(c) = self.battle_clients.get_mut(&client) {
            c.battle = Some(battle);
        }
        Ok(())

    }

    /// An entity of its battle was created on the client.
    pub fn entity_entered(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>) {
        let Self { app, battle_clients, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return };
        if let Err(e) = battle.entity_entered(app, entity) {
            warn!(%client, "Failed to drive the vehicle: {e}");
        }
    }

    /// The client called a cell method.
    pub fn cell_method(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>, call: &MethodCall) {
        let Self { app, battle_clients, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else {
            debug!(%client, %entity, "Ignored a cell method call out of a battle: {call:?}");
            return;
        };
        if let Some((_, call)) = app.decode::<AvatarCell>(entity, call) {
            battle.avatar_call(app, call);
        } else if let Some((_, call)) = app.decode::<VehicleCell>(entity, call) {
            battle.vehicle_call(call);
        } else {
            debug!(%client, %entity, "Ignored a cell method call: {call:?}");
        }
    }

    /// End a tick: tick every battle, see [`Battle::tick`], then give the clients what
    /// changed.
    pub fn battle_tick(&mut self) {
        for battle in self.battle_clients.values_mut().filter_map(|c| c.battle.as_mut()) {
            battle.tick(&mut self.app);
        }
        self.app.end_tick();
    }

}


/// The player's team, and the other.
const PLAYER_TEAM: u8 = 1;
const ENEMY_TEAM: u8 = 2;

/// The scale of the packed horizontal offsets in position updates, as captured: we only
/// send detailed ones.
const PACKED_XZ_SCALE: f32 = 0.007644442;

/// How far the player's vehicle spots others, in metres on the ground plane: the most
/// any vehicle can see, until vehicles have their own.
const VIEW_RANGE: f32 = 445.0;
