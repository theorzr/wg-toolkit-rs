//! A vehicle's movement, driven by the client's `vehicle_moveWith` flags. The real
//! server simulates the vehicle's physics and streams the result; this is a stand-in
//! that only moves and turns on a flat plane at the vehicle's limits, with no terrain,
//! collision or engine model.

use glam::Vec3;


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
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub limits: MotionLimits,
}

impl VehicleMotion {

    pub fn new(position: Vec3, direction: Vec3, limits: MotionLimits) -> Self {
        Self { position, direction, speed: 0.0, flags: 0, limits }
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
        self.direction.z = wrap_angle(self.direction.z + turn * self.limits.rotation_speed * dt);

        let yaw = self.direction.z;
        self.position += Vec3::new(yaw.sin(), 0.0, yaw.cos()) * self.speed * dt;

        self.speed != 0.0 || turn != 0.0

    }

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

}
