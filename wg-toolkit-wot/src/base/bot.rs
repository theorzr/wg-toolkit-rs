//! A vehicle the server drives itself: it closes in on its enemy, then stops facing
//! it, keeps its turret on it and says when its gun is on it.

use glam::{Vec2, Vec3};

use wgtk::space::Space;

use super::physics::{movement_flag, wrap_angle, Vehicle};
use super::gun::Gun;


/// How close to its enemy the bot drives, in metres on the ground plane.
const ENGAGE_RANGE: f32 = 150.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to the enemy to fire, in radians.
const AIM_TOLERANCE: f32 = 0.02;


#[derive(Debug, Clone)]
pub struct Bot {
    pub vehicle: Vehicle,
    pub gun: Gun,
}

impl Bot {

    pub fn new(vehicle: Vehicle, gun: Gun) -> Self {
        Self { vehicle, gun }
    }

    /// Advance by `dt` seconds, its turret on `enemy`, if any: if it `drives`, toward
    /// it until within [`ENGAGE_RANGE`], then turning to face it. Returns whether its
    /// gun points at it.
    pub fn step(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, enemy: Option<Vec3>, drives: bool) -> bool {

        let hull = &mut self.vehicle.hull;
        let mut flags = 0;
        if let (Some(enemy), true) = (enemy, drives) {
            let mut to = enemy - hull.position;
            to.y = 0.0;
            let off = wrap_angle(to.x.atan2(to.z) - hull.direction.z);
            if off > HEADING_TOLERANCE {
                flags |= movement_flag::ROTATE_RIGHT;
            } else if off < -HEADING_TOLERANCE {
                flags |= movement_flag::ROTATE_LEFT;
            }
            if to.length() > ENGAGE_RANGE && off.abs() < TURN_WHILE_DRIVING {
                flags |= movement_flag::FORWARD;
            }
        }
        hull.flags = flags;

        if let Some(enemy) = enemy {
            self.vehicle.turret.track(enemy);
        }
        self.vehicle.step(dt, space, bounds);

        let Some(enemy) = enemy else { return false };
        let (position, velocity, _) = self.vehicle.turret.gun_marker(&self.vehicle.hull);
        velocity.angle_between(enemy - position) < AIM_TOLERANCE

    }

}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn closes_in_then_faces_its_enemy() {
        let mobility = crate::base::physics::tests::chieftain_mobility();
        let aiming = crate::base::physics::tests::chieftain_aiming();
        let vehicle = Vehicle::new(Vec3::ZERO, Vec3::ZERO, mobility, aiming);
        let mut bot = Bot::new(vehicle, Gun::new(Vec::new(), 10.0));
        let enemy = Vec3::new(300.0, 0.0, 300.0);
        for _ in 0..1000 {
            bot.step(0.1, &Space::default(), None, Some(enemy), true);
        }
        let hull = &bot.vehicle.hull;
        let to = enemy - hull.position;
        let distance = to.x.hypot(to.z);
        assert!(distance <= ENGAGE_RANGE && distance > ENGAGE_RANGE - 20.0, "stopped {distance} m away");
        assert!(wrap_angle(to.x.atan2(to.z) - hull.direction.z).abs() <= HEADING_TOLERANCE + 0.05);
    }

}
