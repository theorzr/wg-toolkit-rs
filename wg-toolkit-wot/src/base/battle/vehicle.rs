//! A vehicle of a battle, whoever drives it: created as a player's or a bot's, shown to
//! the clients as it moves, firing and reloading, its settings changed by the client
//! driving it, and taking damage.

use std::sync::Arc;
use std::io;

use tracing::{debug, info, warn};
use glam::Vec3;

use wgtk::app::entity::{Audience, Bytes, ComponentKey, Entity, EntityId, EntityKey, PathStep, Pickle};
use wgtk::app::base::{self, ClientKey, EntityInit, SpaceKey, Visibility};
use wgtk::pickle::Value as PickleValue;

use crate::game::schema::{
    self, Avatar, AvatarVehiclesInfo, BattleDogTag, DeathInfo, DogTag, DogTagComponent, Fire, GunMarker,
    GunMarkerComponent, HitPoint, OwnVehicle, PerkEffects, PublicVehicleInfo, VehicleInfo, VehicleSyncAttrs,
};
use crate::base::{
    ammo_list, gun_reload_time, targeting_info, vehicle_health_info, vehicle_settings, VehicleLoadout,
    NO_PREFAB_EFFECTS, SETTING_AUTOROTATION, SETTING_CURRENT_SHELLS, SETTING_NEXT_SHELLS, SETTING_RELOAD_PARTIAL_CLIP,
    SETTING_RESET_FULL_CLIP,
};
use crate::base::physics::{Dispersion, Vehicle, VehicleId, World};
use crate::base::ballistics::gun::{GunLoader, Shot, ShotTarget};
use crate::game::collision::{VehicleCollision, VehiclePart};
use crate::base::garage::player::Player;
use crate::game::item_defs::VehicleDef;

use super::spawn::TeamStart;
use super::hit::ShellHit;
use super::VIEW_RANGE;
use super::bot::Bot;


/// A vehicle of a battle, whoever drives it, with its entry in `vehiclesInfo`.
#[derive(Debug)]
pub struct BattleVehicle {
    pub entity: EntityKey<schema::Vehicle>,
    pub team: u8,
    /// Its `vehiclesInfo` entry, at `info_index`.
    pub info: VehicleInfo,
    pub info_index: u32,
    /// Who drives it.
    pub driver: Driver,
    /// The vehicle the server simulates, in the battle's world, its gun and its collision
    /// models.
    pub id: VehicleId,
    pub gun: GunLoader,
    pub collision: Arc<VehicleCollision>,
    pub health: i64,
    pub max_health: i64,
    /// The `engineMode` last sent, sent again only once it changes.
    engine_mode: Option<[u8; 2]>,
    /// The `isStrafing` last sent, see [`STRAFING_SLIDE`].
    strafing: bool,
    /// Its `damageStickers`: where shells pierced it.
    damage_stickers: Vec<HitPoint>,
    /// Its fire, with the game time it goes out at, see [`FIRE_DURATION`].
    fire: Option<(ComponentKey<Fire>, f64)>,
}

/// Who drives a vehicle: its controls and its aim.
#[derive(Debug)]
pub enum Driver {
    /// A client of the battle, through its avatar's and the vehicle's cell methods.
    Client(ClientDriver),
    /// The server, see [`Bot`].
    Bot(Bot),
    /// Nobody: the client driving it left the battle, and it stands where it stopped.
    Nobody,
}

/// The client driving a vehicle, and what only it knows of it, once it has it, see
/// [`BattleVehicle::enter`].
#[derive(Debug)]
pub struct ClientDriver {
    client: ClientKey,
    avatar: EntityKey<Avatar>,
    /// The vehicle's components the client has it with.
    own_vehicle: Option<ComponentKey<OwnVehicle>>,
    gun_marker: Option<ComponentKey<GunMarkerComponent>>,
    /// The avatar's `ownVehicleGear` last sent, sent again only once it changes.
    engine_gear: Option<u8>,
}

impl BattleVehicle {

    /// The vehicle of a battle's player, created in its space at `start`, of the given
    /// configuration: it belongs to the player's avatar, and its client drives it once
    /// it has it, see [`Self::enter`].
    #[allow(clippy::too_many_arguments)]
    pub fn player(
        app: &mut base::App,
        world: &mut World,
        client: ClientKey,
        avatar: EntityKey<Avatar>,
        space: SpaceKey,
        loadout: &VehicleLoadout,
        player: Player,
        team: u8,
        start: TeamStart,
        info_index: u32,
    ) -> io::Result<Self> {

        let public = public_info(player.name, &loadout.descr, team, loadout.max_health as u16, loadout.crew.len());
        let vehicle = app.create_entity::<schema::Vehicle>(space, EntityInit {
            position: start.position,
            direction: start.direction,
            alias: true,
            controlled_by: Some(avatar.any()),
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(%avatar, entity_id = %vehicle.id(), "Created the vehicle, at {:?}", start.position);

        let driven = Vehicle::new(start.position, start.direction, loadout.mobility, loadout.aiming.clone());
        let info = vehicle_info(vehicle.id(), &public, player.id);
        let mut entity = app.entity(vehicle)?;
        init_vehicle(&mut entity, avatar.id(), public, driven.turret.gun_angles_packed())?;

        entity.set_isMyVehicle(true)?;
        entity.set_quickShellChangerFactor(QUICK_SHELL_CHANGER_FACTOR)?;
        entity.set_onRespawnReloadTimeFactor(1.0)?;
        entity.set_perkEffects(PerkEffects::default())?;
        entity.set_perks(Vec::new())?;
        entity.set_perksRibbonNotify(Vec::new())?;
        entity.set_crewCompactDescrs(loadout.crew.iter().cloned().map(Bytes).collect())?;
        entity.set_enhancements(empty_dict())?;
        entity.set_setups(Pickle(loadout.setups.clone()))?;
        entity.set_setupsIndexes(Pickle(loadout.setup_indexes.clone()))?;
        entity.set_vehPerks(empty_dict())?;
        entity.set_disabledSwitches(Vec::new())?;
        entity.set_ownVehiclePosition(None)?;

        let ammo = loadout.shots.iter().cloned().zip(loadout.ammo.iter().map(|&(_, count)| count)).collect();
        Ok(Self {
            entity: vehicle,
            team,
            info,
            info_index,
            driver: Driver::Client(ClientDriver {
                client,
                avatar,
                own_vehicle: None,
                gun_marker: None,
                engine_gear: None,
            }),
            id: world.add(driven),
            gun: GunLoader::new(ammo, loadout.aiming.reload_time)
                .with_magazine(loadout.magazine)
                .with_quick_change_factor(QUICK_SHELL_CHANGER_FACTOR),
            collision: loadout.collision.clone(),
            health: loadout.max_health.into(),
            max_health: loadout.max_health.into(),
            engine_mode: None,
            strafing: false,
            damage_stickers: Vec::new(),
            fire: None,
        })

    }

    /// A bot, created in a battle's space at `start`, of this definition and descriptor,
    /// in this team, no avatar, with its `vehiclesInfo` entry at the given index: the
    /// `index`th bot, its shots numbered apart from the others'.
    #[allow(clippy::too_many_arguments)]
    pub fn bot(
        app: &mut base::App,
        world: &mut World,
        space: SpaceKey,
        def: &VehicleDef,
        compact_descr: &[u8],
        team: u8,
        name: String,
        start: TeamStart,
        info_index: u32,
        index: i32,
    ) -> io::Result<Self> {

        let TeamStart { position, direction } = start;
        let ammo = def.shots.iter().cloned().zip(def.default_ammo().into_iter().map(|(_, count)| count)).collect();
        let gun = GunLoader::new(ammo, def.aiming.reload_time).with_magazine(def.magazine).with_shot_ids_from(BOT_FIRST_SHOT_ID * (index + 1));
        let driven = Vehicle::new(position, direction, def.mobility, def.aiming.clone());
        let gun_angles = driven.turret.gun_angles_packed();

        let public = public_info(name, compact_descr, team, def.max_health as u16, def.crew_roles.len());

        let vehicle = app.create_entity::<schema::Vehicle>(space, EntityInit {
            position,
            direction,
            alias: true,
            controlled_by: None,
            visibility: Visibility::Interest,
            ..EntityInit::default()
        })?;
        info!(entity_id = %vehicle.id(), team, "Created a bot: {}, at {position:?}", def.name);
        let info = vehicle_info(vehicle.id(), &public, 0);
        init_vehicle(&mut app.entity(vehicle)?, EntityId(0), public, gun_angles)?;
        Ok(Self {
            entity: vehicle,
            team,
            info,
            info_index,
            driver: Driver::Bot(Bot::default()),
            id: world.add(driven),
            gun,
            collision: def.collision.clone(),
            health: def.max_health.into(),
            max_health: def.max_health.into(),
            engine_mode: None,
            strafing: false,
            damage_stickers: Vec::new(),
            fire: None,
        })

    }

    pub fn alive(&self) -> bool {
        self.health > 0
    }

    /// Where its hull is in the world.
    pub fn hull_center(&self, world: &World) -> Vec3 {
        Vec3::from(world.vehicle(self.id).part_transforms(self.collision.hull_position)[VehiclePart::Hull as usize].translation)
    }

    /// It as shells hit it: its collision models where its parts are.
    pub fn shot_target(&self, world: &World) -> ShotTarget {
        let collision = self.collision.clone();
        ShotTarget {
            entity_id: self.entity.id().0,
            transforms: world.vehicle(self.id).part_transforms(collision.hull_position),
            collision,
        }
    }

    /// Its client has it, of the given configuration: from there on it drives it. Then its
    /// `OwnVehicle` component: the client only turns its turret once it knows how fast it
    /// can. Its `GunMarkerComponent` too, for the client's server reticle.
    pub fn enter(&mut self, app: &mut base::App, loadout: &VehicleLoadout) -> io::Result<()> {

        let Driver::Client(driver) = &mut self.driver else { return Ok(()) };
        let entity_id = self.entity.id();
        let mut entity = app.entity(self.entity)?;
        let mut own_vehicle = entity.add_component::<OwnVehicle>(Audience::OwnClient)?;
        // No crew or equipment bonus: the view range all vehicles have here.
        own_vehicle.set_syncVehicleAttrs(VehicleSyncAttrs { circularVisionRadius: VIEW_RANGE as u16, gunPiercing: 1.0, gunShotsSpeed: 1.0 })?;
        own_vehicle.set_targetingInfo(Some(targeting_info(loadout)))?;
        // What the real server sends right after, as updates: the ammunition, the
        // shells loaded, the reload and the health.
        own_vehicle.set_vehicleAmmoList(gun_ammo_list(&self.gun))?;
        own_vehicle.set_vehicleSettings(vehicle_settings(entity_id, &self.gun))?;
        own_vehicle.set_vehicleGunReloadTime(gun_reload_time(loadout.aiming.reload_time, 0.0, 0.0))?;
        own_vehicle.set_vehicleHealthInfo(vehicle_health_info(entity_id, self.health))?;
        let own_vehicle = own_vehicle.key();
        let mut marker = entity.add_component::<GunMarkerComponent>(Audience::OwnClient)?;
        marker.set_gunMarker(None)?;
        driver.own_vehicle = Some(own_vehicle);
        driver.gun_marker = Some(marker.key());
        Ok(())

    }

    /// Give the clients it as it is, `driven` in the world: where it is, its gun's angles,
    /// and its engine's mode and whether it slides sideways once they change.
    pub fn show(&mut self, app: &mut base::App, driven: &Vehicle) -> io::Result<()> {

        let Vehicle { hull, turret } = driven;
        app.set_position(self.entity, hull.position, hull.direction())?;
        let mut view = app.entity(self.entity)?;
        view.set_gunAnglesPacked(turret.gun_angles_packed())?;

        let mode = hull.engine_mode();
        if self.engine_mode != Some(mode) {
            self.engine_mode = Some(mode);
            view.set_engineMode(mode)?;
        }

        let strafing = hull.slide.abs() > STRAFING_SLIDE;
        if self.strafing != strafing {
            self.strafing = strafing;
            view.set_isStrafing(strafing)?;
        }

        Ok(())

    }

    /// Show a wreck where it is, `driven` in the world, once it was shoved.
    pub fn show_wreck(&self, app: &mut base::App, driven: &Vehicle) -> io::Result<()> {
        let hull = &driven.hull;
        if app.position(self.entity) != Some((hull.position, hull.direction())) {
            app.set_position(self.entity, hull.position, hull.direction())?;
        }
        Ok(())
    }

    /// Put out its fire at game time `now`, if it is due to go out, in this battle.
    pub fn put_out_fire(&mut self, app: &mut base::App, battle: u32, now: f64) -> io::Result<()> {
        if let Some((fire, _)) = self.fire.take_if(|&mut (_, until)| until <= now) {
            app.remove_component(fire)?;
            info!(battle, entity_id = %self.entity.id(), "Put out a vehicle's fire");
        }
        Ok(())
    }

    /// Its gun loaded at game time `now`, its magazine full again: its client is told,
    /// the shells first, as it expects.
    pub fn show_loaded(&mut self, app: &mut base::App, now: f64) -> io::Result<()> {
        let loaded = self.gun.take_loaded(now);
        if let (Some(own_vehicle), true) = (self.client_driver().and_then(|driver| driver.own_vehicle), loaded) {
            let mut own_vehicle = app.component(own_vehicle)?;
            if self.gun.has_magazine() {
                own_vehicle.set_vehicleAmmoList(gun_ammo_list(&self.gun))?;
            }
            own_vehicle.set_vehicleGunReloadTime(reload_time(&self.gun, now))?;
        }
        Ok(())
    }

    /// What its client has of it, if a client drives it.
    pub fn client_driver(&self) -> Option<&ClientDriver> {
        match &self.driver {
            Driver::Client(driver) => Some(driver),
            Driver::Bot(_) | Driver::Nobody => None,
        }
    }

    /// Its gun fired the rest of a burst: its client is told its shells and their
    /// settings, its reload having started with the burst, see `GunLoader::fire`.
    pub fn show_burst(&self, app: &mut base::App) -> io::Result<()> {
        if let Some(own_vehicle) = self.client_driver().and_then(|driver| driver.own_vehicle) {
            let mut own_vehicle = app.component(own_vehicle)?;
            own_vehicle.set_vehicleAmmoList(gun_ammo_list(&self.gun))?;
            own_vehicle.set_vehicleSettings(vehicle_settings(self.entity.id(), &self.gun))?;
        }
        Ok(())
    }

    /// Give its client its gun as it is at game time `now`, if a client drives it.
    pub fn show_gun(&self, app: &mut base::App, now: f64) -> io::Result<()> {
        match self.client_driver().and_then(|driver| driver.own_vehicle) {
            Some(own_vehicle) => show_gun(app, own_vehicle, self.entity.id(), &self.gun, now),
            None => Ok(()),
        }
    }

    /// Give its client, if a client drives it, what only it knows of it, `driven` in the
    /// world, its gun's marker as given, see [`ClientDriver::show`].
    pub fn show_own(&mut self, app: &mut base::App, driven: &Vehicle, marker: (Vec3, Vec3, Dispersion)) -> io::Result<()> {
        match &mut self.driver {
            Driver::Client(driver) => driver.show(app, driven, marker),
            Driver::Bot(_) | Driver::Nobody => Ok(()),
        }
    }

    /// Its client changes one of its settings, by `VEHICLE_SETTING` code, at game time
    /// `now`: the shell its gun loads next, or the one it is loaded with, which the gun
    /// then reloads, or it reloads its magazine. The client predicts the change, so it is
    /// given the gun back even when it doesn't change, to undo what it predicted then.
    pub fn change_setting(&mut self, app: &mut base::App, client: ClientKey, now: f64, code: u8, value: i32) -> io::Result<()> {

        let gun = &mut self.gun;
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

        let Some(own_vehicle) = self.client_driver().and_then(|driver| driver.own_vehicle) else { return Ok(()) };
        if reloads {
            show_gun(app, own_vehicle, self.entity.id(), &self.gun, now)
        } else {
            app.component(own_vehicle)?.set_vehicleSettings(vehicle_settings(self.entity.id(), &self.gun))
        }

    }

    /// It takes a shell's damage at game time `now`, in this battle, varied by up to
    /// [`DAMAGE_SPREAD`] either way: its health drops, in its `OwnVehicle` too when a
    /// client drives it, and the clients see the hit, as the captured server shows one. A
    /// damaging hit sets it on fire, which a kill puts out. Whether it is killed.
    pub fn take_damage(
        &mut self,
        app: &mut base::App,
        battle: u32,
        shooter: EntityId,
        shot: &Shot,
        shell_hit: &ShellHit,
        now: f64,
    ) -> io::Result<bool> {

        let entity_id = self.entity.id();
        let amount = spread_damage(shell_hit.damage);
        let old_health = self.health;
        self.health = (old_health - amount).max(0);
        let new_health = self.health;
        let killed = new_health == 0;
        let bounds = self.collision.parts[shell_hit.hit.part as usize].bounds;
        if amount > 0 {
            info!(battle, %entity_id, %shooter, "Damaged a vehicle: {amount}, {old_health} -> {new_health}{}", if killed { ", destroyed" } else { "" });
            if let Some(own_vehicle) = self.client_driver().and_then(|driver| driver.own_vehicle) {
                app.component(own_vehicle)?.set_vehicleHealthInfo(vehicle_health_info(entity_id, new_health))?;
            }
        }

        let stickers = shell_hit.sticker(shot, bounds).map(|sticker| {
            self.damage_stickers.push(sticker);
            self.damage_stickers.clone()
        });
        let damage = Damage {
            shooter,
            amount,
            max_health: self.max_health,
            old_health,
            new_health,
        };
        show_damage(&mut app.entity(self.entity)?, shot, shell_hit, bounds, &damage, stickers)?;

        match (self.fire.take(), killed) {
            (Some((fire, _)), true) => app.remove_component(fire)?,
            (Some((fire, _)), false) => self.fire = Some((fire, now + FIRE_DURATION)),
            (None, true) => {}
            (None, false) if amount == 0 => {}
            (None, false) => {
                let mut entity = app.entity(self.entity)?;
                let mut fire = entity.add_component::<Fire>(Audience::AllClients)?;
                fire.set_fireInfo(None)?;
                self.fire = Some((fire.key(), now + FIRE_DURATION));
                info!(battle, %entity_id, "Set a vehicle on fire");
            }
        }
        Ok(killed)

    }

    /// Every client lists it destroyed, by whom, in its `vehiclesInfo` entry, on each
    /// of these: the entry's changes apply once its generation changes too.
    pub fn mark_killed(&mut self, app: &mut base::App, vehicles_infos: &[ComponentKey<AvatarVehiclesInfo>], killer: EntityId) -> io::Result<()> {
        let info = &mut self.info;
        let death = DeathInfo {
            victimID: info.vehicleID,
            killerID: killer,
            reasonID: ATTACK_REASON_SHOT,
            equipmentID: 0,
            numVehiclesAffected: 0,
        };
        info.deathInfo = Some(death.clone());
        info.isAlive = false;
        info.__generation += 1;
        let index = self.info_index;
        for &vehicles_info in vehicles_infos {
            let mut vehicles_info = app.component(vehicles_info)?;
            vehicles_info.set_vehiclesInfo_at(info_path(index, "deathInfo"), Some(death.clone()))?;
            vehicles_info.set_vehiclesInfo_at(info_path(index, "isAlive"), false)?;
            vehicles_info.set_vehiclesInfo_at(info_path(index, "__generation"), info.__generation)?;
        }
        Ok(())
    }

}

impl ClientDriver {

    /// The avatar of its client.
    pub fn avatar(&self) -> EntityKey<Avatar> {
        self.avatar
    }

    /// Give its client what only it knows of its vehicle, `driven` in the world: its
    /// server reticle, from the gun's marker, its avatar where it is, what it aims from,
    /// which must turn with the hull, and the engine's RPM, then its gear when it shifts.
    fn show(&mut self, app: &mut base::App, driven: &Vehicle, (position, velocity, dispersion): (Vec3, Vec3, Dispersion)) -> io::Result<()> {
        let (client, avatar) = (self.client, self.avatar);
        let Vehicle { hull, turret } = driven;
        if let Some(marker) = self.gun_marker {
            app.component(marker)?.set_gunMarker(Some(GunMarker { gunPosition: position, shotVector: velocity, dispersion: dispersion.angle }))?;
        }
        // Every tick, to compare with what the client simulates.
        debug!(%client, "Stepped the vehicle: at {:?}, direction {:?}, speed {:.2}, turret {:.4} {:.4}",
            hull.position, hull.direction(), hull.speed, turret.yaw, turret.pitch);
        app.set_position(avatar, hull.position, Vec3::ZERO)?;
        let mut avatar_view = app.entity(avatar)?;
        avatar_view.set_ownVehicleAuxPhysicsData(hull.aux_physics_data())?;
        let (gear, _) = hull.engine();
        if self.engine_gear != Some(gear) {
            self.engine_gear = Some(gear);
            avatar_view.set_ownVehicleGear(gear)?;
        }
        Ok(())
    }

}

/// Stop the gun where our turret is rather than at the client's angles, which its own
/// rotator computes slightly apart: the gun would move as it locks.
pub fn hold_gun(client: ClientKey, vehicle: &mut Vehicle, yaw: f32, pitch: f32) {
    let turret = &mut vehicle.turret;
    debug!(%client, "Held the gun: yaw {:.4}, pitch {:.4} (client: {yaw:.4}, {pitch:.4})", turret.yaw, turret.pitch);
    turret.hold(turret.yaw, turret.pitch);
}

/// The `vehiclesInfo` entry of a vehicle of the given id and public info, alive, as in
/// a random battle: no clan, badges nor missions.
fn vehicle_info(id: EntityId, public: &PublicVehicleInfo, account_dbid: u64) -> VehicleInfo {
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
        personalMissionInfo: empty_dict(),
        events: empty_dict(),
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

/// The path to a field of the `index`th entry of `vehiclesInfo`.
fn info_path(index: u32, field: &str) -> Vec<PathStep> {
    vec![PathStep::Index(index), PathStep::Field(field.into())]
}

/// An empty dict, as a pickle.
fn empty_dict() -> Pickle {
    Pickle(PickleValue::Dict(Default::default()))
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
    entity.set_avatarID(avatar)?;
    entity.set_gunAnglesPacked(gun_angles)?;
    entity.set_dogTag(BattleDogTag { dogTag: default_dog_tag(), defaultDogTag: default_dog_tag(), showDogTagToKiller: false })?;
    entity.set_health(public.maxHealth as i16)?;
    entity.set_publicInfo(public)?;
    entity.set_isStrafing(false)?;
    entity.set_isCrewActive(true)?;
    entity.set_damageStickers(Vec::new())?;
    entity.set_publicStateModifiers(Vec::new())?;
    entity.set_vehPostProgression(Vec::new())
}

/// The default dog tag, of [`DEFAULT_DOG_TAG`]'s components.
fn default_dog_tag() -> DogTag {
    DogTag {
        components: DEFAULT_DOG_TAG.iter()
            .map(|&(id, grade)| DogTagComponent { id, progress: 0.0, grade })
            .collect(),
    }
}

/// Give the client its gun as it is: its shells and how many are left, in its magazine
/// too, the shells it loads, and its reload if any.
fn show_gun(app: &mut base::App, own: ComponentKey<OwnVehicle>, entity_id: EntityId, gun: &GunLoader, now: f64) -> io::Result<()> {
    let mut own = app.component(own)?;
    own.set_vehicleAmmoList(gun_ammo_list(gun))?;
    own.set_vehicleSettings(vehicle_settings(entity_id, gun))?;
    if gun.reload().is_some() {
        own.set_vehicleGunReloadTime(reload_time(gun, now))?;
    }
    Ok(())
}

/// The `vehicleAmmoList` of a gun.
fn gun_ammo_list(gun: &GunLoader) -> Vec<schema::VehicleAmmo> {
    ammo_list(gun.ammo_state().map(|(def, count, in_magazine)| (def.compact_descr.to_int(), count, in_magazine)))
}

/// The `vehicleGunReloadTime` of a gun at game time `now`: the reload going on, else
/// its whole reload's length once loaded. The real server gives a quick shell change as
/// a whole reload first, then as its share, and back on loaded; the share alone is
/// enough here.
fn reload_time(gun: &GunLoader, now: f64) -> schema::GunReloadTime {
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

/// A shell's damage, varied by up to [`DAMAGE_SPREAD`] either way.
fn spread_damage(damage: f32) -> i64 {
    use rand::Rng;
    if damage <= 0.0 {
        return 0;
    }
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

/// Show a vehicle taking a shell's damage: from a shot, one hit point, its segment
/// encoded over the hit part's `bounds`, its params as `HitParamsEncoder` encodes them,
/// or from an explosion beside it, its center; and the damage, also as a fraction of
/// its maximum health out of 255. Then, when it took some, its new health, and its
/// stickers, given when the shell added one, see [`ShellHit::sticker`].
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
            vec![shell_hit.point(shot, bounds, shell_hit.effect)],
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


/// How fast a vehicle slides across its heading, in m/s, before the clients are told it
/// is strafing: their filter, which models its tracks, otherwise holds it back from
/// sliding, and draws it metres behind where it is shoved. Fitted on the collisions
/// capture, where the real server sets `isStrafing` on the ticks the player's vehicle
/// moves sideways faster than 0.03 to 0.05 m/s, pushed, skidding or sliding down a slope.
const STRAFING_SLIDE: f32 = 0.04;

/// How much a shell's damage varies, either way, as a share of it.
const DAMAGE_SPREAD: f32 = 0.25;

/// How long a vehicle burns after its last hit, in seconds.
const FIRE_DURATION: f64 = 5.0;

/// `ATTACK_REASON.SHOT`'s index.
const ATTACK_REASON_SHOT: i8 = 0;

/// The first shot id of the first bot, of the next one twice that, and so on, apart from
/// the player's.
const BOT_FIRST_SHOT_ID: i32 = 1_000_000;

/// The `quickShellChangerFactor` of a vehicle, as captured: how much faster it changes
/// shells, from its crew and equipment.
const QUICK_SHELL_CHANGER_FACTOR: f32 = 0.268;

/// The default dog tag's components, as captured: their ids and grades.
const DEFAULT_DOG_TAG: [(i32, i8); 2] = [(300, 9), (66, 0)];
