//! Packed position/direction types used by [`crate::app::client::element`]'s
//! `AVATAR_UPDATE_*` and `CreateEntity` elements -- lives directly under [`crate::app`]
//! rather than under `client` since the same packed formats are shared by BigWorld's
//! wire protocol in general (e.g. [`crate::app::cell::element`]'s own avatar-update
//! elements), not just the client-facing ones.
//!
//! The formulas below are confirmed against the leaked BigWorld 14.4.1 SDK
//! (`re-work/bigworld-src-14.4.1/programming/bigworld/lib/network/msgtypes.hpp` and its
//! `msgtypes.ipp` template bodies, plus [`super::bit`] for the underlying MSB-first bit
//! packing) -- not yet confirmed against a live WoT capture, but this project's own byte
//! counts for every `AVATAR_UPDATE_*` element (validated earlier against the real
//! client's handler table) match this source's bit widths exactly, which is strong
//! indirect confirmation.

use std::io::{self, Read, Write};
use std::f32::consts::PI;

use glam::Vec3;

use crate::util::io::{WgReadExt, WgWriteExt};
use crate::net::codec::SimpleCodec;

use super::bit::{BitReader, BitWriter};


/// BigWorld's `packFloat<EXPONENT_BITS, MANTISSA_BITS>` (`network/msgtypes.ipp`): packs a
/// sign bit, then a biased exponent, then a rounded mantissa. The `+2.0` trick on the
/// absolute value keeps the stored (biased-by-128) exponent non-negative without having
/// to special-case subnormals -- see [`unpack_float`] for the exact inverse.
fn pack_float(value: f32, exponent_bits: u32, mantissa_bits: u32, writer: &mut BitWriter) {

    let max_mantissa = (1u32 << mantissa_bits) - 1;
    let max_exponent = (1u32 << exponent_bits) - 1;

    writer.add(1, value.to_bits() >> 31);

    let bits = (value.abs() + 2.0).to_bits();
    let mut exponent = (bits >> 23) & 0xFF;
    debug_assert!(exponent >= 128);
    exponent -= 128;

    let mantissa_raw = bits & 0x007F_FFFF;
    let mut mantissa = mantissa_raw >> (23 - mantissa_bits);
    let next_bit = (mantissa_raw >> (22 - mantissa_bits)) & 1;

    if next_bit == 1 {
        if mantissa != max_mantissa {
            mantissa += 1;
        } else {
            exponent += 1;
            mantissa = 0;
        }
    }

    if exponent > max_exponent {
        exponent = max_exponent;
        mantissa = max_mantissa;
    }

    writer.add(exponent_bits, exponent);
    writer.add(mantissa_bits, mantissa);

}

/// Inverse of [`pack_float`].
fn unpack_float(reader: &mut BitReader, exponent_bits: u32, mantissa_bits: u32) -> f32 {
    let sign = reader.get(1) << 31;
    let mut bits = (reader.get(exponent_bits) | 0x80) << 23;
    bits |= reader.get(mantissa_bits) << (23 - mantissa_bits);
    let value = f32::from_bits(bits) - 2.0;
    f32::from_bits(value.to_bits() | sign)
}

/// BigWorld's `angleToInt<8>`/`intToAngle<8>` (`network/msgtypes.ipp`): a full-range
/// angle in `[-pi, pi)` packed into a single signed byte.
fn angle_to_i8(angle: f32) -> i8 {
    ((angle * 128.0 / PI + 0.5).floor() as i32) as i8
}

fn i8_to_angle(compressed: i8) -> f32 {
    f32::from(compressed) * (PI / 128.0)
}

/// BigWorld's `halfAngleToInt<8>`/`intToHalfAngle<8>`: a half-range angle in
/// `[-pi/2, pi/2)` packed into a single signed byte -- used for pitch wherever
/// `HALFPITCH` is `true` (BigWorld's default for `PackedYawPitchRoll`).
fn half_angle_to_i8(angle: f32) -> i8 {
    ((angle * 254.0 / PI + 0.5).floor().clamp(-128.0, 127.0) as i32) as i8
}

fn i8_to_half_angle(compressed: i8) -> f32 {
    f32::from(compressed) * (PI / 254.0)
}

/// A position packed into 5 bytes (BigWorld's `PackedXYZ`, default template params: 3
/// exponent + 8 mantissa bits for `x`/`z`, 4 exponent + 11 mantissa bits for `y`), used
/// by the `FullPos` family of `AVATAR_UPDATE_*` elements. `CreateEntity` doesn't use this
/// compressed form -- its own position is a plain unpacked [`Vec3`].
#[derive(Debug, Clone, Copy)]
pub struct PackedXyz(pub [u8; 5]);

impl PackedXyz {

    /// Decode into `(x, y, z)`: `x`/`z` are offsets in metres from the entity's tracked
    /// reference position (see `RelativePositionReference`/`RelativePosition`, neither
    /// decoded by this project yet) scaled by `xz_scale` (the space's
    /// `CreateCellPlayer::packed_xz_scale`); `y` is absolute and needs neither scale nor
    /// reference (`Y-values in off-Ground updates are always absolute`, `msgtypes.hpp`).
    pub fn unpack(&self, xz_scale: f32) -> Vec3 {
        let mut reader = BitReader::new(&self.0);
        let x = unpack_float(&mut reader, 3, 8) * xz_scale;
        let z = unpack_float(&mut reader, 3, 8) * xz_scale;
        let y = unpack_float(&mut reader, 4, 11);
        Vec3::new(x, y, z)
    }

    /// Inverse of [`Self::unpack`]: `offset.x`/`offset.z` are offsets from the reference
    /// position (divided by `xz_scale` before packing), `offset.y` is absolute.
    pub fn pack(offset: Vec3, xz_scale: f32) -> Self {
        let mut data = [0u8; 5];
        let mut writer = BitWriter::new(&mut data);
        pack_float(offset.x / xz_scale, 3, 8, &mut writer);
        pack_float(offset.z / xz_scale, 3, 8, &mut writer);
        pack_float(offset.y, 4, 11, &mut writer);
        Self(data)
    }

}

impl SimpleCodec for PackedXyz {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.0)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut data = [0; 5];
        read.read_exact(&mut data)?;
        Ok(Self(data))
    }
}

/// A ground-relative position packed into 3 bytes (BigWorld's `PackedXZ`, default
/// template params: 3 exponent + 8 mantissa bits, same as [`PackedXyz`]'s `x`/`z`), used
/// by the `OnGround` family of `AVATAR_UPDATE_*` elements -- only the horizontal offset
/// is sent on the wire, `y` is assumed to come from the terrain at that point.
#[derive(Debug, Clone, Copy)]
pub struct PackedXz(pub [u8; 3]);

impl PackedXz {

    /// Decode into `(dx, dz)`, offsets in metres from the entity's tracked reference
    /// position scaled by `xz_scale` -- see [`PackedXyz::unpack`].
    pub fn unpack(&self, xz_scale: f32) -> (f32, f32) {
        let mut reader = BitReader::new(&self.0);
        let x = unpack_float(&mut reader, 3, 8) * xz_scale;
        let z = unpack_float(&mut reader, 3, 8) * xz_scale;
        (x, z)
    }

    /// Inverse of [`Self::unpack`].
    pub fn pack(dx: f32, dz: f32, xz_scale: f32) -> Self {
        let mut data = [0u8; 3];
        let mut writer = BitWriter::new(&mut data);
        pack_float(dx / xz_scale, 3, 8, &mut writer);
        pack_float(dz / xz_scale, 3, 8, &mut writer);
        Self(data)
    }

}

impl SimpleCodec for PackedXz {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.0)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut data = [0; 3];
        read.read_exact(&mut data)?;
        Ok(Self(data))
    }
}

/// Yaw, pitch and roll each packed into one byte (BigWorld's compressed `Angle`
/// encoding, one byte per component since `YAWBITS`/`PITCHBITS`/`ROLLBITS` all default
/// to 8). Also reused, with `half_pitch = false`, by `CreateEntity::direction`.
#[derive(Debug, Clone, Copy)]
pub struct PackedYawPitchRoll(pub [u8; 3]);

impl PackedYawPitchRoll {

    /// Decode into `(yaw, pitch, roll)` radians. `half_pitch` selects whether pitch was
    /// packed over `[-pi/2, pi/2)` (`true`, BigWorld's default `YAWPITCHROLL_HALFPITCH`,
    /// used by the `AVATAR_UPDATE_*_YAW_PITCH_ROLL` messages) or the full `[-pi, pi)`
    /// range (`false`, confirmed used by `ServerConnection::createEntity`'s explicit
    /// `PackedYawPitchRoll</* HALFPITCH */ false>`, see `CreateEntity`).
    pub fn unpack(&self, half_pitch: bool) -> (f32, f32, f32) {
        let yaw = i8_to_angle(self.0[0] as i8);
        let pitch = if half_pitch { i8_to_half_angle(self.0[1] as i8) } else { i8_to_angle(self.0[1] as i8) };
        let roll = i8_to_angle(self.0[2] as i8);
        (yaw, pitch, roll)
    }

    /// Inverse of [`Self::unpack`].
    pub fn pack(yaw: f32, pitch: f32, roll: f32, half_pitch: bool) -> Self {
        let pitch = if half_pitch { half_angle_to_i8(pitch) } else { angle_to_i8(pitch) };
        Self([angle_to_i8(yaw) as u8, pitch as u8, angle_to_i8(roll) as u8])
    }

}

impl SimpleCodec for PackedYawPitchRoll {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.0)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut data = [0; 3];
        read.read_exact(&mut data)?;
        Ok(Self(data))
    }
}

/// Yaw and pitch each packed into one byte, both full-range (`YAWPITCH_HALFPITCH` is
/// `false`, unlike [`PackedYawPitchRoll`]'s default), see [`PackedYawPitchRoll`].
#[derive(Debug, Clone, Copy)]
pub struct PackedYawPitch(pub [u8; 2]);

impl PackedYawPitch {

    /// Decode into `(yaw, pitch)` radians.
    pub fn unpack(&self) -> (f32, f32) {
        (i8_to_angle(self.0[0] as i8), i8_to_angle(self.0[1] as i8))
    }

    /// Inverse of [`Self::unpack`].
    pub fn pack(yaw: f32, pitch: f32) -> Self {
        Self([angle_to_i8(yaw) as u8, angle_to_i8(pitch) as u8])
    }

}

impl SimpleCodec for PackedYawPitch {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.0)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut data = [0; 2];
        read.read_exact(&mut data)?;
        Ok(Self(data))
    }
}

/// Yaw packed into one byte, full-range, see [`PackedYawPitchRoll`].
#[derive(Debug, Clone, Copy)]
pub struct PackedYaw(pub u8);

impl PackedYaw {

    /// Decode into radians.
    pub fn unpack(&self) -> f32 {
        i8_to_angle(self.0 as i8)
    }

    /// Inverse of [`Self::unpack`].
    pub fn pack(yaw: f32) -> Self {
        Self(angle_to_i8(yaw) as u8)
    }

}

impl SimpleCodec for PackedYaw {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(self.0)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        Ok(Self(read.read_u8()?))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xyz_roundtrip() {
        // Tolerance scales with magnitude: this is a floating-point-style compression
        // (3-bit exponent + 8-bit mantissa for x/z), so absolute error roughly doubles
        // per exponent step -- checked against `getError` in `msgtypes.ipp`.
        for &(x, y, z) in &[(0.0f32, 0.0f32, 0.0f32), (12.34, -5.6, 78.9), (-100.0, 30.0, -0.001), (509.9, 511.9, -509.9)] {
            let scale = 1.0f32;
            let packed = PackedXyz::pack(Vec3::new(x, y, z), scale);
            let unpacked = packed.unpack(scale);
            let tol = |v: f32| (v.abs() / 256.0).max(0.02);
            assert!((unpacked.x - x).abs() < tol(x), "x: {} vs {}", unpacked.x, x);
            assert!((unpacked.y - y).abs() < tol(y), "y: {} vs {}", unpacked.y, y);
            assert!((unpacked.z - z).abs() < tol(z), "z: {} vs {}", unpacked.z, z);
        }
    }

    #[test]
    fn xz_roundtrip() {
        let packed = PackedXz::pack(12.34, -56.78, 1.0);
        let (x, z) = packed.unpack(1.0);
        assert!((x - 12.34).abs() < 0.05);
        assert!((z - (-56.78)).abs() < 0.05);
    }

    #[test]
    fn ypr_roundtrip_half_pitch() {
        let packed = PackedYawPitchRoll::pack(1.0, 0.5, -2.0, true);
        let (yaw, pitch, roll) = packed.unpack(true);
        assert!((yaw - 1.0).abs() < 0.03, "yaw {yaw}");
        assert!((pitch - 0.5).abs() < 0.03, "pitch {pitch}");
        assert!((roll - (-2.0)).abs() < 0.03, "roll {roll}");
    }

    #[test]
    fn ypr_roundtrip_full_pitch() {
        let packed = PackedYawPitchRoll::pack(1.0, -2.5, 3.0, false);
        let (yaw, pitch, roll) = packed.unpack(false);
        assert!((yaw - 1.0).abs() < 0.03, "yaw {yaw}");
        assert!((pitch - (-2.5)).abs() < 0.03, "pitch {pitch}");
        assert!((roll - 3.0).abs() < 0.03, "roll {roll}");
    }

    #[test]
    fn yaw_pitch_roundtrip() {
        let packed = PackedYawPitch::pack(1.2, -0.7);
        let (yaw, pitch) = packed.unpack();
        assert!((yaw - 1.2).abs() < 0.03, "yaw {yaw}");
        assert!((pitch - (-0.7)).abs() < 0.03, "pitch {pitch}");
    }

    #[test]
    fn yaw_roundtrip() {
        let packed = PackedYaw::pack(-1.5);
        assert!((packed.unpack() - (-1.5)).abs() < 0.03);
    }
}
