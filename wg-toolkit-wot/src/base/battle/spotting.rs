//! What each team of a battle spots of the other's vehicles, which its clients see, by
//! the game's rules as they are known: the client doesn't have them.
//!
//! ```text
//!   each tick, for each enemy alive, an observer alive spots it:
//!     - within MIN_RADIUS, always;
//!     - else within MIN_RADIUS + (its view range − MIN_RADIUS) × (1 − its camouflage),
//!         at most VIEW_RANGE, nothing of the map in the way, see `World::sees`;
//!   its camouflage: still or moving, less for a while after it fired, see
//!     `BattleVehicle::camouflage`;
//!   a team spots what any of its vehicles spots, still for LINGER once none does
//! ```

use glam::Vec3;

use crate::base::physics::World;

use super::vehicle::BattleVehicle;
use super::{TEAMS, VIEW_RANGE};


/// How close an enemy is always spotted, in metres on the ground plane, whatever is in
/// the way: `VISIBILITY.MIN_RADIUS`.
const MIN_RADIUS: f32 = 50.0;

/// How long a team still spots an enemy once none of its vehicles does, in seconds. Our
/// choice, about the game's.
const LINGER: f64 = 2.0;


/// See the module's documentation.
#[derive(Debug)]
pub struct Spotting {
    /// For each team, in [`TEAMS`] order, the game time it last spotted each of the
    /// battle's vehicles at, by index.
    teams: [Vec<Option<f64>>; 2],
}

impl Spotting {

    /// No vehicle spotted yet, of a battle's `vehicles`.
    pub fn new(vehicles: usize) -> Self {
        Self {
            teams: [vec![None; vehicles], vec![None; vehicles]],
        }
    }

    /// At game time `now`, what each team spots of the other's vehicles alive, see the
    /// module's documentation.
    pub fn update(&mut self, now: f64, vehicles: &[BattleVehicle], world: &World) {

        let eyes = vehicles.iter().map(|vehicle| eye(world, vehicle)).collect::<Vec<_>>();

        for (index, target) in vehicles.iter().enumerate().filter(|(_, target)| target.alive()) {

            let at = target.hull_center(world);
            let camouflage = target.camouflage(world.vehicle(target.id), now);
            let spotted = vehicles.iter()
                .zip(&eyes)
                .filter(|(observer, _)| observer.alive() && observer.team != target.team)
                .any(|(observer, &eye)| spots(world, eye, observer.vision.view_range, at, camouflage));

            if spotted {
                let enemies = TEAMS.iter().position(|&team| team != target.team).unwrap_or_default();
                self.teams[enemies][index] = Some(now);
            }

        }

    }

    /// Whether `team` spots the battle's `index`th vehicle at game time `now`: it did
    /// within [`LINGER`]. Its own it doesn't, see [`Self::update`].
    pub fn spotted(&self, team: u8, index: usize, now: f64) -> bool {
        let Some(side) = TEAMS.iter().position(|&known| known == team) else { return false };
        self.teams[side][index].is_some_and(|at| now - at <= LINGER)
    }

}

/// Where a vehicle looks from: its gun.
fn eye(world: &World, vehicle: &BattleVehicle) -> Vec3 {
    let driven = world.vehicle(vehicle.id);
    let (position, _, _) = driven.turret.gun_marker(&driven.hull);
    position
}

/// Whether an observer at `eye`, seeing as far as `view_range`, spots a vehicle at `at`
/// of the given camouflage, see the module's documentation.
fn spots(world: &World, eye: Vec3, view_range: f32, at: Vec3, camouflage: f32) -> bool {
    let distance = (at - eye).with_y(0.0).length();
    let range = (MIN_RADIUS + (view_range - MIN_RADIUS) * (1.0 - camouflage)).min(VIEW_RANGE);
    distance <= MIN_RADIUS || (distance <= range && world.sees(eye, at))
}


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use crate::base::physics::fixtures::blocks;
    use super::*;

    #[test]
    fn spots_within_its_range_by_camouflage_and_close_behind_walls() {

        // A wall 20 m wide and 5 m high 30 m in front of the observer.
        let space = blocks(&[(Vec3::new(-10.0, 0.0, -71.0), Vec3::new(10.0, 5.0, -69.0))]);
        let world = World::new(Arc::new(space), None);
        let eye = Vec3::new(0.0, 2.0, -100.0);

        // Along the wall, in the open: seeing 400 m, a camouflage of 0.5 is seen within
        // 225 m, none within 400 m, and nothing beyond 445 m.
        assert!(spots(&world, eye, 400.0, Vec3::new(220.0, 1.5, -100.0), 0.5));
        assert!(!spots(&world, eye, 400.0, Vec3::new(240.0, 1.5, -100.0), 0.5));
        assert!(spots(&world, eye, 400.0, Vec3::new(390.0, 1.5, -100.0), 0.0));
        assert!(!spots(&world, eye, 500.0, Vec3::new(450.0, 1.5, -100.0), 0.0));

        // Behind the wall, only within 50 m.
        assert!(spots(&world, eye, 400.0, Vec3::new(0.0, 1.5, -55.0), 0.0));
        assert!(!spots(&world, eye, 400.0, Vec3::new(0.0, 1.5, -40.0), 0.0));

    }

}
