//! What a shell does to a vehicle, as the vehicle shows it and its shooter learns it:
//! where it hits it, the effect there, and the hit's flags.

use glam::Vec3;

use crate::base::ballistics::armor::{Explosion, Impact, Outcome};
use crate::game::collision::VehicleIntersection;
use crate::base::ballistics::gun::Shot;
use crate::game::schema::HitPoint;


/// What a shell does to a vehicle, see the module's documentation.
#[derive(Debug)]
pub struct ShellHit {
    /// Where it hits: where it pierces, stops or ricochets, or where its explosion beside
    /// the vehicle reaches its armour.
    pub hit: VehicleIntersection,
    /// Whether it exploded beside the vehicle, not on it.
    pub beside: bool,
    /// The `VEHICLE_HIT_EFFECT` of the point it hits.
    pub effect: u8,
    /// Its `VEHICLE_HIT_FLAGS` for the shooter, the kill aside.
    pub flags: u32,
    /// Its damage, before its spread.
    pub damage: f32,
}

impl ShellHit {

    /// A shell's impact on a vehicle, by its outcome on the armour it hit.
    pub fn of_impact(impact: &Impact) -> Self {
        let main = impact.hit.material.is_some_and(|material| material.is_main_armor());
        let (effect, flags) = match impact.outcome {
            Outcome::Pierced => (HIT_EFFECT_ARMOR_PIERCED, HIT_FLAGS_PIERCED),
            Outcome::Ricochet => (HIT_EFFECT_FINAL_RICOCHET, HIT_FLAG_DIRECT | HIT_FLAG_RICOCHET),
            Outcome::Screens => (HIT_EFFECT_ARMOR_PIERCED_NO_DAMAGE, HIT_FLAG_DIRECT | HIT_FLAG_SCREEN_PIERCED),
            Outcome::NotPierced => {
                let stopped = if main { HIT_FLAG_ARMOR_NOT_PIERCED } else { HIT_FLAG_SCREEN_NOT_PIERCED };
                let exploded = if impact.damage > 0.0 { HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION } else { 0 };
                (HIT_EFFECT_ARMOR_NOT_PIERCED, HIT_FLAG_DIRECT | stopped | exploded)
            }
        };
        Self {
            hit: impact.hit,
            beside: false,
            effect,
            flags,
            damage: impact.damage,
        }
    }

    /// An HE shell's explosion beside a vehicle.
    pub fn of_explosion(explosion: &Explosion) -> Self {
        let exploded = if explosion.damage > 0.0 { HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION } else { 0 };
        Self {
            hit: explosion.hit,
            beside: true,
            effect: HIT_EFFECT_ARMOR_NOT_PIERCED,
            flags: HIT_FLAG_EXTERNAL_EXPLOSION | exploded,
            damage: explosion.damage,
        }
    }

    /// Whether it pierced the vehicle's armour, leaving a sticker.
    pub fn pierced(&self) -> bool {
        self.effect == HIT_EFFECT_ARMOR_PIERCED
    }

    /// Its `VEHICLE_HIT_POINT`, its segment encoded over the hit part's `bounds` with
    /// `context`: its effect for `showDamageFromShot`, a sticker id for
    /// `damageStickers`. Its params are as `HitParamsEncoder` encodes them.
    pub fn point(&self, shot: &Shot, bounds: [Vec3; 2], context: u8) -> HitPoint {
        let def = &shot.def;
        let params = ((def.caliber * 10.0) as u32 & 0xFFFF) << 8 | u32::from(def.kind_index & 15) << 4 | u32::from(self.effect);
        HitPoint {
            networkID: 0,
            segment: self.hit.encode_segment(bounds, context),
            params,
        }
    }

    /// The sticker it leaves, if it pierced and its shell leaves one, as
    /// `encodeStickerIdData` encodes it (not parametrized: no prefab effects).
    pub fn sticker(&self, shot: &Shot, bounds: [Vec3; 2]) -> Option<HitPoint> {
        let sticker = shot.def.pierced_sticker.filter(|_| self.pierced())?;
        Some(self.point(shot, bounds, sticker & 0x7F))
    }

}


/// `VEHICLE_HIT_FLAGS` of a shell piercing a vehicle, as captured: a direct projectile,
/// through armour and a module.
const HIT_FLAGS_PIERCED: u32 = HIT_FLAG_DIRECT | 0x100 | 0x10;
/// `VEHICLE_HIT_FLAGS`: a direct projectile, an explosion beside the vehicle, a
/// ricochet, armour of the vehicle's own, then a screen, not pierced, a screen pierced,
/// the vehicle's own armour pierced by an explosion, and the vehicle killed.
const HIT_FLAG_DIRECT: u32 = 0x100000;
const HIT_FLAG_EXTERNAL_EXPLOSION: u32 = 0x200000;
const HIT_FLAG_RICOCHET: u32 = 0x8;
const HIT_FLAG_ARMOR_NOT_PIERCED: u32 = 0x20;
const HIT_FLAG_SCREEN_NOT_PIERCED: u32 = 0x80;
const HIT_FLAG_SCREEN_PIERCED: u32 = 0x40;
const HIT_FLAG_ARMOR_PIERCED_BY_EXPLOSION: u32 = 0x2000;
pub const HIT_FLAG_VEHICLE_KILLED: u32 = 1;
/// `VEHICLE_HIT_EFFECT`.
const HIT_EFFECT_ARMOR_PIERCED_NO_DAMAGE: u8 = 0;
const HIT_EFFECT_FINAL_RICOCHET: u8 = 2;
const HIT_EFFECT_ARMOR_NOT_PIERCED: u8 = 3;
const HIT_EFFECT_ARMOR_PIERCED: u8 = 4;
