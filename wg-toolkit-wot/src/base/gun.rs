//! A vehicle's gun as the server runs it: its ammunition, the shell loaded, its magazine
//! and bursts, the reload, and each shot's flight until it hits a vehicle, the terrain,
//! or runs out of range.
//! Vehicles are hit on their collision models, without armour yet; destructibles are
//! not hit yet.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use rand::Rng;

use wgtk::space::terrain::Terrain;

use crate::wot::collision::{VehicleCollision, VehicleHit};
use crate::wot::item_defs::{MagazineDef, ShotDef};


/// The flight is followed in steps of this many seconds.
const FLIGHT_STEP: f32 = 0.005;


#[derive(Debug, Clone)]
pub struct Gun {
    /// Each shell the gun fires, and how many are left of it, those in the gun included
    /// until fired.
    pub ammo: Vec<(ShotDef, u32)>,
    /// The index in `ammo` of the shell in the gun: the one it fires next, or loads
    /// while it reloads. The client knows it as `VEHICLE_SETTING.CURRENT_SHELLS`, and
    /// changes it by picking a shell twice, see [`Self::change_loaded`]. It only has none
    /// left once the gun has no shell left at all.
    loaded_shell: usize,
    /// The index in `ammo` of the shell the gun loads once its magazine is empty,
    /// `loaded_shell` itself unless the client picked another one since. The client
    /// knows it as `VEHICLE_SETTING.NEXT_SHELLS`, and changes it by picking a shell once,
    /// see [`Self::select_next`]. When there is none of it left by then, the gun loads
    /// the first shell it has some left of instead.
    next_shell: usize,
    /// How the gun's shells come beyond one per reload: its magazine and its bursts.
    magazine: MagazineDef,
    /// The shells of `loaded_shell` in the gun: those it fires before reloading whole,
    /// at most [`Self::magazine_size`], 1 for a gun without a magazine. Fired, they
    /// leave `ammo` too. Empty, the magazine fills again once its whole reload ends, as
    /// the client expects the shells to come.
    magazine_shells: u32,
    /// The burst going on: its shells left to fire, and the game time the next one fires
    /// at, see [`Self::fire_burst`].
    burst_shells: Option<(u32, f64)>,
    /// The length of a whole reload: of the magazine, for a gun with one.
    reload_time: f32,
    /// How long changing the shell of a loaded gun takes, as a share of its whole
    /// reload; it takes a whole reload unless within 0 and 1.
    quick_change_factor: f32,
    /// The reload going on, between two shots of the magazine too.
    reload: Option<Reload>,
    /// The id the next shell fired takes.
    next_shot_id: i32,
}

/// A reload going on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reload {
    /// Its length, in seconds: the magazine's interval between two of its shots. The
    /// client takes it as `baseTime`, and only takes the reload as just started, and
    /// lets the player fire again once it ends, when its time left is the same: else it
    /// holds the next shot for 2 seconds, see `__gunReloadCommandWaitEndTime`.
    pub base_time: f32,
    /// The game time it ends at.
    pub end: f64,
}

/// A shell fired, flying from `start` to `end`.
#[derive(Debug, Clone)]
pub struct Shot {
    pub id: i32,
    pub def: ShotDef,
    pub start: Vec3,
    pub velocity: Vec3,
    pub end: Vec3,
    /// The velocity at `end`.
    pub end_velocity: Vec3,
    /// When it gets to `end`, in seconds after firing.
    pub flight_time: f32,
    pub landing: Landing,
    /// The shells fired at once with it when it is the first of them, it included: the
    /// client shows them all from the first one, see `showShooting`. None for the others
    /// of a burst.
    pub burst: Option<u32>,
}

/// What a shell ends its flight on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Landing {
    /// The vehicle of this entity id, there.
    Vehicle(u32, VehicleHit),
    Terrain,
    /// Nothing, at its maximum distance.
    Range,
}

/// A vehicle shells can hit: its collision models, placed in the world.
#[derive(Debug, Clone)]
pub struct HitTarget {
    pub entity_id: u32,
    pub collision: Arc<VehicleCollision>,
    /// Each part's transform, see [`super::physics::Vehicle::part_transforms`].
    pub transforms: [Affine3A; 4],
}

impl Gun {

    /// A gun with these shells and no magazine, loaded with the first one there is, then
    /// loading it again, ready to fire.
    pub fn new(ammo: Vec<(ShotDef, u32)>, reload_time: f32) -> Self {
        let loaded = first_left(&ammo).unwrap_or(0);
        let mut gun = Self {
            ammo,
            loaded_shell: loaded,
            next_shell: loaded,
            magazine: MagazineDef::default(),
            magazine_shells: 0,
            burst_shells: None,
            reload_time,
            quick_change_factor: 1.0,
            reload: None,
            next_shot_id: 1,
        };
        gun.magazine_shells = gun.full_magazine();
        gun
    }

    /// The same gun numbering its shots from `first`: the client tells shots apart by
    /// their id alone, whoever fires them.
    pub fn with_shot_ids_from(mut self, first: i32) -> Self {
        self.next_shot_id = first;
        self
    }

    /// The same gun changing the shell it is loaded with in this share of its reload,
    /// see [`Self::change_loaded`].
    pub fn with_quick_change_factor(mut self, factor: f32) -> Self {
        self.quick_change_factor = factor;
        self
    }

    /// The same gun with this magazine and these bursts, its magazine full.
    pub fn with_magazine(mut self, magazine: MagazineDef) -> Self {
        self.magazine = magazine;
        self.magazine_shells = self.full_magazine();
        self
    }

    /// Fire the loaded shell at game time `now` from `start` toward `direction`, off by a
    /// random angle within `dispersion` radians, if the gun is loaded and has a shell
    /// left: the first of a burst, which [`Self::fire_burst`] fires the rest of. Once
    /// they are fired, the gun waits for its magazine's interval while it has some left,
    /// else reloads whole with its next shell, or another one once there is none of it
    /// left.
    pub fn fire(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: f32, terrain: Option<&Terrain>, vehicles: &[HitTarget]) -> Option<Shot> {

        self.settle(now);
        if self.burst_shells.is_some() || !self.is_loaded(now) || self.magazine_shells == 0 {
            return None;
        }

        let burst = self.magazine.burst_size.min(self.magazine_shells);
        let mut shot = self.shoot(start, direction, dispersion, terrain, vehicles)?;
        shot.burst = Some(burst);
        if burst > 1 {
            self.burst_shells = Some((burst - 1, now + f64::from(self.magazine.burst_interval)));
        } else {
            self.after_shots(now);
        }
        Some(shot)

    }

    /// Fire the next shell of the burst going on, if it is due by game time `now`, as
    /// [`Self::fire`] does. Once the burst is over, the gun waits or reloads as after
    /// a single shot.
    pub fn fire_burst(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: f32, terrain: Option<&Terrain>, vehicles: &[HitTarget]) -> Option<Shot> {

        let (left, at) = self.burst_shells.filter(|&(_, at)| now >= at)?;
        let Some(shot) = self.shoot(start, direction, dispersion, terrain, vehicles) else {
            self.burst_shells = None;
            self.after_shots(now);
            return None;
        };

        if left > 1 {
            self.burst_shells = Some((left - 1, at + f64::from(self.magazine.burst_interval)));
        } else {
            self.burst_shells = None;
            self.after_shots(now);
        }
        Some(shot)

    }

    /// Load the shell of this integer descriptor once the gun fires, if it has some
    /// left: `VEHICLE_SETTING.NEXT_SHELLS`. Whether it does.
    pub fn select_next(&mut self, compact_descr: i64) -> bool {
        let Some(index) = self.shell_left(compact_descr) else { return false };
        self.next_shell = index;
        true
    }

    /// Load the shell of this integer descriptor instead of the one loaded, and after it
    /// too, if it has some left: `VEHICLE_SETTING.CURRENT_SHELLS`. The gun then reloads,
    /// in its quick change factor of a reload if it was loaded, as captured. Whether it
    /// reloads.
    ///
    /// A magazine is emptied and reloaded whole with it, quickly only when it was full,
    /// as the client predicts. Not while firing a burst.
    pub fn change_loaded(&mut self, now: f64, compact_descr: i64) -> bool {

        self.settle(now);
        if self.burst_shells.is_some() {
            return false;
        }

        let Some(index) = self.shell_left(compact_descr) else {
            return false
        };

        self.next_shell = index;
        if index == self.loaded_shell {
            return false;
        }

        let full = self.magazine_shells > 0 && self.magazine_shells == self.full_magazine();
        let quick = full && self.is_loaded(now) && self.quick_change_factor > 0.0 && self.quick_change_factor < 1.0;
        let time = if quick { self.reload_time * self.quick_change_factor } else { self.reload_time };

        self.loaded_shell = index;
        self.magazine_shells = 0;
        self.start_reload(now, time);
        true

    }

    /// Reload a magazine that is not full or not of the next shell, whole, with the next
    /// shell: `VEHICLE_SETTING.RELOAD_PARTIAL_CLIP`. Whether it reloads: not without a
    /// magazine, while firing a burst, or already reloading whole.
    pub fn reload_magazine(&mut self, now: f64) -> bool {

        self.settle(now);
        if self.magazine_size() <= 1 || self.burst_shells.is_some() || self.magazine_shells == 0 {
            return false;
        }

        if self.magazine_shells == self.full_magazine() && self.next_shell == self.loaded_shell {
            return false;
        }

        self.magazine_shells = 0;
        self.load_next();
        self.start_reload(now, self.reload_time);
        true

    }

    /// The shell loaded, and the shell it loads after.
    pub fn shells(&self) -> (Option<&ShotDef>, Option<&ShotDef>) {
        (self.ammo.get(self.loaded_shell).map(|(def, _)| def), self.ammo.get(self.next_shell).map(|(def, _)| def))
    }

    /// Each shell, how many are left of it, and how many of them are in the magazine:
    /// none without one, as captured, and only the loaded shell's with one.
    pub fn ammo_state(&self) -> impl Iterator<Item = (&ShotDef, u32, u32)> {
        let magazine_shells = if self.magazine_size() > 1 { self.magazine_shells } else { 0 };
        self.ammo.iter().enumerate()
            .map(move |(index, (def, count))| (def, *count, if index == self.loaded_shell { magazine_shells } else { 0 }))
    }

    /// Whether the gun has a magazine, or fires bursts: shells come more than one per
    /// reload then.
    pub fn has_magazine(&self) -> bool {
        self.magazine_size() > 1
    }

    /// Whether the gun is loaded by game time `now`.
    pub fn is_loaded(&self, now: f64) -> bool {
        self.reload.is_none_or(|reload| now >= reload.end)
    }

    /// Whether the gun got loaded by game time `now` since last asked, its magazine
    /// filled again if it reloaded whole.
    pub fn take_loaded(&mut self, now: f64) -> bool {
        self.settle(now);
        self.reload.take_if(|reload| now >= reload.end).is_some()
    }

    /// The reload going on.
    pub fn reload(&self) -> Option<Reload> {
        self.reload
    }

    /// The length of a whole reload, in seconds.
    pub fn reload_time(&self) -> f32 {
        self.reload_time
    }

    /// The shells the gun holds once reloaded: its magazine's, or its burst's when it
    /// fires bursts without a magazine, as the client counts them; 1 without either.
    fn magazine_size(&self) -> u32 {
        self.magazine.size.max(self.magazine.burst_size).max(1)
    }

    /// The shells of `loaded_shell` the gun holds once reloaded whole, fewer when it has fewer
    /// left.
    fn full_magazine(&self) -> u32 {
        self.ammo.get(self.loaded_shell).map_or(0, |&(_, count)| count.min(self.magazine_size()))
    }

    /// Fill the magazine again once its whole reload is over by game time `now`, before
    /// [`Self::take_loaded`] tells it, for the client to fire as soon as it is.
    fn settle(&mut self, now: f64) {
        if self.magazine_shells == 0 && self.is_loaded(now) && self.reload.is_some() {
            self.magazine_shells = self.full_magazine();
        }
    }

    /// Fire a shell of the magazine, without waiting or reloading.
    fn shoot(&mut self, start: Vec3, direction: Vec3, dispersion: f32, terrain: Option<&Terrain>, vehicles: &[HitTarget]) -> Option<Shot> {

        if self.magazine_shells == 0 {
            return None;
        }
        let (def, count) = self.ammo.get_mut(self.loaded_shell).filter(|(_, count)| *count > 0)?;
        *count -= 1;
        self.magazine_shells -= 1;
        let def = def.clone();

        let id = self.next_shot_id;
        self.next_shot_id += 1;

        let velocity = disperse(direction.normalize(), dispersion, &mut rand::thread_rng()) * def.speed;
        let (end, end_velocity, flight_time, landing) = fly(start, velocity, def.gravity, def.max_distance, terrain, vehicles);
        Some(Shot { id, def, start, velocity, end, end_velocity, flight_time, landing, burst: None })

    }

    /// After a shot or a burst: wait for the magazine's interval while it has some left,
    /// else reload whole with the next shell.
    fn after_shots(&mut self, now: f64) {
        if self.magazine_shells > 0 {
            self.start_reload(now, self.magazine.interval);
        } else {
            self.load_next();
            self.start_reload(now, self.reload_time);
        }
    }

    /// Load the next shell, or another one once there is none of it left.
    fn load_next(&mut self) {
        if !self.has_left(self.next_shell) {
            self.next_shell = first_left(&self.ammo).unwrap_or(self.next_shell);
        }
        self.loaded_shell = self.next_shell;
    }

    fn start_reload(&mut self, now: f64, time: f32) {
        self.reload = Some(Reload { base_time: time, end: now + f64::from(time) });
    }

    fn has_left(&self, index: usize) -> bool {
        self.ammo.get(index).is_some_and(|&(_, count)| count > 0)
    }

    /// The index of the shell of this integer descriptor, if the gun has some left.
    fn shell_left(&self, compact_descr: i64) -> Option<usize> {
        self.ammo.iter().position(|&(ref def, count)| def.compact_descr.to_int() == compact_descr && count > 0)
    }

}

/// The index of the first shell there is some left of.
fn first_left(ammo: &[(ShotDef, u32)]) -> Option<usize> {
    ammo.iter().position(|&(_, count)| count > 0)
}

/// A direction off `direction` by an angle within `angle`, uniformly over the cone's
/// disc.
fn disperse(direction: Vec3, angle: f32, rng: &mut impl Rng) -> Vec3 {
    let off = angle * rng.r#gen::<f32>().sqrt();
    let around = rng.r#gen::<f32>() * std::f32::consts::TAU;
    let side = direction.any_orthonormal_vector();
    let up = direction.cross(side);
    (direction * off.cos() + (side * around.cos() + up * around.sin()) * off.sin()).normalize()
}

/// Follow a shell from `start`: where it ends, its velocity and the time then, and what
/// it ends on.
fn fly(start: Vec3, velocity: Vec3, gravity: f32, max_distance: f32, terrain: Option<&Terrain>, vehicles: &[HitTarget]) -> (Vec3, Vec3, f32, Landing) {
    let (mut position, mut velocity, mut time) = (start, velocity, 0.0);
    loop {
        let next = position + velocity * FLIGHT_STEP;
        let next_velocity = velocity - Vec3::Y * gravity * FLIGHT_STEP;
        let hit = vehicles.iter()
            .filter_map(|vehicle| Some((vehicle.entity_id, vehicle.collision.first_hit(&vehicle.transforms, position, next)?)))
            .min_by(|a, b| a.1.fraction.total_cmp(&b.1.fraction));
        if let Some((entity_id, hit)) = hit {
            let t = hit.fraction;
            return (position.lerp(next, t), velocity.lerp(next_velocity, t), time + FLIGHT_STEP * t, Landing::Vehicle(entity_id, hit));
        }
        if let Some(ground) = terrain.and_then(|terrain| terrain.height_at(next.x, next.z)) {
            if next.y <= ground {
                // Where the step crosses the ground, assumed flat over it.
                let above = position.y - terrain.and_then(|t| t.height_at(position.x, position.z)).unwrap_or(ground);
                let below = ground - next.y;
                let t = (above / (above + below)).clamp(0.0, 1.0);
                return (position.lerp(next, t), velocity.lerp(next_velocity, t), time + FLIGHT_STEP * t, Landing::Terrain);
            }
        }
        if (next - start).length() >= max_distance {
            return (next, next_velocity, time + FLIGHT_STEP, Landing::Range);
        }
        (position, velocity, time) = (next, next_velocity, time + FLIGHT_STEP);
    }
}


#[cfg(test)]
mod tests {

    use super::*;
    use crate::wot::collision::{ArmorGroup, CollisionModel, VehiclePart};
    use crate::wot::items::{item_type, IntCompactDescr};

    fn shell() -> ShotDef {
        ShotDef {
            compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id: 1 },
            kind_index: 2,
            caliber: 120.0,
            effects_index: 0,
            pierced_sticker: None,
            prefab_effects_index: None,
            speed: 800.0,
            gravity: 9.81,
            max_distance: 720.0,
            damage: 400.0,
        }
    }

    #[test]
    fn fires_then_reloads() {
        let mut gun = Gun::new(vec![(shell(), 2)], 10.0);
        let shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert_eq!(shot.velocity, Vec3::Z * 800.0);
        assert_eq!(shot.landing, Landing::Range);
        assert!((shot.end.length() - 720.0).abs() < 5.0);
        assert!(gun.fire(5.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).is_none());
        assert!(!gun.take_loaded(5.0));
        assert!(gun.take_loaded(10.0));
        assert!(!gun.take_loaded(10.0));
        assert!(gun.fire(10.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).is_some());
        // Out of shells.
        assert!(gun.fire(20.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).is_none());
    }

    fn shell_of(id: u16) -> ShotDef {
        ShotDef { compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id }, ..shell() }
    }

    fn loaded_ids(gun: &Gun) -> (u16, u16) {
        let (loaded, next) = gun.shells();
        (loaded.unwrap().compact_descr.id, next.unwrap().compact_descr.id)
    }

    #[test]
    fn loads_the_next_shell_after_firing() {
        let mut gun = Gun::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (1, 2));
        assert_eq!(gun.fire(0.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap().def.compact_descr.id, 1);
        assert_eq!(loaded_ids(&gun), (2, 2));
        // None of the first left to select.
        assert!(!gun.select_next(shell_of(1).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn next_shell_falls_back_once_out() {
        let mut gun = Gun::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        gun.fire(0.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn changes_the_loaded_shell_quickly_once_loaded() {
        let mut gun = Gun::new(vec![(shell_of(1), 5), (shell_of(2), 5)], 10.0).with_quick_change_factor(0.25);
        let second = shell_of(2).compact_descr.to_int();
        assert!(gun.change_loaded(0.0, second));
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.5, end: 2.5 }));
        // Again while reloading: a whole reload.
        assert!(gun.change_loaded(1.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 11.0 }));
        assert!(!gun.is_loaded(10.0));
        assert!(gun.take_loaded(11.0));
        // Already loaded with it.
        assert!(!gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), None);
    }

    fn magazine(size: u32, burst_size: u32) -> MagazineDef {
        MagazineDef { size, interval: 2.0, burst_size, burst_interval: 0.5 }
    }

    fn fire(gun: &mut Gun, now: f64) -> Option<Shot> {
        gun.fire(now, Vec3::ZERO, Vec3::Z, 0.0, None, &[])
    }

    fn magazine_shells(gun: &Gun) -> Vec<u32> {
        gun.ammo_state().map(|(_, _, magazine_shells)| magazine_shells).collect()
    }

    #[test]
    fn empties_its_magazine_then_reloads_it_whole() {
        let mut gun = Gun::new(vec![(shell_of(1), 5), (shell_of(2), 5)], 10.0).with_magazine(magazine(3, 1));
        assert_eq!(magazine_shells(&gun), [3, 0]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(1));
        // The magazine's interval, as a reload of its own for the client to fire again.
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.0, end: 2.0 }));
        assert!(fire(&mut gun, 1.0).is_none());
        assert!(gun.take_loaded(2.0));
        fire(&mut gun, 2.0).unwrap();
        // The next shell is only loaded with the magazine.
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert!(gun.take_loaded(4.0));
        assert_eq!(fire(&mut gun, 4.0).unwrap().def.compact_descr.id, 1);
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 14.0 }));
        assert!(gun.take_loaded(14.0));
        assert_eq!(magazine_shells(&gun), [0, 3]);
    }

    #[test]
    fn fills_its_magazine_with_what_is_left() {
        let mut gun = Gun::new(vec![(shell_of(1), 2)], 10.0).with_magazine(magazine(3, 1));
        assert_eq!(magazine_shells(&gun), [2]);
        fire(&mut gun, 0.0).unwrap();
        fire(&mut gun, 2.0).unwrap();
        assert!(fire(&mut gun, 20.0).is_none());
    }

    #[test]
    fn fires_a_burst_then_waits_its_magazine_interval() {
        let mut gun = Gun::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(6, 3));
        let first = fire(&mut gun, 0.0).unwrap();
        assert_eq!(first.burst, Some(3));
        assert_eq!(gun.reload(), None);
        assert!(fire(&mut gun, 0.1).is_none());
        assert!(gun.fire_burst(0.4, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).is_none());
        let second = gun.fire_burst(0.5, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert_eq!((second.burst, second.id), (None, first.id + 1));
        gun.fire_burst(1.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert!(gun.fire_burst(1.5, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).is_none());
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.0, end: 3.0 }));
        assert_eq!(magazine_shells(&gun), [3]);
        // The second burst empties the magazine.
        fire(&mut gun, 3.0).unwrap();
        gun.fire_burst(3.5, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        gun.fire_burst(4.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 14.0 }));
    }

    #[test]
    fn holds_a_burst_without_a_magazine() {
        let mut gun = Gun::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(1, 2));
        assert_eq!(magazine_shells(&gun), [2]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(2));
        gun.fire_burst(0.5, Vec3::ZERO, Vec3::Z, 0.0, None, &[]).unwrap();
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 10.5 }));
    }

    #[test]
    fn reloads_a_partial_magazine() {
        let mut gun = Gun::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1));
        // Full, of the next shell.
        assert!(!gun.reload_magazine(0.0));
        fire(&mut gun, 0.0).unwrap();
        assert!(gun.reload_magazine(1.0));
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 11.0 }));
        // Already reloading whole.
        assert!(!gun.reload_magazine(2.0));
        // Before the tick tells it, the client may fire as soon as it is loaded.
        assert!(fire(&mut gun, 11.0).is_some());
        assert_eq!(magazine_shells(&gun), [2, 0]);
        // A full magazine of another shell than the next.
        let mut gun = Gun::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1));
        gun.select_next(shell_of(2).compact_descr.to_int());
        assert!(gun.reload_magazine(0.0));
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn changes_shells_quickly_only_with_a_full_magazine() {
        let factor = 0.25;
        let mut gun = Gun::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1)).with_quick_change_factor(factor);
        fire(&mut gun, 0.0).unwrap();
        assert!(gun.take_loaded(2.0));
        assert!(gun.change_loaded(2.0, shell_of(2).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 12.0 }));
        assert!(gun.take_loaded(12.0));
        assert_eq!(magazine_shells(&gun), [0, 3]);
        assert!(gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.5, end: 14.5 }));
    }

    #[test]
    fn hits_a_vehicle_in_its_way() {
        let mut gun = Gun::new(vec![(shell(), 1)], 10.0);
        // A plate 3 m in front of the hull's origin, the hull 100 m ahead.
        let plate = |triangles| CollisionModel { groups: vec![ArmorGroup { name: "armor_1".into(), material_kind: 1, triangles }], radius: 5.0, bounds: [Vec3::splat(-5.0), Vec3::splat(5.0)] };
        let (a, b, c, d) = (Vec3::new(-2.0, -1.0, -3.0), Vec3::new(2.0, -1.0, -3.0), Vec3::new(2.0, 1.0, -3.0), Vec3::new(-2.0, 1.0, -3.0));
        let collision = VehicleCollision {
            parts: [plate(vec![]), plate(vec![[a, b, c], [a, c, d]]), plate(vec![]), plate(vec![])],
            hull_position: Vec3::ZERO,
        };
        let hull = Affine3A::from_translation(Vec3::new(0.0, 0.0, 100.0));
        let target = HitTarget { entity_id: 7, collision: Arc::new(collision), transforms: [hull; 4] };
        let shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, 0.0, None, &[target]).unwrap();
        let Landing::Vehicle(7, hit) = shot.landing else { panic!("missed: {:?}", shot.landing) };
        assert_eq!(hit.part, VehiclePart::Hull);
        assert_eq!(hit.material_kind, 1);
        assert_eq!(hit.normal, -Vec3::Z);
        assert!((shot.end.z - 97.0).abs() < 0.01 && hit.local_point.z == -3.0);
    }

    #[test]
    fn dispersion_stays_within_its_angle() {
        let mut rng = rand::thread_rng();
        for _ in 0..1000 {
            let direction = disperse(Vec3::Z, 0.01, &mut rng);
            assert!(direction.angle_between(Vec3::Z) <= 0.01 + 1e-5);
        }
    }

}
