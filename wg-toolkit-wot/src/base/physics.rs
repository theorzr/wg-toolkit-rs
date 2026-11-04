//! A vehicle as the server simulates it: its hull driven by the client's
//! `vehicle_moveWith` flags, its turret turning toward what the client aims at. The real
//! server simulates the vehicle's physics and streams the result; this is a stand-in
//! that moves and turns at the vehicle's limits and rests on the terrain, with no
//! collision, slope resistance or engine model.

use glam::{Affine3A, Quat, Vec3};

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
/// The engine's relative RPM at idle, and just after shifting up, as captured.
const IDLE_RPM: f32 = 0.2;
const SHIFT_RPM: f32 = 0.3;
/// The shares of the top speed the gearbox shifts up at, as captured: 4 gears.
const GEAR_SHIFTS: [f32; 3] = [0.27, 0.52, 0.8];
const HULL_HALF_WIDTH: f32 = 1.7;

/// A vehicle's movement limits, see `VehicleDef`.
#[derive(Debug, Clone, Copy)]
pub struct MotionLimits {
    pub speed_forward: f32,
    pub speed_backward: f32,
    pub rotation_speed: f32,
}

/// A live vehicle: its hull and its turret.
#[derive(Debug, Clone)]
pub struct Vehicle {
    pub hull: Hull,
    pub turret: Turret,
}

impl Vehicle {

    pub fn new(position: Vec3, direction: Vec3, limits: MotionLimits, aiming: AimingDef) -> Self {
        Self { hull: Hull::new(position, direction, limits), turret: Turret::new(aiming) }
    }

    /// Advance by `dt` seconds: the hull moves and rests on the terrain, if any, then the
    /// turret turns. Returns whether the hull moved or turned.
    pub fn step(&mut self, dt: f32, terrain: Option<&Terrain>) -> bool {
        let moved = self.hull.step(dt);
        if let Some(terrain) = terrain.filter(|_| moved) {
            self.hull.settle(terrain);
        }
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
    /// Turning speed, in rad/s, positive to the right.
    pub yaw_rate: f32,
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub limits: MotionLimits,
}

impl Hull {

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

    /// Advance by `dt` seconds. Returns whether the hull moved or turned.
    pub fn step(&mut self, dt: f32) -> bool {

        use movement_flag::*;

        let target = self.target_speed();
        // Braking applies when slowing down or reversing, accelerating otherwise.
        let rate = if target.abs() < self.speed.abs() || target * self.speed < 0.0 { BRAKING } else { ACCELERATION };
        let delta = (target - self.speed).clamp(-rate * dt, rate * dt);
        self.speed += delta;

        let mut turn = match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
            ROTATE_LEFT => -1.0,
            ROTATE_RIGHT => 1.0,
            _ => 0.0,
        };
        // Backing up steers like a car: left swings the rear to the left.
        if self.flags & BACKWARD != 0 {
            turn = -turn;
        }
        self.yaw_rate = turn * self.limits.rotation_speed;
        self.direction.z = wrap_angle(self.direction.z + self.yaw_rate * dt);

        let yaw = self.direction.z;
        self.position += Vec3::new(yaw.sin(), 0.0, yaw.cos()) * self.speed * dt;

        self.speed != 0.0 || turn != 0.0

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
    /// follows the turn rate in first gear.
    pub fn engine(&self) -> (u8, f32) {
        let top_speed = if self.speed < 0.0 { self.limits.speed_backward } else { self.limits.speed_forward };
        let share = (self.speed.abs() / top_speed.max(0.1)).min(1.0);
        let turn_share = (self.yaw_rate.abs() / self.limits.rotation_speed.max(0.01)).min(1.0);
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
        // The inner track goes slower, the outer one faster.
        let turning = self.yaw_rate * HULL_HALF_WIDTH;
        let (left, right) = (self.speed + turning, self.speed - turning);
        let (roll, pitch, yaw) = (self.direction.x, self.direction.y, self.direction.z);
        pack_range(self.engine().1, 0.0, 1.2, 8) << 56
            | pack_range(right, -15.0, 30.0, 8) << 48
            | pack_range(left, -15.0, 30.0, 8) << 40
            | pack_angle(roll, 13) << 27
            | pack_range(pitch, -FRAC_PI_2, FRAC_PI_2, 12) << 15
            | pack_angle(yaw, 15)
    }

    /// Rest on the terrain: at its height, rolled and pitched with its slope. Off the
    /// terrain, the hull stays as it is.
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
        let max_yaw_step = self.aiming.turret_rotation_speed * dt;
        let yaw_step = wrap_angle(yaw - self.yaw).clamp(-max_yaw_step, max_yaw_step);
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

    fn chieftain() -> Hull {
        Hull::new(Vec3::ZERO, Vec3::ZERO, MotionLimits {
            speed_forward: 40.0 / 3.6,
            speed_backward: 15.0 / 3.6,
            rotation_speed: 28f32.to_radians(),
        })
    }

    #[test]
    fn drives_forward_along_heading() {
        let mut hull = chieftain();
        hull.direction.z = 2.771;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..100 {
            hull.step(0.1);
        }
        // At top speed after accelerating, heading south-east as captured.
        assert!((hull.speed - 40.0 / 3.6).abs() < 1e-3);
        assert!(hull.position.x > 0.0 && hull.position.z < 0.0);
        assert_eq!(hull.position.y, 0.0);
    }

    #[test]
    fn stops_and_turns() {
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        hull.step(1.0);
        hull.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..10 {
            hull.step(0.1);
        }
        assert_eq!(hull.speed, 0.0);
        assert!((hull.direction.z - 28f32.to_radians()).abs() < 1e-4);
        // Backing up while steering right turns the hull the other way.
        hull.flags = movement_flag::BACKWARD | movement_flag::ROTATE_RIGHT;
        hull.step(0.1);
        assert!(hull.yaw_rate < 0.0);
        // Standing still with no input reports no movement.
        hull.flags = 0;
        hull.speed = 0.0;
        assert!(!hull.step(0.1));
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
        let top = hull.limits.speed_forward;
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
