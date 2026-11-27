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
//!         ─▶ steer: its turning goal,         ▼  DriveForces::along(v)
//!            from its turning, ramped      a target, and the range of force to push with
//!         ─▶ DriveForces: target, forces,     │
//!            steer                            ▼  drive_tracks
//!                                          a push along and a torque, within its grip
//! ```
//!
//! # Along its heading
//!
//! [`DriveForces::along`] gives, at the hull's speed `v` along its heading, a speed to push it
//! toward and the range of force the push may take, `[low, high]`, in N along its
//! heading. Its body pushes toward the target as hard as that range allows, never past
//! the target in a step: a force bound with a goal, so the engine drives up to the
//! speed asked and holds it there, and the brakes stop it and hold it stopped. The
//! first case that holds, `m` its weight:
//!
//! ```text
//!   case                         target            force range
//!   handbrake (BLOCK_TRACKS)     its pivot, or 0   ± its tracks sliding: SLIDE_GRIP · how hard
//!                                                    they press on the ground (body module);
//!                                                    asked against its way, ± BRAKING · m at
//!                                                    least, its free track braking
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
//! Sliding on it, it swings the way it goes, not the way it is asked: asked to back up
//! as it still slides forward, it swings on as it did, and backs up, swinging back, once
//! it goes backward along its hull, as the real Type 64 did.
//!
//! Driven into a turn on it, it pivots on its blocked inner track: its outer one carries
//! it round at `pivoting = rotation_speed · ROTATION_ON_MOVE`, which moves its middle at
//! `pivot = pivoting · PIVOT_RADIUS`, the target along its heading. Blocked are both its
//! tracks, or, turning, its inner one.
//!
//! # On wheels
//!
//! A wheeled vehicle, its chassis' general wheels known, turns as a car does, on the
//! circle its wheels steer it on, see [`GeneralWheels::curvature`], as the real
//! server's do. Its wheels turn toward the way its keys ask, `Hull::wheels_steer`, and
//! it turns as they have it at once, but swinging up on its handbrake:
//!
//! ```text
//!   wheels → keys · (lock with no way asked, else on_move)
//!            at WHEEL_STEER_SPEED, slower past on_move toward the lock, WHEEL_LOCK_SPEED
//!   steer  = v · GeneralWheels::curvature(wheels)
//! ```
//!
//! Asked to turn with no way, it doesn't turn on the spot: it drives forward at
//! [`CENTER_ROTATION_SPEED`] on its wheels' `lock`, its tightest circle. Driving either
//! way, at any speed, its wheels steer as far as `on_move`, less. Backing up, its wheels
//! steer as its keys ask, its hull turning the other way, as a car's.
//!
//! Turning doesn't slow it, as it does tracks, see [`SPEED_WHILE_TURNING`] and
//! [`TURNING_RESISTANCE`], and its brakes hold harder, [`WHEEL_BRAKING`].
//!
//! A chassis that turns on the spot, `isWheeledOnSpotRotation`, turns as tracks do
//! instead, toward the same goal, as the real server's do: its wheels don't steer it.
//!
//! On its handbrake, all its wheels are blocked, and it doesn't pivot: it slides, and
//! swings as a drift on tracks does, see "Its turning", up to
//! [`WHEEL_HANDBRAKE_ROTATION`] times its turning speed on the spot, less below
//! [`WHEEL_DRIFT_SPEED`], its swing falling as its slide does, at once.
//!
//! # Its tracks
//!
//! Each track has its own speed, its belt's, which the client animates and its engine
//! sound follows. It drives nothing: its body sets it after each step, rolling with the
//! ground under it, at rest blocked, and spinning toward what the drive asks of it,
//! [`DriveForces::track_goal`], where it slips or touches nothing:
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

use crate::game::item_defs::GeneralWheels;

use super::{approach, movement_flag, Hull, GRAVITY, REST_SPEED};


/// How many times its engine's power over its weight drives the vehicle, against its
/// ground's rolling friction: `a = POWER_FACTOR · P / (m · v) - μ · g`. Fitted: on the
/// samples driving straight forward from 4 m/s to near the top speed, forward held for
/// 0.4 s before, the acceleration plus gravity along the slope, on the ground under
/// them, gives a median of 1.125 on the T110E4 (559 samples) and 1.137 on the Type 64
/// (1040). Fitting the friction too trades it against the power (1.2 to 1.36 for 1.25 to
/// 1.3 times the friction), so the game's friction is kept.
pub const POWER_FACTOR: f32 = 1.13;
/// The most the engine accelerates the vehicle, in m/s²: its power, over the speed,
/// would be endless from a standstill. Fitted: the drives' errors fall up to 16 and
/// hardly after, though both vehicles start up to about 6 m/s² in the captured ticks, so
/// it makes up for something else.
pub const MAX_ACCELERATION: f32 = 16.0;
/// How hard its brakes slow the vehicle when released, and against its controls or past
/// the speed asked, in m/s², whatever the vehicle and its ground. Measured: released from
/// forward, both vehicles lose 1.2 to 1.6 m/s a tick (the T110E4 9.8, 8.7, 7.1, 5.7; the
/// Type 64 11.6, 10.0, 8.6, 7.0 on medium ground, 17.0, 15.8, 14.3, 12.7 on soft), and
/// reversing slows them as fast. The drives' errors keep falling up to 20, beyond what
/// they show, so make up for something else.
const COASTING: f32 = 14.0;
const BRAKING: f32 = 14.0;
/// How hard its brakes slow a wheeled vehicle, released and against its controls, in
/// m/s², as [`COASTING`] and [`BRAKING`] do tracks: the real EBR 105, released at
/// 19 m/s, is at 3 m/s a second later.
const WHEEL_BRAKING: f32 = 16.0;
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
/// and to fall back, in seconds. Fitted: the median turn the Type 64 starts driving
/// turns at 17, 39 and 42°/s over its first ticks, of 41.9°/s while moving; the drives'
/// errors are least from 0.2 to 0.25 s. Released, the Type 64 stops in about 0.2 s, the
/// T110E4 within two ticks.
const ROTATION_RISE: f32 = 0.2;
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
/// How fast a wheeled vehicle that doesn't turn on the spot drives forward when asked
/// to turn with no way, on its wheels' lock, in m/s: the real EBR 105's, its
/// `centerRotationFwdSpeed`, which the client's definitions don't have.
const CENTER_ROTATION_SPEED: f32 = 5.6;
/// How fast its general wheels turn, the one that turns it the tightest, in rad/s: as
/// far as they steer driving, and back, and slower past it, toward their lock, as the
/// real EBR 105's do.
const WHEEL_STEER_SPEED: f32 = 55.0f32.to_radians();
const WHEEL_LOCK_SPEED: f32 = 21.0f32.to_radians();
/// How fast a wheeled vehicle swings with its wheels blocked, as [`HANDBRAKE_ROTATION`]
/// does on tracks, and how long its swing takes to grow by its turning speed on the
/// spot, as [`HANDBRAKE_ROTATION_RISE`]: as the real EBR 105, which drifts at about
/// 100°/s, swung up in 0.6 s.
const WHEEL_HANDBRAKE_ROTATION: f32 = 3.8;
const WHEEL_HANDBRAKE_ROTATION_RISE: f32 = 0.15;
/// How fast a wheeled vehicle must slide, in m/s, on its handbrake, to swing as fast as
/// [`WHEEL_HANDBRAKE_ROTATION`] says, slower below as the cube of its share of it, and
/// its swing falls with it at once: as the real EBR 105's drifts, which stop swinging
/// as they stop sliding.
const WHEEL_DRIFT_SPEED: f32 = 4.0;
/// How far from its centre the vehicle pivots, in metres, on its blocked inner track as
/// its outer one drives it. Fitted: backing up into a turn on its handbrake, the Type 64
/// settles at 1.8 m/s turning at its 41.9°/s while moving, on medium ground.
const PIVOT_RADIUS: f32 = 2.5;

impl Hull {

    /// The drive model, for `dt` seconds on this ground: how hard its engine and brakes
    /// push it and its steering turns it, see [`DriveForces`]. It moves nothing:
    /// [`Self::integrate`] does, its tracks passing the pushes on as they grip.
    pub fn drive(&mut self, dt: f32, ground: Ground) -> DriveForces {

        // 1. What the flags ask, and how fast it turns on this ground.
        let ground = ground_index(ground);
        let rotation_speed = self.mobility.rotation_speed[ground];
        let controls = self.controls();

        // 2. Its steering, from the turning it has toward its goal, rising or falling. On
        //    wheels that don't turn on the spot, they turn first, and it turns as they
        //    have it at once, but swinging up on its handbrake.
        let car = self.mobility.general_wheels.filter(|wheels| !wheels.turns_on_spot);
        if let Some(wheels) = &car {
            self.steer_wheels(wheels, &controls, dt);
        }
        let (goal, rise) = self.steer_goal(&controls, rotation_speed);
        let turning = self.yaw_rate;
        let growing = goal.abs() > turning.abs() && goal * turning >= 0.0;
        let rate = rotation_speed / if growing { rise } else { ROTATION_FALL };
        self.steer = if car.is_some() && (!controls.handbrake || !growing) { goal } else { approach(turning, goal, rate * dt) };

        // 3. What its engine and its brakes can do, and which tracks are blocked: on its
        //    handbrake, both, or, turning, its inner one.
        let mobility = &self.mobility;
        let mass = mobility.weight;
        let wheeled = mobility.general_wheels.is_some();
        let (coasting, braking) = if wheeled { (WHEEL_BRAKING, WHEEL_BRAKING) } else { (COASTING, BRAKING) };
        let mut blocked = [false; 2];
        if controls.handbrake {
            for side in Side::BOTH {
                blocked[side.index()] = controls.turn() == 0.0 || !controls.tracked || side == controls.inner();
            }
        }

        DriveForces {
            target: if controls.handbrake { controls.pivot(rotation_speed) } else { controls.target },
            top: if controls.target < 0.0 { mobility.speed_backward } else { mobility.speed_forward },
            blocked,
            turning: controls.turning && !wheeled,
            steer: self.steer,
            half_gauge: mobility.carrying_point.x,
            power: POWER_FACTOR * mobility.engine_power,
            torque: MAX_ACCELERATION * mass,
            coasting: coasting * mass,
            braking: braking * mass,
            overspeed: OVERSPEED_RESIST * mass * GRAVITY,
            resistance: TURNING_RESISTANCE * mass,
            rolling: mobility.rolling_friction[ground],
        }

    }

    /// Its general wheels turning for `dt` seconds toward the way its keys ask, on their
    /// lock with no way asked, else as far as they steer driving: [`WHEEL_STEER_SPEED`],
    /// and slower past how far they steer driving, toward their lock,
    /// [`WHEEL_LOCK_SPEED`], see "On wheels".
    fn steer_wheels(&mut self, wheels: &GeneralWheels, controls: &Controls, dt: f32) {
        let goal = controls.keys * wheels.steering(controls.way == 0.0);
        let angle = self.wheels_steer;
        let past = angle.abs() >= wheels.steering(false);
        let rate = if goal * angle > 0.0 && goal.abs() > angle.abs() && past { WHEEL_LOCK_SPEED } else { WHEEL_STEER_SPEED };
        self.wheels_steer = approach(angle, goal, rate * dt);
    }

    /// What its flags ask of it, see [`Controls`].
    fn controls(&self) -> Controls {

        use movement_flag::*;

        let handbrake = self.flags & BLOCK_TRACKS != 0;
        let keys = match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
            ROTATE_LEFT => -1.0,
            ROTATE_RIGHT => 1.0,
            _ => 0.0,
        };
        // On its handbrake, sliding, it backs up as it goes, whatever it is asked.
        let backing = if handbrake && self.speed.abs() >= ON_SPOT_SPEED {
            self.speed < 0.0
        } else {
            self.flags & BACKWARD != 0
        };
        let way = match self.flags & (FORWARD | BACKWARD) {
            FORWARD => 1.0,
            BACKWARD => -1.0,
            _ => 0.0,
        };

        Controls {
            handbrake,
            tracked: self.mobility.general_wheels.is_none(),
            keys,
            backing,
            way,
            turning: self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0,
            target: if handbrake { 0.0 } else { self.target_speed() },
        }

    }

    /// The speed the flags ask for: the top speed in their direction, reduced by cruise
    /// control, and on tracks by turning. On wheels that don't turn on the spot, turning
    /// with no way asked, [`CENTER_ROTATION_SPEED`] forward, see "On wheels".
    pub fn target_speed(&self) -> f32 {

        use movement_flag::*;

        let turning = self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0;
        let no_way = self.flags & (FORWARD | BACKWARD) == 0;
        if turning && no_way && self.mobility.general_wheels.is_some_and(|wheels| !wheels.turns_on_spot) {
            return CENTER_ROTATION_SPEED.min(self.mobility.speed_forward);
        }

        let share = if self.flags & CRUISE_CONTROL25 != 0 {
            0.25
        } else if self.flags & CRUISE_CONTROL50 != 0 {
            0.5
        } else {
            1.0
        };

        let share = if turning && self.mobility.general_wheels.is_none() { share * SPEED_WHILE_TURNING } else { share };
        if self.flags & FORWARD != 0 {
            self.mobility.speed_forward * share
        } else if self.flags & BACKWARD != 0 {
            -self.mobility.speed_backward * share
        } else {
            0.0
        }

    }

    /// The turning speed its controls ask for, and how long it takes to get there from
    /// rest, in seconds: its ground's turning speed on the spot, less while moving, on
    /// wheels as they turn it, see "On wheels" in the module's documentation; on
    /// its handbrake, faster the faster it slides, see [`HANDBRAKE_ROTATION`].
    fn steer_goal(&self, controls: &Controls, rotation_speed: f32) -> (f32, f32) {

        let Controls { handbrake, target, .. } = *controls;
        let turn = controls.turn();
        let pivot = controls.pivot(rotation_speed);

        if handbrake {
            // Sliding faster than it pivots swings it round faster than it drives.
            let sliding = (self.speed.hypot(self.slide) - pivot.abs()).max(0.0);
            let (share, rotation, rotation_rise) = if self.mobility.general_wheels.is_some() {
                let share = (sliding / WHEEL_DRIFT_SPEED).powi(3).min(1.0);
                (share, WHEEL_HANDBRAKE_ROTATION, WHEEL_HANDBRAKE_ROTATION_RISE)
            } else {
                let share = 1.0 - (-sliding / HANDBRAKE_ROTATION_SPEED).exp();
                (share, HANDBRAKE_ROTATION, HANDBRAKE_ROTATION_RISE)
            };
            let drifting = rotation_speed * rotation * share;
            let pivoting = if pivot != 0.0 { rotation_speed * ROTATION_ON_MOVE } else { 0.0 };
            // A drift swings up only as fast as it slides, a pivot as its outer track drives.
            let swing = (sliding / HANDBRAKE_SWING_SPEED).powi(2).min(1.0);
            let rise = if drifting > pivoting { rotation_rise / swing.max(1e-3) } else { rotation_rise };
            (turn * drifting.max(pivoting), rise)
        } else {
            let moving = self.speed.abs() >= ON_SPOT_SPEED || target != 0.0;
            let goal = turn * rotation_speed * if moving { ROTATION_ON_MOVE } else { 1.0 };
            // On wheels, as their locks have it, unless it turns on the spot as tracks do.
            let Some(wheels) = self.mobility.general_wheels.filter(|wheels| !wheels.turns_on_spot) else {
                return (goal, ROTATION_RISE);
            };
            (self.speed * wheels.curvature(self.wheels_steer), ROTATION_RISE)
        }

    }


    /// Its tracks rolling with it, as if they gripped, at its speed and turning, and its
    /// general wheels steering as it turns: what a hull given its motion from outside
    /// starts with, see `drive_replay.rs`.
    #[cfg(test)]
    pub fn match_tracks(&mut self) {
        let half_gauge = self.mobility.carrying_point.x;
        self.tracks = Side::BOTH.map(|side| self.speed - self.yaw_rate * side.way() * half_gauge);
        self.steer = self.yaw_rate;
        if let Some(wheels) = self.mobility.general_wheels.filter(|_| self.speed.abs() >= ON_SPOT_SPEED) {
            self.wheels_steer = wheels.steering_for(self.yaw_rate / self.speed);
        }
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
#[derive(Debug, Clone, Copy, Default)]
pub struct DriveForces {
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

impl DriveForces {

    /// How its engine or its brakes push it along its heading, going at `speed`: the
    /// speed they push it toward, and the least and the most force, in N, signed along
    /// its heading, by the cases of "Along its heading" in the module's documentation.
    /// On its handbrake, its tracks slide, as hard as `sliding`, in N.
    pub fn along(&self, speed: f32, sliding: f32) -> (f32, f32, f32) {

        let target = self.target;
        let way = target.signum();

        if self.handbrake() {
            // Asked against its way, its free track brakes it, as its blocked ones slide.
            let force = if speed * way < -REST_SPEED { sliding.max(self.braking) } else { sliding };
            return (target, -force, force);
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

    /// Whether its tracks are blocked, either: it is on its handbrake.
    pub fn handbrake(&self) -> bool {
        self.blocked.iter().any(|&blocked| blocked)
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
    /// It has tracks, which it pivots on, on its handbrake: not on wheels.
    tracked: bool,
    /// Which way its keys turn it, -1 left, 1 right, else 0, whichever way it goes.
    keys: f32,
    /// It backs up: asked to, or on its handbrake, sliding backward.
    backing: bool,
    /// Which way it is asked to drive, -1 backward, 1 forward, else 0.
    way: f32,
    /// Either way of turning is asked, even both at once.
    turning: bool,
    /// The speed asked, see [`Hull::target_speed`]; none on its handbrake.
    target: f32,
}

impl Controls {

    /// Which way it turns, -1 left, 1 right, else 0: its keys', reversed backing up,
    /// steering like a car, left swinging its rear to the left.
    fn turn(&self) -> f32 {
        if self.backing { -self.keys } else { self.keys }
    }

    /// On its handbrake, driving into a turn, the speed it pivots at on its blocked inner
    /// track, its outer one carrying it around, in m/s, signed as it drives, at this
    /// turning speed on the spot. On wheels, none: they are all blocked.
    fn pivot(&self, rotation_speed: f32) -> f32 {
        if !self.tracked {
            return 0.0;
        }
        self.way * self.turn().abs() * rotation_speed * ROTATION_ON_MOVE * PIVOT_RADIUS
    }

    /// Its inner track as it turns: turning right, backing up or not, its right one.
    fn inner(&self) -> Side {
        let turn = self.turn();
        if turn * self.way > 0.0 || (self.way == 0.0 && turn > 0.0) { Side::Right } else { Side::Left }
    }

}

/// A ground's index in a vehicle's per-ground mobility, firm, medium then soft.
fn ground_index(ground: Ground) -> usize {
    match ground {
        Ground::Firm => 0,
        Ground::Medium |
        Ground::None => 1,
        Ground::Soft => 2,
    }
}


#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;

    use glam::Vec3;

    use wgtk::space::Space;

    use crate::game::item_defs::MobilityDef;

    use super::super::fixtures::{chieftain, chieftain_mobility, ebr, slope, tick};
    use super::super::{wrap_angle, Rubble, GRAVITY};
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
    fn turns_on_wheels_as_they_steer() {

        // Asked to turn right with no way, it drives forward on its tightest circle, its
        // wheels at their lock, rather than turning on the spot.
        let space = slope(0.0);
        let mut hull = ebr();
        let wheels = hull.mobility.general_wheels.unwrap();
        hull.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!((hull.speed - CENTER_ROTATION_SPEED).abs() < 0.2, "{}", hull.speed);
        let circle = hull.speed * wheels.turning(true);
        assert!((hull.yaw_rate - circle).abs() < 0.03, "{} for {circle}", hull.yaw_rate);

        // Driving, its wheels steer less, at any speed, and it keeps to their line, as on
        // rails, without sliding out.
        hull.flags = movement_flag::FORWARD | movement_flag::ROTATE_RIGHT;
        let mut widest: f32 = 0.0;
        for _ in 0..60 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            widest = widest.max(hull.slide.abs());
        }
        assert!(hull.speed > 10.0, "{}", hull.speed);
        let line = hull.speed * wheels.turning(false);
        assert!((hull.yaw_rate - line).abs() < 0.03, "{} for {line}", hull.yaw_rate);
        assert!(widest < 0.3, "{widest}");

        // Stopped, asked to turn while held still on the handbrake, it doesn't.
        let mut hull = ebr();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.speed.abs() < 0.05 && hull.yaw_rate.abs() < 0.01, "{} m/s, {} rad/s", hull.speed, hull.yaw_rate);

    }

    #[test]
    fn drifts_to_a_stop_on_locked_wheels() {

        // At 18 m/s, on its handbrake, turning, its wheels blocked: it swings round as it
        // slides on, its velocity swinging behind its hull, and stops in about 2 s, turned
        // most of the way to backward, as fast as its firm ground lets it swing.
        let space = slope(0.0);
        let mut hull = ebr();
        hull.speed = 18.0;
        hull.match_tracks();
        let heading = hull.yaw();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::FORWARD | movement_flag::ROTATE_LEFT;
        let mut swing: f32 = 0.0;
        let mut widest: f32 = 0.0;
        let mut stopped = None;
        for tick in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            swing = swing.min(hull.yaw_rate);
            widest = widest.max(hull.slide.abs());
            if stopped.is_none() && hull.speed.hypot(hull.slide) < 0.5 {
                stopped = Some(tick);
            }
        }
        assert_eq!(hull.tracks, [0.0; 2]);
        assert!(swing < -1.4, "{swing}");
        assert!(widest > 2.0, "{widest}");
        assert!(stopped.is_some_and(|tick| (14..=26).contains(&tick)), "{stopped:?}");
        let turned = wrap_angle(heading - hull.yaw());
        assert!((2.2..=3.0).contains(&turned), "{turned}");

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
    fn drifts_round_its_velocity_on_its_handbrake() {
        // At 15 m/s on its handbrake turning left, its hull swings left while it goes on
        // its way: in 2 s, it slides at an angle to its hull, as the real Type 64 did. A Chieftain turning as fast as the
        // Type 64: one slower to turn swings slower than its tracks can turn its way.
        let space = slope(0.0);
        let mobility = MobilityDef {
            rotation_speed: [56f32.to_radians(); 3],
            ..chieftain_mobility()
        };
        let mut hull = Hull::new(Vec3::ZERO, Vec3::ZERO, mobility.clone());
        hull.speed = 15.0;
        hull.match_tracks();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        for _ in 0..20 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let drift = hull.slide.atan2(hull.speed);
        assert!(drift > 15f32.to_radians(), "{}°", drift.to_degrees());
        // Let off, its tracks grip at once, and its slide is gone within half a second.
        hull.flags = movement_flag::FORWARD;
        for _ in 0..5 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.slide.abs() < 0.5, "{} m/s", hull.slide);
        // Asked to back up as it slides on, its free track brakes it, and it swings on the
        // way it did.
        let mut hull = Hull::new(Vec3::ZERO, Vec3::ZERO, mobility);
        hull.speed = 15.0;
        hull.match_tracks();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        for _ in 0..5 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let speed = hull.velocity().length();
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT | movement_flag::BACKWARD;
        for _ in 0..3 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let slowed = (speed - hull.velocity().length()) / 0.3;
        assert!(slowed > 10.0, "{slowed} m/s²");
        assert!(hull.yaw_rate < 0.0, "{}°/s", hull.yaw_rate.to_degrees());
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
