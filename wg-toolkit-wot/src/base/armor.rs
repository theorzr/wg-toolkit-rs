//! What a shell does to the vehicle it meets: whether it pierces its armour, stops or
//! ricochets, and its damage. AP, APCR and HEAT shells follow the client's own rules,
//! `_CrosshairShotResults` in `gun_marker_ctrl.py`. HE shells follow the legacy rules
//! instead: they explode on the armour they don't pierce, and on the ground near a
//! vehicle, damaging it by the explosion's distance and the armour it goes through.

use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;

use glam::{Affine3A, Vec3};
use rand::Rng;

use crate::wot::collision::{VehicleCollision, VehicleHit, VehiclePart};
use crate::wot::item_defs::{ShellKind, ShotDef};


/// How far before the point a shell first hits a vehicle, and past it, its way through
/// is followed: `VEHICLE_TRACE_BACKWARD_LENGTH` and `VEHICLE_TRACE_FORWARD_LENGTH`.
const TRACE_BACKWARD: f32 = 0.1;
const TRACE_FORWARD: f32 = 20.0;

/// `_MAX_HIT_ANGLE_BOUND`: an angle from the normal a shell meets armour at, at most.
const MAX_HIT_ANGLE: f32 = FRAC_PI_2 - 1e-5;

/// The legacy explosion's factors, `miscParams` in the game's common `vehicle.xml`: the
/// share of the shell's damage it deals at its center, the share of that left at its
/// radius, and the damage each millimetre of armour it goes through absorbs.
const EXPLOSION_DAMAGE_FACTOR: f32 = 0.5;
const EXPLOSION_EDGE_DAMAGE_FACTOR: f32 = 0.05;
const EXPLOSION_DAMAGE_ABSORPTION_FACTOR: f32 = 1.3;

/// How far before the armour it stops on an HE shell explodes, in metres, for the
/// explosion to go through that armour.
const EXPLOSION_STANDOFF: f32 = 0.01;

/// The ground, every terrain alike until their materials are read.
pub const GROUND: GroundMaterial = GroundMaterial {
    ricochet_angle: 8.0 * std::f32::consts::PI / 180.0,
};


/// What a material of the world does to the shells meeting it: AP and APCR shells
/// skim off it at a grazing angle, which the game itself never does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundMaterial {
    /// The largest angle between a shell's path and the surface it ricochets off at.
    pub ricochet_angle: f32,
}


/// What a shell does when it hits a vehicle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Impact {
    pub outcome: Outcome,
    /// Where the outcome happens: the armour it pierces, stops on or ricochets off.
    pub hit: VehicleHit,
    /// The damage it deals, before its spread: the shell's when it pierces, its
    /// explosion's for an HE shell that doesn't.
    pub damage: f32,
    /// Whether it exploded on the armour without piercing it, an HE shell's.
    pub exploded: bool,
}

/// How a shell meets a vehicle's armour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Through its main armour: the vehicle takes its damage.
    Pierced,
    /// Stopped by its armour, a screen's or the main armour's.
    NotPierced,
    /// Off its armour, at too shallow an angle.
    Ricochet,
    /// Through its screens and tracks only, missing the main armour behind.
    Screens,
}

/// The damage an explosion deals a vehicle, see [`explosion`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Explosion {
    pub damage: f32,
    /// Where it reaches the vehicle's main armour.
    pub hit: VehicleHit,
    /// How far from the main armour it explodes, in metres.
    pub distance: f32,
}

/// What a shell does to a vehicle, placed in the world by `transforms`, which it first
/// hits at `point`, flying along `direction`, after flying `distance` metres, with this
/// share of its penetration left off its ricochets, see [`ricochet_pen`].
pub fn hit(
    shot: &ShotDef,
    distance: f32,
    pen_factor: f32,
    point: Vec3,
    direction: Vec3,
    collision: &VehicleCollision,
    transforms: &[Affine3A; 4],
    rng: &mut impl Rng,
) -> Option<Impact> {

    let direction = direction.normalize();
    let from = point - direction * TRACE_BACKWARD;
    let hits = collision.hits(transforms, from, point + direction * TRACE_FORWARD);
    let pen = randomize(shot.pen_at(distance) * pen_factor, shot.pen_randomization, rng);
    let (outcome, index) = penetrate(shot, pen, direction, &hits)?;
    let hit = hits[index];

    let damage = match outcome {
        Outcome::Pierced => shot.damage_at(distance) * hit.material.map_or(1.0, |material| material.vehicle_damage_factor),
        _ => 0.0,
    };
    let impact = Impact { outcome, hit, damage, exploded: false };
    if shot.kind() != ShellKind::HighExplosive || outcome == Outcome::Pierced {
        return Some(impact);
    }

    // An HE shell explodes on the armour that stops it, just before it.
    let center = from + direction * (hit.distance - EXPLOSION_STANDOFF);
    match explosion(shot, center, collision, transforms) {
        Some(explosion) => Some(Impact { damage: explosion.damage, exploded: true, ..impact }),
        None => Some(Impact { exploded: true, ..impact }),
    }

}

/// The damage an HE shell exploding at `center` deals a vehicle, placed in the world by
/// `transforms`, as the legacy rules had it: from half its damage where it reaches the
/// vehicle's main armour, down to [`EXPLOSION_EDGE_DAMAGE_FACTOR`] of that at its
/// radius, less what each armour on its way absorbs, screens included. None when its
/// main armour is out of reach.
pub fn explosion(shot: &ShotDef, center: Vec3, collision: &VehicleCollision, transforms: &[Affine3A; 4]) -> Option<Explosion> {

    let radius = shot.explosion_radius;
    let (closest, distance) = collision.closest_point(transforms, center, |group| group.material.is_some_and(|m| m.is_main_armor()))?;
    if distance > radius {
        return None;
    }

    // Its way to the main armour, just past it so that the armour is crossed.
    let direction = (closest - center).normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let hits = collision.hits(transforms, center, closest + direction * EXPLOSION_STANDOFF * 2.0);
    let mut seen = HashSet::new();
    let mut absorbed = 0.0;
    let mut reached = None;
    for hit in hits {
        let Some(material) = hit.material else { continue };
        if material.collide_once_only && !seen.insert((hit.part, hit.group)) {
            continue;
        }
        absorbed += EXPLOSION_DAMAGE_ABSORPTION_FACTOR * material.armor;
        if material.is_main_armor() {
            reached = Some(hit);
            break;
        }
    }

    let falloff = 1.0 - (1.0 - EXPLOSION_EDGE_DAMAGE_FACTOR) * distance / radius;
    let damage = (EXPLOSION_DAMAGE_FACTOR * shot.damage_near * falloff - absorbed).max(0.0);
    Some(Explosion { damage, hit: reached?, distance })

}

/// Whether a shell flying at `velocity` ricochets off the ground, of this material and
/// this upward `normal` where it meets it: AP and APCR shells only, within its
/// ricochet angle.
pub fn ricochets_off_ground(shot: &ShotDef, velocity: Vec3, normal: Vec3, ground: &GroundMaterial) -> bool {
    if !matches!(shot.kind(), ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr) {
        return false;
    }
    let grazing = (-velocity.normalize_or_zero().dot(normal)).clamp(-1.0, 1.0).asin();
    (0.0..=ground.ricochet_angle).contains(&grazing)
}

/// The share of its penetration a shell keeps off a ricochet: [`KINETIC_RICOCHET_PEN`]
/// for AP and APCR, all of it for HEAT. Its speed and damage stay.
pub fn ricochet_pen_factor(shot: &ShotDef) -> f32 {
    match shot.kind() {
        ShellKind::ArmorPiercing | 
        ShellKind::ArmorPiercingCr => 0.75,
        _ => 1.0,
    }
}

/// A velocity mirrored off a surface of this normal, as a shell ricochets, at the same
/// speed.
pub fn reflect(velocity: Vec3, normal: Vec3) -> Vec3 {
    velocity - 2.0 * velocity.dot(normal) * normal
}

/// Follow a shell of this penetration through the armour it meets along `direction`,
/// `hits`, nearest first, as `__shotResultDefault` does: the outcome, and the index in
/// `hits` where it happens. None when it meets no armour.
fn penetrate(shot: &ShotDef, mut pen: f32, direction: Vec3, hits: &[VehicleHit]) -> Option<(Outcome, usize)> {

    let kind = shot.kind();
    let mut seen = HashSet::<(VehiclePart, usize)>::new();
    // A HEAT shell's jet, once through a screen, and where it started.
    let mut jet: Option<f32> = None;
    let mut last = None;

    for (index, hit) in hits.iter().enumerate() {

        // A module the shell can only damage, not stopping it.
        let Some(material) = hit.material else { continue };
        if material.collide_once_only && !seen.insert((hit.part, hit.group)) {
            continue;
        }
        last = Some(index);

        if let Some(start) = jet {
            let flown = hit.distance - start;
            if flown > 0.0 {
                pen *= (1.0 - flown * shot.jet_pen_loss).max(0.0);
            }
        }

        let hit_angle_cos = if material.use_hit_angle { (-direction).dot(hit.normal).clamp(0.0, 1.0) } else { 1.0 };
        if jet.is_none() && ricochets(shot, kind, hit_angle_cos, material.armor, material.may_ricochet, material.check_caliber_for_ricochet) {
            return Some((Outcome::Ricochet, index));
        }

        pen -= penetration_armor(shot, kind, hit_angle_cos, material.armor, material.use_hit_angle, material.check_caliber_for_hit_angle_norm);
        if pen < 0.0 {
            return Some((Outcome::NotPierced, index));
        }
        if material.is_main_armor() {
            return Some((Outcome::Pierced, index));
        }
        if kind == ShellKind::HollowCharge && jet.is_none() {
            jet = Some(hit.distance);
        }

    }

    last.map(|index| (Outcome::Screens, index))

}

/// Whether a shell ricochets off armour it meets at this angle, as `_shouldRicochet`
/// decides: AP and APCR never off armour thinner than a third of their caliber.
fn ricochets(shot: &ShotDef, kind: ShellKind, hit_angle_cos: f32, armor: f32, may_ricochet: bool, check_caliber: bool) -> bool {

    let Some(ricochet_angle_cos) = shot.ricochet_angle_cos else { 
        return false 
    };

    if !may_ricochet || armor == 0.0 || hit_angle_cos > ricochet_angle_cos {
        return false;
    }

    let kind_checks_caliber = matches!(kind, ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr);
    !check_caliber || !kind_checks_caliber || armor * 3.0 >= shot.caliber
    
}

/// The thickness of armour a shell goes through, meeting it at this angle, as
/// `_computePenetrationArmor` computes it: turned toward its normal by its normalization
/// angle first, more for a caliber over twice the thickness.
fn penetration_armor(shot: &ShotDef, kind: ShellKind, hit_angle_cos: f32, armor: f32, use_hit_angle: bool, check_caliber: bool) -> f32 {

    if !use_hit_angle {
        return armor;
    }

    let mut cos = hit_angle_cos;
    let mut normalization = match kind {
        ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::ArmorPiercingHe => shot.normalization_angle,
        _ => 0.0,
    };
    if normalization > 0.0 && cos < 1.0 {
        if check_caliber && shot.caliber > armor * 2.0 && armor > 0.0 {
            normalization *= 1.4 * shot.caliber / (armor * 2.0);
        }
        let angle = cos.acos() - normalization;
        cos = if angle < 0.0 { 1.0 } else { angle.min(MAX_HIT_ANGLE).cos() };
    }
    armor / cos.max(1e-5)

}

/// A penetration varied by up to `randomization` of it either way, normally, most shots
/// within half of that.
fn randomize(pen: f32, randomization: f32, rng: &mut impl Rng) -> f32 {
    // Box–Muller.
    let (u, v): (f32, f32) = (rng.r#gen::<f32>().max(f32::MIN_POSITIVE), rng.r#gen());
    let normal = (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos();
    pen * (1.0 + (normal * 0.5).clamp(-1.0, 1.0) * randomization)
}


#[cfg(test)]
mod tests {

    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::*;
    use crate::wot::collision::{ArmorGroup, CollisionModel, Material};
    use crate::wot::items::{item_type, IntCompactDescr};

    fn shot(kind: ShellKind, caliber: f32, pen: f32) -> ShotDef {
        let ap = matches!(kind, ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::ArmorPiercingHe);
        ShotDef {
            compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id: 1 },
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
            normalization_angle: if ap { 5f32.to_radians() } else { 0.0 },
            ricochet_angle_cos: matches!(kind, ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::HollowCharge)
                .then(|| 70f32.to_radians().cos()),
            jet_pen_loss: if kind == ShellKind::HollowCharge { 0.5 } else { 0.0 },
            explosion_radius: caliber * caliber / 5555.0,
        }
    }

    fn material(armor: f32, main: bool) -> Material {
        Material {
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
    fn plate(z: f32, material: Material) -> ArmorGroup {
        let a = Vec3::new(-2.0, -2.0, z);
        let b = Vec3::new(2.0, -2.0, z);
        let c = Vec3::new(2.0, 2.0, z);
        let d = Vec3::new(-2.0, 2.0, z);
        ArmorGroup { name: "armor".into(), material_kind: 1, material: Some(material), triangles: vec![[a, b, c], [a, c, d]] }
    }

    /// A vehicle of these plates on its hull, at the origin.
    fn vehicle(plates: Vec<ArmorGroup>) -> (VehicleCollision, [Affine3A; 4]) {
        let model = |groups| CollisionModel { groups, radius: 10.0, bounds: [Vec3::splat(-5.0), Vec3::splat(5.0)] };
        let collision = VehicleCollision {
            parts: [model(vec![]), model(plates), model(vec![]), model(vec![])],
            hull_position: Vec3::ZERO,
        };
        (collision, [Affine3A::IDENTITY; 4])
    }

    /// A shell coming down `z` at this angle from the plates' normal, onto the origin's
    /// plate at `z`.
    fn impact(shot: &ShotDef, angle: f32, z: f32, plates: Vec<ArmorGroup>) -> Option<Impact> {
        let (collision, transforms) = vehicle(plates);
        let direction = Vec3::new(angle.to_radians().sin(), 0.0, -angle.to_radians().cos());
        let point = Vec3::new(0.0, 0.0, z);
        hit(shot, 100.0, 1.0, point, direction, &collision, &transforms, &mut StdRng::seed_from_u64(1))
    }

    #[test]
    fn pierces_thinner_armour_and_stops_on_thicker() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let pierced = impact(&ap, 0.0, 1.0, vec![plate(1.0, material(150.0, true))]).unwrap();
        assert_eq!((pierced.outcome, pierced.damage), (Outcome::Pierced, 400.0));
        let stopped = impact(&ap, 0.0, 1.0, vec![plate(1.0, material(250.0, true))]).unwrap();
        assert_eq!((stopped.outcome, stopped.damage), (Outcome::NotPierced, 0.0));
    }

    #[test]
    fn angled_armour_is_thicker() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        // 120 mm at 60°, 55° once normalized: about 209 mm.
        let impact = impact(&ap, 60.0, 1.0, vec![plate(1.0, material(120.0, true))]).unwrap();
        assert_eq!(impact.outcome, Outcome::NotPierced);
        assert!((penetration_armor(&ap, ShellKind::ArmorPiercing, 60f32.to_radians().cos(), 120.0, true, true) - 209.2).abs() < 0.5);
    }

    #[test]
    fn ricochets_unless_it_overmatches() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 400.0);
        let off = impact(&ap, 75.0, 1.0, vec![plate(1.0, material(50.0, true))]).unwrap();
        assert_eq!(off.outcome, Outcome::Ricochet);
        // A caliber of more than 3 times the armour; exactly 3 times still ricochets.
        assert_eq!(impact(&ap, 75.0, 1.0, vec![plate(1.0, material(40.0, true))]).unwrap().outcome, Outcome::Ricochet);
        let overmatched = impact(&ap, 75.0, 1.0, vec![plate(1.0, material(39.0, true))]).unwrap();
        assert_eq!(overmatched.outcome, Outcome::Pierced);
        // HEAT checks no caliber.
        let heat = shot(ShellKind::HollowCharge, 120.0, 400.0);
        assert_eq!(impact(&heat, 75.0, 1.0, vec![plate(1.0, material(40.0, true))]).unwrap().outcome, Outcome::Ricochet);
    }

    #[test]
    fn a_heat_jet_weakens_past_a_screen() {
        let heat = shot(ShellKind::HollowCharge, 120.0, 200.0);
        // A 10 mm screen, then 150 mm of main armour 1 m behind: half its penetration is lost.
        let plates = vec![plate(2.0, material(10.0, false)), plate(1.0, material(150.0, true))];
        assert_eq!(impact(&heat, 0.0, 2.0, plates.clone()).unwrap().outcome, Outcome::NotPierced);
        // An AP shell loses nothing.
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        assert_eq!(impact(&ap, 0.0, 2.0, plates).unwrap().outcome, Outcome::Pierced);
    }

    #[test]
    fn misses_the_main_armour_through_screens() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let impact = impact(&ap, 0.0, 1.0, vec![plate(1.0, material(10.0, false))]).unwrap();
        assert_eq!((impact.outcome, impact.damage), (Outcome::Screens, 0.0));
    }

    #[test]
    fn he_explodes_on_armour_it_does_not_pierce() {
        let he = shot(ShellKind::HighExplosive, 152.0, 40.0);
        // Half its damage, less 1.3 per millimetre of the 100 mm it stops on.
        let impact = impact(&he, 0.0, 1.0, vec![plate(1.0, material(100.0, true))]).unwrap();
        assert_eq!(impact.outcome, Outcome::NotPierced);
        assert!(impact.exploded);
        let falloff = 1.0 - 0.95 * EXPLOSION_STANDOFF / he.explosion_radius;
        assert!((impact.damage - (200.0 * falloff - 130.0)).abs() < 0.1, "{}", impact.damage);
        // Pierced, the whole damage.
        let pierced = super::tests::impact(&he, 0.0, 1.0, vec![plate(1.0, material(30.0, true))]).unwrap();
        assert_eq!((pierced.outcome, pierced.damage), (Outcome::Pierced, 400.0));
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: the Pz. I's 20 mm against the
    /// Chieftain's front, and the Chieftain's shells against the Pz. I, from 100 m.
    #[test]
    fn real_vehicles_against_each_other() {
        use crate::wot::item_defs::VehicleDef;
        use wgtk::res::fs::ResFilesystem;
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let chieftain = VehicleDef::load(&fs, "uk", "GB98_T95_FV4201_Chieftain").unwrap();
        let pz1 = VehicleDef::load(&fs, "germany", "G53_PzI").unwrap();
        let placed = |def: &VehicleDef| [
            Vec3::ZERO, def.collision.hull_position, def.aiming.turret_position, def.aiming.turret_position + def.aiming.gun_position,
        ].map(Affine3A::from_translation);
        let fire = |shot: &ShotDef, at: &VehicleDef, height: f32| {
            let transforms = placed(at);
            let from = Vec3::new(0.0, height, 100.0);
            let to = Vec3::new(0.0, height, -10.0);
            let first = at.collision.first_hit(&transforms, from, to).unwrap();
            let point = from.lerp(to, first.fraction);
            hit(shot, 100.0, 1.0, point, to - from, &at.collision, &transforms, &mut StdRng::seed_from_u64(1)).unwrap()
        };
        for shot in &pz1.shots {
            let impact = fire(shot, &chieftain, 1.2);
            assert_ne!(impact.outcome, Outcome::Pierced, "{shot:?}: {impact:?}");
            assert_eq!(impact.damage, 0.0, "{shot:?}: {impact:?}");
        }
        for shot in &chieftain.shots {
            let impact = fire(shot, &pz1, 1.0);
            assert!(impact.damage > 0.0, "{:?}: {impact:?}", shot.kind());
        }
    }

    #[test]
    fn a_ricocheted_shell_pierces_less_but_damages_as_much() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 180.0);
        let (collision, transforms) = vehicle(vec![plate(1.0, material(150.0, true))]);
        let fire = |pen_factor| hit(&ap, 100.0, pen_factor, Vec3::Z, -Vec3::Z, &collision, &transforms, &mut StdRng::seed_from_u64(1)).unwrap();
        assert_eq!((fire(1.0).outcome, fire(1.0).damage), (Outcome::Pierced, 400.0));
        // Off one ricochet, 135 mm.
        assert_eq!(ricochet_pen_factor(&ap), 0.75);
        assert_eq!(fire(ricochet_pen_factor(&ap)).outcome, Outcome::NotPierced);
        assert_eq!(ricochet_pen_factor(&shot(ShellKind::HollowCharge, 120.0, 180.0)), 1.0);
    }

    #[test]
    fn skims_off_the_ground_at_a_grazing_angle() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let at = |angle: f32| Vec3::new(angle.to_radians().cos(), -angle.to_radians().sin(), 0.0) * 1000.0;
        assert!(ricochets_off_ground(&ap, at(5.0), Vec3::Y, &GROUND));
        assert!(!ricochets_off_ground(&ap, at(12.0), Vec3::Y, &GROUND));
        // Neither HE nor HEAT, which go off on it.
        assert!(!ricochets_off_ground(&shot(ShellKind::HighExplosive, 120.0, 40.0), at(5.0), Vec3::Y, &GROUND));
        assert!(!ricochets_off_ground(&shot(ShellKind::HollowCharge, 120.0, 200.0), at(5.0), Vec3::Y, &GROUND));
        let off = reflect(at(5.0), Vec3::Y);
        assert!((off.length() - 1000.0).abs() < 0.1 && (off.normalize().y - 5f32.to_radians().sin()).abs() < 1e-4, "{off}");
    }

    #[test]
    fn an_explosion_weakens_with_distance_and_screens() {
        let he = shot(ShellKind::HighExplosive, 152.0, 40.0);
        let radius = he.explosion_radius;
        let (collision, transforms) = vehicle(vec![plate(0.0, material(20.0, true))]);
        // Halfway to its radius, 20 mm absorbing 26.
        let half = explosion(&he, Vec3::new(0.0, 0.0, radius / 2.0), &collision, &transforms).unwrap();
        assert!((half.damage - (200.0 * (1.0 - 0.95 * 0.5) - 26.0)).abs() < 0.1, "{half:?}");
        assert!(explosion(&he, Vec3::new(0.0, 0.0, radius * 1.1), &collision, &transforms).is_none());
        // A 10 mm screen in between absorbs 13 more.
        let (screened, transforms) = vehicle(vec![plate(0.0, material(20.0, true)), plate(radius / 4.0, material(10.0, false))]);
        let behind = explosion(&he, Vec3::new(0.0, 0.0, radius / 2.0), &screened, &transforms).unwrap();
        assert!((half.damage - behind.damage - 13.0).abs() < 0.1, "{behind:?}");
    }

}
