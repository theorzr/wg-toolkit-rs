//! The battles of the server: each in a space of its own, with the player's vehicle,
//! which the client drives, and an enemy vehicle in front of it to shoot at.

use std::time::Instant;
use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::Vec3;

use wgtk::app::entity::{
    AnyEntity, Audience, Bytes, ComponentKey, Entity, EntityId, EntityKey, PathStep, Pickle,
};
use wgtk::app::base::{self, CellPlayerInit, ClientKey, EntityInit, Interest, SpaceKey, Visibility};
use wgtk::app::dispatch::MethodCall;
use wgtk::space::terrain::Terrain;
use wgtk::res::fs::ResFilesystem;

use crate::wot::schema::{
    self, Avatar, AvatarCell, AvatarComponentsData, AvatarInfo, AvatarVehiclesInfo, BattleDogTag, DeathInfo, DogTag,
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
use super::gun::{Gun, HitTarget, Landing, Shot};
use super::physics::{MotionLimits, Vehicle};
use super::bot::Bot;


/// The battle of a client, in a space of its own, see [`BaseThread::enter_space`].
#[derive(Debug)]
pub struct Battle {
    avatar: EntityKey<Avatar>,
    space: SpaceKey,
    /// Its vehicle's configuration.
    config: VehicleConfig,
    /// When it started, at 0: the battle's clock.
    joined: Instant,
    /// Its vehicle's entity and where it starts, and the vehicle the server simulates
    /// once the client has it, moving as the client drives it.
    vehicle: (EntityKey<schema::Vehicle>, Start),
    driven: Option<DrivenVehicle>,
    /// The enemy vehicle standing in front of it, to shoot at.
    target: Option<Target>,
    /// The arena's vehicles, on its info entity.
    vehicles_info: Option<ComponentKey<AvatarVehiclesInfo>>,
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

    /// The battle's game time, in seconds.
    fn game_time(&self) -> f64 {
        self.joined.elapsed().as_secs_f64()
    }

}

/// An enemy vehicle the server places in front of the player and drives, with its entry
/// in `vehiclesInfo`.
#[derive(Debug, Clone)]
struct Target {
    /// Its entity and the entity's id.
    entity_id: u32,
    entity: Option<EntityKey<schema::Vehicle>>,
    /// Its `vehiclesInfo` entry, at `info_index`.
    info: VehicleInfo,
    info_index: u32,
    /// Its driver and its collision models, and whether the client has it.
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

    /// Get a client's avatar into its battle's space, with its map: create the vehicle of
    /// the player of the given name, the avatar's cell side on it, then the target and
    /// the arena's info entity, which the client gets in that order. It sees the vehicles
    /// its own spots, see [`Self::step`].
    pub fn enter_space(&mut self, client: ClientKey, avatar: EntityKey<Avatar>, space: SpaceKey, name: String) -> io::Result<()> {

        let Some(config) = self.battle_clients.get(&client).map(|c| c.config.clone()) else { return Ok(()) };
        let (vehicle, public, start) = self.create_vehicle(avatar, space, &config, name)?;
        let mut battle = Battle { avatar, space, config, joined: Instant::now(), vehicle: (vehicle, start), driven: None, target: None, vehicles_info: None };

        self.app.create_cell_player(avatar, space, CellPlayerInit {
            vehicle_id: vehicle.id().0,
            position: start.position,
            direction: start.direction,
            packed_xz_scale: PACKED_XZ_SCALE,
            data: avatar_cell_data(PLAYER_TEAM, vehicle.id()),
            components_data: AvatarComponentsData::default(),
        })?;
        self.app.set_interest(client, Interest::Manual)?;

        // The player's vehicle, then the target, a copy of it in the other team.
        let mut vehicles = vec![vehicle_info(vehicle.id(), &public, PLAYER_ACCOUNT_DBID)];
        let target = self.create_target(space, start, &public.compDescr.0, vehicles.len() as u32)?;
        info!(%client, entity_id = target.entity_id, "Added the target to the arena: {}", target.info.name);
        vehicles.push(target.info.clone());
        battle.target = Some(target);

        // The arena's info entity, created right away as entities without an alias are,
        // with its vehicles.
        let info = self.app.create_entity::<AvatarInfo>(space, EntityInit {
            visibility: Visibility::Always,
            ..EntityInit::default()
        })?;
        let mut entity = self.app.entity(info)?;
        entity.set_avatarID(avatar.id())?;
        let mut vehicles_info = entity.add_component::<AvatarVehiclesInfo>(Audience::AllClients)?;
        vehicles_info.set_vehiclesInfo(vehicles)?;
        battle.vehicles_info = Some(vehicles_info.key());
        info!(%client, entity_id = %info.id(), "Created the arena info");

        self.set_battle(client, battle);
        Ok(())

    }

    fn set_battle(&mut self, client: ClientKey, battle: Battle) {
        if let Some(c) = self.battle_clients.get_mut(&client) {
            c.battle = Some(battle);
        }
    }

    /// Create the player's vehicle in its battle's space, at its team's start, of the
    /// given configuration and named as the player: it belongs to the avatar. Returns it
    /// with its public info and its start.
    fn create_vehicle(
        &mut self,
        avatar: EntityKey<Avatar>,
        space: SpaceKey,
        config: &VehicleConfig,
        name: String,
    ) -> io::Result<(EntityKey<schema::Vehicle>, PublicVehicleInfo, Start)> {

        let public = public_info(name, &config.descr, PLAYER_TEAM, config.max_health as u16, config.crew.len());

        let start = team_start(&self.map, &self.terrain, PLAYER_TEAM);
        let vehicle = self.app.create_entity::<schema::Vehicle>(space, EntityInit {
            position: start.position,
            direction: start.direction,
            alias: true,
            controlled_by: Some(avatar.any()),
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(%avatar, entity_id = %vehicle.id(), "Created the vehicle, at {:?}", start.position);

        // Its turret at rest, as it is once driven.
        let gun_angles = Vehicle::new(start.position, start.direction, config.limits, config.aiming.clone())
            .turret.gun_angles_packed();
        let mut entity = self.app.entity(vehicle)?;
        init_vehicle(&mut entity, avatar.id(), public.clone(), gun_angles)?;

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

        Ok((vehicle, public, start))

    }

    /// The client has its vehicle: from there on it moves as the client drives it. Then
    /// its `OwnVehicle` component: the client only turns its turret once it knows how fast it
    /// can. Its `GunMarkerComponent` too, for the client's server reticle.
    fn vehicle_entered(&mut self, client: ClientKey) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let (vehicle, start) = battle.vehicle;
        let config = &battle.config;
        let mut driven = DrivenVehicle {
            entity: vehicle,
            vehicle: Vehicle::new(start.position, start.direction, config.limits, config.aiming.clone()),
            own_vehicle: None,
            gun_marker: None,
            engine_mode: None,
            engine_gear: None,
            collision: config.collision.clone(),
            gun: Gun::new(config.shots.iter().cloned().zip(config.ammo.iter().map(|&(_, count)| count)).collect(), config.aiming.reload_time)
                .with_magazine(config.magazine)
                .with_quick_change_factor(QUICK_SHELL_CHANGER_FACTOR),
            shots: Vec::new(),
            health: config.max_health.into(),
            max_health: config.max_health.into(),
            damage_stickers: Vec::new(),
        };

        let entity_id = vehicle.id();
        let mut entity = self.app.entity(vehicle)?;
        let mut own = entity.add_component::<OwnVehicle>(Audience::OwnClient)?;
        // No crew or equipment bonus: the view range all vehicles have here.
        own.set_syncVehicleAttrs(VehicleSyncAttrs { circularVisionRadius: VIEW_RANGE as u16, gunPiercing: 1.0, gunShotsSpeed: 1.0 })?;
        own.set_targetingInfo(Some(targeting_info(config)))?;
        // What the real server sends right after, as updates: the ammunition, the
        // shells loaded, the reload and the health.
        own.set_vehicleAmmoList(gun_ammo_list(&driven.gun))?;
        own.set_vehicleSettings(vehicle_settings(entity_id, &driven.gun))?;
        own.set_vehicleGunReloadTime(gun_reload_time(config.aiming.reload_time, 0.0, 0.0))?;
        own.set_vehicleHealthInfo(vehicle_health_info(entity_id, config.max_health.into()))?;
        driven.own_vehicle = Some(own.key());
        let mut marker = entity.add_component::<GunMarkerComponent>(Audience::OwnClient)?;
        marker.set_gunMarker(None)?;
        driven.gun_marker = Some(marker.key());

        battle.driven = Some(driven);
        Ok(())

    }

    /// Create the target in its battle's space, of the given descriptor, in the other
    /// team, no avatar, [`TARGET_DISTANCE`] in front of the player's start, facing it,
    /// with its `vehiclesInfo` entry at the given index.
    fn create_target(&mut self, space: SpaceKey, start: Start, compact_descr: &[u8], info_index: u32) -> io::Result<Target> {

        let yaw = start.direction.z;
        let mut position = start.position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * TARGET_DISTANCE;
        if let Some(height) = self.terrain.height_at(position.x, position.z) {
            position.y = height;
        }
        // Facing the player's start, in [-pi, pi) as angles are packed.
        let facing = (yaw + 2.0 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let direction = Vec3::new(0.0, 0.0, facing);

        // Driven as its own vehicle, by its definition.
        let decoded = VehicleDescr::decode(compact_descr).map_err(|e| invalid_data(&e.to_string()))?;
        let def = VehicleDef::load_by_id(&ResFilesystem::new(self.game_dir.join("res"))?, decoded.nation, decoded.type_id)?;
        let limits = MotionLimits { speed_forward: def.speed_forward, speed_backward: def.speed_backward, rotation_speed: def.rotation_speed };
        let ammo = def.shots.iter().cloned().zip(def.default_ammo().into_iter().map(|(_, count)| count)).collect();
        let gun = Gun::new(ammo, def.aiming.reload_time).with_magazine(def.magazine).with_shot_ids_from(TARGET_FIRST_SHOT_ID);
        let driven = Vehicle::new(position, direction, limits, def.aiming.clone());
        let gun_angles = driven.turret.gun_angles_packed();

        let public = public_info(TARGET_NAME.to_string(), compact_descr, TARGET_TEAM, def.max_health as u16, def.crew_roles.len());

        let vehicle = self.app.create_entity::<schema::Vehicle>(space, EntityInit {
            position,
            direction,
            alias: true,
            controlled_by: None,
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(entity_id = %vehicle.id(), "Created the target: {}, at {position:?}", def.name);
        let info = vehicle_info(vehicle.id(), &public, 0);
        init_vehicle(&mut self.app.entity(vehicle)?, EntityId(0), public, gun_angles)?;
        Ok(Target {
            entity_id: vehicle.id().0,
            entity: Some(vehicle),
            health: def.max_health.into(),
            info,
            info_index,
            bot: Some(Bot::new(driven, gun, position)),
            collision: Some(def.collision.clone()),
            entered: false,
            spotted: false,
            engine_mode: None,
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
        let vehicles = hit_targets(&battle.target);
        let Some(driven) = battle.driven.as_mut() else { return Ok(()) };
        let (position, velocity, dispersion) = driven.vehicle.turret.gun_marker(&driven.vehicle.hull);
        let Some(shot) = driven.gun.fire(now, position, velocity, dispersion, Some(&self.terrain), &vehicles) else {
            debug!(%client, "Refused a shot: not loaded, or out of shells");
            return Ok(());
        };
        info!(%client, shot = shot.id, burst = shot.burst, "Fired a shell from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
        driven.shots.push((now + f64::from(shot.flight_time), driven.entity.id(), shot.clone()));

        if let Some(own) = driven.own_vehicle {
            show_gun(&mut self.app, own, driven.entity.id(), &driven.gun, now)?;
        }
        let vehicle = driven.entity;

        show_shot(&mut self.app, battle.avatar, vehicle, &shot)

    }

    /// A shell gets to its end: what it does to the vehicle it hits, by its armour, see
    /// [`armor::hit`], or it explodes on the ground, where an HE shell damages the
    /// vehicles in its explosion's reach, see [`armor::explosion`].
    fn land_shot(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get(&client).and_then(|c| c.battle.as_ref()) else { return Ok(()) };
        let avatar_key = battle.avatar;
        let def = &shot.def;
        let player_vehicle = battle.driven.as_ref().map(|driven| driven.entity.id());
        self.app.entity(avatar_key)?.stopTracer(shot.id, shot.end)?;

        match shot.landing {
            Landing::Vehicle { entity_id, hit, transforms } => {
                let player = Some(EntityId(entity_id)) == player_vehicle;
                let collision = if player {
                    battle.driven.as_ref().map(|driven| driven.collision.clone())
                } else {
                    battle.target.as_ref().filter(|target| target.entity_id == entity_id).and_then(|target| target.collision.clone())
                };
                let Some(collision) = collision else { return Ok(()) };
                let flown = shot.start.distance(shot.end);
                let Some(impact) = armor::hit(def, flown, shot.end, shot.end_velocity, &collision, &transforms, &mut rand::thread_rng()) else {
                    debug!(%client, shot = shot.id, entity_id, "Hit a vehicle where it has no armour: {:?}", hit.part);
                    return Ok(());
                };
                info!(%client, shot = shot.id, entity_id, "Hit a vehicle: {:?}, {:?} {} mm at {:?}, {:.0} damage{}",
                    impact.outcome, impact.hit.part, impact.hit.material.map_or(0.0, |m| m.armor), impact.hit.local_point,
                    impact.damage, if impact.exploded { ", exploded" } else { "" });
                let shell_hit = ShellHit::of_impact(&impact);
                if player {
                    self.damage_player(client, shooter, shot, &shell_hit)
                } else {
                    self.damage_target(client, entity_id, shot, &shell_hit)
                }
            }
            Landing::Terrain => {
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
                    Vec::new(),
                )?;
                if def.kind() == ShellKind::HighExplosive {
                    self.splash(client, shooter, shot)?;
                }
                Ok(())
            }
            Landing::Range => Ok(()),
        }

    }

    /// An HE shell explodes on the ground: the vehicles in its reach take its damage.
    fn splash(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get(&client).and_then(|c| c.battle.as_ref()) else { return Ok(()) };
        let player = battle.driven.as_ref()
            .filter(|driven| driven.health > 0)
            .and_then(|driven| armor::explosion(&shot.def, shot.end, &driven.collision, &driven.vehicle.part_transforms(driven.collision.hull_position)));
        let target = hit_targets(&battle.target).into_iter()
            .filter_map(|target| Some((target.entity_id, armor::explosion(&shot.def, shot.end, &target.collision, &target.transforms)?)))
            .next();

        if let Some(explosion) = player {
            info!(%client, shot = shot.id, "Exploded beside the player: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            self.damage_player(client, shooter, shot, &ShellHit::of_explosion(&explosion))?;
        }
        if let Some((entity_id, explosion)) = target {
            info!(%client, shot = shot.id, entity_id, "Exploded beside the target: {:.1} m away, {:.0} damage", explosion.distance, explosion.damage);
            self.damage_target(client, entity_id, shot, &ShellHit::of_explosion(&explosion))?;
        }
        Ok(())

    }

    /// The player's vehicle takes a shell's damage, as the target does: its health
    /// drops, in its `OwnVehicle` too.
    fn damage_player(&mut self, client: ClientKey, shooter: EntityId, shot: &Shot, shell_hit: &ShellHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let Some(driven) = battle.driven.as_mut().filter(|driven| driven.health > 0) else { return Ok(()) };
        let amount = spread_damage(shell_hit.damage);
        let old_health = driven.health;
        driven.health = (old_health - amount).max(0);
        let new_health = driven.health;
        let bounds = driven.collision.parts[shell_hit.hit.part as usize].bounds;
        if amount > 0 {
            info!(%client, "Damaged the player: {amount}, {old_health} -> {new_health}{}", if new_health == 0 { ", destroyed" } else { "" });
            if let Some(own) = driven.own_vehicle {
                self.app.component(own)?.set_vehicleHealthInfo(vehicle_health_info(driven.entity.id(), new_health))?;
            }
        }
        let stickers = shell_hit.pierced().then(|| add_sticker(&mut driven.damage_stickers, shot, &shell_hit.hit, bounds)).flatten();
        let damage = Damage { shooter, amount, max_health: driven.max_health, old_health, new_health };
        show_damage(&mut self.app.entity(driven.entity)?, shot, shell_hit, bounds, &damage, stickers)

    }

    /// The target takes a shell's damage, varied by up to [`DAMAGE_SPREAD`] either way:
    /// its health drops, and the player sees the hit, as the captured server shows one.
    fn damage_target(&mut self, client: ClientKey, entity_id: u32, shot: &Shot, shell_hit: &ShellHit) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Some(driven) = battle.driven.as_ref() else { return Ok(()) };
        let shooter = driven.entity.id();
        let Some(target) = battle.target.as_mut().filter(|target| target.entity_id == entity_id && target.health > 0) else {
            return Ok(());
        };
        let Some(vehicle) = target.entity else { return Ok(()) };
        let amount = spread_damage(shell_hit.damage);
        let old_health = target.health;
        target.health = (old_health - amount).max(0);
        let new_health = target.health;
        let killed = new_health == 0;
        let bounds = target.collision.as_ref().map_or([Vec3::ZERO; 2], |collision| collision.parts[shell_hit.hit.part as usize].bounds);
        let max_health = target.info.maxHealth.into();
        if amount > 0 {
            info!(%client, entity_id, "Damaged the target: {amount}, {old_health} -> {new_health}{}", if killed { ", destroyed" } else { "" });
        }

        let stickers = shell_hit.pierced().then(|| add_sticker(&mut target.damage_stickers, shot, &shell_hit.hit, bounds)).flatten();
        let damage = Damage { shooter, amount, max_health, old_health, new_health };
        show_damage(&mut self.app.entity(vehicle)?, shot, shell_hit, bounds, &damage, stickers)?;

        // A damaging hit sets it on fire, which a kill puts out.
        match (target.fire.take(), killed) {
            (Some((fire, _)), true) => self.app.remove_component(fire)?,
            (Some((fire, _)), false) => target.fire = Some((fire, now + TARGET_FIRE_DURATION)),
            (None, true) => {}
            (None, false) if amount == 0 => {}
            (None, false) => {
                let mut entity = self.app.entity(vehicle)?;
                let mut fire = entity.add_component::<Fire>(Audience::AllClients)?;
                fire.set_fireInfo(None)?;
                target.fire = Some((fire.key(), now + TARGET_FIRE_DURATION));
                info!(%client, entity_id, "Set the target on fire");
            }
        }

        // Every client lists it destroyed, by whom: its entry's changes apply once its
        // generation changes too.
        if let (true, Some(vehicles_info)) = (killed, battle.vehicles_info) {
            let death = DeathInfo { victimID: vehicle.id(), killerID: shooter, reasonID: ATTACK_REASON_SHOT, equipmentID: 0, numVehiclesAffected: 0 };
            target.info.deathInfo = Some(death.clone());
            target.info.isAlive = false;
            target.info.__generation += 1;
            let at = |field: &str| vec![PathStep::Index(target.info_index), PathStep::Field(field.into())];
            let mut vehicles_info = self.app.component(vehicles_info)?;
            vehicles_info.set_vehiclesInfo_at(at("deathInfo"), Some(death))?;
            vehicles_info.set_vehiclesInfo_at(at("isAlive"), false)?;
            vehicles_info.set_vehiclesInfo_at(at("__generation"), target.info.__generation)?;
        }

        let mut flags = shell_hit.flags;
        if killed {
            flags |= HIT_FLAG_VEHICLE_KILLED;
        }
        self.app.entity(battle.avatar)?.showShotResults(vec![ShotResult {
            gunInstallationIndex: 0,
            hitFlags: flags,
            vehicleID: vehicle.id(),
        }])

    }

    /// The client changes one of its vehicle's settings, by `VEHICLE_SETTING` code: the
    /// shell its gun loads next, or the one it is loaded with, which the gun then
    /// reloads, or it reloads its magazine. The client predicts the change, so it is
    /// given the gun back even when it doesn't change, to undo what it predicted then.
    fn change_setting(&mut self, client: ClientKey, code: u8, value: i32) -> io::Result<()> {

        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let Some(driven) = battle.driven.as_mut() else {
            warn!(%client, "Ignored a setting without a vehicle: {code} = {value}");
            return Ok(());
        };
        let gun = &mut driven.gun;

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
            SETTING_AUTOROTATION | SETTING_RESET_FULL_CLIP => return Ok(()),
            _ => {
                debug!(%client, "Ignored a setting: {code} = {value}");
                return Ok(());
            }
        };

        let Some(own) = driven.own_vehicle else { return Ok(()) };
        if reloads {
            show_gun(&mut self.app, own, driven.entity.id(), &driven.gun, now)
        } else {
            self.app.component(own)?.set_vehicleSettings(vehicle_settings(driven.entity.id(), &driven.gun))
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

        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(|battle| battle.driven.as_mut()) else {
            warn!(%client, "Ignored a cell method call without a vehicle: {call:?}");
            return;
        };

        match call {
            AvatarCell::VehicleShoot {} | 
            AvatarCell::VehicleChangeSetting { .. } => unreachable!(),
            AvatarCell::VehicleMoveWith { flags } => {
                info!(%client, "Changed the movement flags: {flags:#04x}");
                driven.vehicle.hull.flags = flags;
            }
            AvatarCell::VehicleStopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }

    }

    /// A cell method of its vehicle the client called.
    fn vehicle_cell_call(&mut self, client: ClientKey, call: VehicleCell) {
        
        let Some(driven) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()).and_then(|battle| battle.driven.as_mut()) else {
            warn!(%client, "Ignored a cell method call without a vehicle: {call:?}");
            return;
        };

        match call {
            VehicleCell::TrackRelativePointWithGun { point } => {
                let Vehicle { hull, turret } = &mut driven.vehicle;
                debug!(%client, "Tracked a point with the gun: {point:?}, from {:?}", hull.position);
                turret.track(hull.position + point);
            }
            VehicleCell::StopTrackingWithGun { yaw, pitch } => hold_gun(client, driven, yaw, pitch),
        }
        
    }

    /// Step a battle by a tick: its vehicles move, the player's on the client's controls
    /// and fires the rest of its bursts, and the target on its own, which aims at the
    /// player and may fire on them.
    fn step(&mut self, client: ClientKey) -> io::Result<()> {

        let Self { app, battle_clients, terrain, .. } = self;
        let Some(battle) = battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return Ok(()) };
        let now = battle.game_time();
        let targets = hit_targets(&battle.target);
        let Some(driven) = battle.driven.as_mut() else { return Ok(()) };
        let avatar_key = battle.avatar;

        let DrivenVehicle { entity, vehicle, own_vehicle, gun_marker, engine_mode, engine_gear, gun, shots, .. } = driven;
        vehicle.step(TICK.as_secs_f32(), Some(&*terrain));
        let Vehicle { hull, turret } = vehicle;

        // Its magazine full again, and loaded: the shells first, as the client expects.
        if let (Some(own), true) = (*own_vehicle, gun.take_loaded(now)) {
            let mut own = app.component(own)?;
            if gun.has_magazine() {
                own.set_vehicleAmmoList(gun_ammo_list(gun))?;
            }
            own.set_vehicleGunReloadTime(reload_time(gun, now))?;
        }
        let (position, velocity, dispersion) = turret.gun_marker(hull);
        if let Some(shot) = gun.fire_burst(now, position, velocity, dispersion, Some(&*terrain), &targets) {
            info!(%client, shot = shot.id, "Fired a shell of the burst from {:?}, landing at {:?} in {:.2} s", shot.start, shot.end, shot.flight_time);
            shots.push((now + f64::from(shot.flight_time), entity.id(), shot.clone()));
            // Its reload started with the burst, see `Gun::fire`.
            if let Some(own) = *own_vehicle {
                let mut own = app.component(own)?;
                own.set_vehicleAmmoList(gun_ammo_list(gun))?;
                own.set_vehicleSettings(vehicle_settings(entity.id(), gun))?;
            }
            show_shot(app, avatar_key, *entity, &shot)?;
        }
        if let Some(marker) = *gun_marker {
            app.component(marker)?.set_gunMarker(Some(GunMarker { gunPosition: position, shotVector: velocity, dispersion }))?;
        }

        // Every tick, to compare with what the client simulates.
        debug!(%client, "Stepped the vehicle: at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
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
                    info!(%client, entity_id = target.entity_id, "{} the target", if spotted { "Spotted" } else { "Lost" });
                }
            }
        }

        // The target drives, aims at the player and fires once on them.
        let Some(target) = battle.target.as_mut().filter(|target| target.entered && target.health > 0) else { return Ok(()) };

        if let Some((fire, _)) = target.fire.take_if(|&mut (_, until)| until <= now) {
            app.remove_component(fire)?;
            info!(%client, entity_id = target.entity_id, "Put out the target's fire");
        }

        let (Some(bot), Some(target_entity)) = (target.bot.as_mut(), target.entity) else { return Ok(()) };
        let enemy = hit_target(driven.entity.id().0, &driven.vehicle, &driven.collision);
        let enemy_hull = enemy.transforms[VehiclePart::Hull as usize].translation.into();
        let on_target = bot.step(TICK.as_secs_f32(), Some(&*terrain), enemy_hull, TARGET_DRIVES);
        let mut bot_shot = None;
        if TARGET_FIRES && driven.health > 0 {
            let (position, velocity, dispersion) = bot.vehicle.turret.gun_marker(&bot.vehicle.hull);
            let targets = [enemy];
            let shot = bot.gun.fire_burst(now, position, velocity, dispersion, Some(&*terrain), &targets)
                .or_else(|| on_target.then(|| bot.gun.fire(now, position, velocity, dispersion, Some(&*terrain), &targets)).flatten());
            if let Some(shot) = shot {
                info!(%client, shot = shot.id, "Fired a shell from the target, landing at {:?} in {:.2} s", shot.end, shot.flight_time);
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
                    warn!(%client, shot = shot.id, "Failed to land a shot: {e}");
                }
            }
        }

        self.app.end_tick();

    }

    /// An entity of its battle was created on the client.
    pub fn entity_entered(&mut self, client: ClientKey, entity: EntityKey<AnyEntity>) {
        let Some(battle) = self.battle_clients.get_mut(&client).and_then(|c| c.battle.as_mut()) else { return };
        debug!(%client, %entity, "Created an entity on the client");
        if battle.vehicle.0.any() == entity {
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
            debug!(%client, %entity, "Ignored a cell method call: {call:?}");
        }
    }

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

/// The target's first shot id, apart from the player's.
const TARGET_FIRST_SHOT_ID: i32 = 1_000_000;

/// The player's team, the target being in the other.
const PLAYER_TEAM: u8 = 1;
const TARGET_TEAM: u8 = 2;

/// The target's name, as players see it.
const TARGET_NAME: &str = "Target";

/// The scale of the packed horizontal offsets in position updates, as captured: we only
/// send detailed ones.
const PACKED_XZ_SCALE: f32 = 0.007644442;

/// How far behind its base a team starts, in metres, on maps without spawn points: about
/// as captured.
const SPAWN_BEHIND_BASE: f32 = 40.0;
/// How far inside the map's bounds a team starts at least, in metres.
const SPAWN_MARGIN: f32 = 20.0;

/// How far in front of the player's start the target stands, in metres.
const TARGET_DISTANCE: f32 = 40.0;

/// How far the player's vehicle spots others, in metres on the ground plane: the most
/// any vehicle can see, until vehicles have their own.
const VIEW_RANGE: f32 = 445.0;

/// The `quickShellChangerFactor` of a vehicle, as captured: how much faster it changes
/// shells, from its crew and equipment.
const QUICK_SHELL_CHANGER_FACTOR: f32 = 0.268;

/// The default dog tag's components, as captured: their ids and grades.
const DEFAULT_DOG_TAG: [(i32, i8); 2] = [(300, 9), (66, 0)];

/// How long the target burns after its last hit, in seconds.
const TARGET_FIRE_DURATION: f64 = 5.0;

/// Whether the target fires back at the player.
const TARGET_FIRES: bool = false;
/// Whether the target drives around, else it stays where it is created.
const TARGET_DRIVES: bool = false;

/// Where a team's vehicle starts on a map: at its spawn point, else [`SPAWN_BEHIND_BASE`]
/// behind its base, facing the enemy's spawn, inside the map and on the ground.
fn team_start(map: &ArenaTypeDef, terrain: &Terrain, team: u8) -> Start {
    let at = |spawn: Spawn| match spawn { Spawn::Point(at) | Spawn::Base(at) => at };
    let (own, enemy) = (map.spawns[team as usize - 1], map.spawns[2 - team as usize]);
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
    // Its whole burst from the only gun, on its first shell.
    match shot.burst {
        Some(burst) => app.entity(shooter)?.showShooting(burst.min(u8::MAX.into()) as u8, 0, def.kind_index),
        None => Ok(()),
    }
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
fn hold_gun(client: ClientKey, driven: &mut DrivenVehicle, yaw: f32, pitch: f32) {
    let turret = &mut driven.vehicle.turret;
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

/// The vehicles a battle's shells can hit, besides the player's: its target, alive.
fn hit_targets(target: &Option<Target>) -> Vec<HitTarget> {
    target.iter()
        .filter(|target| target.health > 0)
        .filter_map(|target| Some(hit_target(target.entity_id, &target.bot.as_ref()?.vehicle, target.collision.as_ref()?)))
        .collect()
}

/// A vehicle as shells hit it: its collision models where its parts are.
fn hit_target(entity_id: u32, vehicle: &Vehicle, collision: &Arc<VehicleCollision>) -> HitTarget {
    HitTarget { entity_id, collision: collision.clone(), transforms: vehicle.part_transforms(collision.hull_position) }
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
