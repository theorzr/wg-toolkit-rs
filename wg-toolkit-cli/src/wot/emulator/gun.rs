//! A vehicle's gun as the server runs it: its ammunition, the shell loaded, the reload,
//! and each shot's flight until it hits the terrain or runs out of range. Armour,
//! vehicles and destructibles are not hit yet.

use glam::Vec3;
use rand::Rng;

use wgtk::space::terrain::Terrain;

use crate::wot::item_defs::ShotDef;


/// The flight is followed in steps of this many seconds.
const FLIGHT_STEP: f32 = 0.005;


#[derive(Debug, Clone)]
pub struct Gun {
    /// Each shell the gun fires, and how many are left.
    pub ammo: Vec<(ShotDef, u32)>,
    /// The index in `ammo` of the shell loaded.
    pub loaded: usize,
    reload_time: f32,
    /// The game time the gun is loaded at.
    pub ready_at: f64,
    /// Whether it is reloading, until [`Self::take_loaded`] tells it is done.
    reloading: bool,
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
    /// Whether it ends in the terrain, rather than at its maximum distance.
    pub hit: bool,
}

impl Gun {

    /// A gun with these shells, loaded with the first one there is, ready to fire.
    pub fn new(ammo: Vec<(ShotDef, u32)>, reload_time: f32) -> Self {
        let loaded = ammo.iter().position(|&(_, count)| count > 0).unwrap_or(0);
        Self { ammo, loaded, reload_time, ready_at: 0.0, reloading: false, next_shot_id: 1 }
    }

    /// Fire the loaded shell at game time `now` from `start` toward `direction`, off by a
    /// random angle within `dispersion` radians, if the gun is loaded and has a shell
    /// left. It then reloads.
    pub fn fire(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: f32, terrain: Option<&Terrain>) -> Option<Shot> {

        if now < self.ready_at {
            return None;
        }
        let (def, count) = self.ammo.get_mut(self.loaded).filter(|(_, count)| *count > 0)?;
        *count -= 1;
        let def = def.clone();
        self.ready_at = now + f64::from(self.reload_time);
        self.reloading = true;

        let id = self.next_shot_id;
        self.next_shot_id += 1;

        let velocity = disperse(direction.normalize(), dispersion, &mut rand::thread_rng()) * def.speed;
        let (end, end_velocity, flight_time, hit) = fly(start, velocity, def.gravity, def.max_distance, terrain);
        Some(Shot { id, def, start, velocity, end, end_velocity, flight_time, hit })

    }

    /// Whether the gun got loaded by game time `now` since last asked.
    pub fn take_loaded(&mut self, now: f64) -> bool {
        let loaded = self.reloading && now >= self.ready_at;
        if loaded {
            self.reloading = false;
        }
        loaded
    }

    /// The reload's length, in seconds.
    pub fn reload_time(&self) -> f32 {
        self.reload_time
    }

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

/// Follow a shell from `start`: where it ends, its velocity and the time then, and
/// whether it hit the terrain before `max_distance`.
fn fly(start: Vec3, velocity: Vec3, gravity: f32, max_distance: f32, terrain: Option<&Terrain>) -> (Vec3, Vec3, f32, bool) {
    let (mut position, mut velocity, mut time) = (start, velocity, 0.0);
    loop {
        let next = position + velocity * FLIGHT_STEP;
        let next_velocity = velocity - Vec3::Y * gravity * FLIGHT_STEP;
        if let Some(ground) = terrain.and_then(|terrain| terrain.height_at(next.x, next.z)) {
            if next.y <= ground {
                // Where the step crosses the ground, assumed flat over it.
                let above = position.y - terrain.and_then(|t| t.height_at(position.x, position.z)).unwrap_or(ground);
                let below = ground - next.y;
                let t = (above / (above + below)).clamp(0.0, 1.0);
                return (position.lerp(next, t), velocity.lerp(next_velocity, t), time + FLIGHT_STEP * t, true);
            }
        }
        if (next - start).length() >= max_distance {
            return (next, next_velocity, time + FLIGHT_STEP, false);
        }
        (position, velocity, time) = (next, next_velocity, time + FLIGHT_STEP);
    }
}


#[cfg(test)]
mod tests {

    use super::*;
    use crate::wot::items::{item_type, IntCompactDescr};

    fn shell() -> ShotDef {
        ShotDef {
            compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id: 1 },
            kind_index: 2,
            caliber: 120.0,
            effects_index: 0,
            speed: 800.0,
            gravity: 9.81,
            max_distance: 720.0,
        }
    }

    #[test]
    fn fires_then_reloads() {
        let mut gun = Gun::new(vec![(shell(), 2)], 10.0);
        let shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, 0.0, None).unwrap();
        assert_eq!(shot.velocity, Vec3::Z * 800.0);
        assert!(!shot.hit);
        assert!((shot.end.length() - 720.0).abs() < 5.0);
        assert!(gun.fire(5.0, Vec3::ZERO, Vec3::Z, 0.0, None).is_none());
        assert!(!gun.take_loaded(5.0));
        assert!(gun.take_loaded(10.0));
        assert!(!gun.take_loaded(10.0));
        assert!(gun.fire(10.0, Vec3::ZERO, Vec3::Z, 0.0, None).is_some());
        // Out of shells.
        assert!(gun.fire(20.0, Vec3::ZERO, Vec3::Z, 0.0, None).is_none());
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
