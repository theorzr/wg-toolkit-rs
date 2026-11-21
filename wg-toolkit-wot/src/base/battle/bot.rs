//! The server driving a vehicle itself: it picks its target among the enemies its team
//! spots, see `Spotting`, finds its way to it over the map, see `World::path`, or, while
//! its team spots none, searches where they were last seen, else their team's start. It
//! drives as a client would, with its controls, until close to its target, then stops
//! facing it, keeps its turret on it, leading it, and says when it fires: it sees it
//! itself, its gun on it, its aim settled, and no ally in the way.
//!
//! ```text
//!   each tick:
//!     0. its target, among those its team spots: the one it has while it sees it
//!          itself, within VIEW_RANGE and nothing of the map in the way, else the
//!          nearest it sees, else the one it has, else the nearest
//!        its aim: where its target will be once a shell fired gets there, see
//!          Bot::lead, while it sees it
//!     1. close to its target: it stops and turns to face it; close to where it
//!          searches, it stops
//!     2. else, its way: planned again once it has none, its goal moved off its end,
//!          or it has been some time on it; points it got close to dropped
//!     3. it drives to the next point: turning toward it, and driving on only when
//!          headed near enough it
//!     4. stuck, driving and not moving: it backs off a little, then plans again
//!
//!   it fires once it sees its target, its gun on its aim, see Turret::aim_error,
//!   its shells landing within FIT_RADIUS of it or its aim settled, see Bot::aimed,
//!   and no ally in the way
//! ```

use glam::Vec3;

use crate::base::physics::{movement_flag, wrap_angle, Dispersion, Vehicle, VehicleId, World};
use crate::base::TICK;

use super::VIEW_RANGE;


/// How close to its enemy the bot drives, in metres on the ground plane.
const ENGAGE_RANGE: f32 = 150.0;
/// How close to where it searches the bot drives, in metres on the ground plane. Our
/// choice: its team gets there within `spotting::MIN_RADIUS`, which ends the search.
const SEARCH_RANGE: f32 = 10.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to its aim to fire, in radians, see
/// [`Turret::aim_error`](crate::base::physics::Turret::aim_error).
const AIM_TOLERANCE: f32 = 0.002;
/// How far from its aim the bot's shells may stray for it to fire, in metres: about half
/// a hull's height. Our choice.
const FIT_RADIUS: f32 = 1.0;
/// How settled the bot's aim must be, its dispersion of its gun's best, for it to fire at
/// a target too far for [`FIT_RADIUS`]. Our choice.
const SETTLED_AIM: f32 = 1.1;
/// How many times the bot works out where its target will be, from where the last time
/// said, see [`Bot::lead`].
const LEAD_PASSES: u32 = 3;
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
    /// Where its goal was when it planned its way.
    goal: Option<Vec3>,
    /// How many ticks since it planned its way.
    planned: u32,
    /// How many ticks it has driven without moving.
    stuck: u32,
    /// How many ticks it still backs off for.
    backing: u32,
}

impl Bot {

    /// Set a vehicle's controls and aim for its next step, among the `enemies` its team
    /// spots, each its vehicle and its hull's centre: it picks its target, see
    /// [`Self::spot`], and heads for it, else for where it `search`es, its turret on it;
    /// if it `drives`, finding its way over its world's map until within
    /// [`ENGAGE_RANGE`] of its target, then turning to face it, or within
    /// [`SEARCH_RANGE`] of where it searches. See the module's documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, enemies: &[(VehicleId, Vec3)], search: Vec3, drives: bool) {

        self.spot(world, id, enemies);
        self.lead(world, id);

        let (goal, range) = match self.target {
            Some(target) => (target.at, ENGAGE_RANGE),
            None => (search, SEARCH_RANGE),
        };
        let aim = self.target.map_or(goal, |target| target.aim);
        world.vehicle_mut(id).turret.track(aim);

        let flags = if drives {
            self.controls(world, id, goal, range)
        } else {
            0
        };
        world.vehicle_mut(id).hull.flags = flags;

    }

    /// Pick its target among the `enemies` its team spots: the one it has while it sees
    /// it itself, see [`in_sight`], else the nearest it sees, else the one it has, else
    /// the nearest. One its team no longer spots, or destroyed, is forgotten.
    fn spot(&mut self, world: &World, id: VehicleId, enemies: &[(VehicleId, Vec3)]) {

        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);

        // 1. Its target, forgotten once gone from the enemies spotted, kept while it
        //    sees it.
        let current = self.target
            .and_then(|target| enemies.iter().find(|&&(enemy, _)| enemy == target.vehicle))
            .copied();
        if current.is_none() {
            self.target = None;
        }
        if let (Some(target), Some((_, at))) = (&mut self.target, current) {
            target.at = at;
            target.aim = at;
            target.seen = in_sight(world, eye, at);
            if target.seen {
                return;
            }
        }

        // 2. Else the nearest it sees, if any.
        let seen = enemies.iter()
            .filter(|&&(_, at)| in_sight(world, eye, at))
            .min_by(|(_, a), (_, b)| a.distance_squared(eye).total_cmp(&b.distance_squared(eye)));
        if let Some(&(vehicle, at)) = seen {
            self.target = Some(Target {
                vehicle,
                at,
                aim: at,
                seen: true,
            });
            return;
        }

        // 3. Else the one it has, or the nearest.
        if self.target.is_none() {
            let nearest = enemies.iter()
                .min_by(|(_, a), (_, b)| a.distance_squared(eye).total_cmp(&b.distance_squared(eye)));
            self.target = nearest.map(|&(vehicle, at)| Target {
                vehicle,
                at,
                aim: at,
                seen: false,
            });
        }

    }

    /// Aim where its target will be once a shell it fires gets there, while it sees it:
    /// its hull's centre moved on at its velocity for a tick, the battle firing after
    /// its step, then for the shell's flight to there, worked out again from there
    /// [`LEAD_PASSES`] times. The flight is the distance at the shell's speed, as near
    /// as its low arc is to a line.
    fn lead(&mut self, world: &World, id: VehicleId) {

        let Some(target) = self.target.as_mut().filter(|target| target.seen) else { return };
        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);
        let speed = vehicle.turret.aiming.shot_speed.max(f32::EPSILON);
        let velocity = world.vehicle(target.vehicle).hull.velocity();

        for _ in 0..LEAD_PASSES {
            let flight = TICK.as_secs_f32() + target.aim.distance(eye) / speed;
            target.aim = target.at + velocity * flight;
        }

    }

    /// Where its target is, while it sees it: what it fires at.
    pub fn sighted(&self) -> Option<Vec3> {
        self.target.filter(|target| target.seen).map(|target| target.at)
    }

    /// The controls driving a vehicle to its goal until within `range` of it, see
    /// [`Self::drive`].
    fn controls(&mut self, world: &mut World, id: VehicleId, goal: Vec3, range: f32) -> u8 {

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

        // 1. Close, it stops facing its goal.
        let to_goal = (goal - position).with_y(0.0);
        if to_goal.length() <= range {
            self.way.clear();
            return steer(wrap_angle(to_goal.x.atan2(to_goal.z) - yaw), false);
        }

        // 2. Its way, planned again as needed, the points it got close to dropped.
        self.planned += 1;
        let moved = self.goal.is_none_or(|planned| planned.distance(goal) > REPLAN_DISTANCE);
        if self.way.is_empty() || moved || self.planned >= REPLAN_TICKS {
            let [min, max] = hull.mobility.bounds;
            let radius = max.x.max(-min.x) + WIDTH_MARGIN;
            self.way = world.path(position, goal, radius).unwrap_or_default();
            self.goal = Some(goal);
            self.planned = 0;
        }
        while self.way.first().is_some_and(|point| (*point - position).with_y(0.0).length() < ARRIVE) {
            self.way.remove(0);
        }

        // 3. It drives to the next point, its goal if none.
        let next = self.way.first().copied().unwrap_or(goal);
        let to = (next - position).with_y(0.0);
        let off = wrap_angle(to.x.atan2(to.z) - yaw);
        steer(off, off.abs() < TURN_WHILE_DRIVING)

    }

    /// Whether it fires its vehicle's gun now: at its target, which it sees, its gun on
    /// its aim, see [`Self::lead`], its aim settled, see [`Self::aimed`], and none of
    /// its `allies` in the way, each its hull's centre and how far round it a shell
    /// would hit it, see [`in_the_way`].
    pub fn fires(&self, vehicle: &Vehicle, allies: &[(Vec3, f32)]) -> bool {

        let Some(target) = self.target.filter(|target| target.seen) else { return false };
        let (position, _, dispersion) = vehicle.turret.gun_marker(&vehicle.hull);
        if vehicle.turret.aim_error(&vehicle.hull) >= AIM_TOLERANCE {
            return false;
        }

        Self::aimed(vehicle, dispersion, target.aim.distance(position))
            && !allies.iter().any(|&(center, radius)| in_the_way(position, target.aim, center, radius))

    }

    /// Whether its vehicle's aim is settled enough to fire at `distance` metres with this
    /// `dispersion`: its shells landing within [`FIT_RADIUS`] of its aim, or, too far for
    /// that, its dispersion within [`SETTLED_AIM`] of its gun's best.
    fn aimed(vehicle: &Vehicle, dispersion: Dispersion, distance: f32) -> bool {
        let best = vehicle.turret.aiming.dispersion_angle;
        dispersion.angle.tan() * distance <= FIT_RADIUS || dispersion.angle <= best * SETTLED_AIM
    }

}

/// An enemy a bot aims at, see [`Bot::spot`]: its vehicle, where its hull's centre is,
/// where the bot aims, and whether the bot sees it itself.
#[derive(Debug, Clone, Copy)]
struct Target {
    /// Its vehicle, in the world.
    vehicle: VehicleId,
    /// Where its hull's centre is, as the bot's team spots it.
    at: Vec3,
    /// Where the bot aims: where it will be once a shell fired gets there while the bot
    /// sees it, see [`Bot::lead`], else where it is.
    aim: Vec3,
    /// Whether the bot sees it itself, which it fires at only then.
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

    use std::f32::consts::FRAC_PI_2;
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
            bot.drive(&mut world, id, &enemies, enemy, true);
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
        // Behind the wall, still spotted, with no other in sight, it stays its target,
        // where its team spots it, but isn't fired at.
        bot.spot(&world, id, &[(seen, behind_wall)]);
        assert_eq!(bot.sighted(), None);
        assert_eq!(bot.target.map(|target| target.at), Some(behind_wall));
        // Another in sight, it targets that one; once gone from those spotted, it is
        // forgotten, and the nearest spotted, which it doesn't see, is its target.
        bot.spot(&world, id, &[(seen, behind_wall), (nearer, near_open)]);
        assert_eq!(bot.sighted(), Some(near_open));
        bot.spot(&world, id, &[(hidden, far), (seen, behind_wall)]);
        assert_eq!(bot.sighted(), None);
        assert_eq!(bot.target.map(|target| target.vehicle), Some(seen));
        // None spotted, it has none.
        bot.spot(&world, id, &[]);
        assert!(bot.target.is_none());

    }

    #[test]
    fn searches_where_its_team_last_saw_an_enemy() {
        let space = blocks(&[]);
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(Vec3::new(-120.0, 0.0, -120.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let search = Vec3::new(100.0, 0.0, 100.0);
        let mut bot = Bot::default();
        for _ in 0..900 {
            bot.drive(&mut world, id, &[], search, true);
            world.step(0.1);
        }
        let distance = (search - world.vehicle(id).hull.position).with_y(0.0).length();
        assert!(distance <= SEARCH_RANGE + 5.0, "stopped {distance} m away");
    }

    #[test]
    fn leads_a_moving_target() {

        // An enemy 300 m ahead, crossing at 10 m/s to the right.
        let mut world = World::new(Arc::new(blocks(&[])), None);
        let id = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemy = world.add(Vehicle::new(
            Vec3::new(0.0, 0.0, 300.0),
            Vec3::new(0.0, 0.0, FRAC_PI_2),
            chieftain_mobility(),
            chieftain_aiming(),
        ));
        world.vehicle_mut(enemy).hull.speed = 10.0;
        let at = Vec3::new(0.0, HULL_CENTER, 300.0);
        let mut bot = Bot::default();
        bot.drive(&mut world, id, &[(enemy, at)], at, false);

        // It aims ahead of it by as far as it goes in a tick and the shell's flight.
        let velocity = world.vehicle(enemy).hull.velocity();
        let target = bot.target.unwrap();
        let (eye, _, _) = world.vehicle(id).turret.gun_marker(&world.vehicle(id).hull);
        let flight = TICK.as_secs_f32() + target.aim.distance(eye) / chieftain_aiming().shot_speed;
        assert!((target.aim - (at + velocity * flight)).length() < 0.01, "{}", target.aim);
        assert!(target.aim.x > 4.0, "{}", target.aim);

    }

    #[test]
    fn waits_for_its_aim_to_settle() {

        // An enemy 300 m off to its right: its turret turns to it, its aim spreading.
        let mut world = World::new(Arc::new(blocks(&[])), None);
        let id = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let at = Vec3::new(300.0, 0.0, 0.0);
        let enemy = world.add(Vehicle::new(at, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemies = [(enemy, at + Vec3::Y * HULL_CENTER)];
        let mut bot = Bot::default();

        let mut on_target = None;
        let mut fired = None;
        for tick in 0..200 {
            bot.drive(&mut world, id, &enemies, at, false);
            world.step(TICK.as_secs_f32());
            let vehicle = world.vehicle(id);
            if on_target.is_none() && vehicle.turret.aim_error(&vehicle.hull) < AIM_TOLERANCE {
                on_target = Some(tick);
            }
            if bot.fires(vehicle, &[]) {
                fired = Some(tick);
                break;
            }
        }

        // On it after turning a quarter, it fires only seconds later, settled.
        let (on_target, fired) = (on_target.unwrap(), fired.unwrap());
        assert!(fired > on_target + 20, "on target at {on_target}, fired at {fired}");
        let (_, _, dispersion) = world.vehicle(id).turret.gun_marker(&world.vehicle(id).hull);
        assert!(dispersion.angle <= chieftain_aiming().dispersion_angle * SETTLED_AIM);

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
