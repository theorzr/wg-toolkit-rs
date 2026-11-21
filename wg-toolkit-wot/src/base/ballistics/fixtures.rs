//! Shells for the ballistics' tests.

use crate::game::items::{item_type, IntCompactDescr};
use crate::game::item_defs::{ShellKind, ShotDef};


/// A shell of this kind, caliber and penetration, the same at any distance: 800 m/s,
/// 720 m of range, 400 damage, never varied, 5° of normalization for AP, APCR and APHE,
/// a 70° ricochet angle for AP, APCR and HEAT, a HEAT jet losing half its penetration
/// each metre, and the legacy explosion's radius.
pub fn shot(kind: ShellKind, caliber: f32, pen: f32) -> ShotDef {

    let normalizes = matches!(
        kind,
        ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::ArmorPiercingHe,
    );
    let ricochets = matches!(
        kind,
        ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::HollowCharge,
    );

    ShotDef {
        compact_descr: IntCompactDescr {
            item_type: item_type::SHELL,
            nation: 5,
            id: 1,
        },
        kind_index: kind as u8,
        caliber,
        effects_index: 0,
        pierced_sticker: None,
        prefab_effects_index: None,
        speed: 800.0,
        gravity: 9.81,
        max_distance: 720.0,
        damage_near: 400.0,
        damage_far: 400.0,
        pen_near: pen,
        pen_far: pen,
        pen_randomization: 0.0,
        normalization_angle: if normalizes { 5f32.to_radians() } else { 0.0 },
        ricochet_angle_cos: ricochets.then(|| 70f32.to_radians().cos()),
        jet_pen_loss: if kind == ShellKind::HollowCharge { 0.5 } else { 0.0 },
        explosion_radius: caliber * caliber / 5555.0,
    }

}
