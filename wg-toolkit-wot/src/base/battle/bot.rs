//! The server driving a vehicle as a dev orders it in battle, with `/bot`, see [`Bot`]:
//! it decides nothing itself. Without orders it stands where it spawned, holding fire.
//!
//! Its orders:
//! - where it drives: to a point, or along the battle's route, see [`BotTravel`], the
//!   route traced by a dev, see [`BotRoute`];
//! - how fast it drives at most, if limited;
//! - what it fires at: a spot on a vehicle, which follows the vehicle as it moves and
//!   turns, or a point that stays where it is, see [`BotFire`];
//! - whether it stands still, holding fire, its orders kept.
//!
//! It drives to its goal round its allies, see [`BotAlly`], getting unstuck, see
//! [`BotMotor`].
//!
//! ```text
//!   each tick:
//!     its aim: its fire order's spot, where it will be once a shell fired gets there,
//!       see BotFire::aim
//!     unless stopped, or before the battle's fight: its way to its travel's goal,
//!       stopping within ARRIVE_RANGE of it, see BotTravel::goal, driving forward only
//!       under its speed limit; a vehicle whose turret can't turn all round turning its
//!       hull to its aim
//!
//!   it fires as its fire order says, unless stopped: once its gun is on its aim, see
//!   Turret::aim_error, its shells landing within FIT_RADIUS of it or its aim settled,
//!   see Bot::aimed, and no ally in the way
//! ```

use std::fmt;

use glam::{Vec3, Vec4};

use crate::base::physics::{movement_flag, wrap_angle, Dispersion, Vehicle, VehicleId, World};
use crate::base::debug::{self, DebugChannel, DebugPlace};
use crate::game::collision::VehiclePart;
use crate::base::TICK;


/// How close to its goal the bot gets before stopping, in metres on the ground plane, or
/// heading for the next point of its route: farther than its motor drops the points of
/// its way, see [`WAY_ARRIVE`], not to circle round it. Our choice.
const ARRIVE_RANGE: f32 = 6.0;
/// How close the gun must point to its aim to fire, in radians, see
/// [`Turret::aim_error`](crate::base::physics::Turret::aim_error).
const AIM_TOLERANCE: f32 = 0.002;
/// How far from its aim the bot's shells may stray for it to fire, in metres: about half
/// a hull's height. Our choice.
const FIT_RADIUS: f32 = 1.0;
/// How settled the bot's aim must be, its dispersion of its gun's best, for it to fire at
/// a spot too far for [`FIT_RADIUS`]. Our choice.
const SETTLED_AIM: f32 = 1.1;
/// How many times the bot works out where its spot will be, from where the last time
/// said, see [`BotFire::aim`].
const LEAD_PASSES: u32 = 3;
/// How far a recording dev's vehicle drives before its route gets its next point, in
/// metres on the ground plane. Our choice.
const ROUTE_STEP: f32 = 5.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How near ahead an ally must be for the bot to give way to it, in metres between their
/// centres on the ground plane, and how far off its heading, in radians, see
/// [`gives_way`]. Our choice: a hull's length and some.
const GIVE_WAY_DISTANCE: f32 = 12.0;
const GIVE_WAY_ANGLE: f32 = 35.0f32.to_radians();
/// How fast an ally must drive the bot's way, in m/s, for it to give way: one standing
/// or coming at it is driven round instead.
const GIVE_WAY_SPEED: f32 = 1.0;
/// How close to a point of its way the bot gets before heading for the next, in metres.
/// Our choice: as close as it can turn short of it.
const WAY_ARRIVE: f32 = 4.0;
/// How far its goal may move off its way's end before the bot plans again, in metres.
const REPLAN_DISTANCE: f32 = 25.0;
/// How many ticks the bot keeps a way before planning again anyway: what moved, a
/// vehicle in its way say, is planned around.
const REPLAN_TICKS: u32 = 50;
/// How much clearance the bot keeps past its hull's half width, in metres: a cell's is
/// measured from the middle of the nearest one it can't enter, half a cell off the wall.
const WIDTH_MARGIN: f32 = 1.0;
/// How slow the bot must move, in m/s, driving, for how many ticks, to be stuck: its
/// tracks may turn as it pushes against something.
const STUCK_SPEED: f32 = 0.5;
const STUCK_TICKS: u32 = 20;
/// How many ticks the bot backs off for once stuck.
const BACK_OFF_TICKS: u32 = 15;
/// How the bot is drawn, see [`Bot::debug_draw`]: its orders' text, its way to its goal
/// and its aim, and how high over its hull its text is, in metres.
const DEBUG_TEXT: Vec4 = Vec4::new(255.0, 255.0, 160.0, 255.0);
const DEBUG_GOAL: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);
const DEBUG_AIM: Vec4 = Vec4::new(255.0, 64.0, 64.0, 255.0);
const DEBUG_TEXT_HEIGHT: f32 = 4.0;
/// The colour a route's points and their lines are drawn in, see [`BotRoute::debug_draw`].
const DEBUG_ROUTE: Vec4 = Vec4::new(96.0, 160.0, 255.0, 255.0);


/// The server driving a vehicle as it is ordered, see the module's documentation.
#[derive(Debug, Clone, Default)]
pub struct Bot {
    /// Where it drives, if anywhere.
    travel: Option<BotTravel>,
    /// What it fires at, if anything.
    fire: Option<BotFire>,
    /// Whether it stands still, holding fire, its orders kept.
    stopped: bool,
    /// How fast it drives at most, in m/s, if limited.
    speed: Option<f32>,
    /// How it drives its vehicle.
    motor: BotMotor,
}

impl Bot {

    /// Set a vehicle's controls and aim for its next step, round its `allies`: its turret
    /// on its fire order's spot, and, unless stopped, while `fighting`, on its way to its
    /// travel's goal. See the module's documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, allies: &[BotAlly], fighting: bool) {

        let aim = self.fire.as_mut().map(|fire| fire.aim(world, id));
        if let Some(aim) = aim {
            world.vehicle_mut(id).turret.track(aim);
        }

        let order = if self.stopped || !fighting { None } else { self.order(world.vehicle(id), aim) };
        let flags = match &order {
            Some(order) => self.motor.controls(world, id, order, allies),
            None => 0,
        };
        world.vehicle_mut(id).hull.flags = flags;

        if debug::draws(DebugChannel::Bots) {
            self.debug_draw(world, id, order.as_ref(), aim);
        }

    }

    /// Where its motor drives this vehicle, with its `aim`, if anywhere: to its travel's
    /// goal, see [`BotTravel::goal`], the order done once it has none, else where it is,
    /// to turn its hull to its aim if its turret can't turn all round.
    fn order(&mut self, vehicle: &Vehicle, aim: Option<Vec3>) -> Option<DriveOrder> {

        let position = vehicle.hull.position;
        let face = aim.filter(|_| vehicle.turret.aiming.yaw_limits.is_some());
        let goal = self.travel.as_mut().and_then(|travel| travel.goal(position, ARRIVE_RANGE));
        if goal.is_none() {
            self.travel = None;
        }

        match (goal, face) {
            (None, None) => None,
            (goal, face) => Some(DriveOrder {
                goal: goal.unwrap_or(position),
                range: ARRIVE_RANGE,
                face,
                speed: self.speed,
            }),
        }

    }

    /// Draw its orders, see [`DebugChannel::Bots`]: over its vehicle, what they are; a
    /// line to its goal, and a cross at its aim.
    fn debug_draw(&self, world: &World, id: VehicleId, order: Option<&DriveOrder>, aim: Option<Vec3>) {

        let hull = &world.vehicle(id).hull;
        let text = self.to_string();
        debug::at(DebugPlace::on(id, hull.position, hull.orientation), || {
            debug::text(DebugChannel::Bots, hull.position + Vec3::Y * DEBUG_TEXT_HEIGHT, text, DEBUG_TEXT);
        });

        if let Some(order) = order {
            debug::line(DebugChannel::Bots, hull.position, order.goal, DEBUG_GOAL);
        }
        if let Some(aim) = aim {
            debug::cross(DebugChannel::Bots, aim, DEBUG_AIM);
        }

    }

    // -- Its orders --

    /// Drive as this says, or nowhere.
    pub fn set_travel(&mut self, travel: Option<BotTravel>) {
        self.travel = travel;
    }

    /// Fire as this says, or hold fire.
    pub fn set_fire(&mut self, fire: Option<BotFire>) {
        self.fire = fire;
    }

    /// Drive at most this fast, in m/s, or as fast as it can.
    pub fn set_speed(&mut self, speed: Option<f32>) {
        self.speed = speed;
    }

    /// Stand still holding fire, its orders kept, or carry them out again.
    pub fn set_stopped(&mut self, stopped: bool) {
        self.stopped = stopped;
    }

    /// The vehicle it fires at, if any.
    pub fn target(&self) -> Option<VehicleId> {
        self.fire.as_ref().and_then(|fire| fire.spot.vehicle())
    }

    // -- Its gun --

    /// Whether it fires its vehicle's gun now, unless stopped, as its fire order says,
    /// see [`BotFireMode`]: its gun on its aim, see [`BotFire::aim`], its aim settled,
    /// see [`Self::aimed`], and none of its `allies` in the way.
    pub fn fires(&self, vehicle: &Vehicle, allies: &[BotAlly]) -> bool {

        let Some(fire) = self.fire.as_ref() else { return false };
        if self.stopped || fire.mode == BotFireMode::Hold {
            return false;
        }
        if vehicle.turret.aim_error(&vehicle.hull) >= AIM_TOLERANCE {
            return false;
        }

        let (position, _, dispersion) = vehicle.turret.gun_marker(&vehicle.hull);
        let aim = fire.aimed_at();
        Self::aimed(vehicle, dispersion, aim.distance(position))
            && !allies.iter().any(|ally| ally.in_the_way(position, aim))

    }

    /// Whether its vehicle's aim is settled enough to fire at `distance` metres with this
    /// `dispersion`: its shells landing within [`FIT_RADIUS`] of its aim, or, too far for
    /// that, its dispersion within [`SETTLED_AIM`] of its gun's best.
    fn aimed(vehicle: &Vehicle, dispersion: Dispersion, distance: f32) -> bool {
        let best = vehicle.turret.aiming.dispersion_angle;
        dispersion.angle.tan() * distance <= FIT_RADIUS || dispersion.angle <= best * SETTLED_AIM
    }

    /// Tell it its gun fired: firing once, it holds fire from then on.
    pub fn fired(&mut self) {
        if let Some(fire) = self.fire.as_mut().filter(|fire| fire.mode == BotFireMode::Once) {
            fire.mode = BotFireMode::Hold;
        }
    }

}

impl fmt::Display for Bot {

    /// Its orders: stopped, where it drives, and what it fires at.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        let mut orders = Vec::new();
        if self.stopped {
            orders.push("stopped".to_string());
        }
        if let Some(travel) = &self.travel {
            orders.push(travel.to_string());
        }
        if let Some(speed) = self.speed {
            orders.push(format!("at most {:.0} km/h", speed * 3.6));
        }
        if let Some(fire) = &self.fire {
            orders.push(fire.to_string());
        }
        if orders.is_empty() {
            orders.push("no orders".to_string());
        }
        write!(f, "{}", orders.join(", "))

    }

}

/// Where a bot drives, as ordered: to a point, or along a route.
#[derive(Debug, Clone)]
pub enum BotTravel {
    /// To this point, where it stays.
    Goto(Vec3),
    /// Along these points, from the `next`, back to the first after the last if
    /// `looped`, else done there.
    Follow {
        points: Vec<Vec3>,
        next: usize,
        looped: bool,
    },
}

impl BotTravel {

    /// Where a bot at `position` drives now, none once done: its point, or the next of
    /// its route, past the one it got within `arrive` of, on the ground plane.
    pub fn goal(&mut self, position: Vec3, arrive: f32) -> Option<Vec3> {
        match self {
            Self::Goto(point) => Some(*point),
            Self::Follow { points, next, looped } => {
                let reached = points.get(*next)
                    .is_some_and(|point| (*point - position).with_y(0.0).length() <= arrive);
                if reached {
                    *next += 1;
                    if *looped && *next >= points.len() {
                        *next = 0;
                    }
                }
                points.get(*next).copied()
            }
        }
    }

}

impl fmt::Display for BotTravel {

    /// Where it drives: its point, or how far along its route it is.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Goto(point) => write!(f, "goto {:.0} {:.0}", point.x, point.z),
            Self::Follow { points, next, looped } => {
                let looped = if *looped { ", looped" } else { "" };
                write!(f, "follow {}/{}{looped}", next + 1, points.len())
            }
        }
    }

}

/// What a bot fires at, as ordered, and when.
#[derive(Debug, Clone, Copy)]
pub struct BotFire {
    pub spot: BotSpot,
    /// When the bot fires at it.
    pub mode: BotFireMode,
    /// Where the bot aims, see [`Self::aim`].
    aim: Vec3,
}

/// Where a bot fires, see [`BotFire`].
#[derive(Debug, Clone, Copy)]
pub enum BotSpot {
    /// A spot on a part of a vehicle, which moves and turns with it.
    Vehicle {
        /// The vehicle, in the world.
        vehicle: VehicleId,
        /// Its part the spot is on, and where on it, in the part's frame.
        part: VehiclePart,
        local: Vec3,
        /// Where its hull sits on its chassis, see
        /// [`Vehicle::part_transforms`](crate::base::physics::Vehicle::part_transforms).
        hull_position: Vec3,
    },
    /// A point that stays where it is, on the ground say.
    Point(Vec3),
}

/// When a bot fires at its [`BotFire`] spot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotFireMode {
    /// Each time its gun is loaded and on it.
    Repeat,
    /// Once, then it holds fire.
    Once,
    /// Never: it only aims at it.
    Hold,
}

impl BotFire {

    /// Fire at a spot, as the mode says.
    pub fn new(spot: BotSpot, mode: BotFireMode) -> Self {
        Self { spot, mode, aim: Vec3::ZERO }
    }

    /// Aim where its spot will be once a shell the bot's vehicle of this id fires gets
    /// there: the spot moved on at its velocity for a tick, the battle firing after its
    /// step, then for the shell's flight to there, worked out again from there
    /// [`LEAD_PASSES`] times. The flight is the distance at the shell's speed, as near as
    /// its low arc is to a line. Where it aims.
    pub fn aim(&mut self, world: &World, id: VehicleId) -> Vec3 {

        let (spot, velocity) = self.spot.motion(world);

        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);
        let speed = vehicle.turret.aiming.shot_speed.max(f32::EPSILON);
        self.aim = spot;
        for _ in 0..LEAD_PASSES {
            let flight = TICK.as_secs_f32() + self.aim.distance(eye) / speed;
            self.aim = spot + velocity * flight;
        }
        self.aim

    }

    /// Where the bot aimed last, see [`Self::aim`].
    pub fn aimed_at(&self) -> Vec3 {
        self.aim
    }

}

impl fmt::Display for BotFire {

    /// When it fires, and where.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.mode {
            BotFireMode::Repeat => "fire",
            BotFireMode::Once => "fire once",
            BotFireMode::Hold => "aim",
        };
        write!(f, "{mode} at {}", self.spot)
    }

}

impl BotSpot {

    /// Where it is in the world now, and how fast it moves, in m/s: its vehicle's
    /// velocity, or none for a point.
    pub fn motion(&self, world: &World) -> (Vec3, Vec3) {
        match *self {
            Self::Vehicle { vehicle, part, local, hull_position } => {
                let target = world.vehicle(vehicle);
                let transforms = target.part_transforms(hull_position);
                (transforms[part as usize].transform_point3(local), target.hull.velocity())
            }
            Self::Point(point) => (point, Vec3::ZERO),
        }
    }

    /// The vehicle it is on, if any.
    pub fn vehicle(&self) -> Option<VehicleId> {
        match *self {
            Self::Vehicle { vehicle, .. } => Some(vehicle),
            Self::Point(_) => None,
        }
    }

}

impl fmt::Display for BotSpot {

    /// Its vehicle's part, or the point, on the ground plane.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vehicle { part, .. } => write!(f, "its target's {part:?}"),
            Self::Point(point) => write!(f, "the point {:.0} {:.0}", point.x, point.z),
        }
    }

}

/// The route a battle's bots follow, which a dev traces with `/route`.
#[derive(Debug, Clone, Default)]
pub struct BotRoute {
    /// Its points, in the order bots follow them.
    pub points: Vec<Vec3>,
    /// The participant recording it as it drives, by index, if any, see [`Self::record`].
    pub recorder: Option<usize>,
}

impl BotRoute {

    /// Add where its recorder's vehicle is now, once [`ROUTE_STEP`] from its last point.
    pub fn record(&mut self, position: Vec3) {
        let far = self.points.last()
            .is_none_or(|last| (*last - position).with_y(0.0).length() >= ROUTE_STEP);
        if far {
            self.points.push(position);
        }
    }

    /// Draw it, see [`DebugChannel::Bots`]: a cross at each point, and a line between each
    /// and the next.
    pub fn debug_draw(&self) {
        for point in &self.points {
            debug::cross(DebugChannel::Bots, *point, DEBUG_ROUTE);
        }
        for pair in self.points.windows(2) {
            debug::line(DebugChannel::Bots, pair[0], pair[1], DEBUG_ROUTE);
        }
    }

}

/// An ally of a bot: which it drives round, gives way to, and doesn't fire through.
#[derive(Debug, Clone, Copy)]
pub struct BotAlly {
    /// Where its hull's centre is.
    pub center: Vec3,
    /// How far round its centre a shell hits it, or the bot drives into it: half its
    /// hull's box's diagonal.
    pub radius: f32,
    /// Its velocity on the ground plane, in m/s.
    pub velocity: Vec3,
}

impl BotAlly {

    /// Whether it is in the way from a gun at `from` to `to`: between them, the line
    /// passing within its radius of its centre.
    pub fn in_the_way(&self, from: Vec3, to: Vec3) -> bool {
        let line = to - from;
        let along = (self.center - from).dot(line) / line.length_squared().max(f32::EPSILON);
        along > 0.0 && along < 1.0 && (from + line * along).distance(self.center) < self.radius
    }

}

/// Where a bot's motor drives its vehicle, see [`BotMotor::controls`].
#[derive(Debug, Clone, Copy)]
pub struct DriveOrder {
    /// Where it drives.
    pub goal: Vec3,
    /// How close to its goal it stops, in metres on the ground plane.
    pub range: f32,
    /// What it turns its hull to once there, if anything.
    pub face: Option<Vec3>,
    /// How fast it drives at most, in m/s, if limited.
    pub speed: Option<f32>,
}

impl DriveOrder {

    /// Whether a vehicle at `position` is there: within its range of its goal.
    fn reached(&self, position: Vec3) -> bool {
        (self.goal - position).with_y(0.0).length() <= self.range
    }

}

/// How a bot drives its vehicle, as its [`DriveOrder`] says, see [`Self::controls`].
#[derive(Debug, Clone, Default)]
pub struct BotMotor {
    /// The points of its way it hasn't reached, the next first.
    way: Vec<Vec3>,
    /// Where its goal was when it planned its way.
    goal: Option<Vec3>,
    /// How many ticks since it planned its way.
    planned: u32,
    /// Where its hull was the tick before, which says whether it moves.
    last: Option<Vec3>,
    /// How many ticks it has driven without moving.
    stuck: u32,
    /// How many ticks it still backs off for, and the way it does: back if it was
    /// driving forward, else forward.
    backing: u32,
    escape: u8,
}

impl BotMotor {

    /// The controls driving a vehicle as its `order` says, round its `allies`: on its
    /// way to its goal until within its range of it, driving forward only under its
    /// speed limit, then turning its hull to what it faces, if anything. Limited slower
    /// than [`STUCK_SPEED`], it is stuck under half its limit.
    pub fn controls(&mut self, world: &mut World, id: VehicleId, order: &DriveOrder, allies: &[BotAlly]) -> u8 {

        let hull = &world.vehicle(id).hull;
        let (position, yaw) = (hull.position, hull.yaw());
        let fast = order.speed.is_some_and(|speed| hull.speed.abs() >= speed);
        let goal = order.goal;

        // 4. Stuck, it backs off first, the other way it drove.
        let moved = self.last.replace(position)
            .map_or(f32::INFINITY, |last| last.distance(position) / TICK.as_secs_f32());
        if self.backing > 0 {
            self.backing -= 1;
            return self.escape;
        }
        let driving = hull.flags & (movement_flag::FORWARD | movement_flag::BACKWARD);
        let stuck_speed = order.speed.map_or(STUCK_SPEED, |speed| STUCK_SPEED.min(speed / 2.0));
        if driving != 0 && moved < stuck_speed {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_TICKS {
            self.stuck = 0;
            self.backing = BACK_OFF_TICKS;
            self.escape = if driving & movement_flag::FORWARD != 0 {
                movement_flag::BACKWARD
            } else {
                movement_flag::FORWARD
            };
            self.way.clear();
            return self.escape;
        }

        // 1. Close, it stops, turned to what it faces.
        if order.reached(position) {
            self.way.clear();
            let Some(face) = order.face else { return 0 };
            let to = (face - position).with_y(0.0);
            return steer(wrap_angle(to.x.atan2(to.z) - yaw), false);
        }

        // 2. Its way, round its allies, planned again as needed, the points it got close
        //    to dropped.
        self.planned += 1;
        let moved = self.goal.is_none_or(|planned| planned.distance(goal) > REPLAN_DISTANCE);
        if self.way.is_empty() || moved || self.planned >= REPLAN_TICKS {
            let radius = way_radius(world.vehicle(id));
            let around: Vec<_> = allies.iter()
                .map(|ally| (ally.center, ally.radius))
                .collect();
            self.way = world.path(position, goal, radius, &around).unwrap_or_default();
            self.goal = Some(goal);
            self.planned = 0;
        }
        while self.way.first().is_some_and(|point| (*point - position).with_y(0.0).length() < WAY_ARRIVE) {
            self.way.remove(0);
        }

        // 3. It drives to the next point, its goal if none, but behind an ally its way,
        //    or at its speed limit.
        let next = self.way.first().copied().unwrap_or(goal);
        let to = (next - position).with_y(0.0);
        let off = wrap_angle(to.x.atan2(to.z) - yaw);
        steer(off, off.abs() < TURN_WHILE_DRIVING && !fast && !gives_way(position, yaw, allies))

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

/// How wide a way a vehicle needs, in metres from its middle: its hull's half width and
/// [`WIDTH_MARGIN`].
fn way_radius(vehicle: &Vehicle) -> f32 {
    let [min, max] = vehicle.hull.mobility.bounds;
    max.x.max(-min.x) + WIDTH_MARGIN
}

/// Whether a bot at `position`, heading `yaw`, gives way to one of its `allies`: one
/// driving its way, faster than [`GIVE_WAY_SPEED`], within [`GIVE_WAY_DISTANCE`] ahead of
/// it and [`GIVE_WAY_ANGLE`] of its heading.
fn gives_way(position: Vec3, yaw: f32, allies: &[BotAlly]) -> bool {
    let heading = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    allies.iter().any(|ally| {
        let to = (ally.center - position).with_y(0.0);
        ally.velocity.dot(heading) > GIVE_WAY_SPEED
            && to.length() < GIVE_WAY_DISTANCE
            && wrap_angle(to.x.atan2(to.z) - yaw).abs() < GIVE_WAY_ANGLE
    })
}
