//! A vehicle's turret and its gun ([`Turret`]): turning toward what the client aims at,
//! and the gun's dispersion.
//!
//! [`Turret::step`] turns the turret toward the aim at its rotation speed, within its
//! yaw limits, and raises the gun within its pitch limits (which depend on the yaw). The
//! aim is either a point in the world (the gun is raised so that the shell *drops onto*
//! it, [`Turret::ballistic_direction`]) or fixed angles on the hull. It also keeps the
//! gun's dispersion: it jumps up when the hull moves or turns or the turret turns, and
//! shrinks back over the aiming time. On its roof, the hull rests on its turret, which
//! doesn't turn, nor its gun, until it is back on its tracks, see [`TURRET_UPRIGHT`].

use glam::{Quat, Vec3};

use crate::game::item_defs::AimingDef;

use super::{wrap_angle, Hull};


/// How upright its hull must be for its turret and its gun to turn, as its up axis'
/// height: upside down, on its roof, it lies on its turret. Our choice.
const TURRET_UPRIGHT: f32 = 0.0;

/// A turret and its gun, turning toward what the client aims at at their top speeds,
/// the gun within its pitch limits at the turret's yaw.
#[derive(Debug, Clone)]
pub struct Turret {
    /// Relative to the hull, in radians.
    pub yaw: f32,
    /// Positive downward, in radians.
    pub pitch: f32,
    pub aiming: AimingDef,
    target: Aim,
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
        Self {
            yaw: 0.0,
            pitch: 0.0,
            aiming,
            target: Aim::Angles(0.0, 0.0),
            yaw_rate: 0.0,
            dispersion_factor: 1.0,
        }
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

        // On its roof, it lies on its turret: it keeps its aim for when it is back up.
        if (hull.rotation() * Vec3::Y).y < TURRET_UPRIGHT {
            self.yaw_rate = 0.0;
            self.step_dispersion(dt, hull);
            return;
        }

        // 1. The angles it aims at, on the hull.
        let (yaw, pitch) = self.aimed_angles(hull);

        // 2. The turret turns toward the yaw, as fast as it turns. Within its limits it
        //    only turns inside them, never round the back.
        let max_yaw_step = self.aiming.turret_rotation_speed * dt;
        let yaw_step = match self.aiming.yaw_limits {
            Some((min, max)) => wrap_angle(yaw).clamp(min, max) - self.yaw,
            None => wrap_angle(yaw - self.yaw),
        }.clamp(-max_yaw_step, max_yaw_step);
        self.yaw = wrap_angle(self.yaw + yaw_step);
        self.yaw_rate = yaw_step.abs() / dt;

        // 3. The gun toward the pitch, within its limits at the turret's new yaw.
        let pitch = pitch.clamp(self.aiming.min_pitch.at(self.yaw), self.aiming.max_pitch.at(self.yaw));
        let max_pitch_step = self.aiming.gun_rotation_speed * dt;
        self.pitch += (pitch - self.pitch).clamp(-max_pitch_step, max_pitch_step);

        // 4. Its dispersion, after how it and the hull moved.
        self.step_dispersion(dt, hull);

    }

    /// The yaw and pitch it aims at, on the hull: those held, or those that drop the
    /// shell onto the point it tracks, in the hull's frame, which the turret and gun
    /// turn in.
    fn aimed_angles(&self, hull: &Hull) -> (f32, f32) {
        match self.target {
            Aim::Point(point) => {
                let direction = self.ballistic_direction(point - gun_position(hull, self));
                let to = hull.rotation().inverse() * direction;
                let horizontal = (to.x * to.x + to.z * to.z).sqrt();
                (to.x.atan2(to.z), -to.y.atan2(horizontal))
            }
            Aim::Angles(yaw, pitch) => (yaw, pitch),
        }
    }

    /// The direction to fire the shell in for it to reach `to`, relative to the gun:
    /// the lower of the two trajectories, or 45° up when out of reach.
    ///
    /// With the shell's speed `v`, the gravity `g`, the horizontal distance `d` and the
    /// height `h` of the target above the gun, the elevation `θ` that reaches it solves
    /// `h = d·tan θ − g·d² / (2·v²·cos² θ)`, whose lower root is
    /// `tan θ = (v² − √(v⁴ − g·(g·d² + 2·h·v²))) / (g·d)`. When the root is imaginary the
    /// target is out of reach and the gun takes the 45° that goes the farthest.
    fn ballistic_direction(&self, to: Vec3) -> Vec3 {
        let speed = self.aiming.shot_speed;
        let gravity = self.aiming.shot_gravity;
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

    /// The dispersion's part of a step, as the client's `getOwnVehicleShotDispersionAngle`
    /// computes it: the ideal factor grows with the hull's speed, its turning and the
    /// turret's turning, and the actual one jumps up to it but only shrinks back over the
    /// aiming time. No crew, equipment or shot bonuses.
    ///
    /// ```text
    ///   ideal  = √( 1 + (speed·kₘ)² + (yaw_rate·kᵣ)² + (turret_rate·kₜ)² )
    ///   factor = ideal                                    if ideal ≥ factor    (jumps up)
    ///          = ideal + (factor − ideal)·e^(−dt / aiming_time)    otherwise   (settles)
    /// ```
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
        let dispersion = self.aiming.dispersion_angle * self.dispersion_factor;
        (gun_position(hull, self), direction * self.aiming.shot_speed, dispersion)
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


#[cfg(test)]
mod tests {

    use super::super::fixtures::{chieftain, chieftain_aiming, GUN_HEIGHT};
    use super::*;

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
        hull.set_direction(Vec3::new(0.05, 0.08, 1.2));
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
        let aiming = AimingDef {
            yaw_limits: Some((-10f32.to_radians(), 12f32.to_radians())),
            ..chieftain_aiming()
        };
        let mut turret = Turret::new(aiming);
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
    fn turret_holds_on_its_roof() {
        // Upside down, it lies on its turret, which turns neither to a point nor to angles.
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(std::f32::consts::PI, 0.0, 0.0));
        let mut turret = Turret::new(chieftain_aiming());
        turret.track(Vec3::new(100.0, GUN_HEIGHT, 0.0));
        turret.step(1.0, &hull);
        turret.hold(1.0, 0.1);
        turret.step(1.0, &hull);
        assert_eq!((turret.yaw, turret.pitch), (0.0, 0.0));
        // On its side it still turns, and back on its tracks, toward its aim.
        hull.set_direction(Vec3::new(std::f32::consts::FRAC_PI_2 - 0.1, 0.0, 0.0));
        turret.step(1.0, &hull);
        assert!(turret.yaw > 0.0, "{}", turret.yaw);
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
