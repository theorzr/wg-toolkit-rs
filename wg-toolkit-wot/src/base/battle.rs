//! The battles of the server: each in a space of its own, with the player's vehicle,
//! which the client drives, and bots on either side, see [`BattleOptions`]: every vehicle
//! the same, but who drives it, see [`Controller`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::PI;
use std::time::Instant;
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::{Affine3A, Vec2, Vec3};

use wgtk::app::entity::{
    AnyEntity, Audience, Bytes, ComponentKey, Entity, EntityId, EntityKey, PathStep, Pickle,
};
use wgtk::app::base::{self, CellPlayerInit, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::space::destructibles::DestructibleKind;
use wgtk::app::dispatch::MethodCall;
use wgtk::space::terrain::Terrain;
use wgtk::res::fs::ResFilesystem;

use crate::wot::schema::{
    self, AreaDestructibles, Avatar, AvatarCell, AvatarComponentsData, AvatarInfo, AvatarVehiclesInfo, BattleDogTag, DeathInfo, DogTag,
    DogTagComponent, Fire, GunMarker, GunMarkerComponent, HitPoint, OwnVehicle, PerkEffects, PublicVehicleInfo,
    ShotResult, VehicleCell, VehicleInfo, VehicleSyncAttrs,
};
use crate::wot::collision::{VehicleCollision, VehicleHit, VehiclePart};
use crate::wot::arena_defs::{ArenaTypeDef, Spawn};
use crate::wot::item_defs::{ShellKind, VehicleDef};
use crate::wot::items::VehicleDescr;

use super::{
    ammo_list, avatar_cell_data, gun_reload_time, invalid_data, targeting_info, vehicle_health_info,
    vehicle_settings, BaseThread, VehicleConfig, PLAYER_ACCOUNT_DBID, EXPLOSION_STEP, GROUND_EFFECT_MATERIAL,
    NO_PREFAB_EFFECTS, SETTING_AUTOROTATION, SETTING_CURRENT_SHELLS, SETTING_NEXT_SHELLS, SETTING_RELOAD_PARTIAL_CLIP,
    SETTING_RESET_FULL_CLIP, TICK,
};
use super::armor::{self, Explosion, Impact, Outcome};
use super::gun::{Gun, HitTarget, Landing, Obstacles, Shot, ShotPiece};
use super::physics::{wrap_angle, Breakage, Piece, Vehicle, VehicleId, World};
use super::bot::Bot;


/// The battle of a client, in a space of its own, see [`BaseThread::enter_space`].
#[derive(Debug)]
pub struct Battle {
    avatar: EntityKey<Avatar>,
    space: SpaceKey,
    /// Its client's vehicle's configuration.
    config: VehicleConfig,
    /// When it started, at 0: the battle's clock.
    joined: Instant,
    /// Its vehicles, its client's first, then the bots on either side, and the world
    /// they move in.
    vehicles: Vec<BattleVehicle>,
    world: World,
    /// What shells hit, refilled by each shooter, kept to reuse its memory.
    targets: Vec<HitTarget>,
    /// Whether its client has its vehicle: the battle runs from then on.
    running: bool,
    /// The shells in flight, with the game time they land at and who fired them.
    shots: Vec<(f64, EntityId, Shot)>,
    /// What the shells in flight fly through, with the game time they get there and the
    /// yaw they go: it breaks then, see [`Shot::through`].
    shot_through: Vec<(f64, ShotPiece, f32)>,
    /// The arena's vehicles, on its info entity.
    vehicles_info: Option<ComponentKey<AvatarVehiclesInfo>>,
    /// Whether its countdown is over: bots only aim until then.
    started: bool,
    /// The map's chunks that have destructibles, by their id.
    areas: HashMap<u16, Area>,
}

/// A map chunk's destructibles entity, and what it lists as destroyed, see
/// [`AreaDestructibles`].
#[derive(Debug)]
struct Area {
    entity: EntityKey<AreaDestructibles>,
    modules: Vec<[u8; 3]>,
    fragiles: Vec<[u8; 3]>,
    columns: Vec<[u8; 3]>,
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

/// Where a vehicle starts.
#[derive(Debug, Clone, Copy)]
pub struct Start {
    pub position: Vec3,
    /// Roll, pitch and yaw.
    pub direction: Vec3,
}

impl Battle {

    /// The space it is in.
    pub fn space(&self) -> SpaceKey {
        self.space
    }

    /// Its countdown is over: the bots drive and fire from now on.
    pub fn start(&mut self) {
        self.started = true;
    }

    /// The battle's game time, in seconds.
    fn game_time(&self) -> f64 {
        self.joined.elapsed().as_secs_f64()
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

}

/// A vehicle of a battle, whoever drives it, with its entry in `vehiclesInfo`.
#[derive(Debug)]
struct BattleVehicle {
    entity: EntityKey<schema::Vehicle>,
    team: u8,
    /// Its `vehiclesInfo` entry, at `info_index`.
    info: VehicleInfo,
    info_index: u32,
    /// Who drives it.
    controller: Controller,
    /// The vehicle the server simulates, in the battle's world, its gun and its collision
    /// models.
    id: VehicleId,
    gun: Gun,
    collision: Arc<VehicleCollision>,
    health: i64,
    max_health: i64,
    /// Whether the client sees it, its own aside: its allies always, its enemies while
    /// its vehicle spots them, see [`VIEW_RANGE`].
    spotted: bool,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// The `isStrafing` last sent, see [`STRAFING_SLIDE`].
    strafing: bool,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
    /// Its fire, with the game time it goes out at, see [`FIRE_DURATION`].
    fire: Option<(ComponentKey<Fire>, f64)>,
}

impl BattleVehicle {

    fn alive(&self) -> bool {
        self.health > 0
    }

    /// What its client has of it, if a client drives it.
    fn driver(&self) -> Option<&ClientDriver> {
        match &self.controller {
            Controller::Client(driver) => Some(driver),
            Controller::Bot(_) => None,
        }
    }

}

/// Who drives a vehicle: its controls and its aim.
#[derive(Debug)]
enum Controller {
    /// The battle's client, through its avatar's and the vehicle's cell methods, see
    /// [`BaseThread::avatar_cell_call`].
    Client(ClientDriver),
    /// The server, see [`Bot`].
    Bot(Bot),
}

impl Controller {

    fn is_client(&self) -> bool {
        matches!(self, Self::Client(_))
    }

}

/// What only the client driving a vehicle knows of it, once it has it, see
/// [`BaseThread::vehicle_entered`].
#[derive(Debug, Default)]
struct ClientDriver {
    /// The vehicle's components the client has it with.
    own_vehicle: Option<ComponentKey<OwnVehicle>>,
    gun_marker: Option<ComponentKey<GunMarkerComponent>>,
    /// The avatar's `ownVehicleGear` last sent, sent again only once it changes.
    engine_gear: Option<u8>,
}

impl BaseThread {

    /// Get a client's avatar into its battle's space, with its map: create the vehicle of
    /// the player of the given name, the avatar's cell side on it, then the bots and the
    /// arena's info entity, which the client gets in that order. It sees its allies and
    /// the enemies its vehicle spots, see [`Self::step`].
    pub fn enter_space(&mut self, client: ClientKey, avatar: EntityKey<Avatar>, space: SpaceKey, name: String) -> io::Result<()> {

        let Some(config) = self.battle_clients.get(&client).map(|c| c.config.clone()) else { return Ok(()) };
        let mut world = World::new(self.space.clone(), Some(self.map.bounds));
        let own = self.create_player_vehicle(&mut world, avatar, space, &config, name)?;
        let own_id = own.entity.id();
        let own_descr = own.info.compDescr.0.clone();
        let hull = &world.vehicle(own.id).hull;
        
        let start = Start { 
            position: hull.position, 
            direction: hull.direction(),
        };
        
        let mut battle = Battle {
            avatar, 
            space, 
            config, 
            joined: Instant::now(), 
            vehicles: vec![own], 
            world, 
            targets: Vec::new(),
            running: false, 
            shots: Vec::new(), 
            shot_through: Vec::new(),
            vehicles_info: None, 
            started: false,
            areas: HashMap::new(),
        };

        self.app.create_cell_player(avatar, space, CellPlayerInit {
            vehicle_id: own_id.0,
            position: start.position,
            direction: start.direction,
            packed_xz_scale: PACKED_XZ_SCALE,
            data: avatar_cell_data(PLAYER_TEAM, own_id),
            components_data: AvatarComponentsData::default(),
        })?;
        self.app.set_interest(client, Interest::Manual)?;

        // The bots, allies then enemies, each team in rows from its start, the player
        // first in its own. They drive the given vehicles in turn, else the player's.
        let options = self.battle_options.clone();
        let own_def = if options.bot_vehicles.is_empty() {
            let decoded = VehicleDescr::decode(&own_descr).map_err(|e| invalid_data(&e.to_string()))?;
            Some(VehicleDef::load_by_id(&ResFilesystem::new(self.game_dir.join("res"))?, decoded.nation, decoded.type_id)?)
        } else {
            None
        };
        let enemy_start = match options.enemy_distance {
            Some(distance) => facing_from(start, distance, &self.space.terrain),
            None => team_start(&self.map, &self.space.terrain, ENEMY_TEAM),
        };
        let sides = (1..=options.allies).map(|slot| (PLAYER_TEAM, start, slot))
            .chain((0..options.enemies).map(|slot| (ENEMY_TEAM, enemy_start, slot)));
        for (index, (team, team_start, slot)) in sides.enumerate() {
            let (def, descr) = match &own_def {
                Some(def) => (def, own_descr.clone()),
                None => {
                    let def = &options.bot_vehicles[index % options.bot_vehicles.len()];
                    (def, def.descr.encode())
                }
            };
            let start = slot_start(team_start, slot, &self.space.terrain);
            let bot = self.create_bot(&mut battle.world, space, def, &descr, team, format!("Bot {}", index + 1), start, battle.vehicles.len() as u32, index as i32)?;
            info!(%client, entity_id = %bot.entity.id(), team, "Added a bot to the arena: {}, {}", bot.info.name, def.name);
            battle.vehicles.push(bot);
        }

        // The arena's info entity, created right away as entities without an alias are,
        // with its vehicles.
        let info = self.app.create_entity::<AvatarInfo>(space, EntityInit {
            visibility: Visibility::Always,
            ..EntityInit::default()
        })?;
        let mut entity = self.app.entity(info)?;
        entity.set_avatarID(avatar.id())?;
        let mut vehicles_info = entity.add_component::<AvatarVehiclesInfo>(Audience::AllClients)?;
        vehicles_info.set_vehiclesInfo(battle.vehicles.iter().map(|vehicle| vehicle.info.clone()).collect())?;
        battle.vehicles_info = Some(vehicles_info.key());
        info!(%client, entity_id = %info.id(), "Created the arena info");

        // The destructibles' entity of each chunk that has some, created right away, as
        // the real server does.
        let chunk_ids: BTreeSet<u16> = battle.world.space().statics.destructibles()
            .map(|(_, destructible)| destructible.chunk_id)
            .collect();
        for chunk_id in chunk_ids {
            let entity = self.app.create_entity::<AreaDestructibles>(space, EntityInit {
                position: area_position(chunk_id),
                visibility: Visibility::Always,
                ..EntityInit::default()
            })?;
            battle.areas.insert(chunk_id, Area {
                entity,
                modules: Vec::new(),
                fragiles: Vec::new(),
                columns: Vec::new(),
            });
        }
        info!(%client, chunks = battle.areas.len(), "Created the destructibles' areas");

        self.set_battle(client, battle);
        Ok(())

    }

    fn set_battle(&mut self, client: ClientKey, battle: Battle) {
        if let Some(c) = self.battle_clients.get_mut(&client) {
            c.battle = Some(battle);
        }
    }

    /// Create the player's vehicle in its battle's space, at its team's start, or where
    /// the battles' options spawn it, of the given configuration and named as the player:
    /// it belongs to the avatar, and the client drives it once it has it, see
    /// [`Self::vehicle_entered`].
    fn create_player_vehicle(
        &mut self,
        world: &mut World,
        avatar: EntityKey<Avatar>,
        space: SpaceKey,
        config: &VehicleConfig,
        name: String,
    ) -> io::Result<BattleVehicle> {

        let public = public_info(name, &config.descr, PLAYER_TEAM, config.max_health as u16, config.crew.len());

        let start = match self.battle_options.spawn {
            Some((at, yaw)) => spawn_start(at, yaw, &self.space.terrain),
            None => team_start(&self.map, &self.space.terrain, PLAYER_TEAM),
        };
        let vehicle = self.app.create_entity::<schema::Vehicle>(space, EntityInit {
            position: start.position,
            direction: start.direction,
            alias: true,
            controlled_by: Some(avatar.any()),
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(%avatar, entity_id = %vehicle.id(), "Created the vehicle, at {:?}", start.position);

        let driven = Vehicle::new(start.position, start.direction, config.mobility, config.aiming.clone());
        let info = vehicle_info(vehicle.id(), &public, PLAYER_ACCOUNT_DBID);
        let mut entity = self.app.entity(vehicle)?;
        init_vehicle(&mut entity, avatar.id(), public, driven.turret.gun_angles_packed())?;

        let empty = || Pickle(wgtk::pickle::Value::Dict(Default::default()));
        entity.set_isMyVehicle(true)?;
        entity.set_quickShellChangerFactor(QUICK_SHELL_CHANGER_FACTOR)?;
        entity.set_onRespawnReloadTimeFactor(1.0)?;
        entity.set_perkEffects(PerkEffects::default())?;
        entity.set_perks(Vec::new())?;
        entity.set_perksRibbonNotify(Vec::new())?;
        entity.set_crewCompactDescrs(config.crew.iter().cloned().map(Bytes).collect())?;
        entity.set_enhancements(empty())?;
        entity.set_setups(Pickle(config.setups.clone()))?;
        entity.set_setupsIndexes(Pickle(config.setup_indexes.clone()))?;
        entity.set_vehPerks(empty())?;
        entity.set_disabledSwitches(Vec::new())?;
        entity.set_ownVehiclePosition(None)?;

        let ammo = config.shots.iter().cloned().zip(config.ammo.iter().map(|&(_, count)| count)).collect();
        Ok(BattleVehicle {
            entity: vehicle,
            team: PLAYER_TEAM,
            info,
            info_index: 0,
            controller: Controller::Client(ClientDriver::default()),
            id: world.add(driven),
            gun: Gun::new(ammo, config.aiming.reload_time)
                .with_magazine(config.magazine)
                .with_quick_change_factor(QUICK_SHELL_CHANGER_FACTOR),
            collision: config.collision.clone(),
            health: config.max_health.into(),
            max_health: config.max_health.into(),
            spotted: true,
            engine_mode: None,
            strafing: false,
            damage_stickers: Vec::new(),
            fire: None,
        })

    }

    /// The client has its vehicle: from there on it drives it, and the battle runs. Then
    /// its `OwnVehicle` component: the client only turns its turret once it knows how fast
    /// it can. Its `GunMarkerComponent` too, for the client's server reticle.
    fn vehicle_entered(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let Battle { config, vehicles, running, .. } = battle;
        let Some(own) = vehicles.iter_mut().find(|vehicle| vehicle.controller.is_client()) else { return Ok(()) };

        let entity_id = own.entity.id();
        let mut entity = self.app.entity(own.entity)?;
        let mut own_vehicle = entity.add_component::<OwnVehicle>(Audience::OwnClient)?;
        // No crew or equipment bonus: the view range all vehicles have here.
        own_vehicle.set_syncVehicleAttrs(VehicleSyncAttrs { circularVisionRadius: VIEW_RANGE as u16, gunPiercing: 1.0, gunShotsSpeed: 1.0 })?;
        own_vehicle.set_targetingInfo(Some(targeting_info(config)))?;
        // What the real server sends right after, as updates: the ammunition, the
        // shells loaded, the reload and the health.
        own_vehicle.set_vehicleAmmoList(gun_ammo_list(&own.gun))?;
        own_vehicle.set_vehicleSettings(vehicle_settings(entity_id, &own.gun))?;
        own_vehicle.set_vehicleGunReloadTime(gun_reload_time(config.aiming.reload_time, 0.0, 0.0))?;
        own_vehicle.set_vehicleHealthInfo(vehicle_health_info(entity_id, own.health))?;
        let own_vehicle = own_vehicle.key();
        let mut marker = entity.add_component::<GunMarkerComponent>(Audience::OwnClient)?;
        marker.set_gunMarker(None)?;
        let gun_marker = marker.key();

        own.controller = Controller::Client(ClientDriver {
            own_vehicle: Some(own_vehicle),
            gun_marker: Some(gun_marker),
            engine_gear: None,
        });
        *running = true;
        Ok(())

    }

    /// Create a bot in its battle's space, of this definition and descriptor, in this
    /// team, no avatar, with its `vehiclesInfo` entry at the given index: the `index`th
    /// bot, its shots numbered apart from the others'.
    #[allow(clippy::too_many_arguments)]
    fn create_bot(
        &mut self,
        world: &mut World,
        space: SpaceKey,
        def: &VehicleDef,
        compact_descr: &[u8],
        team: u8,
        name: String,
        start: Start,
        info_index: u32,
        index: i32,
    ) -> io::Result<BattleVehicle> {

        let Start { position, direction } = start;
        let ammo = def.shots.iter().cloned().zip(def.default_ammo().into_iter().map(|(_, count)| count)).collect();
        let gun = Gun::new(ammo, def.aiming.reload_time).with_magazine(def.magazine).with_shot_ids_from(BOT_FIRST_SHOT_ID * (index + 1));
        let driven = Vehicle::new(position, direction, def.mobility, def.aiming.clone());
        let gun_angles = driven.turret.gun_angles_packed();

        let public = public_info(name, compact_descr, team, def.max_health as u16, def.crew_roles.len());

        let vehicle = self.app.create_entity::<schema::Vehicle>(space, EntityInit {
            position,
            direction,
            alias: true,
            controlled_by: None,
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(entity_id = %vehicle.id(), team, "Created a bot: {}, at {position:?}", def.name);
        let info = vehicle_info(vehicle.id(), &public, 0);
        init_vehicle(&mut self.app.entity(vehicle)?, EntityId(0), public, gun_angles)?;
        Ok(BattleVehicle {
            entity: vehicle,
            team,
            info,
            info_index,
            controller: Controller::Bot(Bot::default()),
            id: world.add(driven),
            gun,
            collision: def.collision.clone(),
            health: def.max_health.into(),
            max_health: def.max_health.into(),
            spotted: false,
            engine_mode: None,
            strafing: false,
            damage_stickers: Vec::new(),
            fire: None,
        })

    }

    /// Fire the client's gun, if loaded: the shell's tracer and the gun's effects, of its
    /// whole burst, then one shell fewer, the shell it loads next and the reload, once
    /// the burst's other shells are fired too, see [`Self::step`]. Where the shell lands
    /// follows with [`Self::land_shot`].
    fn shoot(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let avatar = battle.avatar;
        let Some(own_id) = battle.own().filter(|own| own.alive()).map(|own| own.entity.id()) else { return Ok(()) };
        let targets = shell_targets(battle, own_id);
        let Some(own) = battle.vehicles.iter_mut().find(|vehicle| vehicle.controller.is_client()) else { return Ok(()) };
        let driven = battle.world.vehicle(own.id);
        let (position, velocity, dispersion) = driven.turret.gun_marker(&driven.hull);
        let obstacles = Obstacles {
            space: &self.space,
            rubble: battle.world.rubble(),
        };
        let Some(shot) = own.gun.fire(now, position, velocity, dispersion, Some(obstacles), &targets) else {
            debug!(%client, "Refused a shot: not loaded, or out of shells");
            return Ok(());
        };
        info!(%client, shot = shot.id, burst = shot.burst, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
        if let Some(own_vehicle) = own.driver().and_then(|driver| driver.own_vehicle) {
            show_gun(&mut self.app, own_vehicle, own_id, &own.gun, now)?;
        }
        let vehicle = own.entity;
        fly_through(&mut battle.shot_through, now, &shot);
        battle.shots.push((now + f64::from(shot.flight_time), own_id, shot.clone()));

        show_shot(&mut self.app, avatar, vehicle, &shot)

    }

    /// A shell gets to its end, at game time `at` (what it flew through broke as it got
    /// there, see [`Shot::through`]): what it does to the vehicle it hits, by its armour,
    /// see [`armor::hit`], or it explodes on the ground or a static model, a destructible
    /// one taking its damage, see [`World::shoot`], and an HE shell damaging the vehicles
    /// in its explosion's reach, see [`armor::explosion`]. A shell ricocheting off a
    /// vehicle or the ground goes on, see [`Self::ricochet`]; a static model stops it
    /// whole.
    fn land_shot(&mut self, client: ClientKey, at: f64, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        // What a destructible hit breaks the client breaks as the shell explodes.
        let mut damaged = Vec::new();
        let battle = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut());
        if let (Landing::Static { piece: Some(hit), .. }, Some(battle)) = (shot.landing, battle) {
            let yaw = shot.end_velocity.x.atan2(shot.end_velocity.z);
            let damage = shot.def.damage_at(shot.distance());
            let broken = battle.world.shoot(hit.instance, hit.piece, damage, yaw);
            info!(
                %client,
                shot = shot.id,
                chunk_id = hit.piece.chunk_id,
                index = hit.piece.index,
                module = ?hit.piece.module,
                "Hit a destructible for {damage:.0} damage{}",
                if broken.is_empty() { "" } else { ", broke it" },
            );
            damaged.extend(broken.iter().filter_map(|breakage| chunk_obstacle(breakage.piece)));
        }

        let Some(battle) = self.battle_clients.get(&client).and_then(|c| c.battle.as_ref()) else { return Ok(()) };
        let avatar_key = battle.avatar;
        let def = &shot.def;
        self.app.entity(avatar_key)?.stopTracer(shot.id, shot.end)?;

        match shot.landing {
            Landing::Vehicle { entity_id, hit, transforms } => {
                let vehicle = battle.vehicles.iter().find(|vehicle| vehicle.entity.id().0 == entity_id);
                let Some(collision) = vehicle.map(|vehicle| vehicle.collision.clone()) else { return Ok(()) };
                let Some(impact) = armor::hit(def, shot.distance(), shot.pen_factor, shot.end, shot.end_velocity, &collision, &transforms, &mut rand::thread_rng()) else {
                    debug!(%client, shot = shot.id, entity_id, "Hit a vehicle where it has no armour: {:?}", hit.part);
                    return Ok(());
                };
                info!(%client, shot = shot.id, entity_id, "Hit a vehicle: {:?}, {:?} {} mm at {:?}, {:.0} damage{}",
                    impact.outcome, impact.hit.part, impact.hit.material.map_or(0.0, |m| m.armor), impact.hit.local_point,
                    impact.damage, if impact.exploded { ", exploded" } else { "" });
                self.damage(client, shooter, entity_id, shot, &ShellHit::of_impact(&impact))?;
                if impact.outcome == Outcome::Ricochet {
                    let direction = armor::reflect(shot.end_velocity, impact.hit.normal);
                    self.ricochet(client, at, shooter, shot, direction, Some((entity_id, transforms)))?;
                }
                Ok(())
            }
            Landing::Terrain | Landing::Static { .. } => {
                if let Landing::Static { normal, .. } = shot.landing {
                    debug!(%client, shot = shot.id, "Hit a static model at {:?}, facing {normal:?}", shot.end);
                }
                // EXPERIMENT: the explosion comes for a shot the client does not know, so
                // that it shows it from these arguments alone.
                self.app.entity(avatar_key)?.explodeProjectile(
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
                        let normal = self.space.terrain.normal_at(shot.end.x, shot.end.z).unwrap_or(Vec3::Y);
                        armor::ricochets_off_ground(def, shot.end_velocity, normal, &armor::GROUND).then_some(normal)
                    }
                    _ => None,
                };
                if let Some(normal) = skims {
                    let direction = armor::reflect(shot.end_velocity, normal);
                    self.ricochet(client, at, shooter, shot, direction, None)?;
                } else if def.kind() == ShellKind::HighExplosive {
                    self.splash(client, shooter, shot)?;
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
        client: ClientKey,
        at: f64,
        shooter: EntityId,
        shot: &Shot,
        direction: Vec3,
        landed: Option<(u32, [Affine3A; 4])>,
    ) -> io::Result<()> {

        let Self { app, battle_clients, space, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let mut vehicles = shell_targets(battle, shooter);
        if let Some((entity_id, transforms)) = landed {
            for vehicle in vehicles.iter_mut().filter(|vehicle| vehicle.entity_id == entity_id) {
                vehicle.transforms = transforms;
            }
        }
        let obstacles = Obstacles {
            space,
            rubble: battle.world.rubble(),
        };
        let Some(next) = shot.ricochet(direction, armor::ricochet_pen_factor(&shot.def), Some(obstacles), &vehicles) else {
            debug!(%client, shot = shot.id, ricochets = shot.ricochets, "Stopped a shell's ricochets: too many, or out of range");
            return Ok(());
        };
        info!(%client, shot = next.id, ricochets = next.ricochets,
            "Ricocheted a shell off {}, {:.0}% of its penetration left, landing at {:?} in {:.2} s",
            if landed.is_some() { "a vehicle" } else { "the ground" }, next.pen_factor * 100.0, next.end, next.flight_time);
        show_tracer(app, battle.avatar, shooter, &next, true)?;
        fly_through(&mut battle.shot_through, at, &next);
        battle.shots.push((at + f64::from(next.flight_time), shooter, next));
        Ok(())

    }

    /// An HE shell explodes on the ground: the vehicles in its reach take its damage,
    /// but the shooter's own.
    fn splash(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get(&client).and_then(|c| c.battle.as_ref()) else { return Ok(()) };
        let explosions: Vec<_> = shell_targets(battle, shooter).into_iter()
            .filter_map(|target| Some((target.entity_id, armor::explosion(&shot.def, shot.end, &target.collision, &target.transforms)?)))
            .collect();
        for (entity_id, explosion) in explosions {
            info!(%client, shot = shot.id, entity_id, "Exploded beside a vehicle: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            self.damage(client, shooter, entity_id, shot, &ShellHit::of_explosion(&explosion))?;
        }
        Ok(())

    }

    /// A vehicle of the battle takes a shell's damage, varied by up to [`DAMAGE_SPREAD`]
    /// either way: its health drops, in its `OwnVehicle` too when a client drives it, the
    /// clients see the hit, as the captured server shows one, and the client learns its
    /// result when it fired it. A damaging hit sets it on fire, which a kill puts out.
    fn damage(&mut self, client: ClientKey, shooter: EntityId, entity_id: u32, shot: &Shot, shell_hit: &ShellHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Battle { avatar, vehicles, vehicles_info, .. } = battle;
        let by_client = vehicles.iter().any(|vehicle| vehicle.entity.id() == shooter && vehicle.controller.is_client());
        let Some(vehicle) = vehicles.iter_mut().find(|vehicle| vehicle.entity.id().0 == entity_id && vehicle.alive()) else {
            return Ok(());
        };
        let amount = spread_damage(shell_hit.damage);
        let old_health = vehicle.health;
        vehicle.health = (old_health - amount).max(0);
        let new_health = vehicle.health;
        let killed = new_health == 0;
        let bounds = vehicle.collision.parts[shell_hit.hit.part as usize].bounds;
        if amount > 0 {
            info!(%client, entity_id, %shooter, "Damaged a vehicle: {amount}, {old_health} -> {new_health}{}", if killed { ", destroyed" } else { "" });
            if let Some(own_vehicle) = vehicle.driver().and_then(|driver| driver.own_vehicle) {
                self.app.component(own_vehicle)?.set_vehicleHealthInfo(vehicle_health_info(vehicle.entity.id(), new_health))?;
            }
        }

        let stickers = shell_hit.pierced().then(|| add_sticker(&mut vehicle.damage_stickers, shot, &shell_hit.hit, bounds)).flatten();
        let damage = Damage { shooter, amount, max_health: vehicle.max_health, old_health, new_health };
        show_damage(&mut self.app.entity(vehicle.entity)?, shot, shell_hit, bounds, &damage, stickers)?;

        match (vehicle.fire.take(), killed) {
            (Some((fire, _)), true) => self.app.remove_component(fire)?,
            (Some((fire, _)), false) => vehicle.fire = Some((fire, now + FIRE_DURATION)),
            (None, true) => {}
            (None, false) if amount == 0 => {}
            (None, false) => {
                let mut entity = self.app.entity(vehicle.entity)?;
                let mut fire = entity.add_component::<Fire>(Audience::AllClients)?;
                fire.set_fireInfo(None)?;
                vehicle.fire = Some((fire.key(), now + FIRE_DURATION));
                info!(%client, entity_id, "Set a vehicle on fire");
            }
        }
        if killed {
            mark_killed(&mut self.app, *vehicles_info, &mut vehicle.info, vehicle.info_index, shooter)?;
        }

        if !by_client {
            return Ok(());
        }
        let mut flags = shell_hit.flags;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        self.app.entity(*avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.entity.id(),
        }])

    }

    /// The client changes one of its vehicle's settings, by `VEHICLE_SETTING` code: the
    /// shell its gun loads next, or the one it is loaded with, which the gun then
    /// reloads, or it reloads its magazine. The client predicts the change, so it is
    /// given the gun back even when it doesn't change, to undo what it predicted then.
    fn change_setting(&mut self, client: ClientKey, code: u8, value: i32) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Some(own) = battle.own_mut() else {
            warn!(%client, "Ignored a setting without a vehicle: {code} = {value}");
            return Ok(());
        };
        let gun = &mut own.gun;

        let reloads = match code {
            SETTING_NEXT_SHELLS => {
                if !gun.select_next(value.into()) {
                    warn!(%client, "Refused the next shell: none left of {value}");
                }
                false
            }
            SETTING_CURRENT_SHELLS => {
                let reloads = gun.change_loaded(now, value.into());
                info!(%client, reloads, "Changed the loaded shell: {value}");
                reloads
            }
            SETTING_RELOAD_PARTIAL_CLIP => {
                let reloads = gun.reload_magazine(now);
                info!(%client, reloads, "Reloaded the magazine");
                reloads
            }
            // Only the client turns the hull in sniper mode here, and no gun empties its
            // full magazine on demand.
            SETTING_AUTOROTATION | 
            SETTING_RESET_FULL_CLIP => return Ok(()),
            _ => {
                debug!(%client, "Ignored a setting: {code} = {value}");
                return Ok(());
            }
        };

        let Some(own_vehicle) = own.driver().and_then(|driver| driver.own_vehicle) else { return Ok(()) };
        if reloads {
            show_gun(&mut self.app, own_vehicle, own.entity.id(), &own.gun, now)
        } else {
            self.app.component(own_vehicle)?.set_vehicleSettings(vehicle_settings(own.entity.id(), &own.gun))
        }

    }

    /// A cell method of its avatar the client called.
    fn avatar_cell_call(&mut self, client: ClientKey, call: AvatarCell) {
        
        match call {
            AvatarCell::VehicleShoot {} => {
                if let Err(e) = self.shoot(client) {
                    warn!(%client, "Failed to shoot: {e}");
                }
                return;
            }
            AvatarCell::VehicleChangeSetting { code, value } => {
                if let Err(e) = self.change_setting(client, code, value) {
                    warn!(%client, "Failed to change setting {code} to {value}: {e}");
                }
                return;
            }
            _ => {}
        }

        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(Battle::own_vehicle_mut) else {
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

    /// A cell method of its vehicle the client called.
    fn vehicle_cell_call(&mut self, client: ClientKey, call: VehicleCell) {
        
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(Battle::own_vehicle_mut) else {
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

    /// Step a battle by a tick, once its client has its vehicle: every vehicle moves as
    /// its controller drives it, a wreck stopping where it was shoved, and those that ran
    /// into each other are pushed apart, see [`World::step`]. Then each fires the rest of
    /// its burst, a bot once its gun is on its enemy too, as the battles' options say, and
    /// the client sees each where it is, see [`show_vehicle`].
    fn step(&mut self, client: ClientKey) -> io::Result<()> {

        let Self { app, battle_clients, battle_options, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).filter(|battle| battle.running) else {
            return Ok(());
        };
        let now = battle.game_time();
        let drives = battle.started && battle_options.bots_drive;
        let bots_fire = battle.started && battle_options.bots_fire;
        let own = battle.own().map(|own| (own.team, battle.world.vehicle(own.id).hull.position));
        let Battle { avatar, vehicles, world, shots, shot_through, targets, .. } = battle;

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

            spot(app, client, &mut vehicles[index], world, own)?;
            if !vehicles[index].alive() {
                let (entity, hull) = (vehicles[index].entity, &world.vehicle(vehicles[index].id).hull);
                if app.position(entity) != Some((hull.position, hull.direction())) {
                    app.set_position(entity, hull.position, hull.direction())?;
                }
                continue;
            }
            fill_targets(targets, vehicles, world, vehicles[index].entity.id());
            let vehicle = &mut vehicles[index];
            let driven = world.vehicle(vehicle.id);

            if let Some((fire, _)) = vehicle.fire.take_if(|&mut (_, until)| until <= now) {
                app.remove_component(fire)?;
                info!(%client, entity_id = %vehicle.entity.id(), "Put out a vehicle's fire");
            }

            // Its magazine full again, and loaded: the shells first, as the client expects.
            let loaded = vehicle.gun.take_loaded(now);
            if let (Some(own_vehicle), true) = (vehicle.driver().and_then(|driver| driver.own_vehicle), loaded) {
                let mut own_vehicle = app.component(own_vehicle)?;
                if vehicle.gun.has_magazine() {
                    own_vehicle.set_vehicleAmmoList(gun_ammo_list(&vehicle.gun))?;
                }
                own_vehicle.set_vehicleGunReloadTime(reload_time(&vehicle.gun, now))?;
            }

            let obstacles = Some(Obstacles {
                space: world.space(),
                rubble: world.rubble(),
            });
            let (position, velocity, dispersion) = driven.turret.gun_marker(&driven.hull);
            let mut shot = vehicle.gun.fire_burst(now, position, velocity, dispersion, obstacles, targets);
            if let (None, Controller::Bot(bot), Some(enemy), true) = (&shot, &vehicle.controller, enemy, bots_fire) {
                if bot.on_target(driven, enemy) {
                    shot = vehicle.gun.fire(now, position, velocity, dispersion, obstacles, targets);
                }
            }
            if let Some(shot) = shot {
                info!(%client, entity_id = %vehicle.entity.id(), shot = shot.id, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
                fly_through(shot_through, now, &shot);
                shots.push((now + f64::from(shot.flight_time), vehicle.entity.id(), shot.clone()));
                // Its reload started with the burst, see `Gun::fire`.
                if let Some(own_vehicle) = vehicle.driver().and_then(|driver| driver.own_vehicle) {
                    let mut own_vehicle = app.component(own_vehicle)?;
                    own_vehicle.set_vehicleAmmoList(gun_ammo_list(&vehicle.gun))?;
                    own_vehicle.set_vehicleSettings(vehicle_settings(vehicle.entity.id(), &vehicle.gun))?;
                }
                show_shot(app, *avatar, vehicle.entity, &shot)?;
            }

            show_vehicle(app, vehicle, driven)?;
            if let Controller::Client(driver) = &mut vehicle.controller {
                show_own_vehicle(app, client, *avatar, driver, driven, (position, velocity, dispersion))?;
            }

        }
        Ok(())

    }

    /// End a tick: step every battle, break what shells got through, land the shells
    /// due, tell what broke, see [`Self::show_rubble`], then give the clients what
    /// changed.
    pub fn battle_tick(&mut self) {

        let clients: Vec<_> = self.battle_clients.iter()
            .filter(|(_, c)| c.battle.is_some())
            .map(|(&client, _)| client)
            .collect();
        for &client in &clients {
            if let Err(e) = self.step(client) {
                warn!(%client, "Failed to step the battle: {e}");
            }
        }

        for &client in &clients {
            let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { continue };
            let now = battle.game_time();
            let (reached, ahead): (Vec<_>, _) = std::mem::take(&mut battle.shot_through)
                .into_iter()
                .partition(|&(at, _, _)| at <= now);
            battle.shot_through = ahead;
            for (_, through, yaw) in reached {
                battle.world.break_shot(through.instance, through.piece, yaw);
            }
            let (landed, flying): (Vec<_>, _) = std::mem::take(&mut battle.shots).into_iter().partition(|&(at, _, _)| at <= now);
            battle.shots = flying;
            for (at, shooter, shot) in landed {
                if let Err(e) = self.land_shot(client, at, shooter, &shot) {
                    warn!(%client, shot = shot.id, "Failed to land a shot: {e}");
                }
            }
            if let Err(e) = self.show_rubble(client) {
                warn!(%client, "Failed to tell the destructibles broken: {e}");
            }
        }

        self.app.end_tick();

    }

    /// Tell a battle's client what broke this tick, rammed or shot: each list once,
    /// however much of it broke, a house bringing down a dozen modules at once.
    fn show_rubble(&mut self, client: ClientKey) -> io::Result<()> {
        let Self { app, battle_clients, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let Battle { world, areas, .. } = battle;
        let mut broken = HashSet::new();
        for breakage in world.take_breakages() {
            if let Some(list) = record_breakage(client, areas, breakage) {
                broken.insert(list);
            }
        }
        for (chunk_id, kind) in broken {
            show_breakages(app, &areas[&chunk_id], kind)?;
        }
        Ok(())
    }

    /// An entity of its battle was created on the client.
    pub fn entity_entered(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>) {
        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return };
        debug!(%client, %entity, "Created an entity on the client");
        if battle.own().is_some_and(|own| own.entity.any() == entity) {
            if let Err(e) = self.vehicle_entered(client) {
                warn!(%client, "Failed to drive the vehicle: {e}");
            }
        }
    }

    /// The client called a cell method.
    pub fn cell_method(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>, call: &MethodCall) {
        if let Some((_, call)) = self.app.decode::<AvatarCell>(entity, call) {
            self.avatar_cell_call(client, call);
        } else if let Some((_, call)) = self.app.decode::<VehicleCell>(entity, call) {
            self.vehicle_cell_call(client, call);
        } else {
            debug!(%client, %entity, "Ignored a cell method call: {call:?}");
        }
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
        .map(|other| hull_center(other, world))
        .min_by(|a, b| a.distance_squared(position).total_cmp(&b.distance_squared(position)))
}

/// Where a vehicle's hull is in the world.
fn hull_center(vehicle: &BattleVehicle, world: &World) -> Vec3 {
    Vec3::from(world.vehicle(vehicle.id).part_transforms(vehicle.collision.hull_position)[VehiclePart::Hull as usize].translation)
}

/// Show a vehicle to the client, or hide it, as its own vehicle spots it, given by its
/// team and position: its allies always, its enemies within [`VIEW_RANGE`].
fn spot(app: &mut base::App, client: ClientKey, vehicle: &mut BattleVehicle, world: &World, own: Option<(u8, Vec3)>) -> io::Result<()> {
    let Some((team, position)) = own else { return Ok(()) };
    if vehicle.controller.is_client() {
        return Ok(());
    }
    let offset = world.vehicle(vehicle.id).hull.position - position;
    let spotted = vehicle.team == team || offset.x.hypot(offset.z) <= VIEW_RANGE;
    if spotted == vehicle.spotted {
        return Ok(());
    }
    vehicle.spotted = spotted;
    if spotted {
        app.show(client, vehicle.entity)?;
    } else {
        app.hide(client, vehicle.entity)?;
    }
    info!(%client, entity_id = %vehicle.entity.id(), team = vehicle.team, "{} a vehicle", if spotted { "Spotted" } else { "Lost" });
    Ok(())
}

/// The position of a chunk's [`AreaDestructibles`] entity: near the middle of the chunk,
/// `areaDestructiblesPositionFromChunkID` in the client's `DestructiblesCache.py`.
fn area_position(chunk_id: u16) -> Vec3 {
    let x = i32::from(chunk_id >> 8) - 127;
    let z = i32::from(chunk_id & 0xff) - 127;
    let angle = PI / 10.0 * (x + z) as f32;
    Vec3::new(
        x as f32 * 100.0 + 50.0 + AREA_OFFSET * angle.cos(),
        0.0,
        z as f32 * 100.0 + 50.0 + AREA_OFFSET * angle.sin(),
    )
}

/// Keep what a shell flies through to break as it gets there, the shell fired at game
/// time `fired`.
fn fly_through(shot_through: &mut Vec<(f64, ShotPiece, f32)>, fired: f64, shot: &Shot) {
    let yaw = shot.velocity.x.atan2(shot.velocity.z);
    for &through in &shot.through {
        shot_through.push((fired + f64::from(through.time), through, yaw));
    }
}

/// A piece as an explosion lists it, see `explodeProjectile`: none of an index the code
/// has no room for.
fn chunk_obstacle(piece: Piece) -> Option<u32> {
    let index = u8::try_from(piece.index).ok()?;
    Some(u32::from(piece.chunk_id) << 16 | u32::from(index) << 8 | u32::from(piece.module.unwrap_or(0)))
}

/// List a destructible broken in its chunk's entity, encoded as its kind is, see
/// [`AreaDestructibles`]: the list it changed, by its chunk's id and its kind.
fn record_breakage(client: ClientKey, areas: &mut HashMap<u16, Area>, breakage: Breakage) -> Option<(u16, DestructibleKind)> {

    let Breakage { piece, kind, yaw, speed, shot } = breakage;
    let Some(area) = areas.get_mut(&piece.chunk_id) else {
        warn!(%client, chunk_id = piece.chunk_id, index = piece.index, "Failed to tell of a destructible broken out of the areas");
        return None;
    };

    let [high, low] = piece.index.to_be_bytes();
    match kind {
        DestructibleKind::Fragile => {
            area.fragiles.push([high, low, u8::from(shot)]);
        }
        DestructibleKind::FallingAtom => {
            let fall = ((speed / FALL_SPEED_STEP) as u8).min(3);
            area.columns.push([high, low, pack_angle(yaw) << 2 | fall]);
        }
        DestructibleKind::Structure => {
            let module = piece.module.unwrap_or_default();
            area.modules.push([high, low, (module & 0x7f) << 1 | u8::from(shot)]);
        }
        // Not numbered: they are speed trees, see `Rubble`.
        DestructibleKind::Tree => return None,
    }

    info!(%client, chunk_id = piece.chunk_id, index = piece.index, ?kind, module = ?piece.module, shot, "Broke a destructible");
    Some((piece.chunk_id, kind))

}

/// Tell the clients a chunk's list of destructibles of a kind broken, see
/// [`record_breakage`].
fn show_breakages(app: &mut base::App, area: &Area, kind: DestructibleKind) -> io::Result<()> {
    let mut entity = app.entity(area.entity)?;
    match kind {
        DestructibleKind::Fragile => entity.set_destroyedFragiles(area.fragiles.clone()),
        DestructibleKind::FallingAtom => entity.set_fallenColumns(area.columns.clone()),
        DestructibleKind::Structure => entity.set_destroyedModules(area.modules.clone()),
        DestructibleKind::Tree => Ok(()),
    }
}

/// An angle on 6 bits, as the client's native `packAngleToUint` does: from -π to π
/// over 0 to 63. Fitted: the two lamp posts the captured Pershing knocked down fell its
/// way, within 0.25 rad, read so.
fn pack_angle(angle: f32) -> u8 {
    ((wrap_angle(angle) + PI) / (2.0 * PI) * 63.0).round() as u8
}

/// Give the clients a vehicle as it is: where it is, its gun's angles, and its engine's
/// mode and whether it slides sideways once they change.
fn show_vehicle(app: &mut base::App, vehicle: &mut BattleVehicle, driven: &Vehicle) -> io::Result<()> {

    let Vehicle { hull, turret } = driven;
    app.set_position(vehicle.entity, hull.position, hull.direction())?;
    let mut view = app.entity(vehicle.entity)?;
    view.set_gunAnglesPacked(turret.gun_angles_packed())?;

    let mode = hull.engine_mode();
    if vehicle.engine_mode != Some(mode) {
        vehicle.engine_mode = Some(mode);
        view.set_engineMode(mode)?;
    }

    let strafing = hull.slide.abs() > STRAFING_SLIDE;
    if vehicle.strafing != strafing {
        vehicle.strafing = strafing;
        view.set_isStrafing(strafing)?;
    }

    Ok(())

}

/// Give the client driving a vehicle what only it knows of it: its server reticle, from
/// the gun's marker, its avatar where it is, what it aims from, which must turn with
/// the hull, and the engine's RPM, then its gear when it shifts.
fn show_own_vehicle(
    app: &mut base::App,
    client: ClientKey,
    avatar: EntityKey<Avatar>,
    driver: &mut ClientDriver,
    vehicle: &Vehicle,
    (position, velocity, dispersion): (Vec3, Vec3, f32),
) -> io::Result<()> {
    let Vehicle { hull, turret } = vehicle;
    if let Some(marker) = driver.gun_marker {
        app.component(marker)?.set_gunMarker(Some(GunMarker { gunPosition: position, shotVector: velocity, dispersion }))?;
    }
    // Every tick, to compare with what the client simulates.
    debug!(%client, "Stepped the vehicle: at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
        hull.position, hull.direction(), hull.speed, turret.yaw, turret.pitch);
    app.set_position(avatar, hull.position, Vec3::ZERO)?;
    let mut avatar_view = app.entity(avatar)?;
    avatar_view.set_ownVehicleAuxPhysicsData(hull.aux_physics_data())?;
    let (gear, _) = hull.engine();
    if driver.engine_gear != Some(gear) {
        driver.engine_gear = Some(gear);
        avatar_view.set_ownVehicleGear(gear)?;
    }
    Ok(())
}

/// Every client lists a vehicle destroyed, by whom, in its `vehiclesInfo` entry, at
/// `info_index`: the entry's changes apply once its generation changes too.
fn mark_killed(
    app: &mut base::App,
    vehicles_info: Option<ComponentKey<AvatarVehiclesInfo>>,
    info: &mut VehicleInfo,
    info_index: u32,
    killer: EntityId,
) -> io::Result<()> {
    let Some(vehicles_info) = vehicles_info else { return Ok(()) };
    let death = DeathInfo { victimID: info.vehicleID, killerID: killer, reasonID: ATTACK_REASON_SHOT, equipmentID: 0, numVehiclesAffected: 0 };
    info.deathInfo = Some(death.clone());
    info.isAlive = false;
    info.__generation += 1;
    let at = |field: &str| vec![PathStep::Index(info_index), PathStep::Field(field.into())];
    let mut vehicles_info = app.component(vehicles_info)?;
    vehicles_info.set_vehiclesInfo_at(at("deathInfo"), Some(death))?;
    vehicles_info.set_vehiclesInfo_at(at("isAlive"), false)?;
    vehicles_info.set_vehiclesInfo_at(at("__generation"), info.__generation)
}

/// The `vehiclesInfo` entry of a vehicle of the given id and public info, alive, as in
/// a random battle: no clan, badges nor missions.
fn vehicle_info(id: EntityId, public: &PublicVehicleInfo, account_dbid: u64) -> VehicleInfo {
    use wgtk::pickle::Value as PickleValue;
    let empty = || Pickle(PickleValue::Dict(Default::default()));
    VehicleInfo {
        vehicleID: id,
        isAlive: true,
        outfitCD: public.outfit.clone(),
        compDescr: public.compDescr.clone(),
        fakeName: public.name.clone(),
        name: public.name.clone(),
        team: public.team as i8,
        isAvatarReady: false,
        isTeamKiller: false,
        accountDBID: account_dbid,
        clanAbbrev: String::new(),
        clanDBID: 0,
        prebattleID: public.prebattleID,
        isPrebattleCreator: false,
        forbidInBattleInvitations: false,
        igrType: 0,
        avatarSessionID: id.0.to_string(),
        overriddenBadge: 0,
        customRoleSlotTypeId: 0,
        botDisplayStatus: 0,
        teamPanelMode: 0,
        maxHealth: public.maxHealth as i16,
        prestigeLevel: 0,
        prestigeGradeMarkID: 0,
        vehPostProgression: Vec::new(),
        personalMissionIDs: Vec::new(),
        personalMissionInfo: empty(),
        events: empty(),
        // Its badges, and its suffix badges.
        badges: Pickle(PickleValue::Tuple(vec![PickleValue::List(Vec::new()), PickleValue::List(Vec::new())])),
        ranked: vec![0, 0],
        deathInfo: None,
        respawnID: public.respawnID,
        stFrags: public.stFrags,
        frags: 0,
        activePetID: 0,
        tkills: 0,
        fogOfWar: 0,
        // Unknown, as captured.
        position: [i16::MIN; 2],
        __generation: 0,
    }
}

/// How fast a vehicle slides across its heading, in m/s, before the clients are told it
/// is strafing: their filter, which models its tracks, otherwise holds it back from
/// sliding, and draws it metres behind where it is shoved. Fitted on the collisions
/// capture, where the real server sets `isStrafing` on the ticks the player's vehicle
/// moves sideways faster than 0.03 to 0.05 m/s, pushed, skidding or sliding down a slope.
const STRAFING_SLIDE: f32 = 0.04;

/// How far from the middle of its chunk an [`AreaDestructibles`] entity is, in metres:
/// `RADIUS_FOR_LOCATION_AD` in the client's `DestructiblesCache.py`.
const AREA_OFFSET: f32 = 4.5;
/// How fast a vehicle goes for each step of a column's fall speed, in m/s, of 3 at
/// most. Fitted: the captured Pershing knocked down lamp posts at 5.5 and 12 m/s, which
/// fell at 2 and 3.
const FALL_SPEED_STEP: f32 = 2.5;

/// How much a shell's damage varies, either way, as a share of it.
const DAMAGE_SPREAD: f32 = 0.25;

/// `VEHICLE_HIT_FLAGS` of a shell piercing a vehicle, as captured: a direct projectile,
/// through armour and a module.
const HIT_FLAGS_PIERCED: u32 = HIT_FLAG_DIRECT | 0x100 | 0x10;
/// `VEHICLE_HIT_FLAGS`: a direct projectile, an explosion beside the vehicle, a
/// ricochet, armour of the vehicle's own, then a screen, not pierced, a screen pierced,
/// and the vehicle's own armour pierced by an explosion.
const HIT_FLAG_DIRECT: u32 = 0x100000;
const HIT_FLAG_EXTERNAL_EXPLOSION: u32 = 0x200000;
const HIT_FLAG_RICOCHET: u32 = 0x8;
const HIT_FLAG_ARMOR_NOT_PIERCED: u32 = 0x20;
const HIT_FLAG_SCREEN_NOT_PIERCED: u32 = 0x80;
const HIT_FLAG_SCREEN_PIERCED: u32 = 0x40;
const HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION: u32 = 0x2000;
/// `VEHICLE_HIT_EFFECT`.
const HIT_EFFECT_ARMOR_PIERCED_NO_DAMAGE: u8 = 0;
const HIT_EFFECT_FINAL_RICOCHET: u8 = 2;
const HIT_EFFECT_ARMOR_NOT_PIERCED: u8 = 3;
const HIT_EFFECT_ARMOR_PIERCED: u8 = 4;

/// `ATTACK_REASON.SHOT`'s index.
const ATTACK_REASON_SHOT: i8 = 0;

/// `VEHICLE_HIT_FLAGS.VEHICLE_KILLED`.
const HIT_FLAG_VEHICLE_KILLED: u32 = 1;

/// The first shot id of the first bot, of the next one twice that, and so on, apart from
/// the player's.
const BOT_FIRST_SHOT_ID: i32 = 1_000_000;

/// The player's team, and the other.
const PLAYER_TEAM: u8 = 1;
const ENEMY_TEAM: u8 = 2;

/// The scale of the packed horizontal offsets in position updates, as captured: we only
/// send detailed ones.
const PACKED_XZ_SCALE: f32 = 0.007644442;

/// How far behind its base a team starts, in metres, on maps without spawn points: about
/// as captured.
const SPAWN_BEHIND_BASE: f32 = 40.0;
/// How far inside the map's bounds a team starts at least, in metres.
const SPAWN_MARGIN: f32 = 20.0;

/// How far apart a team's vehicles start side by side, in metres, and its rows of
/// [`SLOT_ROW`] one behind the other.
const SLOT_SPACING: f32 = 12.0;
const ROW_SPACING: f32 = 15.0;
const SLOT_ROW: u32 = 5;

/// How far the player's vehicle spots others, in metres on the ground plane: the most
/// any vehicle can see, until vehicles have their own.
const VIEW_RANGE: f32 = 445.0;

/// The `quickShellChangerFactor` of a vehicle, as captured: how much faster it changes
/// shells, from its crew and equipment.
const QUICK_SHELL_CHANGER_FACTOR: f32 = 0.268;

/// The default dog tag's components, as captured: their ids and grades.
const DEFAULT_DOG_TAG: [(i32, i8); 2] = [(300, 9), (66, 0)];

/// How long a vehicle burns after its last hit, in seconds.
const FIRE_DURATION: f64 = 5.0;

/// Where a team's vehicle starts on a map: at its spawn point, else [`SPAWN_BEHIND_BASE`]
/// behind its base, facing the enemy's spawn, inside the map and on the ground.
fn team_start(map: &ArenaTypeDef, terrain: &Terrain, team: u8) -> Start {
    let at = |spawn: Spawn| match spawn { Spawn::Point(at) | Spawn::Base(at) => at };
    let own = map.spawns[team as usize - 1];
    let enemy = map.spawns[2 - team as usize];
    let forward = (at(enemy) - at(own)).normalize_or_zero();
    let at = match own {
        Spawn::Point(at) => at,
        Spawn::Base(at) => at - forward * SPAWN_BEHIND_BASE,
    };
    let (min, max) = map.bounds;
    let at = at.clamp(min + SPAWN_MARGIN, max - SPAWN_MARGIN);
    let height = terrain.height_at(at.x, at.y).unwrap_or(0.0);
    Start {
        position: Vec3::new(at.x, height, at.y),
        direction: Vec3::new(0.0, 0.0, forward.x.atan2(forward.y)),
    }
}

/// A start at the given point and yaw, on the ground.
fn spawn_start(at: Vec2, yaw: f32, terrain: &Terrain) -> Start {
    let height = terrain.height_at(at.x, at.y).unwrap_or(0.0);
    // In [-pi, pi) as angles are packed.
    let yaw = (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    Start {
        position: Vec3::new(at.x, height, at.y),
        direction: Vec3::new(0.0, 0.0, yaw),
    }
}

/// Where a team's vehicle of this slot starts, from its team's start: the first there,
/// the next ones beside it, either side in turn, in rows of [`SLOT_ROW`], each behind
/// the last, on the ground.
fn slot_start(start: Start, slot: u32, terrain: &Terrain) -> Start {
    let row = slot / SLOT_ROW;
    let column = slot % SLOT_ROW;
    let side = (column + 1) / 2;
    let side = if column % 2 == 1 { side as f32 } else { -(side as f32) };
    let yaw = start.direction.z;
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let mut position = start.position + right * side * SLOT_SPACING - forward * row as f32 * ROW_SPACING;
    position.y = terrain.height_at(position.x, position.z).unwrap_or(position.y);
    Start { position, direction: start.direction }
}

/// A start `distance` metres in front of another, facing it, on the ground.
fn facing_from(start: Start, distance: f32, terrain: &Terrain) -> Start {
    let yaw = start.direction.z;
    let mut position = start.position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * distance;
    position.y = terrain.height_at(position.x, position.z).unwrap_or(position.y);
    // In [-pi, pi) as angles are packed.
    let facing = (yaw + 2.0 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    Start { position, direction: Vec3::new(0.0, 0.0, facing) }
}

/// The `publicInfo` of a vehicle of the given name, descriptor, team, health and crew
/// size: no camouflage, marks, crew skins nor frags.
fn public_info(name: String, descr: &[u8], team: u8, max_health: u16, crew: usize) -> PublicVehicleInfo {
    PublicVehicleInfo {
        name,
        compDescr: Bytes(descr.to_vec()),
        outfit: Bytes::default(),
        outfitLevel: 0,
        index: 0,
        team,
        prebattleID: EntityId(0),
        marksOnGun: 0,
        crewGroups: vec![0; crew],
        commanderSkinID: 0,
        maxHealth: max_health,
        respawnID: 0,
        stFrags: 0,
    }
}

/// Set what every client gets of a fresh vehicle, before its creation on any: its
/// public info, its full health, its gun's angles, and its avatar if driven by a
/// client's. Its dog tag is the default one.
fn init_vehicle(entity: &mut Entity<'_, schema::Vehicle>, avatar: EntityId, public: PublicVehicleInfo, gun_angles: u16) -> io::Result<()> {
    let dog_tag = || DogTag {
        components: DEFAULT_DOG_TAG.iter()
            .map(|&(id, grade)| DogTagComponent { id, progress: 0.0, grade })
            .collect(),
    };
    entity.set_avatarID(avatar)?;
    entity.set_gunAnglesPacked(gun_angles)?;
    entity.set_dogTag(BattleDogTag { dogTag: dog_tag(), defaultDogTag: dog_tag(), showDogTagToKiller: false })?;
    entity.set_health(public.maxHealth as i16)?;
    entity.set_publicInfo(public)?;
    entity.set_isStrafing(false)?;
    entity.set_isCrewActive(true)?;
    entity.set_damageStickers(Vec::new())?;
    entity.set_publicStateModifiers(Vec::new())?;
    entity.set_vehPostProgression(Vec::new())
}

/// Show a shot to the player: its tracer, and the gun firing on the shooter.
fn show_shot(app: &mut base::App, avatar: EntityKey<Avatar>, shooter: EntityKey<schema::Vehicle>, shot: &Shot) -> io::Result<()> {
    show_tracer(app, avatar, shooter.id(), shot, false)?;
    // Its whole burst from the only gun, on its first shell.
    match shot.burst {
        Some(burst) => app.entity(shooter)?.showShooting(burst.min(u8::MAX.into()) as u8, 0, shot.def.kind_index),
        None => Ok(()),
    }
}

/// Show a shell's tracer to the player, flying from its start, off what it ricocheted
/// off for a ricochet: the client then shows it from there, not from the gun.
fn show_tracer(app: &mut base::App, avatar: EntityKey<Avatar>, shooter: EntityId, shot: &Shot, ricochet: bool) -> io::Result<()> {
    let def = &shot.def;
    app.entity(avatar)?.showTracer(
        shooter,
        shot.id,
        ricochet.into(),
        def.effects_index,
        def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
        def.kind_index,
        def.caliber,
        shot.start,
        shot.velocity,
        def.gravity,
        shot.range,
        0,
        0,
    )
}

/// Give the client its gun as it is: its shells and how many are left, in its magazine
/// too, the shells it loads, and its reload if any.
fn show_gun(app: &mut base::App, own: ComponentKey<OwnVehicle>, entity_id: EntityId, gun: &Gun, now: f64) -> io::Result<()> {
    let mut own = app.component(own)?;
    own.set_vehicleAmmoList(gun_ammo_list(gun))?;
    own.set_vehicleSettings(vehicle_settings(entity_id, gun))?;
    if gun.reload().is_some() {
        own.set_vehicleGunReloadTime(reload_time(gun, now))?;
    }
    Ok(())
}

/// The `vehicleAmmoList` of a gun.
fn gun_ammo_list(gun: &Gun) -> Vec<schema::VehicleAmmo> {
    ammo_list(gun.ammo_state().map(|(def, count, in_magazine)| (def.compact_descr.to_int(), count, in_magazine)))
}

/// Stop the gun where our turret is rather than at the client's angles, which its own
/// rotator computes slightly apart: the gun would move as it locks.
fn hold_gun(client: ClientKey, vehicle: &mut Vehicle, yaw: f32, pitch: f32) {
    let turret = &mut vehicle.turret;
    debug!(%client, "Held the gun: yaw {:.4}, pitch {:.4} (client: {yaw:.4}, {pitch:.4})", turret.yaw, turret.pitch);
    turret.hold(turret.yaw, turret.pitch);
}

/// The `vehicleGunReloadTime` of a gun at game time `now`: the reload going on, else
/// its whole reload's length once loaded. The real server gives a quick shell change as
/// a whole reload first, then as its share, and back on loaded; the share alone is
/// enough here.
fn reload_time(gun: &Gun, now: f64) -> schema::GunReloadTime {
    match gun.reload() {
        Some(reload) => {
            // Just started, exactly its length, as the client checks it.
            let left = (reload.end - now).max(0.0) as f32;
            let left = if (reload.base_time - left).abs() < 1e-3 { reload.base_time } else { left };
            gun_reload_time(reload.base_time, reload.end as f32, left)
        }
        None => gun_reload_time(gun.reload_time(), 0.0, 0.0),
    }
}

/// The vehicles the shells of `shooter` can hit: every one alive but its own.
fn shell_targets(battle: &Battle, shooter: EntityId) -> Vec<HitTarget> {
    let mut targets = Vec::with_capacity(battle.vehicles.len());
    fill_targets(&mut targets, &battle.vehicles, &battle.world, shooter);
    targets
}

/// Replace `targets` with the vehicles the shells of `shooter` can hit, as
/// [`shell_targets`], reusing its memory.
fn fill_targets(targets: &mut Vec<HitTarget>, vehicles: &[BattleVehicle], world: &World, shooter: EntityId) {
    targets.clear();
    targets.extend(vehicles.iter()
        .filter(|vehicle| vehicle.alive() && vehicle.entity.id() != shooter)
        .map(|vehicle| hit_target(vehicle, world)));
}

/// A vehicle as shells hit it: its collision models where its parts are.
fn hit_target(vehicle: &BattleVehicle, world: &World) -> HitTarget {
    let collision = vehicle.collision.clone();
    HitTarget {
        entity_id: vehicle.entity.id().0,
        transforms: world.vehicle(vehicle.id).part_transforms(collision.hull_position),
        collision,
    }
}

/// A shell's damage, varied by up to [`DAMAGE_SPREAD`] either way.
fn spread_damage(damage: f32) -> i64 {
    use rand::Rng;
    if damage <= 0.0 {
        return 0;
    }
    (damage * rand::thread_rng().gen_range(1.0 - DAMAGE_SPREAD..=1.0 + DAMAGE_SPREAD)).round() as i64
}

/// What a shell does to a vehicle, as the vehicle shows it and the shooter learns it.
struct ShellHit {
    /// Where it hits: where it pierces, stops or ricochets, or where its explosion beside
    /// the vehicle reaches its armour.
    hit: VehicleHit,
    /// Whether it exploded beside the vehicle, not on it.
    beside: bool,
    /// The `VEHICLE_HIT_EFFECT` of the point it hits.
    effect: u8,
    /// Its `VEHICLE_HIT_FLAGS` for the shooter, the kill aside.
    flags: u32,
    /// Its damage, before its spread.
    damage: f32,
}

impl ShellHit {

    fn of_impact(impact: &Impact) -> Self {
        let main = impact.hit.material.is_some_and(|material| material.is_main_armor());
        let (effect, flags) = match impact.outcome {
            Outcome::Pierced => (HIT_EFFECT_ARMOR_PIERCED, HIT_FLAGS_PIERCED),
            Outcome::Ricochet => (HIT_EFFECT_FINAL_RICOCHET, HIT_FLAG_DIRECT | HIT_FLAG_RICOCHET),
            Outcome::Screens => (HIT_EFFECT_ARMOR_PIERCED_NO_DAMAGE, HIT_FLAG_DIRECT | HIT_FLAG_SCREEN_PIERCED),
            Outcome::NotPierced => {
                let stopped = if main { HIT_FLAG_ARMOR_NOT_PIERCED } else { HIT_FLAG_SCREEN_NOT_PIERCED };
                let exploded = if impact.damage > 0.0 { HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION } else { 0 };
                (HIT_EFFECT_ARMOR_NOT_PIERCED, HIT_FLAG_DIRECT | stopped | exploded)
            }
        };
        Self { hit: impact.hit, beside: false, effect, flags, damage: impact.damage }
    }

    fn of_explosion(explosion: &Explosion) -> Self {
        let exploded = if explosion.damage > 0.0 { HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION } else { 0 };
        Self {
            hit: explosion.hit,
            beside: true,
            effect: HIT_EFFECT_ARMOR_NOT_PIERCED,
            flags: HIT_FLAG_EXTERNAL_EXPLOSION | exploded,
            damage: explosion.damage,
        }
    }

    /// Whether it pierced the vehicle's armour, leaving a sticker.
    fn pierced(&self) -> bool {
        self.effect == HIT_EFFECT_ARMOR_PIERCED
    }

}

/// A shell's damage to a vehicle.
struct Damage {
    shooter: EntityId,
    amount: i64,
    max_health: i64,
    old_health: i64,
    new_health: i64,
}

/// Show a vehicle taking a shell's damage: from a shot, one hit point, its segment
/// encoded over the hit part's `bounds`, its params as `HitParamsEncoder` encodes them,
/// or from an explosion beside it, its center; and the damage, also as a fraction of
/// its maximum health out of 255. Then, when it took some, its new health, and its
/// stickers, given when the shell added one, see [`add_sticker`].
fn show_damage(
    vehicle: &mut Entity<'_, schema::Vehicle>,
    shot: &Shot,
    shell_hit: &ShellHit,
    bounds: [Vec3; 2],
    damage: &Damage,
    stickers: Option<Vec<HitPoint>>,
) -> io::Result<()> {
    let def = &shot.def;
    let damage_factor = (damage.amount * 255 / damage.max_health.max(1)).clamp(0, 255) as u8;
    if shell_hit.beside {
        vehicle.showDamageFromExplosion(damage.shooter, shot.end, def.effects_index, damage.amount as i32, damage_factor, 0)?;
    } else {
        vehicle.showDamageFromShot(
            damage.shooter,
            vec![hit_point(shot, &shell_hit.hit, bounds, shell_hit.effect, shell_hit.effect)],
            def.effects_index,
            def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
            damage.amount as i32,
            damage_factor,
            0, // The last material is not a shield.
            shot.end_velocity.length(),
            0,
        )?;
    }
    if damage.amount == 0 {
        return Ok(());
    }
    vehicle.set_health(damage.new_health as i16)?;
    if let Some(stickers) = stickers {
        vehicle.set_damageStickers(stickers)?;
    }
    // No extended reason.
    vehicle.onHealthChanged(damage.new_health as i16, damage.old_health as i16, damage.shooter, ATTACK_REASON_SHOT as u8, -1)
}

/// A `VEHICLE_HIT_POINT` for a shell at `hit` with this `VEHICLE_HIT_EFFECT`, its
/// segment encoded over the hit part's `bounds` with `context`: the effect for
/// `showDamageFromShot`, a sticker id for `damageStickers`. Its params are as
/// `HitParamsEncoder` encodes them.
fn hit_point(shot: &Shot, hit: &VehicleHit, bounds: [Vec3; 2], effect: u8, context: u8) -> HitPoint {
    let def = &shot.def;
    let params = ((def.caliber * 10.0) as u32 & 0xFFFF) << 8 | u32::from(def.kind_index & 15) << 4 | u32::from(effect);
    HitPoint { networkID: 0, segment: hit.encode_segment(bounds, context), params }
}

/// Add the shell's sticker to a vehicle's `damageStickers`, if it leaves one, as
/// `encodeStickerIdData` encodes it (not parametrized: no prefab effects). Returns the
/// stickers to send when it did.
fn add_sticker(stickers: &mut Vec<HitPoint>, shot: &Shot, hit: &VehicleHit, bounds: [Vec3; 2]) -> Option<Vec<HitPoint>> {
    let sticker = shot.def.pierced_sticker?;
    stickers.push(hit_point(shot, hit, bounds, HIT_EFFECT_ARMOR_PIERCED, sticker & 0x7F));
    Some(stickers.clone())
}
