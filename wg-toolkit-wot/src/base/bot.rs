//! The server driving a vehicle itself: it closes in on its enemy, then stops facing
//! it, keeps its turret on it and says when its gun is on it.

use glam::Vec3;

use super::physics::{movement_flag, wrap_angle, Vehicle};


/// How close to its enemy the bot drives, in metres on the ground plane.
const ENGAGE_RANGE: f32 = 150.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to the enemy to fire, in radians.
const AIM_TOLERANCE: f32 = 0.02;


/// The server driving a vehicle, as a client would: it sets its controls and its aim.
#[derive(Debug, Clone, Default)]
pub struct Bot;

impl Bot {

    /// Set a vehicle's controls and aim for its next step, its turret on `enemy`, if any:
    /// if it `drives`, toward it until within [`ENGAGE_RANGE`], then turning to face it.
    pub fn drive(&self, vehicle: &mut Vehicle, enemy: Option<Vec3>, drives: bool) {

        let hull = &mut vehicle.hull;
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
            vehicle.turret.track(enemy);
        }

    }

    /// Whether a vehicle's gun points at its enemy, to fire.
    pub fn on_target(&self, vehicle: &Vehicle, enemy: Vec3) -> bool {
        let (position, velocity, _) = vehicle.turret.gun_marker(&vehicle.hull);
        velocity.angle_between(enemy - position) < AIM_TOLERANCE
    }

}


#[cfg(test)]
mod tests {

    use wgtk::space::Space;

    use super::*;

    #[test]
    fn closes_in_then_faces_its_enemy() {
        let mobility = crate::base::physics::tests::chieftain_mobility();
        let aiming = crate::base::physics::tests::chieftain_aiming();
        let mut vehicle = Vehicle::new(Vec3::ZERO, Vec3::ZERO, mobility, aiming);
        let enemy = Vec3::new(300.0, 0.0, 300.0);
        for _ in 0..1000 {
            Bot.drive(&mut vehicle, Some(enemy), true);
            vehicle.step(0.1, &Space::default(), None);
        }
        let hull = &vehicle.hull;
        let to = enemy - hull.position;
        let distance = to.x.hypot(to.z);
        assert!(distance <= ENGAGE_RANGE && distance > ENGAGE_RANGE - 20.0, "stopped {distance} m away");
        assert!(wrap_angle(to.x.atan2(to.z) - hull.direction.z).abs() <= HEADING_TOLERANCE + 0.05);
    }

}
