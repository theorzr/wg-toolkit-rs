//! A vehicle the server drives itself: it patrols around where it starts, keeps its
//! turret on an enemy and says when its gun is on it.

use glam::Vec3;

use wgtk::space::terrain::Terrain;

use super::gun::Gun;
use super::physics::{movement_flag, wrap_angle, Vehicle};


/// How far from its start the bot patrols, in metres.
const PATROL_RADIUS: f32 = 25.0;
/// How close to a waypoint counts as reached, in metres.
const WAYPOINT_REACHED: f32 = 5.0;
/// How far off its heading a waypoint may be for the bot to turn without stopping,
/// in radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading a waypoint may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to the enemy to fire, in radians.
const AIM_TOLERANCE: f32 = 0.02;


#[derive(Debug, Clone)]
pub struct Bot {
    pub vehicle: Vehicle,
    pub gun: Gun,
    waypoints: [Vec3; 4],
    next: usize,
}

impl Bot {

    /// A bot patrolling the corners of a square around `home`.
    pub fn new(vehicle: Vehicle, gun: Gun, home: Vec3) -> Self {
        let corner = |x: f32, z: f32| home + Vec3::new(x, 0.0, z) * PATROL_RADIUS;
        Self { vehicle, gun, waypoints: [corner(1.0, 0.0), corner(0.0, 1.0), corner(-1.0, 0.0), corner(0.0, -1.0)], next: 0 }
    }

    /// Advance by `dt` seconds toward its waypoint if it `drives`, its turret on
    /// `enemy`. Returns whether its gun points at the enemy.
    pub fn step(&mut self, dt: f32, terrain: Option<&Terrain>, enemy: Vec3, drives: bool) -> bool {

        let hull = &mut self.vehicle.hull;
        let mut to = self.waypoints[self.next] - hull.position;
        to.y = 0.0;
        if to.length() < WAYPOINT_REACHED {
            self.next = (self.next + 1) % self.waypoints.len();
            to = self.waypoints[self.next] - hull.position;
        }
        let off = wrap_angle(to.x.atan2(to.z) - hull.direction.z);
        let mut flags = 0;
        if off > HEADING_TOLERANCE {
            flags |= movement_flag::ROTATE_RIGHT;
        } else if off < -HEADING_TOLERANCE {
            flags |= movement_flag::ROTATE_LEFT;
        }
        if off.abs() < TURN_WHILE_DRIVING {
            flags |= movement_flag::FORWARD;
        }
        hull.flags = if drives { flags } else { 0 };

        self.vehicle.turret.track(enemy);
        self.vehicle.step(dt, terrain);

        let (position, velocity, _) = self.vehicle.turret.gun_marker(&self.vehicle.hull);
        velocity.angle_between(enemy - position) < AIM_TOLERANCE

    }

}


#[cfg(test)]
mod tests {

    use super::*;
    use crate::base::physics::MotionLimits;

    #[test]
    fn drives_to_its_waypoints() {
        let limits = MotionLimits { speed_forward: 10.0, speed_backward: 5.0, rotation_speed: 1.0 };
        let aiming = crate::base::physics::tests::chieftain_aiming();
        let vehicle = Vehicle::new(Vec3::ZERO, Vec3::ZERO, limits, aiming.clone());
        let mut bot = Bot::new(vehicle, Gun::new(Vec::new(), 10.0), Vec3::ZERO);
        let first = bot.waypoints[0];
        let mut closest = f32::MAX;
        for _ in 0..300 {
            bot.step(0.1, None, Vec3::new(0.0, 0.0, 100.0), true);
            closest = closest.min((bot.vehicle.hull.position - first).length());
        }
        assert!(closest < WAYPOINT_REACHED + 1.0, "got within {closest} m");
        assert!(bot.next > 0);
    }

}
