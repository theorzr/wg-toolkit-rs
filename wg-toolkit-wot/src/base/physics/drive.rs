//! The drive model: what a hull's controls ask of its engine, its brakes and its
//! steering, as forces and a turning goal, once a tick ([`Hull::drive`]). It moves
//! nothing: in each step of its body, its tracks pass them on to the ground, as much as
//! they grip it, see the `body` module. Off the ground, on its roof or in the air, they
//! pass nothing.
//!
//! This is how the game's own model works (`physics_shared.py`, whose tuning is the
//! server's): the engine pushes the whole hull along its heading, from its power, and
//! a torque about the vertical, the game's "gimlet", turns it toward a turning speed. It
//! has no speed for each track to reach: a tank's two tracks aren't simulated as each
//! steering the hull.
//!
//! ```text
//!   once a tick, Hull::drive               in each step of its body (body module)
//!
//!   flags ─▶ Controls: the speed asked,    the hull's speed v along its heading
//!            the way it turns, handbrake      │
//!         ─▶ steer: its turning goal,         ▼  Drive::along(v)
//!            from its turning, ramped      a target, and the range of force to push with
//!         ─▶ Drive: target, forces, steer     │
//!                                             ▼  drive_tracks
//!                                          a push along and a torque, within its grip
//! ```
//!
//! # Along its heading
//!
//! [`Drive::along`] gives, at the hull's speed `v` along its heading, a speed to push it
//! toward and the range of force the push may take, `[low, high]`, in N along its
//! heading. Its body pushes toward the target as hard as that range allows, never past
//! the target in a step: a force bound with a goal, so the engine drives up to the
//! speed asked and holds it there, and the brakes stop it and hold it stopped. The
//! first case that holds, `m` its weight:
//!
//! ```text
//!   case                         target            force range
//!   handbrake (BLOCK_TRACKS)     its pivot, or 0   ± its tracks sliding: SLIDE_GRIP · how hard
//!                                                    they press on the ground (body module)
//!   released (no way asked)      0                 ± COASTING · m
//!   going against the way        the speed asked   ± BRAKING · m: through a standstill, then
//!                                                    the engine
//!   faster than the speed asked  the speed asked   ± BRAKING · m, below its top speed;
//!                                                    past it, ± OVERSPEED_RESIST · m · g · over,
//!                                                    over = (|v| − top) / top, BRAKING · m at most
//!   the way asked                the speed asked   from 0 to F, the way asked:
//!
//!     F = min(POWER_FACTOR · P / |v|, MAX_ACCELERATION · m) − (TURNING_RESISTANCE · m, turning)
//! ```
//!
//! so the engine pushes hard from a standstill (the cap), less and less as the speed
//! rises (`power / |v|`), and stalls on a slope too steep for its power. Short of the
//! resistance, `F < 0`, the push is `F`: turning slows a vehicle its engine barely holds.
//! Downhill past its top speed, the engine pushes no more and gravity speeds it up until
//! the resistance holds it, as the game's `overspeedResistBaseFactor` does: a soft
//! limit. Below its top speed, past a speed asked lower (cruise control, turning), the
//! brakes take it down to it.
//!
//! Against the way, the target is the speed asked, not 0: braked to a standstill on a
//! slope, gravity would take the hull back down faster than [`REST_SPEED`] each step,
//! and its brakes would hold it at 0 for good.
//!
//! # Its turning
//!
//! Once a tick, its steering's goal `steer` moves from the turning speed the hull has
//! toward the one the flags ask, `goal`, no faster than a rate:
//!
//! ```text
//!   goal   = turn · rotation_speed(ground) · (ROTATION_ON_MOVE if moving, else 1)
//!   steer  = approach(yaw_rate, goal, rotation_speed / time · dt)
//!   time   = ROTATION_RISE growing the same way, ROTATION_FALL slowing or reversing
//! ```
//!
//! In each step, its body pushes its turning toward `steer` with a torque, within its
//! tracks' grip. Starting from the turning the hull has, rather than from its last
//! goal, `steer` never runs away from it: a hull its tracks can't turn (a slope, a wall,
//! a hit) is steered from where it is, and one a hit spun is brought back no faster
//! than it would stop turning.
//!
//! On its handbrake, it swings the faster the faster it slides, as the real drifts did:
//!
//! ```text
//!   sliding = max(0, |velocity| − |pivot|), its speed past what its pivot carries it at
//!   goal    = turn · max(drift, pivoting)
//!   drift   = rotation_speed · HANDBRAKE_ROTATION · (1 − e^(−sliding / HANDBRAKE_ROTATION_SPEED))
//!   rise    = HANDBRAKE_ROTATION_RISE, over min(1, (sliding / HANDBRAKE_SWING_SPEED)²) drifting
//! ```
//!
//! Driven into a turn on it, it pivots on its blocked inner track: its outer one carries
//! it round at `pivoting = rotation_speed · ROTATION_ON_MOVE`, which moves its middle at
//! `pivot = pivoting · PIVOT_RADIUS`, the target along its heading. Blocked are both its
//! tracks, or, turning, its inner one.
//!
//! # Its tracks
//!
//! Each track has its own speed, its belt's, which the client animates and its engine
//! sound follows. It drives nothing: its body sets it after each step, rolling with the
//! ground under it, at rest blocked, and spinning toward what the drive asks of it,
//! [`Drive::track_goal`], where it slips or touches nothing:
//!
//! ```text
//!   track goal = target − steer · x        x its offset to the right: turning right,
//!                                          its left one is faster
//! ```
//!
//! # Where the numbers come from
//!
//! Each constant says where it comes from: the game's sources, the drives captured from
//! the real server the hull is fitted on (a T110E4, 950 hp for 66 t, on Ruinberg's mostly
//! firm ground, captured with its setup; a stock Type 64, 528 hp for 23 t, on Live Oaks'
//! mostly medium ground), or our own choice.

use wgtk::space::terrain::Ground;

use super::{approach, movement_flag, Hull, GRAVITY, REST_SPEED};


/// How many times its engine's power over its weight drives the vehicle, against its
/// ground's rolling friction: `a = POWER_FACTOR · P / (m · v) - μ · g`. Fitted: on the
/// samples driving straight forward from 4 m/s to near the top speed, forward held for
/// 0.4 s before, the acceleration plus gravity along the slope, on the ground under
/// them, gives a median of 1.125 on the T110E4 (559 samples) and 1.137 on the Type 64
/// (1040). Fitting the friction too trades it against the power (1.2 to 1.36 for 1.25 to
/// 1.3 times the friction), so the game's friction is kept.
const POWER_FACTOR: f32 = 1.13;
/// The most the engine accelerates the vehicle, in m/s²: its power, over the speed,
/// would be endless from a standstill. Fitted: the drives' errors fall up to 16 and
/// hardly after, though both vehicles start up to about 6 m/s² in the captured ticks, so
/// it makes up for something else.
const MAX_ACCELERATION: f32 = 16.0;
/// How hard its brakes slow the vehicle when released, and against its controls or past
/// the speed asked, in m/s², whatever the vehicle and its ground. Measured: released from
/// forward, both vehicles lose 1.2 to 1.6 m/s a tick (the T110E4 9.8, 8.7, 7.1, 5.7; the
/// Type 64 11.6, 10.0, 8.6, 7.0 on medium ground, 17.0, 15.8, 14.3, 12.7 on soft), and
/// reversing slows them as fast. The drives' errors keep falling up to 20, beyond what
/// they show, so make up for something else.
const COASTING: f32 = 14.0;
const BRAKING: f32 = 14.0;
/// How hard the vehicle is held back past its top speed, downhill, as a share of its
/// weight for its share of its top speed it goes over it: at 15% over, 0.075 g. From the
/// game: `overspeedResistBaseFactor`, the default of `physics_shared.py`, over the top
/// speed; that it is a share of the weight is our reading. The real Type 64 ran 15% past
/// its top speed down Live Oaks' slopes.
const OVERSPEED_RESIST: f32 = 0.5;
/// How much turning while moving slows the vehicle on top of its ground's resistance, in
/// m/s². Fitted: the power its acceleration shows while turning falls short of driving
/// straight by about 0.45 to 1 m/s², on both drives and at every speed.
const TURNING_RESISTANCE: f32 = 0.8;
/// The share of its turning speed on the spot it turns at while moving. Fitted: the
/// T110E4 turns at 26.8°/s on the spot and 22.8°/s while moving, medians, and the Type
/// 64 at 49.3°/s and 41.9°/s on medium ground.
const ROTATION_ON_MOVE: f32 = 0.85;
/// The share of its top speed it drives at while turning. Fitted: the 90th percentile
/// speed while turning against straight, 9.0 and 9.9 m/s for the T110E4, 18.1 and
/// 20.0 m/s for the Type 64, 0.91 both.
const SPEED_WHILE_TURNING: f32 = 0.91;
/// Under this speed, in m/s, the vehicle turns as on the spot. Our choice, the split the
/// fit above uses between turning on the spot and moving.
const ON_SPOT_SPEED: f32 = 0.5;
/// How long its steering takes to grow from nothing to its turning speed on the spot,
/// and to fall back, in seconds. Fitted: turning from rest, the T110E4 (26.8°/s) and the
/// Type 64 (49.3°/s) both get there in about 0.3 s; released, the Type 64 stops in about
/// 0.2 s, the T110E4 within two ticks.
const ROTATION_RISE: f32 = 0.3;
const ROTATION_FALL: f32 = 0.2;
/// How fast the vehicle swings with its tracks blocked: this many times its turning
/// speed on the spot, the less it slides, `1 - e^(-v / HANDBRAKE_ROTATION_SPEED)`.
/// Fitted: the Type 64 (49.3°/s on the spot) drifts up to 124°/s, then turns at 121,
/// 115, 101, 90, 74 and 35°/s as it slides at 10, 7.6, 5, 3.8, 2.7 and 1 m/s.
const HANDBRAKE_ROTATION: f32 = 2.5;
const HANDBRAKE_ROTATION_SPEED: f32 = 3.0;
/// How long its turning speed takes to grow by its turning speed on the spot with its
/// tracks blocked, in seconds. Fitted: the Type 64's drifts turn 72°/s faster each
/// second.
const HANDBRAKE_ROTATION_RISE: f32 = 0.7;
/// How fast the vehicle must slide, in m/s, for a drift to swing up as fast as
/// [`HANDBRAKE_ROTATION_RISE`] says, slower below as the square of its share of it: from
/// a crawl, its tracks blocked and turning, it hardly turns, while a drift started fast
/// keeps its swing down to a crawl, see [`HANDBRAKE_ROTATION`]. Fitted: the Type 64's
/// swing grows 1.3 to 1.5 rad/s² whether it slides at 13 or 20 m/s, and it turns 0.07 rad
/// in its first second at 1.85 m/s; the square is our choice between.
const HANDBRAKE_SWING_SPEED: f32 = 6.0;
/// How far from its centre the vehicle pivots, in metres, on its blocked inner track as
/// its outer one drives it. Fitted: backing up into a turn on its handbrake, the Type 64
/// settles at 1.8 m/s turning at its 41.9°/s while moving, on medium ground.
const PIVOT_RADIUS: f32 = 2.5;

impl Hull {

    /// The drive model, for `dt` seconds on this ground: how hard its engine and brakes
    /// push it and its steering turns it, see [`Drive`]. It moves nothing:
    /// [`Self::integrate`] does, its tracks passing the pushes on as they grip.
    pub fn drive(&mut self, dt: f32, ground: Ground) -> Drive {

        // 1. What the flags ask, and how fast it turns on this ground.
        let ground = ground_index(ground);
        let rotation_speed = self.mobility.rotation_speed[ground];
        let controls = self.controls(rotation_speed);

        // 2. Its steering, from the turning it has toward its goal, rising or falling.
        let (goal, rise) = self.steer_goal(&controls, rotation_speed);
        let turning = self.yaw_rate;
        let time = if goal.abs() > turning.abs() && goal * turning >= 0.0 { rise } else { ROTATION_FALL };
        let rate = rotation_speed / time;
        self.steer = approach(turning, goal, rate * dt);

        // 3. What its engine and its brakes can do, and which tracks are blocked: on its
        //    handbrake, both, or, turning, its inner one.
        let mobility = &self.mobility;
        let mass = mobility.weight;
        let mut blocked = [false; 2];
        if controls.handbrake {
            for side in Side::BOTH {
                blocked[side.index()] = controls.turn == 0.0 || side == controls.inner;
            }
        }

        Drive {
            target: if controls.handbrake { controls.pivot } else { controls.target },
            top: if controls.target < 0.0 { mobility.speed_backward } else { mobility.speed_forward },
            blocked,
            turning: controls.turning,
            steer: self.steer,
            half_gauge: mobility.carrying_point.x,
            power: POWER_FACTOR * mobility.engine_power,
            torque: MAX_ACCELERATION * mass,
            coasting: COASTING * mass,
            braking: BRAKING * mass,
            overspeed: OVERSPEED_RESIST * mass * GRAVITY,
            resistance: TURNING_RESISTANCE * mass,
            rolling: mobility.rolling_friction[ground],
        }

    }

    /// What its flags ask of it, see [`Controls`].
    fn controls(&self, rotation_speed: f32) -> Controls {

        use movement_flag::*;

        let handbrake = self.flags & BLOCK_TRACKS != 0;

        let mut turn: f32 = match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
            ROTATE_LEFT => -1.0,
            ROTATE_RIGHT => 1.0,
            _ => 0.0,
        };
        // Backing up steers like a car: left swings the rear to the left.
        if self.flags & BACKWARD != 0 {
            turn = -turn;
        }

        // On its handbrake, driving into a turn pivots it on its blocked inner track,
        // its outer one carrying it around at this speed.
        let drive_way = match self.flags & (FORWARD | BACKWARD) {
            FORWARD => 1.0,
            BACKWARD => -1.0,
            _ => 0.0,
        };
        let pivot = drive_way * turn.abs() * rotation_speed * ROTATION_ON_MOVE * PIVOT_RADIUS;
        // Turning right, backing up or not, its right track is the inner one.
        let inner = if turn * drive_way > 0.0 || (drive_way == 0.0 && turn > 0.0) { Side::Right } else { Side::Left };

        Controls {
            handbrake,
            turn,
            turning: self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0,
            pivot,
            inner,
            target: if handbrake { 0.0 } else { self.target_speed() },
        }

    }

    /// The speed the flags ask for: the top speed in their direction, reduced by cruise
    /// control, and by turning.
    pub fn target_speed(&self) -> f32 {

        use movement_flag::*;

        let share = if self.flags & CRUISE_CONTROL25 != 0 {
            0.25
        } else if self.flags & CRUISE_CONTROL50 != 0 {
            0.5
        } else {
            1.0
        };

        let share = if self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0 { share * SPEED_WHILE_TURNING } else { share };
        if self.flags & FORWARD != 0 {
            self.mobility.speed_forward * share
        } else if self.flags & BACKWARD != 0 {
            -self.mobility.speed_backward * share
        } else {
            0.0
        }

    }

    /// The turning speed its controls ask for, and how long it takes to get there from
    /// rest, in seconds: its ground's turning speed on the spot, less while moving; on
    /// its handbrake, faster the faster it slides, see [`HANDBRAKE_ROTATION`].
    fn steer_goal(&self, controls: &Controls, rotation_speed: f32) -> (f32, f32) {

        let Controls { handbrake, turn, pivot, target, .. } = *controls;

        if handbrake {
            // Sliding faster than it pivots swings it round faster than it drives.
            let sliding = (self.speed.hypot(self.slide) - pivot.abs()).max(0.0);
            let share = 1.0 - (-sliding / HANDBRAKE_ROTATION_SPEED).exp();
            let drifting = rotation_speed * HANDBRAKE_ROTATION * share;
            let pivoting = if pivot != 0.0 { rotation_speed * ROTATION_ON_MOVE } else { 0.0 };
            // A drift swings up only as fast as it slides, a pivot as its outer track drives.
            let swing = (sliding / HANDBRAKE_SWING_SPEED).powi(2).min(1.0);
            let rise = if drifting > pivoting { HANDBRAKE_ROTATION_RISE / swing.max(1e-3) } else { HANDBRAKE_ROTATION_RISE };
            (turn * drifting.max(pivoting), rise)
        } else {
            let moving = self.speed.abs() >= ON_SPOT_SPEED || target != 0.0;
            (turn * rotation_speed * if moving { ROTATION_ON_MOVE } else { 1.0 }, ROTATION_RISE)
        }

    }

    /// Its tracks rolling with it, as if they gripped, at its speed and turning: what a
    /// hull given its motion from outside starts with, see `drive_replay.rs`.
    #[cfg(test)]
    pub fn match_tracks(&mut self) {
        let half_gauge = self.mobility.carrying_point.x;
        self.tracks = Side::BOTH.map(|side| self.speed - self.yaw_rate * side.way() * half_gauge);
        self.steer = self.yaw_rate;
    }

}

/// One of a hull's two tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {

    /// Both, left then right, the order a pair of anything of theirs is in.
    pub const BOTH: [Self; 2] = [Self::Left, Self::Right];

    /// Its place in a pair, left then right.
    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    /// Which way it is from the hull's middle, along its right: -1 or 1.
    pub fn way(self) -> f32 {
        match self {
            Self::Left => -1.0,
            Self::Right => 1.0,
        }
    }

    /// The track a point is over, by its offset to the hull's right.
    pub fn of(x: f32) -> Self {
        if x < 0.0 { Self::Left } else { Self::Right }
    }

}

/// How hard a hull's engine and brakes push it in a tick, and its steering turns it, see
/// [`Hull::drive`]: forces, which its body's tracks pass on to the ground as they grip
/// it, see [`Self::along`].
#[derive(Debug, Clone, Copy)]
pub struct Drive {
    /// The speed asked along its heading, in m/s, signed: on its handbrake, the speed it
    /// pivots at, else none.
    pub target: f32,
    /// Its top speed the way asked, in m/s, past which it is held back softly.
    pub top: f32,
    /// Each track is blocked, on its handbrake: it slides along the ground rather than
    /// rolls.
    pub blocked: [bool; 2],
    /// Turning is asked, which slows it, see [`TURNING_RESISTANCE`].
    pub turning: bool,
    /// The turning speed its steering pushes it toward, in rad/s, positive to the right.
    pub steer: f32,
    /// How far each track is from its middle, in metres.
    pub half_gauge: f32,
    /// Its engine's power, in W, and the most force its torque gives, in N.
    pub power: f32,
    pub torque: f32,
    /// The force its brakes hold it with, released, and against its controls or past the
    /// speed asked, in N.
    pub coasting: f32,
    pub braking: f32,
    /// How hard it is held back past its top speed, in N, for its top speed over it.
    pub overspeed: f32,
    /// How hard turning holds it back, in N.
    pub resistance: f32,
    /// The ground's rolling friction, as a share of how hard each track is pressed on it.
    pub rolling: f32,
}

impl Drive {

    /// Whether its tracks are blocked, either: it is on its handbrake.
    pub fn handbrake(&self) -> bool {
        self.blocked.iter().any(|&blocked| blocked)
    }

    /// How its engine or its brakes push it along its heading, going at `speed`: the
    /// speed they push it toward, and the least and the most force, in N, signed along
    /// its heading, by the cases of "Along its heading" in the module's documentation.
    /// On its handbrake, its tracks slide, as hard as `sliding`, in N.
    pub fn along(&self, speed: f32, sliding: f32) -> (f32, f32, f32) {

        let target = self.target;
        let way = target.signum();

        if self.handbrake() {
            return (target, -sliding, sliding);
        }
        if target == 0.0 {
            return (0.0, -self.coasting, self.coasting);
        }
        if speed * way < -REST_SPEED {
            // Against its way, not by what its body jitters at rest, it brakes, on through
            // a standstill: on a slope, gravity takes it back faster than that each step.
            return (target, -self.braking, self.braking);
        }

        let along = speed.abs();
        if along > target.abs() {
            // Past what is asked, braked to it; past its top speed, softly.
            let over = (along - self.top).max(0.0) / self.top;
            let force = if target.abs() < self.top { self.braking } else { (self.overspeed * over).min(self.braking) };
            return (target, -force, force);
        }

        // Its engine, less what turning holds it back by: short of it, it slows.
        let resistance = if self.turning { self.resistance } else { 0.0 };
        let force = (self.power / along.max(1e-3)).min(self.torque) - resistance;
        let least = force.min(0.0);
        if way > 0.0 { (target, least, force) } else { (target, -force, -least) }

    }

    /// The speed it asks of a track, in m/s: the speed asked less its steering's share,
    /// none if blocked.
    pub fn track_goal(&self, side: Side) -> f32 {
        if self.blocked[side.index()] {
            return 0.0;
        }
        self.target - self.steer * side.way() * self.half_gauge
    }

}

/// What a hull's flags ask of it, for a tick of the drive model.
#[derive(Debug, Clone, Copy)]
struct Controls {
    /// Its tracks are blocked.
    handbrake: bool,
    /// Which way it turns, -1 left, 1 right, else 0: reversed backing up.
    turn: f32,
    /// Either way of turning is asked, even both at once.
    turning: bool,
    /// On its handbrake, driving into a turn, the speed it pivots at around its blocked
    /// inner track, in m/s, signed as it drives.
    pivot: f32,
    /// Its inner track as it turns.
    inner: Side,
    /// The speed asked, see [`Hull::target_speed`]; none on its handbrake.
    target: f32,
}

/// A ground's index in a vehicle's per-ground mobility, firm, medium then soft.
fn ground_index(ground: Ground) -> usize {
    match ground {
        Ground::Firm => 0,
        Ground::Medium | Ground::None => 1,
        Ground::Soft => 2,
    }
}


#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;

    use glam::Vec3;

    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, chieftain_mobility, slope, tick};
    use super::super::{Rubble, GRAVITY};
    use super::*;

    #[test]
    fn drives_forward_along_heading() {
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(0.0, 0.0, 2.771));
        hull.flags = movement_flag::FORWARD;
        // Quickly at first, its torque's cap, slower as the engine's power spreads over
        // the speed, step by step of its body: the ground's rolling friction holds it back
        // once it rolls, from its second step.
        tick(&mut hull, 0.1, Ground::Firm);
        let first = hull.speed;
        let mobility = &hull.mobility;
        let power = POWER_FACTOR * mobility.engine_power / mobility.weight;
        let resistance = mobility.rolling_friction[0] * GRAVITY;
        let mut expected = 0.0;
        for step in 0..10 {
            let rolling = if step > 0 { resistance } else { 0.0 };
            let rolled = expected - rolling * 0.01;
            expected = rolled + (power / f32::max(rolled, 1e-3)).min(MAX_ACCELERATION) * 0.01;
        }
        assert!((first - expected).abs() < 1e-3, "{first} for {expected}");
        for _ in 0..300 {
            tick(&mut hull, 0.1, Ground::Firm);
        }
        // At top speed after accelerating, heading south-east as captured.
        assert!((hull.speed - 40.0 / 3.6).abs() < 1e-3, "{}", hull.speed);
        // Short of it on soft ground, where its power only holds it at P / (m · μ · g).
        for _ in 0..300 {
            tick(&mut hull, 0.1, Ground::Soft);
        }
        let mobility = &hull.mobility;
        let held = POWER_FACTOR * mobility.engine_power / (mobility.weight * mobility.rolling_friction[2] * GRAVITY);
        assert!((hull.speed - held).abs() < 0.05, "{} for {held}", hull.speed);
        assert!(hull.position.x > 0.0 && hull.position.z < 0.0);
        assert!(hull.position.y.abs() < 1e-4, "{}", hull.position.y);
    }

    #[test]
    fn stops_and_turns() {
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        tick(&mut hull, 1.0, Ground::Firm);
        hull.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            tick(&mut hull, 0.1, Ground::Firm);
        }
        // Stopped, then turning on the spot at full speed, its tracks going opposite ways.
        assert!(hull.speed.abs() < 1e-4, "{}", hull.speed);
        assert!((hull.yaw_rate - 28f32.to_radians()).abs() < 1e-4, "{}", hull.yaw_rate);
        assert!(hull.tracks[0] > 0.0 && hull.tracks[1] < 0.0, "{:?}", hull.tracks);
        // Slower while driving, once at its speed: accelerating, its faster outer track has
        // less of its engine's power, and it turns less.
        hull.flags = movement_flag::FORWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..40 {
            tick(&mut hull, 0.1, Ground::Firm);
        }
        assert!((hull.yaw_rate - 28f32.to_radians() * ROTATION_ON_MOVE).abs() < 1e-4, "{}", hull.yaw_rate);
        // Backing up while steering right turns the hull the other way, soon.
        hull.flags = movement_flag::BACKWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..5 {
            tick(&mut hull, 0.1, Ground::Firm);
        }
        assert!(hull.yaw_rate < 0.0);
        // Standing still with no input, it soon rests.
        hull.flags = 0;
        hull.speed = 0.0;
        hull.yaw_rate = 0.0;
        let space = Space::default();
        let moved: Vec<bool> = (0..10)
            .map(|_| hull.advance(0.1, &space, None, &mut Rubble::default()))
            .collect();
        assert!(!moved[9], "{moved:?}");
    }

    #[test]
    fn slides_on_its_handbrake() {
        // At 10 m/s on flat ground, its tracks blocked, it slides on, slowed by its blocked
        // tracks sliding on the ground, and stops.
        let space = slope(0.0);
        let mut hull = chieftain();
        hull.speed = 10.0;
        hull.match_tracks();
        hull.flags = movement_flag::BLOCK_TRACKS;
        let mut speeds = Vec::new();
        for _ in 0..50 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            speeds.push(hull.speed);
        }
        assert_eq!(hull.tracks, [0.0; 2]);
        // About 3 m/s², its tracks' sliding grip.
        let slowed = (speeds[0] - speeds[10]) / 1.0;
        assert!((2.0..4.5).contains(&slowed), "{slowed} m/s²: {speeds:?}");
        assert!(hull.speed.abs() < 0.1, "{}", hull.speed);
        // Turning left, its left track blocked, its right one driving it round, it swings
        // left.
        let mut hull = chieftain();
        hull.speed = 10.0;
        hull.match_tracks();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        for _ in 0..20 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.yaw() < -5f32.to_radians(), "{}°", hull.yaw().to_degrees());
        assert_eq!(hull.tracks[0], 0.0);
    }

    #[test]
    fn holds_on_slopes_it_grips() {
        // Parked nose down, it holds on 20°, and slides down 35°, its tracks blocked by
        // their brakes but its grip too little.
        assert!(speed_after_3s(20.0, 0).abs() < 0.05);
        assert!(speed_after_3s(35.0, 0) > 1.0);
        // Driving up 20°, it crawls as fast as its power holds it against the slope; up
        // 35°, it slides back, its tracks spinning.
        let mobility = chieftain_mobility();
        let angle = 20f32.to_radians();
        let held = POWER_FACTOR * mobility.engine_power / mobility.weight
            / (mobility.rolling_friction[1] * GRAVITY * angle.cos() + GRAVITY * angle.sin());
        let up = speed_after_3s(-20.0, movement_flag::FORWARD);
        assert!((up - held).abs() < 0.2, "{up} for {held}");
        assert!(speed_after_3s(-35.0, movement_flag::FORWARD) < -1.0);
        // Across 35°, its right side up, it slides down to its left.
        let space = slope(35.0);
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(35f32.to_radians(), 0.0, FRAC_PI_2));
        for _ in 0..10 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.slide < -1.0, "{}", hull.slide);
    }

    /// Its speed after 3 s on a slope pitching it this many degrees, on these flags.
    fn speed_after_3s(pitch: f32, flags: u8) -> f32 {
        let space = slope(pitch);
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(0.0, pitch.to_radians(), 0.0));
        hull.flags = flags;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        hull.speed
    }

}
