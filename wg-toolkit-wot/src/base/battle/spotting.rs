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
//!   a team spots what any of its vehicles spots, still for LINGER once none does;
//!   it remembers where it last spotted each enemy, until one of its vehicles gets
//!     within MIN_RADIUS of there, which the enemy left then: what bots search
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
    /// For each team, in [`TEAMS`] order, when and where it last spotted each of the
    /// battle's vehicles, by index: none once searched, or destroyed.
    teams: [Vec<Option<Sighting>>; 2],
}

/// When and where a team last spotted an enemy.
#[derive(Debug, Clone, Copy)]
struct Sighting {
    /// The game time it last did at.
    at: f64,
    /// Where the enemy's hull's centre was then.
    position: Vec3,
}

impl Spotting {

    /// No vehicle spotted yet, of a battle's `vehicles`.
    pub fn new(vehicles: usize) -> Self {
        Self {
            teams: [vec![None; vehicles], vec![None; vehicles]],
        }
    }

    /// At game time `now`, what each team spots of the other's vehicles alive, and
    /// where it searched those it no longer does, see the module's documentation. A
    /// destroyed one is forgotten.
    pub fn update(&mut self, now: f64, vehicles: &[BattleVehicle], world: &World) {

        let eyes = vehicles.iter().map(|vehicle| eye(world, vehicle)).collect::<Vec<_>>();

        for (index, target) in vehicles.iter().enumerate() {

            let enemies = TEAMS.iter().position(|&team| team != target.team).unwrap_or_default();
            if !target.alive() {
                self.teams[enemies][index] = None;
                continue;
            }

            let center = target.hull_center(world);
            let camouflage = target.camouflage(world.vehicle(target.id), now);
            let observers = vehicles.iter()
                .zip(&eyes)
                .filter(|(observer, _)| observer.alive() && observer.team != target.team);
            if observers.clone().any(|(observer, &eye)| spots(world, eye, observer.vision.view_range, center, camouflage)) {
                self.teams[enemies][index] = Some(Sighting {
                    at: now,
                    position: center,
                });
                continue;
            }

            // Lost: searched once one of the team gets close to where it was last seen.
            let Some(sighting) = self.teams[enemies][index] else { continue };
            let searched = now - sighting.at > LINGER
                && observers.clone().any(|(_, &eye)| (sighting.position - eye).with_y(0.0).length() <= MIN_RADIUS);
            if searched {
                self.teams[enemies][index] = None;
            }

        }

    }

    /// Whether `team` spots the battle's `index`th vehicle at game time `now`: it did
    /// within [`LINGER`]. Its own it doesn't, see [`Self::update`].
    pub fn spotted(&self, team: u8, index: usize, now: f64) -> bool {
        let Some(side) = TEAMS.iter().position(|&known| known == team) else { return false };
        self.teams[side][index].is_some_and(|sighting| now - sighting.at <= LINGER)
    }

    /// Where `team` last spotted the enemies it no longer does at game time `now`, and
    /// hasn't searched yet, see [`Self::update`].
    pub fn last_seen(&self, team: u8, now: f64) -> impl Iterator<Item = Vec3> + '_ {
        let side = TEAMS.iter().position(|&known| known == team);
        side.into_iter()
            .flat_map(|side| self.teams[side].iter().flatten())
            .filter(move |sighting| now - sighting.at > LINGER)
            .map(|sighting| sighting.position)
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
