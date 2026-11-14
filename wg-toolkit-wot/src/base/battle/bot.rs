//! The server driving a vehicle itself: it picks its target among the enemies it sees,
//! finds its way to it over the map, see `World::path`, or to the nearest enemy while it
//! sees none, drives it as a client would, with its controls, until close, then stops
//! facing it, keeps its turret on it and says when it fires: its gun on it, and no ally
//! in the way.
//!
//! ```text
//!   each tick:
//!     0. its target: the one it has while it still sees it, else the nearest it sees,
//!          within VIEW_RANGE and nothing of the map in the way; one it lost sight of
//!          remembered where it last saw it
//!     1. close to its target, or the nearest enemy: it stops and turns to face it
//!     2. else, its way: planned again once it has none, its enemy moved off its end,
//!          or it has been some time on it; points it got close to dropped
//!     3. it drives to the next point: turning toward it, and driving on only when
//!          headed near enough it
//!     4. stuck, driving and not moving: it backs off a little, then plans again
//! ```

use glam::Vec3;

use crate::base::physics::{movement_flag, wrap_angle, Vehicle, VehicleId, World};

use super::VIEW_RANGE;


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
    /// The enemy it aims at, see [`Self::spot`].
    target: Option<Target>,
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

    /// Set a vehicle's controls and aim for its next step, among its `enemies` alive,
    /// each its vehicle and its hull's centre: it picks its target, see [`Self::spot`],
    /// and heads for it while it sees it, else for the nearest enemy, its turret on it;
    /// if it `drives`, finding its way to it over its world's map until within
    /// [`ENGAGE_RANGE`], then turning to face it. See the module's documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, enemies: &[(VehicleId, Vec3)], drives: bool) {

        self.spot(world, id, enemies);

        let position = world.vehicle(id).hull.position;
        let goal = self.sighted().or_else(|| {
            enemies.iter()
                .map(|&(_, at)| at)
                .min_by(|a, b| a.distance_squared(position).total_cmp(&b.distance_squared(position)))
        });
        if let Some(goal) = goal {
            world.vehicle_mut(id).turret.track(goal);
        }

        let flags = match goal.filter(|_| drives) {
            Some(goal) => self.controls(world, id, goal),
            None => 0,
        };
        world.vehicle_mut(id).hull.flags = flags;

    }

    /// Pick its target among its `enemies`, as its gun sees them, see [`in_sight`]: the
    /// one it has while it still sees it, else the nearest it sees. One it no longer
    /// sees stays its target, where it last saw it, until it sees another, and is
    /// forgotten once gone from its enemies, destroyed.
    fn spot(&mut self, world: &World, id: VehicleId, enemies: &[(VehicleId, Vec3)]) {

        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);

        // 1. Its target, forgotten once destroyed, kept while it sees it.
        let current = self.target
            .and_then(|target| enemies.iter().find(|&&(enemy, _)| enemy == target.vehicle))
            .copied();
        if current.is_none() {
            self.target = None;
        }
        if let (Some(target), Some((_, at))) = (&mut self.target, current) {
            target.seen = in_sight(world, eye, at);
            if target.seen {
                target.at = at;
                return;
            }
        }

        // 2. Else the nearest enemy it sees, if any.
        let seen = enemies.iter()
            .filter(|&&(_, at)| in_sight(world, eye, at))
            .min_by(|(_, a), (_, b)| a.distance_squared(eye).total_cmp(&b.distance_squared(eye)));
        if let Some(&(vehicle, at)) = seen {
            self.target = Some(Target {
                vehicle,
                at,
                seen: true,
            });
        }

    }

    /// Where its target is, while it sees it: what it fires at.
    pub fn sighted(&self) -> Option<Vec3> {
        self.target.filter(|target| target.seen).map(|target| target.at)
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

    /// Whether it fires its vehicle's gun now: at its target, which it sees, its gun on
    /// it, and none of its `allies` in the way, each its hull's centre and how far round
    /// it a shell would hit it, see [`in_the_way`].
    pub fn fires(&self, vehicle: &Vehicle, allies: &[(Vec3, f32)]) -> bool {
        let Some(target) = self.sighted() else { return false };
        let (position, velocity, _) = vehicle.turret.gun_marker(&vehicle.hull);
        velocity.angle_between(target - position) < AIM_TOLERANCE
            && !allies.iter().any(|&(center, radius)| in_the_way(position, target, center, radius))
    }

}

/// An enemy a bot aims at, see [`Bot::spot`]: its vehicle, where the bot last saw its
/// hull's centre, and whether it sees it now.
#[derive(Debug, Clone, Copy)]
struct Target {
    /// Its vehicle, in the world.
    vehicle: VehicleId,
    /// Where the bot last saw its hull's centre.
    at: Vec3,
    /// Whether the bot sees it now, which it fires at only then.
    seen: bool,
}

/// Whether a bot's gun at `eye` sees a point: within [`VIEW_RANGE`] on the ground plane,
/// and nothing of the map in the way, see [`World::sees`].
fn in_sight(world: &World, eye: Vec3, at: Vec3) -> bool {
    (at - eye).with_y(0.0).length() <= VIEW_RANGE && world.sees(eye, at)
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

/// Whether something within `radius` of `center` is in the way from a gun at `from` to
/// `to`: between them, the line passing within its radius.
fn in_the_way(from: Vec3, to: Vec3, center: Vec3, radius: f32) -> bool {
    let line = to - from;
    let along = (center - from).dot(line) / line.length_squared().max(f32::EPSILON);
    along > 0.0 && along < 1.0 && (from + line * along).distance(center) < radius
}


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use glam::Vec2;

    use wgtk::space::Space;

    use crate::base::physics::fixtures::{blocks, chieftain_aiming, chieftain_mobility};
    use super::*;

    /// How high an enemy's hull's centre is over the ground, about, in metres.
    const HULL_CENTER: f32 = 1.5;

    /// Drive a bot at its enemy, a vehicle standing at `enemy`, for `ticks`, from
    /// `start`, facing +Z.
    fn drive_at(space: Space, start: Vec3, enemy: Vec3, ticks: usize) -> Vec3 {
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(start, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let other = world.add(Vehicle::new(enemy, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemies = [(other, enemy + Vec3::Y * HULL_CENTER)];
        let mut bot = Bot::default();
        for _ in 0..ticks {
            bot.drive(&mut world, id, &enemies, true);
            world.step(0.1);
        }
        let hull = &world.vehicle(id).hull;
        let to = (enemy - hull.position).with_y(0.0);
        assert!(wrap_angle(to.x.atan2(to.z) - hull.yaw()).abs() <= HEADING_TOLERANCE + 0.05, "not facing its enemy");
        hull.position
    }

    #[test]
    fn targets_the_nearest_enemy_it_sees_and_keeps_it() {

        // A wall 20 m wide and 5 m high 30 m in front of the bot.
        let space = blocks(&[(Vec3::new(-10.0, 0.0, -71.0), Vec3::new(10.0, 5.0, -69.0))]);
        let mut world = World::new(Arc::new(space), None);
        let id = world.add(Vehicle::new(Vec3::new(0.0, 0.0, -100.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let [hidden, seen, nearer] = [-140.0, -130.0, -120.0].map(|x| {
            world.add(Vehicle::new(Vec3::new(x, 0.0, 140.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()))
        });
        let mut bot = Bot::default();
        let behind_wall = Vec3::new(0.0, HULL_CENTER, -40.0);
        let in_the_open = Vec3::new(60.0, HULL_CENTER, 0.0);
        let far = Vec3::new(0.0, HULL_CENTER, 400.0);

        // The nearest is behind the wall, so it targets the one in the open; one past
        // its view range it doesn't see.
        bot.spot(&world, id, &[(hidden, behind_wall), (seen, in_the_open), (nearer, far)]);
        assert_eq!(bot.sighted(), Some(in_the_open));
        // It keeps it while it sees it, a nearer one in the open or not.
        let near_open = Vec3::new(-30.0, HULL_CENTER, -80.0);
        bot.spot(&world, id, &[(seen, in_the_open), (nearer, near_open)]);
        assert_eq!(bot.sighted(), Some(in_the_open));
        // Hidden behind the wall, with no other in sight, it is remembered where it was
        // last seen, but no longer fired at.
        bot.spot(&world, id, &[(seen, behind_wall)]);
        assert_eq!(bot.sighted(), None);
        assert_eq!(bot.target.map(|target| target.at), Some(in_the_open));
        // Another in sight, it targets that one; once destroyed, gone from its enemies,
        // it is forgotten, and none other is in sight.
        bot.spot(&world, id, &[(seen, behind_wall), (nearer, near_open)]);
        assert_eq!(bot.sighted(), Some(near_open));
        bot.spot(&world, id, &[(seen, behind_wall)]);
        assert!(bot.target.is_none());

    }

    #[test]
    fn holds_fire_with_an_ally_in_the_way() {
        let (from, to) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 100.0));
        // In the line, or near enough it.
        assert!(in_the_way(from, to, Vec3::new(0.0, 0.0, 30.0), 3.0));
        assert!(in_the_way(from, to, Vec3::new(2.0, 0.0, 30.0), 3.0));
        // Off it, behind the gun, or past the target.
        assert!(!in_the_way(from, to, Vec3::new(5.0, 0.0, 30.0), 3.0));
        assert!(!in_the_way(from, to, Vec3::new(0.0, 0.0, -2.0), 3.0));
        assert!(!in_the_way(from, to, Vec3::new(0.0, 0.0, 120.0), 3.0));
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
