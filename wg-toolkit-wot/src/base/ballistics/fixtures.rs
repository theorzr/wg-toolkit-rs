//! Shells and armour for the ballistics' tests, and the bots'.

use glam::{Affine3A, Vec3};

use crate::game::collision::{ArmorGroup, ArmorMaterial, CollisionModel, VehicleCollision};
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

/// Armour this thick, main, its damage let through, or a screen, letting none.
pub fn material(armor: f32, main: bool) -> ArmorMaterial {
    ArmorMaterial {
        armor,
        vehicle_damage_factor: if main { 1.0 } else { 0.0 },
        use_hit_angle: true,
        may_ricochet: true,
        check_caliber_for_ricochet: true,
        check_caliber_for_hit_angle_norm: true,
        collide_once_only: false,
    }
}

/// A square plate across `z`, 4 m wide, in a group of its own.
pub fn plate(z: f32, material: ArmorMaterial) -> ArmorGroup {
    let a = Vec3::new(-2.0, -2.0, z);
    let b = Vec3::new(2.0, -2.0, z);
    let c = Vec3::new(2.0, 2.0, z);
    let d = Vec3::new(-2.0, 2.0, z);
    ArmorGroup {
        name: "armor".into(),
        material_kind: 1,
        material: Some(material),
        triangles: vec![[a, b, c], [a, c, d]],
    }
}

/// A vehicle of these plates on its hull, at the origin.
pub fn vehicle(plates: Vec<ArmorGroup>) -> (VehicleCollision, [Affine3A; 4]) {
    let collision = VehicleCollision {
        parts: [model(vec![]), model(plates), model(vec![]), model(vec![])],
        hull_position: Vec3::ZERO,
    };
    (collision, [Affine3A::IDENTITY; 4])
}

/// A part's collision model of these groups.
fn model(groups: Vec<ArmorGroup>) -> CollisionModel {
    CollisionModel {
        groups,
        radius: 10.0,
        bounds: [Vec3::splat(-5.0), Vec3::splat(5.0)],
    }
}
