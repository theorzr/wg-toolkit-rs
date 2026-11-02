//! The battles of the emulator: each in a space of its own, with the player's vehicle,
//! which the client drives, and an enemy vehicle in front of it to shoot at.

use std::collections::BTreeMap;
use std::time::Instant;
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::Vec3;

use wgtk::app::entity::{AnyEntity, Audience, Bytes, ComponentKey, Entity, EntityId, EntityKey, Pickle, ScriptType};
use wgtk::app::base::{self, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::app::client::element::{ChangedProperties, NrlMessage};
use wgtk::script::{StringValue, Value};
use wgtk::app::dispatch::MethodCall;
use wgtk::res::fs::ResFilesystem;

use crate::wot::schema::{
    self, Avatar, AvatarCell, AvatarInfo, AvatarVehiclesInfo, Fire, GunMarker, GunMarkerComponent, HitPoint, OwnVehicle, PerkEffects,
    PublicVehicleInfo, ShotResult, VehicleCell,
};
use crate::wot::collision::{VehicleCollision, VehicleHit, VehiclePart};
use crate::wot::item_defs::VehicleDef;
use crate::wot::items::VehicleDescr;

use super::{
    ammo_list, arena_vehicles, captured_property, gun_reload_time, invalid_data, targeting_info,
    vehicle_health_info, vehicle_settings, with_int,
    BaseThread, VehicleConfig, EXPLOSION_STEP, GROUND_EFFECT_MATERIAL, NO_PREFAB_EFFECTS, TICK,
};
use super::gun::{Gun, HitTarget, Landing, Shot};
use super::physics::{MotionLimits, Vehicle};
use super::bot::Bot;


/// The battle of a client, in a space of its own, see [`BaseThread::enter_space`].
#[derive(Debug)]
pub struct Battle {
    avatar: EntityKey<Avatar>,
    space: SpaceKey,
    /// Its vehicle's configuration, the captured vehicle's when `None`.
    config: Option<VehicleConfig>,
    /// When it started, at 0: the battle's clock.
    joined: Instant,
    /// Its vehicle's entity, and the vehicle the server simulates once the client has
    /// it, moving as the client drives it.
    vehicle: Option<EntityKey<schema::Vehicle>>,
    driven: Option<DrivenVehicle>,
    /// The enemy vehicle standing in front of it, to shoot at.
    target: Option<Target>,
}

impl Battle {

    /// The space it is in.
    pub fn space(&self) -> SpaceKey {
        self.space
    }

    /// The battle's game time, in seconds.
    fn game_time(&self) -> f64 {
        self.joined.elapsed().as_secs_f64()
    }

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
    /// Its driver and its collision models, once created, and whether the client has it.
    bot: Option<Bot>,
    collision: Option<Arc<VehicleCollision>>,
    entered: bool,
    /// Whether the player's vehicle spots it, see [`VIEW_RANGE`].
    spotted: bool,
    health: i64,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
    /// Its fire, with the game time it goes out at, see [`TARGET_FIRE_DURATION`].
    fire: Option<(ComponentKey<Fire>, f64)>,
}

/// A battle client's vehicle: the vehicle the server simulates, and how the client
/// knows it.
#[derive(Debug)]
struct DrivenVehicle {
    entity: EntityKey<schema::Vehicle>,
    vehicle: Vehicle,
    /// Its components the client has it with, when the capture has them.
    own_vehicle: Option<ComponentKey<OwnVehicle>>,
    gun_marker: Option<ComponentKey<GunMarkerComponent>>,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// The avatar's `ownVehicleGear` last sent, likewise.
    engine_gear: Option<u8>,
    collision: Arc<VehicleCollision>,
    gun: Gun,
    /// The shells in flight, its and the target's, with the game time they land at and
    /// who fired them.
    shots: Vec<(f64, EntityId, Shot)>,
    health: i64,
    max_health: i64,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
}

impl BaseThread {

    /// A client's avatar got into its battle's space: set the space up as the captured
    /// server did, with its properties and its map, then the player's vehicle, the
    /// target and the arena's info entity, which the client gets in that order. It sees
    /// the vehicles its own spots, see [`Self::step`].
    pub fn enter_space(&mut self, client: ClientKey, avatar: EntityKey<Avatar>, space: SpaceKey) -> io::Result<()> {

        self.app.set_interest(client, Interest::Manual)?;

        let config = self.battle_clients.get(&client).and_then(|c| c.config.clone());
        let mut battle = Battle { avatar, space, config, joined: Instant::now(), vehicle: None, driven: None, target: None };

        let session = self.session.clone();
        let Some(captured) = session.as_ref().and_then(|s| s.battle.as_ref()) else {
            self.set_battle(client, battle);
            return Ok(());
        };

        for data in &captured.space_properties {
            self.app.set_space_property(space, data.clone())?;
        }
        if let Some(geometry) = &captured.geometry {
            self.app.add_space_geometry_mapping(space, geometry.entry_id.addr, &geometry.path)?;
            info!(%client, "Entering space {space} with {}", geometry.path);
        }

        battle.vehicle = self.create_vehicle(&battle)?;

        let vehicle_id = captured.cell_player.as_ref().map(|cell| cell.vehicle_id);
        battle.target = captured.arena_info.as_ref()
            .zip(vehicle_id)
            .and_then(|(arena, vehicle_id)| enemy_of(&arena.nrl, vehicle_id));
        if let Some(target) = &mut battle.target {
            info!(%client, "Target {} ({:?})", target.entity_id, target.info.get("name"));
            self.create_target(space, target)?;
        }

        // The arena's info entity, created right away as entities without an alias are,
        // then given its vehicles: only the player's and the target.
        if let (Some(arena), Some(vehicle_id)) = (&captured.arena_info, vehicle_id) {
            let Some((_, properties)) = &arena.created else {
                warn!(%client, "The capture's arena info entity has no creation");
                self.set_battle(client, battle);
                return Ok(());
            };
            let mut properties = properties.clone();
            for (_, name, value) in &mut properties.values {
                if &**name == "avatarID" {
                    *value = with_int(value, avatar.id().0.into());
                }
            }
            let target_id = battle.target.as_ref().map(|target| target.entity_id);
            let vehicles = arena_vehicles(&arena.nrl, vehicle_id, target_id, battle.config.as_ref());
            let info = self.app.create_entity::<AvatarInfo>(space, EntityInit {
                id: Some(EntityId(arena.entity_id)),
                visibility: Visibility::Always,
                properties,
                ..EntityInit::default()
            })?;
            self.app.entity(info)?
                .add_component::<AvatarVehiclesInfo>(Audience::AllClients)?
                .set_vehiclesInfo(vehicles)?;
            info!(%client, "Created arena info {}", arena.entity_id);
        }

        self.set_battle(client, battle);
        Ok(())

    }

    fn set_battle(&mut self, client: ClientKey, battle: Battle) {
        if let Some(c) = self.battle_clients.get_mut(&client) {
            c.battle = Some(battle);
        }
    }

    /// Create the player's vehicle in its battle's space, as captured, with its own
    /// configuration: it belongs to the avatar.
    fn create_vehicle(&mut self, battle: &Battle) -> io::Result<Option<EntityKey<schema::Vehicle>>> {

        let Some(captured) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(None) };
        let (Some(entity), Some(properties), Some(position)) = (&captured.vehicle_entity, &captured.vehicle_properties, &captured.vehicle_position) else {
            warn!(avatar = %battle.avatar, "The capture has no vehicle to create");
            return Ok(None);
        };

        let mut public = captured_public_info(properties)?;
        if let Some(config) = &battle.config {
            public.compDescr = Bytes(config.descr.clone());
            public.maxHealth = config.max_health as u16;
        }

        info!(avatar = %battle.avatar, "Creating vehicle {}", entity.entity_id);
        let vehicle = self.app.create_entity::<schema::Vehicle>(battle.space, EntityInit {
            id: Some(EntityId(entity.entity_id)),
            position: position.position,
            direction: position.direction,
            alias: true,
            controlled_by: Some(battle.avatar.any()),
            visibility: Visibility::Interest,
            properties: properties.clone(),
        })?;

        let mut entity = self.app.entity(vehicle)?;
        init_vehicle(&mut entity, battle.avatar.id(), public)?;

        if let Some(config) = &battle.config {
            let empty = || Pickle(serde_pickle::Value::Dict(Default::default()));
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
        }

        Ok(Some(vehicle))

    }

    /// The client has its vehicle: from there on it moves as the client drives it. Then
    /// its `OwnVehicle` component: the client only turns its turret once it knows how fast it
    /// can. Its `GunMarkerComponent` too, for the client's server reticle.
    fn vehicle_entered(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let Some(captured) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let (Some(vehicle), Some(position)) = (battle.vehicle, &captured.vehicle_position) else { return Ok(()) };
        let config = battle.config.as_ref();

        let Some(config) = config else { return Ok(()) };
        let mut driven = DrivenVehicle {
            entity: vehicle,
            vehicle: Vehicle::new(position.position, position.direction, config.limits, config.aiming.clone()),
            own_vehicle: None,
            gun_marker: None,
            engine_mode: None,
            engine_gear: None,
            collision: config.collision.clone(),
            gun: Gun::new(config.shots.iter().cloned().zip(config.ammo.iter().map(|&(_, count)| count)).collect(), config.aiming.reload_time),
            shots: Vec::new(),
            health: config.max_health.into(),
            max_health: config.max_health.into(),
            damage_stickers: Vec::new(),
        };

        if let Some(captured) = &captured.own_vehicle {
            let entity_id = vehicle.id();
            let mut entity = self.app.entity(vehicle)?;
            let mut own = entity.add_component::<OwnVehicle>(Audience::OwnClient)?;
            if let Some(attrs) = captured_property(captured, "syncVehicleAttrs") {
                own.set_syncVehicleAttrs(attrs.clone())?;
            }
            if let Some(targeting) = targeting_info(captured, config) {
                own.set_targetingInfo(targeting)?;
            }
            // What the real server sends right after, as updates: the ammunition, the
            // shells loaded, the reload and the health.
            own.set_vehicleAmmoList(ammo_list(config.ammo.iter().copied()))?;
            own.set_vehicleSettings(vehicle_settings(entity_id, config))?;
            own.set_vehicleGunReloadTime(gun_reload_time(config.aiming.reload_time, 0.0, 0.0))?;
            own.set_vehicleHealthInfo(vehicle_health_info(entity_id, config.max_health.into()))?;
            driven.own_vehicle = Some(own.key());
            let mut marker = entity.add_component::<GunMarkerComponent>(Audience::OwnClient)?;
            marker.set_gunMarker(None)?;
            driven.gun_marker = Some(marker.key());
        }

        battle.driven = Some(driven);
        Ok(())

    }

    /// Create the target in its battle's space, from the player's captured vehicle: its
    /// own id, descriptor, name, team and health, no avatar, [`TARGET_DISTANCE`] in front
    /// of the player's start, facing it.
    fn create_target(&mut self, space: SpaceKey, target: &mut Target) -> io::Result<()> {

        let Some(captured) = self.session.as_ref().and_then(|s| s.battle.as_ref()) else { return Ok(()) };
        let (Some(properties), Some(start)) = (&captured.vehicle_properties, &captured.vehicle_position) else {
            warn!("The capture has no vehicle to make the target from");
            return Ok(());
        };

        // The player's vehicle's, with the target's own name, vehicle, team and health,
        // and neither the player's camouflage nor marks.
        let field = |name: &str| target.info.get(name).cloned();
        let mut public = captured_public_info(properties)?;
        public.name = field("name").and_then(String::from_value).unwrap_or(public.name);
        public.compDescr = field("compDescr").and_then(Bytes::from_value).unwrap_or(public.compDescr);
        public.team = field("team").as_ref().and_then(Value::as_i64).map_or(public.team, |team| team as u8);
        public.maxHealth = field("maxHealth").as_ref().and_then(Value::as_i64).map_or(public.maxHealth, |health| health as u16);
        public.outfit = Bytes::default();
        public.marksOnGun = 0;

        let yaw = start.direction.z;
        let mut position = start.position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * TARGET_DISTANCE;
        if let Some(height) = self.terrain.as_ref().and_then(|terrain| terrain.height_at(position.x, position.z)) {
            position.y = height;
        }
        // Facing the player's start, in [-pi, pi) as angles are packed.
        let facing = (yaw + 2.0 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let direction = Vec3::new(0.0, 0.0, facing);

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

        info!("Creating target {} ({}) at {position:?}", target.entity_id, def.name);
        let vehicle = self.app.create_entity::<schema::Vehicle>(space, EntityInit {
            id: Some(EntityId(target.entity_id)),
            position,
            direction,
            alias: true,
            controlled_by: None,
            visibility: Visibility::Interest,
            properties: properties.clone(),
        })?;
        init_vehicle(&mut self.app.entity(vehicle)?, EntityId(0), public)?;
        target.bot = Some(Bot::new(Vehicle::new(position, direction, limits, def.aiming.clone()), gun, position));
        target.collision = Some(def.collision.clone());
        target.entity = Some(vehicle);
        Ok(())

    }

    /// Fire the client's gun, if loaded: the shell's tracer and the gun's effects, then
    /// one shell fewer and the reload. Where the shell lands follows with
    /// [`Self::land_shot`].
    fn shoot(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
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

        if let Some(own) = driven.own_vehicle {
            let mut own = self.app.component(own)?;
            own.set_vehicleAmmoList(ammo_list(driven.gun.ammo.iter().map(|(def, count)| (def.compact_descr.to_int(), *count))))?;
            own.set_vehicleGunReloadTime(gun_reload_time(driven.gun.reload_time(), driven.gun.ready_at as f32, driven.gun.reload_time()))?;
        }
        let vehicle = driven.entity;

        show_shot(&mut self.app, battle.avatar, vehicle, &shot)

    }

    /// A shell gets to its end: it damages the vehicle it hits, explodes on the
    /// terrain, or its tracer stops at its maximum distance.
    fn land_shot(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot) -> io::Result<()> {
        let Some(battle) = self.battle_clients.get(&client).and_then(|c| c.battle.as_ref()) else { return Ok(()) };
        let avatar_key = battle.avatar;
        let def = &shot.def;
        self.app.entity(avatar_key)?.stopTracer(shot.id, shot.end)?;
        match shot.landing {
            Landing::Vehicle(entity_id, hit) => {
                info!(%client, "Shot {} hits {entity_id}'s {:?}, material {}, at {:?}", shot.id, hit.part, hit.material_kind, hit.local_point);
                let player_vehicle = battle.driven.as_ref().map(|driven| driven.entity.id());
                if Some(EntityId(entity_id)) == player_vehicle {
                    self.damage_player(client, shooter, shot, &hit)
                } else {
                    self.damage_target(client, entity_id, shot, &hit)
                }
            }
            // EXPERIMENT: the explosion comes for a shot the client does not know, so that
            // it shows it from these arguments alone.
            Landing::Terrain => self.app.entity(avatar_key)?.explodeProjectile(
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

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let Some(driven) = battle.driven.as_mut().filter(|driven| driven.health > 0) else { return Ok(()) };
        let amount = spread_damage(shot.def.damage);
        let old_health = driven.health;
        driven.health = (old_health - amount).max(0);
        let new_health = driven.health;
        let bounds = driven.collision.parts[hit.part as usize].bounds;
        info!(%client, "Player takes {amount} damage, {old_health} -> {new_health}{}", if new_health == 0 { ", destroyed" } else { "" });

        if let Some(own) = driven.own_vehicle {
            self.app.component(own)?.set_vehicleHealthInfo(vehicle_health_info(driven.entity.id(), new_health))?;
        }
        let stickers = add_sticker(&mut driven.damage_stickers, shot, hit, bounds);
        let damage = Damage { shooter, amount, max_health: driven.max_health, old_health, new_health };
        show_damage(&mut self.app.entity(driven.entity)?, shot, hit, bounds, &damage, stickers)

    }

    /// The target takes a shell's damage, varied by up to [`DAMAGE_SPREAD`] either way:
    /// its health drops, and the player sees the hit, as the captured server shows one.
    fn damage_target(&mut self, client: ClientKey, entity_id: u32, shot: &Shot, hit: &VehicleHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Some(driven) = battle.driven.as_ref() else { return Ok(()) };
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

        // A hit sets it on fire, which a kill puts out.
        match (target.fire.take(), killed) {
            (Some((fire, _)), true) => self.app.remove_component(fire)?,
            (Some((fire, _)), false) => target.fire = Some((fire, now + TARGET_FIRE_DURATION)),
            (None, true) => {}
            (None, false) => {
                let mut entity = self.app.entity(vehicle)?;
                let mut fire = entity.add_component::<Fire>(Audience::AllClients)?;
                fire.set_fireInfo(Value::None)?;
                target.fire = Some((fire.key(), now + TARGET_FIRE_DURATION));
                info!(%client, "Target {entity_id} on fire");
            }
        }

        let mut flags = HIT_FLAGS_PIERCED;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        self.app.entity(battle.avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.id(),
        }])

    }

    /// A cell method of its avatar the client called.
    fn avatar_cell_call(&mut self, client: ClientKey, call: AvatarCell) {
        if let AvatarCell::VehicleShoot {} = call {
            if let Err(e) = self.shoot(client) {
                warn!(%client, "Failed to shoot: {e}");
            }
            return;
        }
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(|battle| battle.driven.as_mut()) else {
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
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(|battle| battle.driven.as_mut()) else {
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

    /// Step a battle by a tick: its vehicles move, the player's on the client's controls
    /// and the target on its own, which aims at the player and may fire on them.
    fn step(&mut self, client: ClientKey) -> io::Result<()> {

        let Self { app, battle_clients, terrain, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Some(driven) = battle.driven.as_mut() else { return Ok(()) };
        let avatar_key = battle.avatar;

        let DrivenVehicle { entity, vehicle, own_vehicle, gun_marker, engine_mode, engine_gear, gun, .. } = driven;
        vehicle.step(TICK.as_secs_f32(), terrain.as_ref());
        let Vehicle { hull, turret } = vehicle;

        if let (Some(own), true) = (*own_vehicle, gun.take_loaded(now)) {
            app.component(own)?.set_vehicleGunReloadTime(gun_reload_time(gun.reload_time(), 0.0, 0.0))?;
        }
        if let Some(marker) = *gun_marker {
            let (position, velocity, dispersion) = turret.gun_marker(hull);
            app.component(marker)?.set_gunMarker(Some(GunMarker { gunPosition: position, shotVector: velocity, dispersion }))?;
        }

        // Every tick, to compare with what the client simulates.
        debug!(%client, "Vehicle at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
            hull.position, hull.direction, hull.speed, turret.yaw, turret.pitch);
        app.set_position(*entity, hull.position, hull.direction)?;
        app.set_position(avatar_key, hull.position, Vec3::ZERO)?;

        let mut vehicle_view = app.entity(*entity)?;
        vehicle_view.set_gunAnglesPacked(turret.gun_angles_packed())?;
        let mode = hull.engine_mode();
        if *engine_mode != Some(mode) {
            *engine_mode = Some(mode);
            vehicle_view.set_engineMode(mode)?;
        }

        // What the client aims from, so it must turn with the hull, and the engine's
        // RPM, then its gear when it shifts.
        let mut avatar_view = app.entity(avatar_key)?;
        avatar_view.set_ownVehicleAuxPhysicsData(hull.aux_physics_data())?;
        let (gear, _) = hull.engine();
        if *engine_gear != Some(gear) {
            *engine_gear = Some(gear);
            avatar_view.set_ownVehicleGear(gear)?;
        }

        // The player's vehicle spots the target while within its view range, and its client
        // only sees it then.
        if let Some(target) = battle.target.as_mut() {
            if let (Some(entity), Some(bot)) = (target.entity, &target.bot) {
                let offset = bot.vehicle.hull.position - driven.vehicle.hull.position;
                let spotted = offset.x.hypot(offset.z) <= VIEW_RANGE;
                if spotted != target.spotted {
                    target.spotted = spotted;
                    if spotted {
                        app.show(client, entity)?;
                    } else {
                        app.hide(client, entity)?;
                    }
                    info!(%client, "Target {} {}", target.entity_id, if spotted { "spotted" } else { "lost" });
                }
            }
        }

        // The target drives, aims at the player and fires once on them.
        let Some(target) = battle.target.as_mut().filter(|target| target.entered && target.health > 0) else { return Ok(()) };

        if let Some((fire, _)) = target.fire.take_if(|&mut (_, until)| until <= now) {
            app.remove_component(fire)?;
            info!(%client, "Target {} fire out", target.entity_id);
        }

        let (Some(bot), Some(target_entity)) = (target.bot.as_mut(), target.entity) else { return Ok(()) };
        let enemy = hit_target(driven.entity.id().0, &driven.vehicle, &driven.collision);
        let enemy_hull = enemy.transforms[VehiclePart::Hull as usize].translation.into();
        let on_target = bot.step(TICK.as_secs_f32(), terrain.as_ref(), enemy_hull, TARGET_DRIVES);
        let mut bot_shot = None;
        if TARGET_FIRES && on_target && driven.health > 0 {
            let (position, velocity, dispersion) = bot.vehicle.turret.gun_marker(&bot.vehicle.hull);
            if let Some(shot) = bot.gun.fire(now, position, velocity, dispersion, terrain.as_ref(), &[enemy]) {
                info!(%client, "Target shot {} lands at {:?} in {:.2} s", shot.id, shot.end, shot.flight_time);
                driven.shots.push((now + f64::from(shot.flight_time), target_entity.id(), shot.clone()));
                bot_shot = Some(shot);
            }
        }
        let Vehicle { hull, turret } = &bot.vehicle;
        app.set_position(target_entity, hull.position, hull.direction)?;
        let mut target_view = app.entity(target_entity)?;
        target_view.set_gunAnglesPacked(turret.gun_angles_packed())?;
        let mode = hull.engine_mode();
        if target.engine_mode != Some(mode) {
            target.engine_mode = Some(mode);
            target_view.set_engineMode(mode)?;
        }
        if let Some(shot) = bot_shot {
            show_shot(app, avatar_key, target_entity, &shot)?;
        }

        Ok(())

    }

    /// End a tick: step every battle, land the shells due, then give the clients what
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
            let Some(driven) = battle.driven.as_mut() else { continue };
            let (landed, flying): (Vec<_>, _) = std::mem::take(&mut driven.shots).into_iter().partition(|&(at, _, _)| at <= now);
            driven.shots = flying;
            for (_, shooter, shot) in landed {
                if let Err(e) = self.land_shot(client, shooter, &shot) {
                    warn!(%client, "Failed to land shot {}: {e}", shot.id);
                }
            }
        }

        self.app.end_tick();

    }

    /// An entity of its battle was created on the client.
    pub fn entity_entered(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>) {
        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return };
        debug!(%client, "Entity {entity} created on the client");
        if battle.vehicle.is_some_and(|vehicle| vehicle.any() == entity) {
            if let Err(e) = self.vehicle_entered(client) {
                warn!(%client, "Failed to drive the vehicle: {e}");
            }
        } else if let Some(target) = battle.target.as_mut().filter(|target| target.entity.is_some_and(|key| key.any() == entity)) {
            target.entered = true;
        }
    }

    /// The client called a cell method.
    pub fn cell_method(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>, call: &MethodCall) {
        if let Some((_, call)) = self.app.decode::<AvatarCell>(entity, call) {
            self.avatar_cell_call(client, call);
        } else if let Some((_, call)) = self.app.decode::<VehicleCell>(entity, call) {
            self.vehicle_cell_call(client, call);
        } else {
            debug!(%client, "Cell method call on entity {entity}: {call:?}");
        }
    }

}

/// The first vehicle of the captured arena not in the given vehicle's team, as a
/// target.
fn enemy_of(arena: &[NrlMessage], vehicle_id: u32) -> Option<Target> {
    let Value::Seq(entries) = arena.iter().find_map(|message| captured_property(message, "vehiclesInfo"))? else {
        return None;
    };
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
            entered: false,
            spotted: false,
            engine_mode: None,
            damage_stickers: Vec::new(),
            fire: None,
        }))
}

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

/// How far the player's vehicle spots others, in metres on the ground plane: the most
/// any vehicle can see, until vehicles have their own.
const VIEW_RANGE: f32 = 445.0;

/// The `quickShellChangerFactor` of a vehicle, as captured: how much faster it changes
/// shells, from its crew and equipment.
const QUICK_SHELL_CHANGER_FACTOR: f32 = 0.268;

/// How long the target burns after its last hit, in seconds.
const TARGET_FIRE_DURATION: f64 = 5.0;

/// Whether the target fires back at the player.
const TARGET_FIRES: bool = false;
/// Whether the target drives around, else it stays where it is created.
const TARGET_DRIVES: bool = false;

/// The `publicInfo` of the captured player's vehicle, which vehicles are made from.
fn captured_public_info(properties: &ChangedProperties) -> io::Result<PublicVehicleInfo> {
    properties.values.iter()
        .find(|(_, name, _)| &**name == "publicInfo")
        .and_then(|(_, _, value)| PublicVehicleInfo::from_value(value.clone()))
        .ok_or_else(|| invalid_data("the captured vehicle has no public info"))
}

/// Set what every client gets of a fresh vehicle, before its creation on any: its
/// public info, its full health, and its avatar if driven by a client's.
fn init_vehicle(entity: &mut Entity<'_, schema::Vehicle>, avatar: EntityId, public: PublicVehicleInfo) -> io::Result<()> {
    entity.set_avatarID(avatar)?;
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
