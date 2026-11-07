//! A vehicle as the server simulates it: its hull driven by the client's
//! `vehicle_moveWith` flags, its turret turning toward what the client aims at. The real
//! server runs a rigid body on springs (`WGTankPhysics`) with server-only tuning; the
//! client only shows its results. This one keeps its drive: the engine's power against
//! the ground's resistance and the slope, up to the top speeds, braking, and turning
//! with its own acceleration, all depending on the ground under it, its constants fitted
//! on drives captured from the real server. It rests on the terrain, stays within the
//! map's bounds, and is stopped by the solid static models, its collision box against
//! their triangles.

use glam::{Affine3A, Quat, Vec2, Vec3, Vec3Swizzles};

use wgtk::space::statics::{BodyKind, StaticScene, StaticTriangle};
use wgtk::space::terrain::{Ground, Terrain};
use wgtk::space::Space;

use crate::wot::item_defs::{AimingDef, MobilityDef};


/// How far inside the map's bounds the hull's centre stays, in metres. Fitted: the real
/// server holds it 0.3 m inside driven square on, 0.0 to 0.1 m inside at 30°.
const BOUNDS_MARGIN: f32 = 0.2;
/// How much of the speed an edge stops it also takes along it: their friction. Fitted
/// on the real server's drag along an edge driven into at 30°.
const EDGE_FRICTION: f32 = 0.25;
/// How high above the ground under its centre the hull's box starts, for the static
/// models it collides with: what is lower it drives over. Fitted roughly: the drives'
/// real hulls went over debris and porches up to 0.76 m high.
const STEP_HEIGHT: f32 = 0.8;
/// How much of the speed a wall stops it also takes along it. Our choice, an edge's
/// until fitted.
const WALL_FRICTION: f32 = EDGE_FRICTION;
/// How far from vertical a solid static triangle stops the hull, in radians: flatter,
/// it is ground the hull would climb. Our choice, before fitting.
const WALL_ANGLE: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// How many times a step pushes the hull out of the static models it overlaps, the
/// deepest overlap first.
const WALL_PASSES: usize = 4;

/// The `vehicle_moveWith` flags, `MOVEMENT_FLAGS` in the client's `Avatar.py`.
pub mod movement_flag {
    pub const FORWARD: u8 = 1;
    pub const BACKWARD: u8 = 2;
    pub const ROTATE_LEFT: u8 = 4;
    pub const ROTATE_RIGHT: u8 = 8;
    pub const CRUISE_CONTROL50: u8 = 16;
    pub const CRUISE_CONTROL25: u8 = 32;
    /// The handbrake.
    pub const BLOCK_TRACKS: u8 = 64;
}

// Where each constant comes from: the game's sources, the drives captured from the real
// server the hull is fitted on (a T110E4, 950 hp for 66 t, on Ruinberg's mostly firm
// ground, captured with its setup; a stock Type 64, 528 hp for 23 t, on Live Oaks'
// mostly medium ground), or our own choice.

/// Gravity, in m/s². From the game: `G` in `physics_shared.py`, which the server's
/// physics multiplies by the vehicle's `gravityFactor`, 1 when its definitions don't set
/// it, as the client's never do.
const GRAVITY: f32 = 9.81;
/// How many times its engine's power over its weight drives the vehicle, against its
/// ground's rolling friction: `a = POWER_FACTOR · P / (m · v) - μ · g`. Fitted: on the
/// samples driving straight forward from 4 m/s to near the top speed, forward held for
/// 0.4 s before, the acceleration plus gravity along the slope, on the ground under
/// them, gives a median of 1.125 on the T110E4 (559 samples) and 1.137 on the Type 64
/// (1040). Fitting the friction too trades it against the power (1.2 to 1.36 for 1.25 to
/// 1.3 times the friction), so the game's friction is kept.
const POWER_FACTOR: f32 = 1.13;
/// The ground the vehicle drives on where the terrain gives none, or off it. Our choice:
/// unpainted cells are mostly beyond the battle's bounds, and medium is most of the
/// painted ones.
const DEFAULT_GROUND: Ground = Ground::Medium;
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
/// How its tracks grip the terrain the steeper it is, along them and across: fully up to
/// the first angle, in degrees, least, the share given, from the second, in between as the
/// cosines go. From the game: `slopeGripLngTerrain` and `slopeGripSdwTerrain`, the
/// defaults of `physics_shared.py`, which the server's vehicle sections may override.
const SLOPE_GRIP_ALONG: (f32, f32, f32) = (27.5, 32.0, 0.1);
const SLOPE_GRIP_ACROSS: (f32, f32, f32) = (24.5, 29.0, 0.1);
/// Under this speed, in m/s, the vehicle turns as on the spot. Our choice, the split the
/// fit above uses between turning on the spot and moving.
const ON_SPOT_SPEED: f32 = 0.5;

/// The engine's relative RPM at idle, and just after shifting up. Captured: the RPM
/// byte of `ownVehicleAuxPhysicsData` in the battle capture of 2026-10-31, whose 5th
/// percentile is 0.20.
const IDLE_RPM: f32 = 0.2;
const SHIFT_RPM: f32 = 0.3;
/// The shares of the top speed the gearbox shifts up at: 4 gears. Captured, roughly: in
/// the same capture, `ownVehicleGear` shifts up to the 2nd gear at a median 0.31 of the
/// top speed and to the 4th at 0.82, while the 3rd comes anywhere (median 0.29): the
/// real gearbox (`fakegearbox`, server-only) shifts on more than the speed. The 3rd's
/// share is our choice, between the others.
const GEAR_SHIFTS: [f32; 3] = [0.27, 0.52, 0.8];

/// The ground at a position on the terrain, else [`DEFAULT_GROUND`].
pub fn ground_at(terrain: &Terrain, position: Vec3) -> Ground {
    terrain.ground_at(position.x, position.z)
        .filter(|&ground| ground != Ground::None)
        .unwrap_or(DEFAULT_GROUND)
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

/// A live vehicle: its hull and its turret.
#[derive(Debug, Clone)]
pub struct Vehicle {
    pub hull: Hull,
    pub turret: Turret,
}

impl Vehicle {

    pub fn new(position: Vec3, direction: Vec3, mobility: MobilityDef, aiming: AimingDef) -> Self {
        Self { hull: Hull::new(position, direction, mobility), turret: Turret::new(aiming) }
    }

    /// Advance by `dt` seconds in its space, within the map's bounds if any: the hull
    /// moves, see [`Hull::advance`], then the turret turns. Returns whether the hull
    /// moved or turned.
    pub fn step(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>) -> bool {
        let moved = self.hull.advance(dt, space, bounds);
        self.turret.step(dt, &self.hull);
        moved
    }

    /// Where its chassis, hull, turret and gun are in the world, as transforms from
    /// each part's frame, the hull sitting at `hull_position` on the chassis.
    pub fn part_transforms(&self, hull_position: Vec3) -> [Affine3A; 4] {
        let aiming = &self.turret.aiming;
        let chassis = Affine3A::from_rotation_translation(self.hull.rotation(), self.hull.position);
        let hull = chassis * Affine3A::from_translation(hull_position);
        let turret = chassis * Affine3A::from_rotation_translation(Quat::from_rotation_y(self.turret.yaw), aiming.turret_position);
        let gun = turret * Affine3A::from_rotation_translation(Quat::from_rotation_x(self.turret.pitch), aiming.gun_position);
        [chassis, hull, turret, gun]
    }

}

/// A vehicle's hull, moving as the client drives it.
#[derive(Debug, Clone)]
pub struct Hull {
    pub position: Vec3,
    /// Roll, pitch and yaw, in radians, the order position updates carry them. A yaw
    /// of 0 faces +Z, and turning right increases it.
    pub direction: Vec3,
    /// Signed speed along the vehicle's heading, in m/s.
    pub speed: f32,
    /// Signed speed across its heading, positive to the right, in m/s: it slides with
    /// its tracks blocked, then its tracks hold it again.
    pub slide: f32,
    /// Turning speed, in rad/s, positive to the right.
    pub yaw_rate: f32,
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub mobility: MobilityDef,
}

impl Hull {

    pub fn new(position: Vec3, direction: Vec3, mobility: MobilityDef) -> Self {
        Self { position, direction, speed: 0.0, slide: 0.0, yaw_rate: 0.0, flags: 0, mobility }
    }

    /// The speed the flags ask for: the top speed in their direction, reduced by cruise
    /// control, and by turning.
    fn target_speed(&self) -> f32 {
        
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

    /// Advance by `dt` seconds in its space: [`Self::step`] on the ground under it,
    /// within the map's bounds if any, see [`Self::keep_within`], out of the solid
    /// static models, see [`Self::push_out`], then resting on the terrain. Returns
    /// whether the hull moved or turned.
    pub fn advance(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>) -> bool {
        let moved = self.step(dt, ground_at(&space.terrain, self.position));
        if moved {
            self.push_out(&space.statics);
        }
        if let Some(bounds) = bounds {
            self.keep_within(bounds);
        }
        if moved {
            self.settle(&space.terrain);
        }
        moved
    }

    /// Push its collision box out of the solid static models it overlaps, on the ground
    /// plane: their triangles within [`WALL_ANGLE`] of vertical, between [`STEP_HEIGHT`]
    /// and its top, the deepest overlap first, [`WALL_PASSES`] at most. What of its velocity goes into a wall stops, and
    /// slows the rest by [`WALL_FRICTION`]. Its pitch and roll are left out.
    pub fn push_out(&mut self, statics: &StaticScene) {

        for _ in 0..WALL_PASSES {

            let push = self.overlaps(statics)
                .map(|(_, push)| push)
                .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
            let Some(push) = push else { break };

            self.position += Vec3::new(push.x, 0.0, push.y);
            let normal = Vec3::new(push.x, 0.0, push.y).normalize();
            let mut velocity = self.velocity();
            let into = velocity.dot(normal);
            if into < 0.0 {
                velocity -= normal * into;
                let along = velocity - normal * velocity.dot(normal);
                velocity -= along.clamp_length_max(WALL_FRICTION * -into);
                let (heading, right) = self.axes();
                (self.speed, self.slide) = (velocity.dot(heading), velocity.dot(right));
            }

        }

    }

    /// The solid static triangles its collision box overlaps, see [`Self::push_out`],
    /// each with how far to push it out of it on the ground plane.
    pub fn overlaps<'a>(&self, statics: &'a StaticScene) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {

        let (heading, right) = self.axes();
        let (heading, right) = (heading.xz(), right.xz());
        let [min, max] = self.mobility.bounds;
        let (center, half) = ((min + max) / 2.0, (max - min) / 2.0);
        let half = Vec2::new(half.x, half.z);
        let (low, high) = (self.position.y + STEP_HEIGHT.max(min.y), self.position.y + max.y);

        let middle = self.position.xz() + heading * center.z + right * center.x;
        let reach = half.length();
        let (least, most) = (Vec3::new(middle.x - reach, low, middle.y - reach), Vec3::new(middle.x + reach, high, middle.y + reach));
        statics.triangles_in(BodyKind::Solid, least, most)
            .filter(|triangle| triangle.normal.y.abs() < WALL_ANGLE.sin())
            .filter_map(move |triangle| {
                let outline = clip_to_heights(&triangle.vertices, low, high)?;
                Some((triangle, box_push(middle, heading, right, half, &outline)?))
            })

    }

    /// Keep its centre within the map's bounds, [`BOUNDS_MARGIN`] inside them: what of
    /// its velocity goes into an edge stops, and slows the rest by [`EDGE_FRICTION`].
    /// Driven into an edge at an angle, it drags along it: its tracks keep pushing it in,
    /// and lose that share again each step.
    pub fn keep_within(&mut self, (min, max): (Vec2, Vec2)) {
        let mut velocity = self.velocity();
        let mut stopped = Vec3::ZERO;
        for (axis, (least, most)) in [(0, (min.x, max.x)), (2, (min.y, max.y))] {
            let (least, most) = (least + BOUNDS_MARGIN, most - BOUNDS_MARGIN);
            let into = if self.position[axis] < least {
                self.position[axis] = least;
                velocity[axis].min(0.0)
            } else if self.position[axis] > most {
                self.position[axis] = most;
                velocity[axis].max(0.0)
            } else {
                continue;
            };
            stopped[axis] = into;
            velocity[axis] -= into;
        }
        if stopped == Vec3::ZERO {
            return;
        }
        // Along the edges, slowed as much as their friction takes of what they stopped.
        let along = velocity - velocity.project_onto(stopped);
        velocity -= along.clamp_length_max(EDGE_FRICTION * stopped.length());
        let (heading, right) = self.axes();
        (self.speed, self.slide) = (velocity.dot(heading), velocity.dot(right));
    }

    /// Advance by `dt` seconds on this ground. Returns whether the hull moved or turned.
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
    pub fn step(&mut self, dt: f32, ground: Ground) -> bool {

        use movement_flag::*;

        let ground = ground_index(ground);
        let rotation_speed = self.mobility.rotation_speed[ground];
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
        let way = target.signum();

        // Gravity down the slope, pitched positively nose down and rolled positively
        // right side up, which the tracks hold against as much as they grip it.
        let (roll, pitch) = (self.direction.x, self.direction.y);
        let (grip_along, grip_across) = (grip(pitch, SLOPE_GRIP_ALONG), grip(roll, SLOPE_GRIP_ACROSS));
        let slope = GRAVITY * pitch.sin();
        self.slide = approach(self.slide - GRAVITY * roll.sin() * dt, 0.0, SIDE_FRICTION * grip_across * dt);
        self.speed = 
            if handbrake {
                approach(self.speed + slope * dt, pivot, HANDBRAKE_FRICTION * grip_along * dt)
            } else if target == 0.0 {
                approach(self.speed + slope * dt, 0.0, COASTING * grip_along * dt)
            } else if self.speed * way < 0.0 {
                approach(self.speed + slope * dt, 0.0, BRAKING * grip_along * dt)
            } else {
                // Along its way: the drive, less the ground's resistance, and the slope.
                let mobility = &self.mobility;
                let along = self.speed.abs();
                let power = POWER_FACTOR * mobility.engine_power / mobility.weight;
                let drive = (power / along.max(1e-3)).min(MAX_ACCELERATION) * grip_along;
                let turning = if self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0 { TURNING_RESISTANCE } else { 0.0 };
                let resistance = mobility.rolling_friction[ground] * GRAVITY + turning;
                let along = if along > target.abs() {
                    approach(along, target.abs(), BRAKING * grip_along * dt)
                } else {
                    // Short of power it stalls, unless the slope takes it back down past
                    // what its tracks hold.
                    let floor = if slope * way + BRAKING * grip_along >= 0.0 { 0.0 } else { f32::NEG_INFINITY };
                    (along + (drive - resistance + slope * way) * dt).clamp(floor, target.abs())
                };
                along * way
            };

        let (goal, rise) = if handbrake {
            // Sliding faster than it pivots swings it round faster than it drives.
            let sliding = (self.speed.hypot(self.slide) - pivot.abs()).max(0.0);
            let share = 1.0 - (-sliding / HANDBRAKE_ROTATION_SPEED).exp();
            let drifting = rotation_speed * HANDBRAKE_ROTATION * share;
            let pivoting = if pivot != 0.0 { rotation_speed * ROTATION_ON_MOVE } else { 0.0 };
            (turn * drifting.max(pivoting), HANDBRAKE_ROTATION_RISE)
        } else {
            let moving = self.speed.abs() >= ON_SPOT_SPEED || target != 0.0;
            (turn * rotation_speed * if moving { ROTATION_ON_MOVE } else { 1.0 }, ROTATION_RISE)
        };
        let time = if goal.abs() > self.yaw_rate.abs() && goal * self.yaw_rate >= 0.0 { rise } else { ROTATION_FALL };
        let rate = rotation_speed / time;

        let velocity = self.velocity();
        self.yaw_rate = approach(self.yaw_rate, goal, rate * dt);
        self.direction.z = wrap_angle(self.direction.z + self.yaw_rate * dt);
        // Its tracks blocked, it keeps going where it went as its hull turns, else its
        // tracks carry its speed around.
        if handbrake {
            let (heading, right) = self.axes();
            (self.speed, self.slide) = (velocity.dot(heading), velocity.dot(right));
        }
        self.position += self.velocity() * dt;

        self.speed != 0.0 || self.slide != 0.0 || self.yaw_rate != 0.0

    }

    /// Its heading and its right on the ground plane.
    fn axes(&self) -> (Vec3, Vec3) {
        let yaw = self.direction.z;
        (Vec3::new(yaw.sin(), 0.0, yaw.cos()), Vec3::new(yaw.cos(), 0.0, -yaw.sin()))
    }

    /// Its velocity on the ground plane, in m/s.
    pub fn velocity(&self) -> Vec3 {
        let (heading, right) = self.axes();
        heading * self.speed + right * self.slide
    }

    /// The vehicle's `engineMode`, which the client's track scroll follows: idle (1)
    /// without flags, accelerating (3) until at the speed the flags ask for, then
    /// cruising (2), and the flags themselves.
    pub fn engine_mode(&self) -> [u8; 2] {
        
        let mode = if self.flags == 0 {
            1
        } else if (self.target_speed() - self.speed).abs() > 0.01 {
            3
        } else {
            2
        };

        [mode, self.flags]

    }

    /// The engine's gear and its RPM relative to its maximum, which the client's engine
    /// sound follows. As the captured server does: in neutral (gear 0) at idle RPM when
    /// still, else the speed's share of the top speed picks one of [`GEAR_SHIFTS`]'s
    /// gears, the RPM going from [`SHIFT_RPM`] to 1 across it; turning in place, the RPM
    /// follows the turn rate in first gear, against its fastest, on firm ground.
    pub fn engine(&self) -> (u8, f32) {

        let top_speed = if self.speed < 0.0 { self.mobility.speed_backward } else { self.mobility.speed_forward };
        let share = (self.speed.abs() / top_speed.max(0.1)).min(1.0);
        let turn_share = (self.yaw_rate.abs() / self.mobility.rotation_speed[0].max(0.01)).min(1.0);
        if share < 0.02 {
            if turn_share < 0.02 {
                return (0, IDLE_RPM);
            }
            return (1, SHIFT_RPM + (1.0 - SHIFT_RPM) * turn_share);
        }

        let gear = GEAR_SHIFTS.iter().take_while(|&&shift| share >= shift).count();
        let low = if gear == 0 { 0.0 } else { GEAR_SHIFTS[gear - 1] };
        let high = GEAR_SHIFTS.get(gear).copied().unwrap_or(1.0);
        let in_gear = ((share - low) / (high - low)).clamp(0.0, 1.0);
        (gear as u8 + 1, SHIFT_RPM + (1.0 - SHIFT_RPM) * in_gear)
        
    }

    /// The hull's orientation: yaw about Y, then pitch about X (nose down), then roll
    /// about Z (right side up), as the client's matrices compose them.
    pub fn rotation(&self) -> Quat {
        let (roll, pitch, yaw) = (self.direction.x, self.direction.y, self.direction.z);
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll)
    }

    /// The hull's orientation and its tracks' speeds, as the avatar's
    /// `ownVehicleAuxPhysicsData` carries them for the client to aim from and animate the
    /// tracks with, see `WoT.packAuxVehiclePhysicsData`. From the lowest bit: yaw on 15
    /// bits over a turn, pitch on 12 bits over ±π/2, roll on 13 bits over a turn, the left
    /// then right track speeds on 8 bits each over -15..30 m/s, and the engine's relative
    /// RPM on 8 bits over 0..1.2, see [`Self::engine`].
    pub fn aux_physics_data(&self) -> u64 {
        
        use std::f32::consts::FRAC_PI_2;
        use movement_flag::*;
        
        let (left, right) = if self.flags & BLOCK_TRACKS != 0 {
            // On its handbrake, its inner track blocked and its outer one carrying it as
            // fast as it slides, as the real server's show; both blocked going straight.
            let outer = self.speed.hypot(self.slide);
            match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
                ROTATE_LEFT => (0.0, outer),
                ROTATE_RIGHT => (outer, 0.0),
                _ => (0.0, 0.0),
            }
        } else {
            // The inner track goes slower, the outer one faster.
            let turning = self.yaw_rate * self.mobility.carrying_point.x;
            (self.speed + turning, self.speed - turning)
        };

        let (roll, pitch, yaw) = (self.direction.x, self.direction.y, self.direction.z);
        pack_range(self.engine().1, 0.0, 1.2, 8) << 56
            | pack_range(right, -15.0, 30.0, 8) << 48
            | pack_range(left, -15.0, 30.0, 8) << 40
            | pack_angle(roll, 13) << 27
            | pack_range(pitch, -FRAC_PI_2, FRAC_PI_2, 12) << 15
            | pack_angle(yaw, 15)

    }

    /// Rest on the terrain: at its height, rolled and pitched with its slope under the
    /// corners its tracks rest on, see `MobilityDef::carrying_point`. Off the terrain,
    /// the hull stays as it is.
    pub fn settle(&mut self, terrain: &Terrain) {

        let (heading, right) = self.axes();
        let carrying = self.mobility.carrying_point;
        let height = |along: f32, across: f32| {
            let at = self.position + heading * along * carrying.y + right * across * carrying.x;
            terrain.height_at(at.x, at.z)
        };

        let (Some(center), Some(front_right), Some(front_left), Some(back_right), Some(back_left)) =
            (height(0.0, 0.0), height(1.0, 1.0), height(1.0, -1.0), height(-1.0, 1.0), height(-1.0, -1.0)) else {
            return;
        };
        let (front, back) = (front_right + front_left, back_right + back_left);
        let (right_side, left_side) = (front_right + back_right, front_left + back_left);
        self.position.y = center;
        // Rolled positively with its right side up, pitched positively nose down.
        self.direction.x = ((right_side - left_side) / (4.0 * carrying.x)).atan();
        self.direction.y = -((front - back) / (4.0 * carrying.y)).atan();

    }

}

/// A turret and its gun, turning toward what the client aims at at their top speeds,
/// the gun within its pitch limits at the turret's yaw.
#[derive(Debug, Clone)]
pub struct Turret {
    /// Relative to the hull, in radians.
    pub yaw: f32,
    /// Positive downward, in radians.
    pub pitch: f32,
    target: Aim,
    aiming: AimingDef,
    /// How fast the turret turned in the last step, in rad/s.
    yaw_rate: f32,
    /// The dispersion as a factor of the gun's base one, shrinking back to its ideal
    /// over the aiming time.
    dispersion_factor: f32,
}

/// What the turret turns toward.
#[derive(Debug, Clone, Copy)]
enum Aim {
    /// A point in the world. The client gives it relative to the vehicle, with
    /// `trackRelativePointWithGun`, and only sends it again when it changes, so it
    /// stays where it was in the world as the vehicle moves.
    Point(Vec3),
    /// Angles relative to the hull, as `stopTrackingWithGun` gives them.
    Angles(f32, f32),
}

impl Turret {

    pub fn new(aiming: AimingDef) -> Self {
        Self { yaw: 0.0, pitch: 0.0, target: Aim::Angles(0.0, 0.0), aiming, yaw_rate: 0.0, dispersion_factor: 1.0 }
    }

    /// Turn toward a point in the world.
    pub fn track(&mut self, point: Vec3) {
        self.target = Aim::Point(point);
    }

    /// Turn to angles relative to the hull, and stay there.
    pub fn hold(&mut self, yaw: f32, pitch: f32) {
        self.target = Aim::Angles(yaw, pitch);
    }

    /// Advance by `dt` seconds on the given hull, already advanced: turn toward the aim,
    /// then update the dispersion.
    pub fn step(&mut self, dt: f32, hull: &Hull) {
        let (yaw, pitch) = match self.target {
            // Raised for the shell to drop onto the point, then in the hull's frame,
            // which the turret and gun turn in.
            Aim::Point(point) => {
                let direction = self.ballistic_direction(point - gun_position(hull, self));
                let to = hull.rotation().inverse() * direction;
                let horizontal = (to.x * to.x + to.z * to.z).sqrt();
                (to.x.atan2(to.z), -to.y.atan2(horizontal))
            }
            Aim::Angles(yaw, pitch) => (yaw, pitch),
        };
        // Within its limits it only turns inside them, never round the back.
        let max_yaw_step = self.aiming.turret_rotation_speed * dt;
        let yaw_step = match self.aiming.yaw_limits {
            Some((min, max)) => wrap_angle(yaw).clamp(min, max) - self.yaw,
            None => wrap_angle(yaw - self.yaw),
        }.clamp(-max_yaw_step, max_yaw_step);
        self.yaw = wrap_angle(self.yaw + yaw_step);
        self.yaw_rate = yaw_step.abs() / dt;
        let pitch = pitch.clamp(self.aiming.min_pitch.at(self.yaw), self.aiming.max_pitch.at(self.yaw));
        let max_pitch_step = self.aiming.gun_rotation_speed * dt;
        self.pitch += (pitch - self.pitch).clamp(-max_pitch_step, max_pitch_step);
        self.step_dispersion(dt, hull);
    }

    /// The dispersion's part of a step, as the client's `getOwnVehicleShotDispersionAngle`
    /// computes it: the ideal factor grows with the hull's speed, its turning and the
    /// turret's turning, and the actual one jumps up to it but only shrinks back over the
    /// aiming time. No crew, equipment or shot bonuses.
    /// The direction to fire the shell in for it to reach `to`, relative to the gun:
    /// the lower of the two trajectories, or 45° up when out of reach.
    fn ballistic_direction(&self, to: Vec3) -> Vec3 {
        let (speed, gravity) = (self.aiming.shot_speed, self.aiming.shot_gravity);
        let distance = (to.x * to.x + to.z * to.z).sqrt();
        if distance < 1e-3 || gravity <= 0.0 {
            return to.normalize_or(Vec3::Z);
        }
        let v2 = speed * speed;
        let discriminant = v2 * v2 - gravity * (gravity * distance * distance + 2.0 * to.y * v2);
        let elevation = if discriminant >= 0.0 {
            ((v2 - discriminant.sqrt()) / (gravity * distance)).atan()
        } else {
            std::f32::consts::FRAC_PI_4
        };
        let yaw = to.x.atan2(to.z);
        Vec3::new(yaw.sin() * elevation.cos(), elevation.sin(), yaw.cos() * elevation.cos())
    }

    fn step_dispersion(&mut self, dt: f32, hull: &Hull) {
        let movement = hull.speed * self.aiming.dispersion_movement;
        let rotation = hull.yaw_rate * self.aiming.dispersion_rotation;
        let turret = self.yaw_rate * self.aiming.dispersion_turret_rotation;
        let ideal = (1.0 + movement * movement + rotation * rotation + turret * turret).sqrt();
        self.dispersion_factor = if ideal >= self.dispersion_factor {
            ideal
        } else {
            ideal + (self.dispersion_factor - ideal) * (-dt / self.aiming.aiming_time).exp()
        };
    }

    /// Where the gun fires from and the shell's initial velocity, as the client's
    /// `getCurShotPosition` gives them, and the current dispersion angle: what the
    /// server's gun marker shows.
    pub fn gun_marker(&self, hull: &Hull) -> (Vec3, Vec3, f32) {
        // Turret yaw, then gun pitch (positive downward), on the hull.
        let gun = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch);
        let direction = hull.rotation() * gun * Vec3::Z;
        (gun_position(hull, self), direction * self.aiming.shot_speed, self.aiming.dispersion_angle * self.dispersion_factor)
    }

    /// The angles as the `gunAnglesPacked` property carries them, see `encodeGunAngles`:
    /// the yaw on 10 bits over a turn, the pitch on 6 bits over its absolute limits.
    pub fn gun_angles_packed(&self) -> u16 {
        use std::f32::consts::{PI, TAU};
        let yaw = ((1024.0 * (self.yaw + PI) / TAU).round() as u16) & 1023;
        let (min_pitch, max_pitch) = self.aiming.absolute_pitch_limits();
        let t = if max_pitch == min_pitch { 0.0 } else { ((self.pitch - min_pitch) / (max_pitch - min_pitch)).clamp(0.0, 1.0) };
        yaw << 6 | (63.0 * t).round() as u16
    }

}

/// Where the gun fires from: its pivot on the turret, turned with it, on the hull.
fn gun_position(hull: &Hull, turret: &Turret) -> Vec3 {
    let aiming = &turret.aiming;
    hull.position + hull.rotation() * (aiming.turret_position + Quat::from_rotation_y(turret.yaw) * aiming.gun_position)
}

/// A value moved toward a goal by at most `step`.
/// What of a triangle lies between two heights, on the ground plane: its outline's x
/// and z, none if it lies all above or below.
fn clip_to_heights(triangle: &[Vec3; 3], low: f32, high: f32) -> Option<Vec<Vec2>> {

    // Clip the polygon to one side of a height, `keep` telling the side kept.
    fn clip(polygon: Vec<Vec3>, height: f32, keep: impl Fn(f32) -> bool) -> Vec<Vec3> {
        let mut clipped = Vec::with_capacity(polygon.len() + 1);
        for (index, &from) in polygon.iter().enumerate() {
            let to = polygon[(index + 1) % polygon.len()];
            if keep(from.y) {
                clipped.push(from);
            }
            if keep(from.y) != keep(to.y) {
                clipped.push(from.lerp(to, (height - from.y) / (to.y - from.y)));
            }
        }
        clipped
    }

    let polygon = clip(triangle.to_vec(), low, |y| y >= low);
    let polygon = clip(polygon, high, |y| y <= high);
    (!polygon.is_empty()).then(|| polygon.into_iter().map(|vertex| vertex.xz()).collect())
}

/// How far to push a box on the ground plane, at `middle` with its `heading` and `right`
/// axes and its `half` sizes across then along, out of a convex outline, least first:
/// along the axis that separates them least, of the box's two and the outline's edges'
/// normals, none if they don't overlap.
fn box_push(middle: Vec2, heading: Vec2, right: Vec2, half: Vec2, outline: &[Vec2]) -> Option<Vec2> {

    let edges = outline.iter().zip(outline.iter().cycle().skip(1))
        .map(|(&from, &to)| (to - from).perp())
        .filter(|normal| normal.length_squared() > 1e-8)
        .map(Vec2::normalize);

    let mut least: Option<Vec2> = None;
    for axis in [heading, right].into_iter().chain(edges) {
        let center = middle.dot(axis);
        let extent = half.x * right.dot(axis).abs() + half.y * heading.dot(axis).abs();
        let (low, high) = outline.iter().map(|vertex| vertex.dot(axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), p| (low.min(p), high.max(p)));
        // Pushed back past the outline's low side, or forward past its high side.
        let (back, forward) = (center + extent - low, high - (center - extent));
        if back <= 0.0 || forward <= 0.0 {
            return None;
        }
        let push = if back < forward { -axis * back } else { axis * forward };
        if least.is_none_or(|least| push.length_squared() < least.length_squared()) {
            least = Some(push);
        }
    }
    least

}

fn approach(value: f32, goal: f32, step: f32) -> f32 {
    value + (goal - value).clamp(-step, step)
}

/// A value between bounds on `bits` bits, rounded, as the client's packing does.
fn pack_range(value: f32, min: f32, max: f32, bits: u32) -> u64 {
    let max_code = (1u64 << bits) - 1;
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    (t * max_code as f32).round() as u64 & max_code
}

/// An angle on `bits` bits over a turn from -π, rounded, as the client's packing does:
/// π wraps to -π.
fn pack_angle(angle: f32, bits: u32) -> u64 {
    let t = (angle / std::f32::consts::TAU + 0.5).clamp(0.0, 1.0);
    (t * (1u64 << bits) as f32).round() as u64 & ((1u64 << bits) - 1)
}

/// An angle in `(-π, π]`.
pub fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI { wrapped - TAU } else { wrapped }
}


#[cfg(test)]
pub(super) mod tests {

    use super::*;

    /// How high the test gun is above the vehicle's origin.
    const GUN_HEIGHT: f32 = 1.7;

    /// About a Chieftain's: 750 hp for 55 t, with its physics section's rolling friction,
    /// its terrain resistance 1, 1.3 and 2.5.
    pub(crate) fn chieftain_mobility() -> MobilityDef {
        MobilityDef {
            speed_forward: 40.0 / 3.6,
            speed_backward: 15.0 / 3.6,
            rotation_speed: [28f32.to_radians(), 28f32.to_radians() / 1.3, 28f32.to_radians() / 2.5],
            engine_power: 750.0 * 735.5,
            weight: 55_000.0,
            rolling_friction: [0.0805, 0.10465, 0.20125],
            carrying_point: glam::Vec2::new(1.6, 2.4),
            bounds: [Vec3::new(-1.8, 0.0, -3.7), Vec3::new(1.8, 2.6, 3.8)],
        }
    }

    fn chieftain() -> Hull {
        Hull::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility())
    }

    #[test]
    fn drives_forward_along_heading() {
        let mut hull = chieftain();
        hull.direction.z = 2.771;
        hull.flags = movement_flag::FORWARD;
        // Quickly at first, slower as the engine's power spreads over the speed.
        hull.step(0.1, Ground::Firm);
        let first = hull.speed;
        let resistance = hull.mobility.rolling_friction[0] * GRAVITY;
        assert!((first - (MAX_ACCELERATION - resistance) * 0.1).abs() < 1e-4, "{first}");
        for _ in 0..300 {
            hull.step(0.1, Ground::Firm);
        }
        // At top speed after accelerating, heading south-east as captured.
        assert!((hull.speed - 40.0 / 3.6).abs() < 1e-3, "{}", hull.speed);
        // Short of it on soft ground, where its power only holds it at P / (m · μ · g).
        for _ in 0..300 {
            hull.step(0.1, Ground::Soft);
        }
        let mobility = &hull.mobility;
        let held = POWER_FACTOR * mobility.engine_power / (mobility.weight * mobility.rolling_friction[2] * GRAVITY);
        assert!((hull.speed - held).abs() < 0.05, "{} for {held}", hull.speed);
        assert!(hull.position.x > 0.0 && hull.position.z < 0.0);
        assert_eq!(hull.position.y, 0.0);
    }

    #[test]
    fn stops_and_turns() {
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        hull.step(1.0, Ground::Firm);
        hull.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            hull.step(0.1, Ground::Firm);
        }
        // Stopped, then turning on the spot at full speed.
        assert_eq!(hull.speed, 0.0);
        assert!((hull.yaw_rate - 28f32.to_radians()).abs() < 1e-4, "{}", hull.yaw_rate);
        // Slower while driving.
        hull.flags = movement_flag::FORWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            hull.step(0.1, Ground::Firm);
        }
        assert!((hull.yaw_rate - 28f32.to_radians() * ROTATION_ON_MOVE).abs() < 1e-4, "{}", hull.yaw_rate);
        // Backing up while steering right turns the hull the other way, soon.
        hull.flags = movement_flag::BACKWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..5 {
            hull.step(0.1, Ground::Firm);
        }
        assert!(hull.yaw_rate < 0.0);
        // Standing still with no input reports no movement.
        hull.flags = 0;
        hull.speed = 0.0;
        hull.yaw_rate = 0.0;
        assert!(!hull.step(0.1, Ground::Firm));
    }

    #[test]
    fn drifts_on_its_handbrake() {
        let mut hull = chieftain();
        hull.speed = 10.0;
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        let normal = hull.mobility.rotation_speed[1] * ROTATION_ON_MOVE;
        let mut fastest: f32 = 0.0;
        for _ in 0..10 {
            hull.step(0.1, Ground::Medium);
            fastest = fastest.max(-hull.yaw_rate);
        }
        // Turning faster than it would driving, its hull swung left of where it slides.
        assert!(fastest > normal * 1.5, "{fastest} for {normal}");
        assert!(hull.slide > 0.2, "{}", hull.slide);
        // Until its blocked tracks stop it.
        for _ in 0..50 {
            hull.step(0.1, Ground::Medium);
        }
        assert_eq!((hull.speed, hull.slide), (0.0, 0.0));
    }

    #[test]
    fn holds_on_slopes_it_grips() {
        let still = |pitch: f32, flags: u8| {
            let mut hull = chieftain();
            hull.direction.y = pitch.to_radians();
            hull.flags = flags;
            for _ in 0..30 {
                hull.step(0.1, Ground::Medium);
            }
            hull.speed
        };
        // Parked nose down, it holds on 20°, and slides down 35°.
        assert_eq!(still(20.0, 0), 0.0);
        assert!(still(35.0, 0) > 1.0);
        // Driving up 20°, it crawls as fast as its power holds it against the slope; up
        // 35°, it slides back.
        let mobility = chieftain_mobility();
        let held = POWER_FACTOR * mobility.engine_power / mobility.weight
            / (mobility.rolling_friction[1] * GRAVITY + GRAVITY * 20f32.to_radians().sin());
        let up = still(-20.0, movement_flag::FORWARD);
        assert!((up - held).abs() < 0.05, "{up} for {held}");
        assert!(still(-35.0, movement_flag::FORWARD) < -1.0);
        // Across 35°, it slides down to its left, right side up.
        let mut hull = chieftain();
        hull.direction.x = 35f32.to_radians();
        for _ in 0..10 {
            hull.step(0.1, Ground::Medium);
        }
        assert!(hull.slide < -1.0, "{}", hull.slide);
    }

    /// A wall across the way at z = 10, `height` high, as two triangles.
    fn wall(height: f32) -> Space {
        let (low_left, low_right) = (Vec3::new(-20.0, -1.0, 10.0), Vec3::new(20.0, -1.0, 10.0));
        let (high_left, high_right) = (Vec3::new(-20.0, height, 10.0), Vec3::new(20.0, height, 10.0));
        let triangles = vec![
            StaticTriangle::new([low_left, low_right, high_right], 0, 0, BodyKind::Solid),
            StaticTriangle::new([low_left, high_right, high_left], 0, 0, BodyKind::Solid),
        ];
        Space { statics: StaticScene::from_triangles(triangles), ..Space::default() }
    }

    #[test]
    fn stops_against_walls_it_cannot_drive_over() {
        // Square on at full speed, it stops with its front against the wall.
        let space = wall(3.0);
        let mut hull = chieftain();
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None);
        }
        let front = hull.position.z + hull.mobility.bounds[1].z;
        assert!((front - 10.0).abs() < 1e-3 && hull.speed.abs() < 1e-3, "front at {front}, {} m/s", hull.speed);
        // Lower than its step, it drives over it.
        let space = wall(STEP_HEIGHT - 0.1);
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None);
        }
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
    }

    #[test]
    fn stops_at_the_map_edge() {
        let (space, bounds) = (Space::default(), Some((Vec2::splat(-100.0), Vec2::splat(100.0))));
        let edge = 100.0 - BOUNDS_MARGIN;
        // Square on at full speed, it stops dead, still pushing.
        let mut hull = chieftain();
        hull.position.z = 90.0;
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..20 {
            hull.advance(0.1, &space, bounds);
        }
        assert_eq!(hull.position.z, edge);
        assert!(hull.speed.abs() < 1e-4 && hull.position.x.abs() < 1e-4, "{} {}", hull.speed, hull.position.x);
        // At 30°, it drags along it, slower and slower: the real server's went 2.6, 1.8,
        // 0.7 then 0.2 m/s once against it.
        let mut hull = chieftain();
        (hull.position.z, hull.direction.z) = (edge, -30f32.to_radians());
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        let mut along = Vec::new();
        for _ in 0..6 {
            let x = hull.position.x;
            hull.advance(0.1, &space, bounds);
            along.push((x - hull.position.x) / 0.1);
        }
        assert_eq!(hull.position.z, edge);
        assert!(along.windows(2).all(|pair| pair[1] <= pair[0]) && (2.0..3.0).contains(&along[1]) && along[4] < 1.0, "{along:?}");
    }

    pub(crate) fn chieftain_aiming() -> AimingDef {
        use crate::wot::item_defs::PitchCurve;
        let curve = |points: &[(f32, f32)]| PitchCurve(points.iter().map(|&(t, a)| (t * std::f32::consts::TAU, a.to_radians())).collect());
        AimingDef {
            turret_rotation_speed: 30f32.to_radians(),
            gun_rotation_speed: 26f32.to_radians(),
            aiming_time: 2.1,
            dispersion_turret_rotation: 0.11f32.to_degrees(),
            dispersion_after_shot: 4.0,
            dispersion_movement: 0.14 * 3.6,
            dispersion_rotation: 0.14f32.to_degrees(),
            yaw_limits: None,
            min_pitch: curve(&[(0.0, -20.0), (1.0, -20.0)]),
            // 10° of depression at the front, -1° over the engine deck at the back.
            max_pitch: curve(&[(0.0, 10.0), (0.4, 10.0), (0.45, -1.0), (0.55, -1.0), (0.6, 10.0), (1.0, 10.0)]),
            dispersion_angle: 0.0035,
            shot_speed: 716.0,
            shot_gravity: 6.28,
            reload_time: 16.0,
            // Straight above the origin, to keep the tests' geometry simple.
            turret_position: Vec3::new(0.0, 1.4, 0.0),
            gun_position: Vec3::new(0.0, GUN_HEIGHT - 1.4, 0.0),
        }
    }

    #[test]
    fn turret_turns_toward_point() {
        let mut hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        // Straight to the right of a hull facing +Z, level with the gun.
        turret.track(Vec3::new(100.0, GUN_HEIGHT, 0.0));
        turret.step(1.0, &hull);
        assert!((turret.yaw - 30f32.to_radians()).abs() < 1e-5);
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // Raised a little for the shell to drop onto it.
        assert!(turret.pitch < 0.0 && turret.pitch > -0.01);
        // The point stays put in the world as the vehicle drives past it.
        hull.position = Vec3::new(0.0, 0.0, 200.0);
        turret.step(1.0, &hull);
        assert!(turret.yaw > std::f32::consts::FRAC_PI_2);
        // Yaw 0 and the middle of the pitch range, as `encodeGunAngles` packs them.
        turret.hold(0.0, -5f32.to_radians());
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert_eq!(turret.gun_angles_packed(), 512 << 6 | 32);
    }

    #[test]
    fn holding_where_it_tracks_keeps_the_aim_on_a_tilted_hull() {
        let mut hull = chieftain();
        hull.direction = Vec3::new(0.05, 0.08, 1.2);
        let mut turret = Turret::new(chieftain_aiming());
        let point = Vec3::new(150.0, 5.0, 40.0);
        turret.track(point);
        for _ in 0..20 {
            turret.step(1.0, &hull);
        }
        // The shell's arc goes through the point.
        let (position, velocity, _) = turret.gun_marker(&hull);
        let to = point - position;
        let horizontal = Vec3::new(velocity.x, 0.0, velocity.z);
        let time = Vec3::new(to.x, 0.0, to.z).length() / horizontal.length();
        let at = position + velocity * time - Vec3::Y * 0.5 * turret.aiming.shot_gravity * time * time;
        assert!((at - point).length() < 0.05, "lands at {at}, not {point}");
        // Locking sends back the same hull-relative angles, which change nothing.
        let (yaw, pitch) = (turret.yaw, turret.pitch);
        turret.hold(yaw, pitch);
        turret.step(1.0, &hull);
        assert_eq!(turret.gun_marker(&hull).1, velocity);
    }

    #[test]
    fn engine_shifts_up_with_speed() {
        let mut hull = chieftain();
        assert_eq!(hull.engine(), (0, IDLE_RPM));
        let top = hull.mobility.speed_forward;
        let mut last = (0, 0.0);
        // Below 2% of the top speed, it counts as still.
        for step in 2..=100 {
            hull.speed = top * step as f32 / 100.0;
            let (gear, rpm) = hull.engine();
            assert!(gear >= last.0 && (SHIFT_RPM..=1.0).contains(&rpm), "{gear} {rpm} at {}", hull.speed);
            // The RPM drops only when shifting up.
            assert!(rpm >= last.1 || gear > last.0);
            last = (gear, rpm);
        }
        assert_eq!(last, (4, 1.0));
    }

    #[test]
    fn aux_physics_data_packs_as_captured() {
        // A captured value, slowing down: yaw 2.7713, pitch and roll near 0, both tracks
        // at 1.76 m/s, the RPM at 0xaf.
        let captured: u64 = 0xaf5f5f813c00f875;
        let mut hull = chieftain();
        hull.direction = Vec3::new(0.0299, 0.0012, 2.7713);
        hull.speed = 1.76;
        // Without the RPM, which depends on the engine model.
        assert_eq!(hull.aux_physics_data() & 0x00ff_ffff_ffff_ffff, captured & 0x00ff_ffff_ffff_ffff);
    }

    #[test]
    fn dispersion_widens_then_shrinks_over_aiming_time() {
        let mut hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        hull.speed = 10.0;
        turret.step(0.1, &hull);
        let moving = turret.dispersion_factor;
        assert!(moving > 1.0);
        hull.speed = 0.0;
        // After one aiming time, the excess shrinks by e.
        for _ in 0..21 {
            turret.step(0.1, &hull);
        }
        assert!(((turret.dispersion_factor - 1.0) - (moving - 1.0) / std::f32::consts::E).abs() < 1e-3);
        // Aimed straight ahead, the shell leaves at the shot speed.
        let (position, velocity, angle) = turret.gun_marker(&hull);
        assert_eq!(position, Vec3::new(0.0, GUN_HEIGHT, 0.0));
        assert!((velocity - Vec3::new(0.0, 0.0, 716.0)).length() < 1e-3);
        assert!((angle - 0.0035 * turret.dispersion_factor).abs() < 1e-6);
    }

    #[test]
    fn turret_stays_within_its_yaw_limits() {
        let hull = chieftain();
        let mut turret = Turret::new(AimingDef { yaw_limits: Some((-10f32.to_radians(), 12f32.to_radians())), ..chieftain_aiming() });
        // Behind on the right: as far right as it goes.
        turret.track(Vec3::new(10.0, GUN_HEIGHT, -100.0));
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw - 12f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
        // Behind on the left: back across the front, not round the back.
        turret.track(Vec3::new(-10.0, GUN_HEIGHT, -100.0));
        turret.step(0.5, &hull);
        assert!((turret.yaw - (12.0 - 15.0f32).to_radians()).abs() < 1e-5, "{}", turret.yaw);
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw + 10f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
        // The client's angles too.
        turret.hold(1.0, 0.0);
        turret.step(1.0, &hull);
        assert!((turret.yaw - 12f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
    }

    #[test]
    fn gun_depression_depends_on_turret_yaw() {
        let hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        // Far below, in front then behind.
        turret.track(Vec3::new(0.0, -100.0, 10.0));
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.pitch - 10f32.to_radians()).abs() < 1e-5);
        turret.track(Vec3::new(0.0, -100.0, -10.0));
        for _ in 0..20 {
            turret.step(1.0, &hull);
        }
        assert!((turret.pitch + 1f32.to_radians()).abs() < 1e-5);
    }

}
