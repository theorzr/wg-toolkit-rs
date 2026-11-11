//! What a hull's client is told of it beyond where it is: its engine's mode, gear and
//! RPM, which its engine sound and its tracks' scroll follow, and the aux physics data
//! its own client aims from and animates its tracks with.

use super::Hull;


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

impl Hull {

    /// The vehicle's `engineMode`, which the client's track scroll follows: idle (1)
    /// without flags, accelerating (3) until its tracks are at the speed the flags ask
    /// for, then cruising (2), and the flags themselves.
    pub fn engine_mode(&self) -> [u8; 2] {

        let mode = if self.flags == 0 {
            1
        } else if (self.target_speed() - self.track_speed()).abs() > 0.01 {
            3
        } else {
            2
        };

        [mode, self.flags]

    }

    /// The engine's gear and its RPM relative to its maximum, which the client's engine
    /// sound follows, by its tracks' speeds: off the ground they run free and it revs, and
    /// landing pulls them to the ground's speed, as the captured server does. In neutral
    /// (gear 0) at idle RPM when still, else the speed's share of the top speed picks one
    /// of [`GEAR_SHIFTS`]'s gears, the RPM going from [`SHIFT_RPM`] to 1 across it;
    /// turning in place, the RPM follows the turn rate in first gear, against its
    /// fastest, on firm ground.
    pub fn engine(&self) -> (u8, f32) {

        // Still, or turning in place.
        let speed = self.track_speed();
        let [left, right] = self.tracks;
        let turning = (left - right) / (2.0 * self.mobility.carrying_point.x.max(1e-3));
        let top_speed = if speed < 0.0 { self.mobility.speed_backward } else { self.mobility.speed_forward };
        let share = (speed.abs() / top_speed.max(0.1)).min(1.0);
        let turn_share = (turning.abs() / self.mobility.rotation_speed[0].max(0.01)).min(1.0);
        if share < 0.02 {
            if turn_share < 0.02 {
                return (0, IDLE_RPM);
            }
            return (1, SHIFT_RPM + (1.0 - SHIFT_RPM) * turn_share);
        }

        // Moving, in the gear its speed is in, as far into it as it is.
        let gear = GEAR_SHIFTS.iter().take_while(|&&shift| share >= shift).count();
        let low = if gear == 0 { 0.0 } else { GEAR_SHIFTS[gear - 1] };
        let high = GEAR_SHIFTS.get(gear).copied().unwrap_or(1.0);
        let in_gear = ((share - low) / (high - low)).clamp(0.0, 1.0);
        (gear as u8 + 1, SHIFT_RPM + (1.0 - SHIFT_RPM) * in_gear)

    }

    /// How fast its tracks go together, the middle of their speeds, in m/s.
    fn track_speed(&self) -> f32 {
        let [left, right] = self.tracks;
        (left + right) / 2.0
    }

    /// The hull's orientation and its tracks' speeds, as the avatar's
    /// `ownVehicleAuxPhysicsData` carries them for the client to aim from and animate the
    /// tracks with, see `WoT.packAuxVehiclePhysicsData`. From the lowest bit: yaw on 15
    /// bits over a turn, pitch on 12 bits over ±π/2, roll on 13 bits over a turn, the left
    /// then right track speeds on 8 bits each over -15..30 m/s, and the engine's relative
    /// RPM on 8 bits over 0..1.2, see [`Self::engine`].
    pub fn aux_physics_data(&self) -> u64 {

        use std::f32::consts::FRAC_PI_2;

        let [left, right] = self.tracks;

        let direction = self.direction();
        let roll = direction.x;
        let pitch = direction.y;
        let yaw = direction.z;
        pack_range(self.engine().1, 0.0, 1.2, 8) << 56
            | pack_range(right, -15.0, 30.0, 8) << 48
            | pack_range(left, -15.0, 30.0, 8) << 40
            | pack_angle(roll, 13) << 27
            | pack_range(pitch, -FRAC_PI_2, FRAC_PI_2, 12) << 15
            | pack_angle(yaw, 15)

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


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use super::super::fixtures::chieftain;
    use super::*;

    #[test]
    fn engine_shifts_up_with_speed() {
        let mut hull = chieftain();
        assert_eq!(hull.engine(), (0, IDLE_RPM));
        let top = hull.mobility.speed_forward;
        let mut last = (0, 0.0);
        // Below 2% of the top speed, it counts as still.
        for step in 2..=100 {
            hull.tracks = [top * step as f32 / 100.0; 2];
            let (gear, rpm) = hull.engine();
            assert!(gear >= last.0 && (SHIFT_RPM..=1.0).contains(&rpm), "{gear} {rpm} at {:?}", hull.tracks);
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
        hull.set_direction(Vec3::new(0.0299, 0.0012, 2.7713));
        hull.tracks = [1.76; 2];
        // Without the RPM, which depends on the engine model.
        assert_eq!(hull.aux_physics_data() & 0x00ff_ffff_ffff_ffff, captured & 0x00ff_ffff_ffff_ffff);
    }

}
