//! The drive model: what a hull's tracks drive it to, its speeds on the ground plane and
//! its turning, from its controls, its ground and its slope ([`Hull::drive`]). It doesn't
//! move the hull: its body does, its tracks pushing it toward these speeds where they
//! touch the ground, see the `body` module.
//!
//! A hull has two speeds on the ground plane, along its heading (`speed`) and across it
//! (`slide`), so its velocity is `heading · speed + right · slide`. The tracks hold
//! `slide` at zero (at [`SIDE_FRICTION`]) unless the handbrake is on.
//!
//! # Its speed along its heading
//!
//! It follows the flags, in one of four cases:
//!
//! ```text
//!   handbrake (BLOCK_TRACKS) ──▶ slows toward 0 (or the pivot speed) at HANDBRAKE_FRICTION
//!   no direction asked ───────▶ slows toward 0 at COASTING
//!   going against the way ────▶ slows toward 0 at BRAKING            (then reverses)
//!   going the way asked ──────▶ the engine accelerates it, capped at the target speed:
//!
//!        power      = POWER_FACTOR · P / m
//!        drive      = min( power / |v| , MAX_ACCELERATION ) · grip_along
//!        resistance = μ(ground) · g + (TURNING_RESISTANCE while turning)
//!        a          = drive − resistance + g · sin(pitch) · way
//!        |v|′       = min( |v| + a · dt , target )
//! ```
//!
//! so the engine pushes hard from a standstill (the cap), less and less as the speed
//! rises (`power / |v|`), and stalls on a slope too steep for its power. `grip_along`
//! and `grip_across` fall from 1 to 0.1 as a slope steepens ([`SLOPE_GRIP_ALONG`]): the
//! tracks hold less, and gravity wins.
//!
//! # Its turning
//!
//! The turning speed follows a goal with a rise and a fall time, so it doesn't jump:
//!
//! ```text
//!   goal       = turn · rotation_speed(ground) · (ROTATION_ON_MOVE if moving)
//!   yaw_rate  →  goal   at   rotation_speed / ROTATION_RISE   (or ROTATION_FALL)
//!   spin      →  0      at   what its tracks' friction leaves it (track_friction)
//! ```
//!
//! `spin` is turning that hits gave the hull: unlike `yaw_rate` the tracks don't drive
//! it, they only hold it. To turn, a hull skids its tracks sideways, against the friction
//! that also holds its slide ([`SIDE_FRICTION`], about 1.1 g), which the two share: a
//! hull sliding sideways turns freely, as a rammed one does, and one at rest holds the
//! turn a weak push would give it, see [`Hull::track_friction`].
//!
//! # Where the numbers come from
//!
//! Each constant says where it comes from: the game's sources, the drives captured from
//! the real server the hull is fitted on (a T110E4, 950 hp for 66 t, on Ruinberg's mostly
//! firm ground, captured with its setup; a stock Type 64, 528 hp for 23 t, on Live Oaks'
//! mostly medium ground), or our own choice.

use glam::Vec3Swizzles;

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
/// The most the tracks accelerate the vehicle, in m/s²: the engine's power, over the
/// speed, would be endless from a standstill. Fitted: the drives' errors are lowest from
/// 10 to 14, both vehicles starting up to about 6 m/s² in the captured ticks.
const MAX_ACCELERATION: f32 = 10.0;
/// How fast the vehicle slows down without controls, and brakes against them, in m/s²,
/// whatever the vehicle and its ground. Measured: released from forward, both vehicles
/// lose 1.2 to 1.6 m/s a tick (the T110E4 9.8, 8.7, 7.1, 5.7; the Type 64 11.6, 10.0,
/// 8.6, 7.0 on medium ground, 17.0, 15.8, 14.3, 12.7 on soft), and reversing slows them
/// as fast. The drives' errors keep falling up to 20, beyond what they show, so make up
/// for something else.
const COASTING: f32 = 14.0;
const BRAKING: f32 = 14.0;
/// The share of its turning speed on the spot it turns at while moving. Fitted: the
/// T110E4 turns at 26.8°/s on the spot and 22.8°/s while moving, medians, and the Type
/// 64 at 49.3°/s and 41.9°/s on medium ground.
const ROTATION_ON_MOVE: f32 = 0.85;
/// The share of its top speed it drives at while turning. Fitted: the 90th percentile
/// speed while turning against straight, 9.0 and 9.9 m/s for the T110E4, 18.1 and
/// 20.0 m/s for the Type 64, 0.91 both.
const SPEED_WHILE_TURNING: f32 = 0.91;
/// How much turning while moving slows the vehicle on top of its ground's resistance, in
/// m/s². Fitted: the power its acceleration shows while turning falls short of driving
/// straight by about 0.45 to 1 m/s², on both drives and at every speed.
const TURNING_RESISTANCE: f32 = 0.8;
/// Under this speed, in m/s, the vehicle turns as on the spot. Our choice, the split the
/// fit above uses between turning on the spot and moving.
const ON_SPOT_SPEED: f32 = 0.5;
/// How long its turning speed takes to grow from nothing to its turning speed on the
/// spot, and to fall back, in seconds: the faster it turns, the faster it gets there.
/// Fitted: turning from rest, the T110E4 (26.8°/s) and the Type 64 (49.3°/s) both get
/// there in about 0.3 s; released, the Type 64 stops in about 0.2 s, the T110E4 within
/// two ticks.
const ROTATION_RISE: f32 = 0.3;
const ROTATION_FALL: f32 = 0.2;
/// How fast its blocked tracks slow the vehicle sliding along them, and its tracks
/// sideways, in m/s². Fitted: on the Type 64's two drifts at 20 and 16.7 m/s, its
/// velocity kept in the world as its hull swings under it, slowed by each separately,
/// follows the real one within 0.2 to 0.8 m/s for 2 to 3.5 along, 9.5 to 12 across; a
/// friction on the whole velocity, weighed by the slip angle, fits twice as badly.
const HANDBRAKE_FRICTION: f32 = 3.0;
const SIDE_FRICTION: f32 = 11.0;
/// The share of the torque its tracks would hold, skidding sideways about their middle,
/// that holds a hull against a hit's spin, see [`Hull::track_friction`]. Our choice, by
/// feel: in full, a steady push off its middle never turned the pusher, and one 30 km/h
/// ram in a side turned it 13°; at 0.3, the pusher turns 9° in 5 s and the ram 22°.
const SPIN_GRIP: f32 = 0.3;
/// How fast the vehicle turns with its tracks blocked: this many times its turning speed
/// on the spot, the less it slides, `1 - e^(-v / HANDBRAKE_ROTATION_SPEED)`. Fitted: the
/// Type 64 (49.3°/s on the spot) drifts up to 124°/s, then turns at 121, 115, 101, 90,
/// 74 and 35°/s as it slides at 10, 7.6, 5, 3.8, 2.7 and 1 m/s.
const HANDBRAKE_ROTATION: f32 = 2.5;
const HANDBRAKE_ROTATION_SPEED: f32 = 3.0;
/// How far from its centre the vehicle pivots, in metres, on its blocked inner track as
/// its outer one drives it. Fitted: backing up into a turn on its handbrake, the Type 64
/// settles at 1.8 m/s turning at its 41.9°/s while moving, on medium ground.
const PIVOT_RADIUS: f32 = 2.5;
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
/// How its tracks grip the terrain the steeper it is, along them and across: fully up to
/// the first angle, in degrees, least, the share given, from the second, in between as the
/// cosines go. From the game: `slopeGripLngTerrain` and `slopeGripSdwTerrain`, the
/// defaults of `physics_shared.py`, which the server's vehicle sections may override.
const SLOPE_GRIP_ALONG: (f32, f32, f32) = (27.5, 32.0, 0.1);
const SLOPE_GRIP_ACROSS: (f32, f32, f32) = (24.5, 29.0, 0.1);

impl Hull {

    /// The drive model, for `dt` seconds on this ground: the speeds and turning its
    /// tracks drive it to. It only changes `speed`, `slide`, `yaw_rate` and `spin`:
    /// [`Self::integrate`] moves it, its tracks pushing it toward them as they grip.
    ///
    /// Driving its way, the engine's power accelerates it, as much as it is slow, up to
    /// [`MAX_ACCELERATION`], against the ground's resistance and the slope, up to the
    /// speed the flags ask for. Without controls it stops at [`COASTING`], and against
    /// them it brakes at [`BRAKING`]. It turns at its ground's turning speed on the spot,
    /// less while moving, reaching it in [`ROTATION_RISE`].
    ///
    /// With its tracks blocked, it slides where it was going as its hull swings under it,
    /// slowed by [`HANDBRAKE_FRICTION`] along its tracks and [`SIDE_FRICTION`] across
    /// them, and turns faster the faster it slides, see [`HANDBRAKE_ROTATION`]. Driven
    /// into a turn, it pivots on its blocked inner track, see [`PIVOT_RADIUS`].
    pub fn drive(&mut self, dt: f32, ground: Ground) {

        // 1. What the flags ask, and how fast it turns on this ground.
        let ground = ground_index(ground);
        let rotation_speed = self.mobility.rotation_speed[ground];
        let controls = self.controls(rotation_speed);

        // 2. Its slope: gravity down it, which its tracks hold against as much as they
        //    grip it, and the friction they have left across.
        let slope = self.slope();
        let (slide_friction, spin_friction) = self.track_friction(slope.grip_across);

        // 3. Its slide: down the slope across, its tracks holding it.
        self.slide = approach(self.slide - slope.across * dt, 0.0, slide_friction * dt);

        // 4. Its speed along its heading, in one of the four cases.
        self.speed = self.speed_along(&controls, ground, &slope, dt);

        // 5. Its turning, toward its goal, rising or falling.
        let (goal, rise) = self.turn_goal(&controls, rotation_speed);
        let time = if goal.abs() > self.yaw_rate.abs() && goal * self.yaw_rate >= 0.0 { rise } else { ROTATION_FALL };
        let rate = rotation_speed / time;
        self.yaw_rate = approach(self.yaw_rate, goal, rate * dt);

        // 6. Its tracks hold a spin before it turns: one too weak to skid them doesn't.
        self.spin = approach(self.spin, 0.0, spin_friction * dt);

    }

    /// What its flags ask of its tracks, see [`Controls`].
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

        let target = if handbrake { 0.0 } else { self.target_speed() };

        Controls {
            handbrake,
            turn,
            turning: self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0,
            pivot,
            target,
            way: target.signum(),
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

    /// Its slope, as its tilt says: pitched positively nose down, rolled positively right
    /// side up.
    fn slope(&self) -> Slope {
        let direction = self.direction();
        let roll = direction.x;
        let pitch = direction.y;
        Slope {
            grip_along: grip(pitch, SLOPE_GRIP_ALONG),
            grip_across: grip(roll, SLOPE_GRIP_ACROSS),
            along: GRAVITY * pitch.sin(),
            across: GRAVITY * roll.sin(),
        }
    }

    /// Its speed along its heading after `dt` seconds of its controls on its slope, see
    /// the module's documentation for the four cases.
    fn speed_along(&self, controls: &Controls, ground: usize, slope: &Slope, dt: f32) -> f32 {

        let Controls { handbrake, turning, pivot, target, way, .. } = *controls;
        let grip_along = slope.grip_along;
        let rolled = self.speed + slope.along * dt;

        if handbrake {
            // Its tracks blocked, they slow its slide along them, toward its pivot.
            approach(rolled, pivot, HANDBRAKE_FRICTION * grip_along * dt)
        } else if target == 0.0 {
            // Released, it coasts to a stop.
            approach(rolled, 0.0, COASTING * grip_along * dt)
        } else if self.speed * way < -REST_SPEED {
            // Against its way, not by what its body jitters at rest, it brakes.
            approach(rolled, 0.0, BRAKING * grip_along * dt)
        } else {
            // Along its way: the drive, less the ground's resistance, and the slope.
            let mobility = &self.mobility;
            let along = self.speed.abs();
            let power = POWER_FACTOR * mobility.engine_power / mobility.weight;
            let drive = (power / along.max(1e-3)).min(MAX_ACCELERATION) * grip_along;
            let turning = if turning { TURNING_RESISTANCE } else { 0.0 };
            let resistance = mobility.rolling_friction[ground] * GRAVITY + turning;
            let along = if along > target.abs() {
                approach(along, target.abs(), BRAKING * grip_along * dt)
            } else {
                // Short of power it stalls, unless the slope takes it back down past
                // what its tracks hold.
                let floor = if slope.along * way + BRAKING * grip_along >= 0.0 { 0.0 } else { f32::NEG_INFINITY };
                (along + (drive - resistance + slope.along * way) * dt).clamp(floor, target.abs())
            };
            along * way
        }

    }

    /// The turning speed its controls ask for, and how long it takes to get there from
    /// rest, in seconds: on its handbrake, faster the faster it slides.
    fn turn_goal(&self, controls: &Controls, rotation_speed: f32) -> (f32, f32) {

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

    /// How fast its tracks stop its slide, in m/s², and its spin, in rad/s², as much as
    /// they grip. Each point of its tracks, `z` along them from their middle, skids
    /// sideways at `slide + spin · z` against `friction`, [`SIDE_FRICTION`]: the real
    /// server stops a hull sliding along the map's edge, its tracks free, as fast as one
    /// drifting on its handbrake, see `stops_at_the_map_edge`.
    ///
    /// ```text
    ///   turning point   z₀ = −slide / spin,   r = |z₀| / (l/2)
    ///
    ///   r ≥ 1   all the tracks skid one way:   slide  →  0  at  friction
    ///   ┃▶▶▶▶▶┃                                 spin   free
    ///   r < 1   they skid both ways:           slide  →  0  at  friction · r
    ///   ┃◀◀·▶▶┃                                 spin   →  0  at  spin_max · (1 − r²)
    ///
    ///   spin_max = SPIN_GRIP · friction · 3·l / (w² + l²)
    /// ```
    ///
    /// `spin_max` is the torque `μ·m·g·l/4` of tracks of length `l` skidding about their
    /// middle against its yaw inertia `m (w² + l²) / 12` (see `Hull::yaw_inertia`), of
    /// which [`SPIN_GRIP`] holds: about 1.1 rad/s² for a Leopard 1. So a hull that slides
    /// turns freely, as one rammed does, and one at rest holds a weak push's turn a
    /// little.
    fn track_friction(&self, grip_across: f32) -> (f32, f32) {
        let [min, max] = self.mobility.bounds;
        let size = max.xz() - min.xz();
        let half_length = size.y / 2.0;
        let ratio = if self.spin.abs() < 1e-6 { f32::INFINITY } else { self.slide.abs() / (self.spin.abs() * half_length) };
        let friction = SIDE_FRICTION * grip_across;
        let spin_max = SPIN_GRIP * friction * 3.0 * size.y / size.length_squared();
        (friction * ratio.min(1.0), spin_max * (1.0 - ratio * ratio).max(0.0))
    }

    /// Its tracks holding its spin for `time` seconds, as [`Self::drive`] does: what a
    /// hull hit by another keeps of it for the rest of a tick.
    pub fn hold_spin(&mut self, time: f32) {
        let grip_across = grip(self.direction().x, SLOPE_GRIP_ACROSS);
        let (_, spin_friction) = self.track_friction(grip_across);
        self.spin = approach(self.spin, 0.0, spin_friction * time);
    }

}

/// What a hull's flags ask of its tracks, for a tick of the drive model.
#[derive(Debug, Clone, Copy)]
struct Controls {
    /// Its tracks are blocked.
    handbrake: bool,
    /// Which way it turns, -1 left, 1 right, else 0: reversed backing up.
    turn: f32,
    /// Either way of turning is asked, even both at once.
    turning: bool,
    /// On its handbrake, driving into a turn, the speed its outer track carries it at
    /// around its blocked inner one, in m/s, signed as it drives.
    pivot: f32,
    /// The speed asked, see [`Hull::target_speed`]; none on its handbrake.
    target: f32,
    /// The way asked, the target's sign: 1 when none is asked.
    way: f32,
}

/// The slope a hull is on, by its tilt.
#[derive(Debug, Clone, Copy)]
struct Slope {
    /// How much its tracks grip along it and across it, from 1 to the least
    /// [`SLOPE_GRIP_ALONG`] and [`SLOPE_GRIP_ACROSS`] give.
    grip_along: f32,
    grip_across: f32,
    /// Gravity down it along its heading, forward, and across it, to the left, in m/s².
    along: f32,
    across: f32,
}

/// How much the tracks grip on a slope of this angle, in radians, by one of the curves
/// [`SLOPE_GRIP_ALONG`] and [`SLOPE_GRIP_ACROSS`].
fn grip(angle: f32, (full, least, least_grip): (f32, f32, f32)) -> f32 {
    let (full, least) = (full.to_radians().cos(), least.to_radians().cos());
    let cos = angle.cos();
    if cos >= full {
        1.0
    } else if cos <= least {
        least_grip
    } else {
        least_grip + (1.0 - least_grip) * (cos - least) / (full - least)
    }
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

    use glam::Vec3;

    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, chieftain_mobility, tick};
    use super::super::Rubble;
    use super::*;

    #[test]
    fn drives_forward_along_heading() {
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(0.0, 0.0, 2.771));
        hull.flags = movement_flag::FORWARD;
        // Quickly at first, slower as the engine's power spreads over the speed.
        tick(&mut hull, 0.1, Ground::Firm);
        let first = hull.speed;
        let resistance = hull.mobility.rolling_friction[0] * GRAVITY;
        assert!((first - (MAX_ACCELERATION - resistance) * 0.1).abs() < 1e-4, "{first}");
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
        // Stopped, then turning on the spot at full speed.
        assert_eq!(hull.speed, 0.0);
        assert!((hull.yaw_rate - 28f32.to_radians()).abs() < 1e-4, "{}", hull.yaw_rate);
        // Slower while driving.
        hull.flags = movement_flag::FORWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
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
    fn drifts_on_its_handbrake() {
        let mut hull = chieftain();
        hull.speed = 10.0;
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        let normal = hull.mobility.rotation_speed[1] * ROTATION_ON_MOVE;
        let mut fastest: f32 = 0.0;
        for _ in 0..10 {
            tick(&mut hull, 0.1, Ground::Medium);
            fastest = fastest.max(-hull.yaw_rate);
        }
        // Turning faster than it would driving, its hull swung left of where it slides.
        assert!(fastest > normal * 1.5, "{fastest} for {normal}");
        assert!(hull.slide > 0.2, "{}", hull.slide);
        // Until its blocked tracks stop it.
        for _ in 0..50 {
            tick(&mut hull, 0.1, Ground::Medium);
        }
        assert_eq!((hull.speed, hull.slide), (0.0, 0.0));
    }

    #[test]
    fn holds_on_slopes_it_grips() {
        // Parked nose down, it holds on 20°, and slides down 35°.
        assert_eq!(speed_after_3s(20.0, 0), 0.0);
        assert!(speed_after_3s(35.0, 0) > 1.0);
        // Driving up 20°, it crawls as fast as its power holds it against the slope; up
        // 35°, it slides back.
        let mobility = chieftain_mobility();
        let held = POWER_FACTOR * mobility.engine_power / mobility.weight
            / (mobility.rolling_friction[1] * GRAVITY + GRAVITY * 20f32.to_radians().sin());
        let up = speed_after_3s(-20.0, movement_flag::FORWARD);
        assert!((up - held).abs() < 0.05, "{up} for {held}");
        assert!(speed_after_3s(-35.0, movement_flag::FORWARD) < -1.0);
        // Across 35°, it slides down to its left, right side up.
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(35f32.to_radians(), 0.0, 0.0));
        for _ in 0..10 {
            tick(&mut hull, 0.1, Ground::Medium);
        }
        assert!(hull.slide < -1.0, "{}", hull.slide);
    }

    /// Its speed after 3 s pitched this many degrees, on these flags.
    fn speed_after_3s(pitch: f32, flags: u8) -> f32 {
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(0.0, pitch.to_radians(), 0.0));
        hull.flags = flags;
        for _ in 0..30 {
            tick(&mut hull, 0.1, Ground::Medium);
        }
        hull.speed
    }

}
