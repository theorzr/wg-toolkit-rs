//! A vehicle's movement, driven by the client's `vehicle_moveWith` flags. The real
//! server simulates the vehicle's physics and streams the result; this is a stand-in
//! that moves and turns at the vehicle's limits and rests on the terrain, with no
//! collision, slope resistance or engine model.

use glam::Vec3;

use wgtk::space::terrain::Terrain;

use crate::wot::item_defs::AimingDef;


/// The `vehicle_moveWith` flags, `MOVEMENT_FLAGS` in the client's `Avatar.py`.
pub mod movement_flag {
    pub const FORWARD: u8 = 1;
    pub const BACKWARD: u8 = 2;
    pub const ROTATE_LEFT: u8 = 4;
    pub const ROTATE_RIGHT: u8 = 8;
    pub const CRUISE_CONTROL50: u8 = 16;
    pub const CRUISE_CONTROL25: u8 = 32;
}

/// How quickly the speed reaches what the flags ask for, in m/s².
const ACCELERATION: f32 = 4.0;
const BRAKING: f32 = 10.0;

/// Where the hull is sampled to tilt it with the ground: half its length and width, a
/// typical tank's rather than the vehicle's own.
const HULL_HALF_LENGTH: f32 = 3.0;
const HULL_HALF_WIDTH: f32 = 1.7;

/// A vehicle's movement limits, see `VehicleDef`.
#[derive(Debug, Clone, Copy)]
pub struct MotionLimits {
    pub speed_forward: f32,
    pub speed_backward: f32,
    pub rotation_speed: f32,
}

/// A moving vehicle.
#[derive(Debug, Clone)]
pub struct VehicleMotion {
    pub position: Vec3,
    /// Roll, pitch and yaw, in radians, the order position updates carry them. A yaw
    /// of 0 faces +Z, and turning right increases it.
    pub direction: Vec3,
    /// Signed speed along the vehicle's heading, in m/s.
    pub speed: f32,
    /// Turning speed, in rad/s, positive to the right.
    pub yaw_rate: f32,
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub limits: MotionLimits,
}

impl VehicleMotion {

    pub fn new(position: Vec3, direction: Vec3, limits: MotionLimits) -> Self {
        Self { position, direction, speed: 0.0, yaw_rate: 0.0, flags: 0, limits }
    }

    /// The speed the flags ask for: the top speed in their direction, reduced by cruise
    /// control.
    fn target_speed(&self) -> f32 {
        use movement_flag::*;
        let share = if self.flags & CRUISE_CONTROL25 != 0 {
            0.25
        } else if self.flags & CRUISE_CONTROL50 != 0 {
            0.5
        } else {
            1.0
        };
        if self.flags & FORWARD != 0 {
            self.limits.speed_forward * share
        } else if self.flags & BACKWARD != 0 {
            -self.limits.speed_backward * share
        } else {
            0.0
        }
    }

    /// Advance by `dt` seconds. Returns whether the vehicle moved or turned.
    pub fn step(&mut self, dt: f32) -> bool {

        use movement_flag::*;

        let target = self.target_speed();
        // Braking applies when slowing down or reversing, accelerating otherwise.
        let rate = if target.abs() < self.speed.abs() || target * self.speed < 0.0 { BRAKING } else { ACCELERATION };
        let delta = (target - self.speed).clamp(-rate * dt, rate * dt);
        self.speed += delta;

        let turn = match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
            ROTATE_LEFT => -1.0,
            ROTATE_RIGHT => 1.0,
            _ => 0.0,
        };
        self.yaw_rate = turn * self.limits.rotation_speed;
        self.direction.z = wrap_angle(self.direction.z + self.yaw_rate * dt);

        let yaw = self.direction.z;
        self.position += Vec3::new(yaw.sin(), 0.0, yaw.cos()) * self.speed * dt;

        self.speed != 0.0 || turn != 0.0

    }

    /// The hull's orientation and its tracks' speeds, as the avatar's
    /// `ownVehicleAuxPhysicsData` carries them for the client to aim from and animate the
    /// tracks with, see `WoT.packAuxVehiclePhysicsData`. From the lowest bit: yaw on 15
    /// bits over a turn, pitch on 12 bits over ±π/2, roll on 13 bits over a turn, the left
    /// then right track speeds on 8 bits each over -15..30 m/s, and 8 bits the client
    /// ignores, over 0..1.2, left at 0.
    pub fn aux_physics_data(&self) -> u64 {
        use std::f32::consts::FRAC_PI_2;
        // The inner track goes slower, the outer one faster.
        let turning = self.yaw_rate * HULL_HALF_WIDTH;
        let (left, right) = (self.speed + turning, self.speed - turning);
        let (roll, pitch, yaw) = (self.direction.x, self.direction.y, self.direction.z);
        pack_range(0.0, 0.0, 1.2, 8) << 56
            | pack_range(right, -15.0, 30.0, 8) << 48
            | pack_range(left, -15.0, 30.0, 8) << 40
            | pack_angle(roll, 13) << 27
            | pack_range(pitch, -FRAC_PI_2, FRAC_PI_2, 12) << 15
            | pack_angle(yaw, 15)
    }

    /// Rest on the terrain: at its height, rolled and pitched with its slope. Off the
    /// terrain, the vehicle stays as it is.
    pub fn settle(&mut self, terrain: &Terrain) {

        let height = |offset: Vec3| {
            let at = self.position + offset;
            terrain.height_at(at.x, at.z)
        };
        let yaw = self.direction.z;
        let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos()) * HULL_HALF_LENGTH;
        let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin()) * HULL_HALF_WIDTH;

        let (Some(center), Some(front), Some(back), Some(right_side), Some(left_side)) =
            (height(Vec3::ZERO), height(forward), height(-forward), height(right), height(-right)) else {
            return;
        };
        self.position.y = center;
        // Rolled positively with its right side up, pitched positively nose down.
        self.direction.x = ((right_side - left_side) / (2.0 * HULL_HALF_WIDTH)).atan();
        self.direction.y = -((front - back) / (2.0 * HULL_HALF_LENGTH)).atan();

    }

}

/// How high above the vehicle's origin its gun's sight is, for the pitch toward a
/// point: a typical tank's rather than the vehicle's own.
const GUN_HEIGHT: f32 = 2.0;

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
        Self { yaw: 0.0, pitch: 0.0, target: Aim::Angles(0.0, 0.0), aiming }
    }

    /// Turn toward a point in the world.
    pub fn track(&mut self, point: Vec3) {
        self.target = Aim::Point(point);
    }

    /// Turn to angles relative to the hull, and stay there.
    pub fn hold(&mut self, yaw: f32, pitch: f32) {
        self.target = Aim::Angles(yaw, pitch);
    }

    /// Advance by `dt` seconds, the vehicle being at the given position and yaw.
    pub fn step(&mut self, dt: f32, position: Vec3, hull_yaw: f32) {
        let (yaw, pitch) = match self.target {
            Aim::Point(point) => {
                let to = point - position;
                let horizontal = (to.x * to.x + to.z * to.z).sqrt();
                (to.x.atan2(to.z) - hull_yaw, -(to.y - GUN_HEIGHT).atan2(horizontal))
            }
            Aim::Angles(yaw, pitch) => (yaw, pitch),
        };
        let max_yaw_step = self.aiming.turret_rotation_speed * dt;
        self.yaw = wrap_angle(self.yaw + wrap_angle(yaw - self.yaw).clamp(-max_yaw_step, max_yaw_step));
        let pitch = pitch.clamp(self.aiming.min_pitch.at(self.yaw), self.aiming.max_pitch.at(self.yaw));
        let max_pitch_step = self.aiming.gun_rotation_speed * dt;
        self.pitch += (pitch - self.pitch).clamp(-max_pitch_step, max_pitch_step);
    }

    /// The angles as the `gunAnglesPacked` property carries them, see `encodeGunAngles`:
    /// the yaw on 10 bits over a turn, the pitch on 6 bits over its absolute limits.
    pub fn packed(&self) -> u16 {
        use std::f32::consts::{PI, TAU};
        let yaw = ((1024.0 * (self.yaw + PI) / TAU).round() as u16) & 1023;
        let (min_pitch, max_pitch) = self.aiming.absolute_pitch_limits();
        let t = if max_pitch == min_pitch { 0.0 } else { ((self.pitch - min_pitch) / (max_pitch - min_pitch)).clamp(0.0, 1.0) };
        yaw << 6 | (63.0 * t).round() as u16
    }

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
fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI { wrapped - TAU } else { wrapped }
}


#[cfg(test)]
mod tests {

    use super::*;

    fn chieftain() -> VehicleMotion {
        VehicleMotion::new(Vec3::ZERO, Vec3::ZERO, MotionLimits {
            speed_forward: 40.0 / 3.6,
            speed_backward: 15.0 / 3.6,
            rotation_speed: 28f32.to_radians(),
        })
    }

    #[test]
    fn drives_forward_along_heading() {
        let mut motion = chieftain();
        motion.direction.z = 2.771;
        motion.flags = movement_flag::FORWARD;
        for _ in 0..100 {
            motion.step(0.1);
        }
        // At top speed after accelerating, heading south-east as captured.
        assert!((motion.speed - 40.0 / 3.6).abs() < 1e-3);
        assert!(motion.position.x > 0.0 && motion.position.z < 0.0);
        assert_eq!(motion.position.y, 0.0);
    }

    #[test]
    fn stops_and_turns() {
        let mut motion = chieftain();
        motion.flags = movement_flag::FORWARD;
        motion.step(1.0);
        motion.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..10 {
            motion.step(0.1);
        }
        assert_eq!(motion.speed, 0.0);
        assert!((motion.direction.z - 28f32.to_radians()).abs() < 1e-4);
        // Standing still with no input reports no movement.
        motion.flags = 0;
        assert!(!motion.step(0.1));
    }

    fn chieftain_aiming() -> AimingDef {
        use crate::wot::item_defs::PitchCurve;
        let curve = |points: &[(f32, f32)]| PitchCurve(points.iter().map(|&(t, a)| (t * std::f32::consts::TAU, a.to_radians())).collect());
        AimingDef {
            turret_rotation_speed: 30f32.to_radians(),
            gun_rotation_speed: 26f32.to_radians(),
            aiming_time: 2.1,
            dispersion_turret_rotation: 0.0,
            dispersion_after_shot: 0.0,
            dispersion_movement: 0.0,
            dispersion_rotation: 0.0,
            min_pitch: curve(&[(0.0, -20.0), (1.0, -20.0)]),
            // 10° of depression at the front, -1° over the engine deck at the back.
            max_pitch: curve(&[(0.0, 10.0), (0.4, 10.0), (0.45, -1.0), (0.55, -1.0), (0.6, 10.0), (1.0, 10.0)]),
        }
    }

    #[test]
    fn turret_turns_toward_point() {
        let mut turret = Turret::new(chieftain_aiming());
        // Straight to the right of a hull facing +Z, level with the gun.
        turret.track(Vec3::new(100.0, GUN_HEIGHT, 0.0));
        turret.step(1.0, Vec3::ZERO, 0.0);
        assert!((turret.yaw - 30f32.to_radians()).abs() < 1e-5);
        for _ in 0..10 {
            turret.step(1.0, Vec3::ZERO, 0.0);
        }
        assert!((turret.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(turret.pitch.abs() < 1e-5);
        // The point stays put in the world as the vehicle drives past it.
        turret.step(1.0, Vec3::new(0.0, 0.0, 200.0), 0.0);
        assert!(turret.yaw > std::f32::consts::FRAC_PI_2);
        // Yaw 0 and the middle of the pitch range, as `encodeGunAngles` packs them.
        turret.hold(0.0, -5f32.to_radians());
        for _ in 0..10 {
            turret.step(1.0, Vec3::ZERO, 0.0);
        }
        assert_eq!(turret.packed(), 512 << 6 | 32);
    }

    #[test]
    fn aux_physics_data_packs_as_captured() {
        // A captured value, standing still after driving: yaw 2.7713, pitch and roll
        // near 0, both tracks at 1.76 m/s, the ignored last byte 0xaf.
        let captured: u64 = 0xaf5f5f813c00f875;
        let mut motion = chieftain();
        motion.direction = Vec3::new(0.0299, 0.0012, 2.7713);
        motion.speed = 1.76;
        assert_eq!(motion.aux_physics_data(), captured & 0x00ff_ffff_ffff_ffff);
    }

    #[test]
    fn gun_depression_depends_on_turret_yaw() {
        let mut turret = Turret::new(chieftain_aiming());
        // Far below, in front then behind.
        turret.track(Vec3::new(0.0, -100.0, 10.0));
        for _ in 0..10 {
            turret.step(1.0, Vec3::ZERO, 0.0);
        }
        assert!((turret.pitch - 10f32.to_radians()).abs() < 1e-5);
        turret.track(Vec3::new(0.0, -100.0, -10.0));
        for _ in 0..20 {
            turret.step(1.0, Vec3::ZERO, 0.0);
        }
        assert!((turret.pitch + 1f32.to_radians()).abs() < 1e-5);
    }

}
