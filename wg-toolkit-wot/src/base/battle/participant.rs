//! A client in a battle: its avatar, its vehicle, its own arena info, what it sees of the
//! others' vehicles, what its auto-aim is locked on, what it watches once its vehicle is
//! destroyed, and whether it is done loading.

use std::io;

use tracing::info;
use glam::Vec3;

use wgtk::app::entity::{ComponentKey, EntityId, EntityKey};
use wgtk::app::base::{self, ClientKey};

use crate::game::schema::{Avatar, AvatarVehiclesInfo};
use crate::base::{VehicleLoadout, TICK};
use crate::base::physics::World;

use super::vehicle::BattleVehicle;
use super::spotting::Spotting;
use super::period::ArenaPeriod;


/// `TARGET_LOST_FLAGS.INVALID` in the client's constants.py: its auto-aim can't lock
/// on that vehicle.
const TARGET_LOST_INVALID: u8 = 1;
/// `TARGET_LOST_FLAGS.KILLED_BY_ME`: its auto-aim's target was destroyed by its own
/// vehicle.
const TARGET_LOST_KILLED_BY_ME: u8 = 2;
/// `TARGET_LOST_FLAGS.KILLED_BY_OTHERS`: its auto-aim's target was destroyed by another.
const TARGET_LOST_KILLED_BY_OTHERS: u8 = 4;
/// `TARGET_LOST_FLAGS.UNREACHABLE`: its auto-aim's target is no longer spotted.
const TARGET_LOST_UNREACHABLE: u8 = 8;


/// See the module's documentation.
#[derive(Debug)]
pub struct Participant {
    /// Its client, which the battle tells what it sees.
    pub client: ClientKey,
    /// Its avatar, the player's entity, told the battle's period and shots.
    pub avatar: EntityKey<Avatar>,
    /// Its vehicle's configuration.
    pub loadout: VehicleLoadout,
    /// Its vehicle, by its index among the battle's.
    pub vehicle: usize,
    /// The arena's vehicles, on its own info entity.
    pub vehicles_info: ComponentKey<AvatarVehiclesInfo>,
    /// Whether it sees each of the battle's vehicles, by index, see [`Self::spot`].
    spotted: Vec<bool>,
    /// The vehicle its auto-aim is locked on, by index, see [`Self::auto_aim`].
    locked: Option<usize>,
    /// What it watches once its vehicle is destroyed, see [`Self::watch`]: a vehicle, an
    /// ally or its own wreck, by its index, its avatar riding and following it, or none,
    /// its free camera, its avatar where the camera moves it, see [`Self::move_to`].
    watched: Option<usize>,
    /// Whether its client is done loading, ready for the countdown.
    pub ready: bool,
    /// Whether its player may use the server's dev commands, see `Player::dev`.
    pub dev: bool,
}

impl Participant {

    /// A client in a battle of `vehicles` vehicles, with its avatar and its vehicle's,
    /// and whether its player is a dev.
    pub fn new(
        client: ClientKey,
        avatar: EntityKey<Avatar>,
        loadout: VehicleLoadout,
        vehicle: usize,
        vehicles_info: ComponentKey<AvatarVehiclesInfo>,
        vehicles: usize,
        dev: bool,
    ) -> Self {
        Self {
            client,
            avatar,
            loadout,
            vehicle,
            vehicles_info,
            spotted: vec![false; vehicles],
            locked: None,
            watched: None,
            ready: false,
            dev,
        }
    }

    /// Show it the battle's vehicles, or hide them, as its team spots them at game time
    /// `now`, see [`Spotting`]: its allies always, its enemies once spotted, and wrecks
    /// always, from when they are destroyed. Its own it always sees.
    pub fn spot(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], spotting: &Spotting, now: f64) -> io::Result<()> {
        
        let team = vehicles[self.vehicle].team;
        for (index, vehicle) in vehicles.iter().enumerate() {
            
            if index == self.vehicle {
                continue;
            }

            let spotted = !vehicle.alive() || vehicle.team == team || spotting.spotted(team, index, now);
            if spotted == self.spotted[index] {
                continue;
            }

            self.spotted[index] = spotted;
            if spotted {
                app.show(self.client, vehicle.entity)?;
            } else {
                app.hide(self.client, vehicle.entity)?;
            }

            info!(client = %self.client, entity_id = %vehicle.entity.id(), team = vehicle.team, "{} a vehicle", if spotted { "Spotted" } else { "Lost" });

        }

        Ok(())
        
    }

    /// Its client locks its auto-aim on the vehicle of this entity id, or unlocks it
    /// with 0. It locks only on an enemy alive that it sees, its client told otherwise
    /// that it lost it, as invalid.
    pub fn auto_aim(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], target: EntityId) -> io::Result<()> {

        self.locked = None;
        if target.0 == 0 {
            info!(client = %self.client, "Unlocked the auto-aim");
            return Ok(());
        }

        let team = vehicles[self.vehicle].team;
        let locked = vehicles.iter()
            .position(|vehicle| vehicle.entity.id() == target)
            .filter(|&index| self.spotted[index] && vehicles[index].alive() && vehicles[index].team != team);
        let Some(index) = locked else {
            info!(client = %self.client, entity_id = %target, "Refused to lock the auto-aim: not an enemy alive in sight");
            return app.entity(self.avatar)?.onAutoAimVehicleLost(TARGET_LOST_INVALID);
        };

        self.locked = Some(index);
        info!(client = %self.client, entity_id = %target, "Locked the auto-aim");
        Ok(())

    }

    /// Unlock its auto-aim once its target is destroyed, by its own vehicle or another,
    /// or once it no longer sees it, see [`Self::spot`], its client told why.
    pub fn keep_lock(&mut self, app: &mut base::App, vehicles: &[BattleVehicle]) -> io::Result<()> {

        let Some(index) = self.locked else { return Ok(()) };
        let target = &vehicles[index];
        let own = vehicles[self.vehicle].entity.id();
        let killer = target.info.deathInfo.as_ref().map(|death| death.killerID);

        let flags = if !target.alive() && killer == Some(own) {
            TARGET_LOST_KILLED_BY_ME
        } else if !target.alive() {
            TARGET_LOST_KILLED_BY_OTHERS
        } else if !self.spotted[index] {
            TARGET_LOST_UNREACHABLE
        } else {
            return Ok(());
        };

        self.locked = None;
        app.entity(self.avatar)?.onAutoAimVehicleLost(flags)?;
        info!(client = %self.client, entity_id = %target.entity.id(), flags, "Lost the auto-aim's target");
        Ok(())

    }

    /// Its client, its vehicle destroyed, watches the vehicle of this entity id, see
    /// [`Self::watchable`], which it is told to switch to, then its avatar rides it, see
    /// [`Self::board`]: the client binds its camera to the vehicle its avatar rides,
    /// once switched to it.
    pub fn watch(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World, target: EntityId) -> io::Result<()> {

        let Some(watched) = self.watchable(vehicles, target) else { return Ok(()) };

        if watched.is_some() {
            app.entity(self.avatar)?.onSwitchViewpoint(target, Vec3::ZERO)?;
            info!(client = %self.client, entity_id = %target, "Switched the viewpoint to a vehicle");
        }

        self.board(app, vehicles, world, watched)

    }

    /// Its avatar rides the vehicle of this entity id, its own destroyed, see
    /// [`Self::watchable`], as its client binds it, or none, for its free camera.
    pub fn bind(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World, target: EntityId) -> io::Result<()> {
        let Some(watched) = self.watchable(vehicles, target) else { return Ok(()) };
        self.board(app, vehicles, world, watched)
    }

    /// What it may watch of the vehicle of this entity id, its own destroyed: an ally's,
    /// or its own wreck, by index; with 0, none, its free camera. None if refused.
    fn watchable(&self, vehicles: &[BattleVehicle], target: EntityId) -> Option<Option<usize>> {

        let own = &vehicles[self.vehicle];
        if own.alive() {
            info!(client = %self.client, entity_id = %target, "Refused to switch the viewpoint: its vehicle is alive");
            return None;
        }

        if target.0 == 0 {
            return Some(None);
        }

        let watched = vehicles.iter()
            .position(|vehicle| vehicle.entity.id() == target)
            .filter(|&index| vehicles[index].team == own.team);
        if watched.is_none() {
            info!(client = %self.client, entity_id = %target, "Refused to switch the viewpoint: not an ally");
            return None;
        }

        Some(watched)

    }

    /// Its avatar rides the vehicle it now watches, by index, from where it is, see
    /// [`Self::follow`], or none, staying where it is, see [`base::App::teleport`].
    fn board(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World, watched: Option<usize>) -> io::Result<()> {

        self.watched = watched;

        let Some(index) = watched else {
            let (position, _) = app.position(self.avatar).unwrap_or_default();
            info!(client = %self.client, "Freed the camera");
            return app.teleport(self.avatar, None, position, Vec3::ZERO);
        };

        let vehicle = &vehicles[index];
        let position = world.vehicle(vehicle.id).hull.position;
        app.teleport(self.avatar, Some(vehicle.entity.id()), position, Vec3::ZERO)

    }

    /// Its free camera moves to a position, its vehicle destroyed: its avatar, off any
    /// vehicle, goes there, what it sees with it.
    pub fn move_to(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], position: Vec3) -> io::Result<()> {

        if vehicles[self.vehicle].alive() {
            return Ok(());
        }

        self.watched = None;
        app.teleport(self.avatar, None, position, Vec3::ZERO)

    }

    /// Its avatar follows the vehicle it watches, its own destroyed, where it is in the
    /// world, see [`Self::watch`].
    pub fn follow(&self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World) -> io::Result<()> {
        let Some(index) = self.watched.filter(|_| !vehicles[self.vehicle].alive()) else { return Ok(()) };
        let position = world.vehicle(vehicles[index].id).hull.position;
        app.set_position(self.avatar, position, Vec3::ZERO)
    }

    /// Tell its client the battle's period, as long as is left of it: its game time is
    /// the ticks it got. Once over, it is first told who won, which it shows.
    pub fn show_period(&self, app: &mut base::App, period: ArenaPeriod) -> io::Result<()> {

        if let ArenaPeriod::AfterBattle { winner, reason, .. } = period {
            app.entity(self.avatar)?.onRoundFinished(winner as i8, reason)?;
            info!(client = %self.client, winner, reason, "Told the battle's end");
        }

        let game_time = f64::from(app.ticks(self.client).unwrap_or(0)) * TICK.as_secs_f64();
        let Some((kind, data)) = period.arena_update(game_time)? else { return Ok(()) };
        app.entity(self.avatar)?.updateArena(kind, data)?;
        info!(client = %self.client, ?period, "Changed the arena period");
        Ok(())

    }

}
