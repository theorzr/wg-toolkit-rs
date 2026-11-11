//! The server driving a vehicle itself: it finds its way to its enemy over the map, see
//! `World::path`, drives it as a client would, with its controls, until close, then
//! stops facing it, keeps its turret on it and says when its gun is on it.
//!
//! ```text
//!   each tick:
//!     1. close to its enemy: it stops and turns to face it
//!     2. else, its way: planned again once it has none, its enemy moved off its end,
//!          or it has been some time on it; points it got close to dropped
//!     3. it drives to the next point: turning toward it, and driving on only when
//!          headed near enough it
//!     4. stuck, driving and not moving: it backs off a little, then plans again
//! ```

use glam::Vec3;

use crate::base::physics::{movement_flag, wrap_angle, Vehicle, VehicleId, World};


/// How close to its enemy the bot drives, in metres on the ground plane.
const ENGAGE_RANGE: f32 = 150.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to the enemy to fire, in radians.
const AIM_TOLERANCE: f32 = 0.02;
/// How close to a point of its way the bot gets before heading for the next, in metres.
/// Our choice: as close as it can turn short of it.
const ARRIVE: f32 = 4.0;
/// How far its enemy may move off its way's end before the bot plans again, in metres.
const REPLAN_DISTANCE: f32 = 25.0;
/// How many ticks the bot keeps a way before planning again anyway: what moved, a
/// vehicle in its way say, is planned around.
const REPLAN_TICKS: u32 = 50;
/// How much clearance the bot keeps past its hull's half width, in metres: a cell's is
/// measured from the middle of the nearest one it can't enter, half a cell off the wall.
const WIDTH_MARGIN: f32 = 1.0;
/// How slow the bot must go, in m/s, driving, for how many ticks, to be stuck.
const STUCK_SPEED: f32 = 0.5;
const STUCK_TICKS: u32 = 20;
/// How many ticks the bot backs off for once stuck.
const BACK_OFF_TICKS: u32 = 15;


/// The server driving a vehicle, as a client would: it sets its controls and its aim.
#[derive(Debug, Clone, Default)]
pub struct Bot {
    /// The points of its way it hasn't reached, the next first.
    way: Vec<Vec3>,
    /// Where its enemy was when it planned its way.
    goal: Option<Vec3>,
    /// How many ticks since it planned its way.
    planned: u32,
    /// How many ticks it has driven without moving.
    stuck: u32,
    /// How many ticks it still backs off for.
    backing: u32,
}

impl Bot {

    /// Set a vehicle's controls and aim for its next step, its turret on `enemy`, if any:
    /// if it `drives`, finding its way to it over its world's map until within
    /// [`ENGAGE_RANGE`], then turning to face it. See the module's documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, enemy: Option<Vec3>, drives: bool) {
        let vehicle = world.vehicle_mut(id);
        if let Some(enemy) = enemy {
            vehicle.turret.track(enemy);
        }
        let flags = match enemy.filter(|_| drives) {
            Some(enemy) => self.controls(world, id, enemy),
            None => 0,
        };
        world.vehicle_mut(id).hull.flags = flags;
    }

    /// The controls driving a vehicle at its enemy, see [`Self::drive`].
    fn controls(&mut self, world: &mut World, id: VehicleId, enemy: Vec3) -> u8 {

        let hull = &world.vehicle(id).hull;
        let (position, yaw) = (hull.position, hull.yaw());

        // 4. Stuck, it backs off first.
        if self.backing > 0 {
            self.backing -= 1;
            return movement_flag::BACKWARD;
        }
        if hull.flags & movement_flag::FORWARD != 0 && hull.speed.abs() < STUCK_SPEED {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_TICKS {
            self.stuck = 0;
            self.backing = BACK_OFF_TICKS;
            self.way.clear();
            return movement_flag::BACKWARD;
        }

        // 1. Close, it stops facing its enemy.
        let to_enemy = (enemy - position).with_y(0.0);
        if to_enemy.length() <= ENGAGE_RANGE {
            self.way.clear();
            return steer(wrap_angle(to_enemy.x.atan2(to_enemy.z) - yaw), false);
        }

        // 2. Its way, planned again as needed, the points it got close to dropped.
        self.planned += 1;
        let moved = self.goal.is_none_or(|goal| goal.distance(enemy) > REPLAN_DISTANCE);
        if self.way.is_empty() || moved || self.planned >= REPLAN_TICKS {
            let [min, max] = hull.mobility.bounds;
            let radius = max.x.max(-min.x) + WIDTH_MARGIN;
            self.way = world.path(position, enemy, radius).unwrap_or_default();
            self.goal = Some(enemy);
            self.planned = 0;
        }
        while self.way.first().is_some_and(|point| (*point - position).with_y(0.0).length() < ARRIVE) {
            self.way.remove(0);
        }

        // 3. It drives to the next point, its enemy's if none.
        let next = self.way.first().copied().unwrap_or(enemy);
        let to = (next - position).with_y(0.0);
        let off = wrap_angle(to.x.atan2(to.z) - yaw);
        steer(off, off.abs() < TURN_WHILE_DRIVING)

    }

    /// Whether a vehicle's gun points at its enemy, to fire.
    pub fn on_target(&self, vehicle: &Vehicle, enemy: Vec3) -> bool {
        let (position, velocity, _) = vehicle.turret.gun_marker(&vehicle.hull);
        velocity.angle_between(enemy - position) < AIM_TOLERANCE
    }

}

/// The controls turning toward a way `off` the heading, in radians, positive to the
/// right, driving forward if `forward`.
fn steer(off: f32, forward: bool) -> u8 {
    let mut flags = 0;
    if off > HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_RIGHT;
    } else if off < -HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_LEFT;
    }
    if forward {
        flags |= movement_flag::FORWARD;
    }
    flags
}


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use glam::Vec2;

    use wgtk::space::Space;

    use crate::base::physics::fixtures::{blocks, chieftain_aiming, chieftain_mobility};
    use super::*;

    /// Drive a bot at its enemy for `ticks`, from `start`, facing +Z.
    fn drive_at(space: Space, start: Vec3, enemy: Vec3, ticks: usize) -> Vec3 {
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(start, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let mut bot = Bot::default();
        for _ in 0..ticks {
            bot.drive(&mut world, id, Some(enemy), true);
            world.step(0.1);
        }
        let hull = &world.vehicle(id).hull;
        let to = (enemy - hull.position).with_y(0.0);
        assert!(wrap_angle(to.x.atan2(to.z) - hull.yaw()).abs() <= HEADING_TOLERANCE + 0.05, "not facing its enemy");
        hull.position
    }

    #[test]
    fn closes_in_then_faces_its_enemy() {
        let space = blocks(&[]);
        let enemy = Vec3::new(120.0, 0.0, 120.0);
        let end = drive_at(space, Vec3::new(-120.0, 0.0, -120.0), enemy, 600);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(distance <= ENGAGE_RANGE && distance > ENGAGE_RANGE - 20.0, "stopped {distance} m away");
    }

    #[test]
    fn drives_around_a_block_in_its_way() {
        // A block 40 m wide and 5 m high across its way: it goes round, and on past it.
        let space = blocks(&[(Vec3::new(-20.0, 0.0, -62.0), Vec3::new(20.0, 5.0, -58.0))]);
        let enemy = Vec3::new(0.0, 0.0, 140.0);
        let end = drive_at(space, Vec3::new(0.0, 0.0, -120.0), enemy, 900);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(end.z > -50.0 && distance <= ENGAGE_RANGE, "stopped at {end:?}, {distance} m away");
    }

}
