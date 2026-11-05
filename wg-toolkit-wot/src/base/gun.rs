//! A vehicle's gun as the server runs it: its ammunition, the shell loaded, the reload,
//! and each shot's flight until it hits a vehicle, the terrain, or runs out of range.
//! Vehicles are hit on their collision models, without armour yet; destructibles are
//! not hit yet.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use rand::Rng;

use wgtk::space::terrain::Terrain;

use crate::wot::collision::{VehicleCollision, VehicleHit};
use crate::wot::item_defs::ShotDef;


/// The flight is followed in steps of this many seconds.
const FLIGHT_STEP: f32 = 0.005;


#[derive(Debug, Clone)]
pub struct Gun {
    /// Each shell the gun fires, and how many are left, the one loaded among them.
    pub ammo: Vec<(ShotDef, u32)>,
    /// The indices in `ammo` of the shell loaded, and of the shell it loads after.
    loaded: usize,
    next: usize,
    reload_time: f32,
    /// How long changing the shell of a loaded gun takes, as a share of the reload; it
    /// takes a whole reload unless within 0 and 1.
    quick_change_factor: f32,
    /// The reload going on: how long it takes, and the game time it ends at.
    reload: Option<(f32, f64)>,
    next_shot_id: i32,
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

    /// A gun with these shells, loaded with the first one there is, then loading it
    /// again, ready to fire.
    pub fn new(ammo: Vec<(ShotDef, u32)>, reload_time: f32) -> Self {
        let loaded = first_left(&ammo).unwrap_or(0);
        Self { ammo, loaded, next: loaded, reload_time, quick_change_factor: 1.0, reload: None, next_shot_id: 1 }
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

    /// Fire the loaded shell at game time `now` from `start` toward `direction`, off by a
    /// random angle within `dispersion` radians, if the gun is loaded and has a shell
    /// left. It then loads its next shell, or another one once there is none of it left.
    pub fn fire(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: f32, terrain: Option<&Terrain>, vehicles: &[HitTarget]) -> Option<Shot> {

        if !self.is_loaded(now) {
            return None;
        }
        let (def, count) = self.ammo.get_mut(self.loaded).filter(|(_, count)| *count > 0)?;
        *count -= 1;
        let def = def.clone();
        self.start_reload(now, self.reload_time);
        if !self.has_left(self.next) {
            self.next = first_left(&self.ammo).unwrap_or(self.next);
        }
        self.loaded = self.next;

        let id = self.next_shot_id;
        self.next_shot_id += 1;

        let velocity = disperse(direction.normalize(), dispersion, &mut rand::thread_rng()) * def.speed;
        let (end, end_velocity, flight_time, landing) = fly(start, velocity, def.gravity, def.max_distance, terrain, vehicles);
        Some(Shot { id, def, start, velocity, end, end_velocity, flight_time, landing })

    }

    /// Load the shell of this integer descriptor once the gun fires, if it has some
    /// left: `VEHICLE_SETTING.NEXT_SHELLS`. Whether it does.
    pub fn select_next(&mut self, compact_descr: i64) -> bool {
        let Some(index) = self.shell_left(compact_descr) else { return false };
        self.next = index;
        true
    }

    /// Load the shell of this integer descriptor instead of the one loaded, and after it
    /// too, if it has some left: `VEHICLE_SETTING.CURRENT_SHELLS`. The gun then reloads,
    /// in its quick change factor of a reload if it was loaded, as captured. Whether it
    /// reloads.
    pub fn change_loaded(&mut self, now: f64, compact_descr: i64) -> bool {
        let Some(index) = self.shell_left(compact_descr) else { return false };
        self.next = index;
        if index == self.loaded {
            return false;
        }
        self.loaded = index;
        let quick = self.is_loaded(now) && self.quick_change_factor > 0.0 && self.quick_change_factor < 1.0;
        let time = if quick { self.reload_time * self.quick_change_factor } else { self.reload_time };
        self.start_reload(now, time);
        true
    }

    /// The shell loaded, and the shell it loads after.
    pub fn shells(&self) -> (Option<&ShotDef>, Option<&ShotDef>) {
        (self.ammo.get(self.loaded).map(|(def, _)| def), self.ammo.get(self.next).map(|(def, _)| def))
    }

    /// Whether the gun is loaded by game time `now`.
    pub fn is_loaded(&self, now: f64) -> bool {
        self.reload.is_none_or(|(_, end)| now >= end)
    }

    /// Whether the gun got loaded by game time `now` since last asked.
    pub fn take_loaded(&mut self, now: f64) -> bool {
        self.reload.take_if(|&mut (_, end)| now >= end).is_some()
    }

    /// The reload going on: how long it takes, and the game time it ends at.
    pub fn reload(&self) -> Option<(f32, f64)> {
        self.reload
    }

    /// The length of a whole reload, in seconds.
    pub fn reload_time(&self) -> f32 {
        self.reload_time
    }

    fn start_reload(&mut self, now: f64, time: f32) {
        self.reload = Some((time, now + f64::from(time)));
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
        assert_eq!(gun.reload(), Some((2.5, 2.5)));
        // Again while reloading: a whole reload.
        assert!(gun.change_loaded(1.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some((10.0, 11.0)));
        assert!(!gun.is_loaded(10.0));
        assert!(gun.take_loaded(11.0));
        // Already loaded with it.
        assert!(!gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), None);
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
