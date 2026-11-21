//! What a shell does to a vehicle it hits: whether it pierces its armour, stops on it or
//! ricochets off it, and its damage, see [`hit`]. AP, APCR and HEAT shells follow the
//! client's own rules, `_CrosshairShotResults` in `gun_marker_ctrl.py`. HE shells follow
//! the legacy rules instead: they explode on the armour they don't pierce, and on the
//! ground near a vehicle, damaging it by the explosion's distance and the armour it
//! goes through, see [`explosion`].
//!
//! A shell's way through a vehicle, see [`penetrate`]:
//!
//! ```text
//!   its penetration    by its speed, see `speed`, varied by its randomization
//!   each armour met    nearest first, along its way
//!     a module         damaged, not stopping it
//!     too shallow      a ricochet, unless overmatched: AP and APCR, armour < caliber / 3
//!     its thickness    along its way, turned toward the normal by its normalization
//!     thicker          stopped: not pierced
//!     main armour      pierced: its damage
//!     a screen         gone through, a HEAT jet weakening as it flies on behind it
//!   only screens       through its screens, missing the main armour
//! ```

use std::f32::consts::{FRAC_PI_2, TAU};
use std::collections::HashSet;

use glam::{Affine3A, Vec3};
use rand::Rng;

use crate::game::collision::{ArmorMaterial, VehicleCollision, VehicleIntersection, VehiclePart};
use crate::game::item_defs::{ShellKind, ShotDef};
use crate::base::debug::{self, DebugChannel};

use super::speed;


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

/// The share of a shell's penetration randomization a gun marker is unsure within,
/// either way, see [`indicate`]: `_PP_RANDOM_ADJUSTMENT_MIN` and `_MAX`.
const INDICATION_RANDOMIZATION: f32 = 0.5;


/// What a player's gun marker shows of a shot at a vehicle, see [`indicate`]: red, then
/// yellow, then green, `SHOT_RESULT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Indication {
    /// Red: it doesn't pierce, or ricochets.
    NotPierced,
    /// Yellow: it may pierce.
    LittlePierced,
    /// Green: it pierces.
    GreatPierced,
}

/// What a shell does when it hits a vehicle.
#[derive(Debug, Clone, PartialEq)]
pub struct Impact {
    /// Whether it pierced, stopped, ricocheted, or went through screens only.
    pub outcome: ImpactOutcome,
    /// Where the outcome happens: the armour it pierces, stops on or ricochets off.
    pub hit: VehicleIntersection,
    /// The damage it deals, before its spread: the shell's when it pierces, its
    /// explosion's for an HE shell that doesn't.
    pub damage: f32,
    /// Whether it exploded on the armour without piercing it, an HE shell's.
    pub exploded: bool,
    /// How it went, line by line, to be debugged when [`DebugChannel::Shells`] is drawn,
    /// else empty: its penetration, each armour it met, and its explosion.
    pub trace: Vec<String>,
}

/// How a shell meets a vehicle's armour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactOutcome {
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
#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    /// The damage it deals, before its spread, the armour on its way taken.
    pub damage: f32,
    /// Where it reaches the vehicle's main armour.
    pub hit: VehicleIntersection,
    /// How far from the main armour it explodes, in metres.
    pub distance: f32,
    /// How it went, to be debugged, as [`Impact::trace`]: its falloff and the damage the
    /// armour absorbed.
    pub trace: Vec<String>,
}


// -- A hit --

/// What a shell does to a vehicle, placed in the world by `transforms`, which it first
/// hits at `point`, flying along `direction`, after flying `distance` metres, at
/// `speed_factor` of its speed from its gun, see [`speed::pen_at_speed`].
pub fn hit(
    shot: &ShotDef,
    distance: f32,
    speed_factor: f32,
    point: Vec3,
    direction: Vec3,
    collision: &VehicleCollision,
    transforms: &[Affine3A; 4],
    rng: &mut impl Rng,
) -> Option<Impact> {

    // Its way through the vehicle, from just before where it first hits it.
    let direction = direction.normalize();
    let from = point - direction * TRACE_BACKWARD;
    let hits = collision.hits(transforms, from, point + direction * TRACE_FORWARD);

    // Its penetration, by its speed, varied as the game varies it.
    let average = speed::pen_at_speed(shot, speed_factor);
    let pen = randomize(average, shot.pen_randomization, rng);

    let debugging = debug::draws(DebugChannel::Shells);
    let mut trace = Vec::new();
    if debugging {
        trace.push(debug_pen(shot, distance, speed_factor, average, pen));
    }

    let (outcome, index) = penetrate(shot, pen, direction, &hits, debugging.then_some(&mut trace))?;
    let hit = hits[index];

    // Its damage once pierced, as much as the armour it pierced lets through.
    let damage = match outcome {
        ImpactOutcome::Pierced => {
            let factor = hit.material.map_or(1.0, |material| material.vehicle_damage_factor);
            shot.damage_at(distance) * factor
        }
        _ => 0.0,
    };

    let impact = Impact {
        outcome,
        hit,
        damage,
        exploded: false,
        trace,
    };
    if shot.kind() != ShellKind::HighExplosive || outcome == ImpactOutcome::Pierced {
        return Some(impact);
    }

    // An HE shell explodes on the armour that stops it, just before it.
    let center = from + direction * (hit.distance - EXPLOSION_STANDOFF);
    let Some(explosion) = explosion(shot, center, collision, transforms) else {
        return Some(Impact {
            exploded: true,
            ..impact
        });
    };

    let mut trace = impact.trace;
    trace.extend(explosion.trace);
    Some(Impact {
        damage: explosion.damage,
        exploded: true,
        trace,
        ..impact
    })

}

/// The damage an HE shell exploding at `center` deals a vehicle, placed in the world by
/// `transforms`, as the legacy rules had it: from half its damage where it reaches the
/// vehicle's main armour, down to [`EXPLOSION_EDGE_DAMAGE_FACTOR`] of that at its
/// radius, less what each armour on its way absorbs, screens included. None when its
/// main armour is out of reach.
pub fn explosion(
    shot: &ShotDef,
    center: Vec3,
    collision: &VehicleCollision,
    transforms: &[Affine3A; 4],
) -> Option<Explosion> {

    // The nearest of its main armour, within its radius.
    let radius = shot.explosion_radius;
    let (closest, distance) = collision.closest_point(transforms, center, |group| {
        group.material.is_some_and(|material| material.is_main_armor())
    })?;
    if distance > radius {
        return None;
    }

    // Its way there, just past it so that the armour is crossed.
    let direction = (closest - center).normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let hits = collision.hits(transforms, center, closest + direction * EXPLOSION_STANDOFF * 2.0);

    // Each armour on the way absorbs some of it, up to the main armour.
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

    // Weaker the farther away, down to its edge's share at its radius.
    let falloff = 1.0 - (1.0 - EXPLOSION_EDGE_DAMAGE_FACTOR) * distance / radius;
    let damage = (EXPLOSION_DAMAGE_FACTOR * shot.damage_near * falloff - absorbed).max(0.0);

    let mut trace = Vec::new();
    if debug::draws(DebugChannel::Shells) {
        trace.push(format!(
            "Exploded {distance:.2} m from the armour, {radius:.2} m radius: \
            {:.0}% of half its damage, {absorbed:.0} absorbed",
            falloff * 100.0,
        ));
    }

    Some(Explosion {
        damage,
        hit: reached?,
        distance,
        trace,
    })

}

/// What a player's gun marker shows of a shell fired from `from` at `to` into a
/// vehicle, placed in the world by `transforms`, as `__shotResultDefault` in the
/// client's `gun_marker_ctrl.py` works it out: the shell's penetration at the distance,
/// as the game's table gives it, against the thickness of each armour on its way, in
/// percents of it, its main armour's within half its randomization either way unsure.
/// None when its way meets no armour of it, as the client's undefined result.
pub fn indicate(
    shot: &ShotDef,
    from: Vec3,
    to: Vec3,
    collision: &VehicleCollision,
    transforms: &[Affine3A; 4],
) -> Option<Indication> {

    let direction = (to - from).normalize_or_zero();
    let hits = collision.hits(transforms, from, to + direction * TRACE_FORWARD);
    let full = shot.pen_at(hits.first()?.distance);
    let sure = 100.0 * (1.0 - shot.pen_randomization * INDICATION_RANDOMIZATION);
    let unsure = 100.0 * (1.0 + shot.pen_randomization * INDICATION_RANDOMIZATION);

    let mut pen = full;
    let mut jet: Option<f32> = None;
    let mut seen = HashSet::<(VehiclePart, u32)>::new();
    for hit in &hits {

        // A jet loses some of its penetration from the armour it last went through.
        if let Some(start) = jet {
            pen *= 1.0 - (hit.distance - start).max(0.0) * shot.jet_pen_loss;
        }

        let Some(material) = hit.material else { continue };
        if !seen.insert((hit.part, hit.material_kind)) && material.collide_once_only {
            continue;
        }

        let hit_angle_cos = if material.use_hit_angle {
            (-direction).dot(hit.normal).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if jet.is_none() && ricochets(shot, &material, hit_angle_cos) {
            return Some(Indication::NotPierced);
        }

        // Its thickness, in percents of the shell's penetration, past what it pierced.
        let mut percent = 1000.0;
        if pen > 0.0 && full > 0.0 {
            let thickness = penetration_armor(shot, &material, hit_angle_cos);
            percent = 100.0 + (thickness - pen) / full * 100.0;
            pen -= thickness;
        }
        if material.is_main_armor() {
            return Some(if percent <= sure {
                Indication::GreatPierced
            } else if percent < unsure {
                Indication::LittlePierced
            } else {
                Indication::NotPierced
            });
        }

        if pen <= 0.0 {
            break;
        }
        if shot.kind() == ShellKind::HollowCharge {
            jet = Some(hit.distance + material.armor * 0.001);
        }

    }
    Some(Indication::NotPierced)

}

/// Follow a shell of this penetration through the armour it meets along `direction`,
/// `hits`, nearest first, as `__shotResultDefault` does, see the module's
/// documentation: the outcome, and the index in `hits` where it happens, each armour
/// met told in `trace`, if any, see [`Impact::trace`]. None when it meets no armour.
fn penetrate(
    shot: &ShotDef,
    mut pen: f32,
    direction: Vec3,
    hits: &[VehicleIntersection],
    mut trace: Option<&mut Vec<String>>,
) -> Option<(ImpactOutcome, usize)> {

    let mut seen = HashSet::<(VehiclePart, usize)>::new();
    // Where a HEAT shell's jet started, once through a screen.
    let mut jet: Option<f32> = None;
    let mut last = None;

    for (index, hit) in hits.iter().enumerate() {

        // A module the shell can only damage, or armour it already went through.
        let Some(material) = hit.material else { continue };
        if material.collide_once_only && !seen.insert((hit.part, hit.group)) {
            continue;
        }
        last = Some(index);

        // A jet loses some of its penetration for each metre it flies.
        if let Some(start) = jet {
            let flown = hit.distance - start;
            if flown > 0.0 {
                let loss = (1.0 - flown * shot.jet_pen_loss).max(0.0);
                pen *= loss;
                if let Some(trace) = &mut trace {
                    trace.push(format!("Jet over {flown:.2} m: {pen:.0} mm pen left ({:.0}% kept)", loss * 100.0));
                }
            }
        }

        // Off it at too shallow an angle, but a jet, which goes on whatever its way.
        let hit_angle_cos = if material.use_hit_angle {
            (-direction).dot(hit.normal).clamp(0.0, 1.0)
        } else {
            1.0
        };

        if jet.is_none() && ricochets(shot, &material, hit_angle_cos) {
            if let Some(trace) = &mut trace {
                trace.push(debug_ricochet(shot, hit, &material, hit_angle_cos));
            }
            return Some((ImpactOutcome::Ricochet, index));
        }

        // Through it if it pierces its thickness along its way.
        let thickness = penetration_armor(shot, &material, hit_angle_cos);
        if let Some(trace) = &mut trace {
            trace.push(debug_armor(shot, hit, &material, hit_angle_cos, thickness, pen));
        }

        pen -= thickness;
        if pen < 0.0 {
            return Some((ImpactOutcome::NotPierced, index));
        }
        if material.is_main_armor() {
            return Some((ImpactOutcome::Pierced, index));
        }

        // Through a screen, a HEAT shell goes on as a jet.
        if shot.kind() == ShellKind::HollowCharge && jet.is_none() {
            jet = Some(hit.distance);
        }

    }

    last.map(|index| (ImpactOutcome::Screens, index))

}

/// Whether a shell ricochets off armour of this material it meets at this angle, as
/// `_shouldRicochet` decides: past its ricochet angle, unless it overmatches the armour,
/// see [`overmatches`].
fn ricochets(shot: &ShotDef, material: &ArmorMaterial, hit_angle_cos: f32) -> bool {

    let Some(ricochet_angle_cos) = shot.ricochet_angle_cos else {
        return false
    };

    if !material.may_ricochet || material.armor == 0.0 || hit_angle_cos > ricochet_angle_cos {
        return false;
    }

    !overmatches(shot, material)

}

/// The thickness of armour of this material a shell goes through, meeting it at this
/// angle, as `_computePenetrationArmor` computes it: turned toward its normal by its
/// normalization angle first, see [`normalization`].
fn penetration_armor(shot: &ShotDef, material: &ArmorMaterial, hit_angle_cos: f32) -> f32 {

    if !material.use_hit_angle {
        return material.armor;
    }

    let mut cos = hit_angle_cos;
    let normalization = normalization(shot, material);
    if normalization > 0.0 && cos < 1.0 {
        let angle = cos.acos() - normalization;
        cos = if angle < 0.0 { 1.0 } else { angle.min(MAX_HIT_ANGLE).cos() };
    }
    material.armor / cos.max(1e-5)

}


// -- Its trace, to be debugged --

/// A line of a shell's trace, see [`Impact::trace`]: its kind, the distance it flew,
/// its speed, of its speed from its gun, when it pierces by it, its penetration rolled,
/// then its average there.
fn debug_pen(shot: &ShotDef, distance: f32, speed_factor: f32, average: f32, pen: f32) -> String {

    let kind = match shot.kind() {
        ShellKind::HollowCharge => "HEAT",
        ShellKind::HighExplosive => "HE",
        ShellKind::ArmorPiercing => "AP",
        ShellKind::ArmorPiercingHe => "APHE",
        ShellKind::ArmorPiercingCr => "APCR",
        ShellKind::Smoke => "Smoke",
    };

    if speed::is_kinetic(shot) {
        format!(
            "{kind} from {distance:.0} m at {:.0}% speed: {pen:.0} mm pen ({average:.0} average)",
            speed_factor * 100.0,
        )
    } else {
        format!("{kind} from {distance:.0} m: {pen:.0} mm pen ({average:.0} average)")
    }

}

/// A line of a shell's trace, see [`Impact::trace`]: armour it ricocheted off, at this
/// angle from its normal, over the shell's ricochet angle.
fn debug_ricochet(
    shot: &ShotDef,
    hit: &VehicleIntersection,
    material: &ArmorMaterial,
    hit_angle_cos: f32,
) -> String {
    let ricochet = shot.ricochet_angle_cos.map_or(0.0, |cos| cos.acos().to_degrees());
    format!(
        "{} at {:.1}°: ricochet (over {ricochet:.0}°)",
        debug_armor_name(hit, material),
        hit_angle_cos.acos().to_degrees(),
    )
}

/// A line of a shell's trace, see [`Impact::trace`]: armour it meets, at this angle from
/// its normal, then normalized, and `thickness` that way, then the penetration it has
/// left of `pen`, or how much it fell short.
fn debug_armor(
    shot: &ShotDef,
    hit: &VehicleIntersection,
    material: &ArmorMaterial,
    hit_angle_cos: f32,
    thickness: f32,
    pen: f32,
) -> String {

    let name = debug_armor_name(hit, material);
    let left = pen - thickness;
    let result = if left < 0.0 {
        format!("stopped, {:.0} mm short", -left)
    } else {
        format!("{left:.0} mm pen left")
    };

    if !material.use_hit_angle {
        return format!("{name}: {result}");
    }

    let angle = hit_angle_cos.acos();
    let normalization = normalization(shot, material);
    let normalized = if normalization > 0.0 {
        format!(" ({:.1}° normalized)", (angle - normalization).max(0.0).to_degrees())
    } else {
        String::new()
    };

    format!(
        "{name} at {:.1}°{normalized}{}: {thickness:.0} mm effective, {result}",
        angle.to_degrees(),
        if overmatches(shot, material) { ", overmatched" } else { "" },
    )

}

/// The armour a shell meets, to be debugged: its part, whether a screen, and its
/// thickness.
fn debug_armor_name(hit: &VehicleIntersection, material: &ArmorMaterial) -> String {
    let screen = if material.is_main_armor() { "" } else { " screen" };
    format!("{:?}{screen} {:.0} mm", hit.part, material.armor)
}


// -- The rules shared --

/// The angle a shell turns toward the normal of armour of this material, in radians:
/// its normalization angle, an AP, APCR or APHE shell's only, as `_SHELL_EXTRA_DATA`
/// says, more for a caliber over twice the armour's thickness.
fn normalization(shot: &ShotDef, material: &ArmorMaterial) -> f32 {

    let normalizes = matches!(
        shot.kind(),
        ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr | ShellKind::ArmorPiercingHe,
    );
    if !normalizes {
        return 0.0;
    }

    // More for a caliber over twice the armour's thickness.
    let normalization = shot.normalization_angle;
    let armor = material.armor;
    let big_caliber = material.check_caliber_for_hit_angle_norm && armor > 0.0 && shot.caliber > armor * 2.0;
    if normalization > 0.0 && big_caliber {
        normalization * 1.4 * shot.caliber / (armor * 2.0)
    } else {
        normalization
    }

}

/// Whether a shell is too big to ricochet off armour of this material: an AP or APCR
/// shell over three times its thickness.
fn overmatches(shot: &ShotDef, material: &ArmorMaterial) -> bool {
    let checks_caliber = matches!(shot.kind(), ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr);
    checks_caliber && material.check_caliber_for_ricochet && material.armor * 3.0 < shot.caliber
}

/// A penetration varied by up to `randomization` of it either way, normally, most shots
/// within half of that.
fn randomize(pen: f32, randomization: f32, rng: &mut impl Rng) -> f32 {

    // A normal draw, by Box–Muller.
    let u = rng.r#gen::<f32>().max(f32::MIN_POSITIVE);
    let v = rng.r#gen::<f32>();
    let normal = (-2.0 * u.ln()).sqrt() * (TAU * v).cos();

    // Half of it to the randomization, cut at it.
    pen * (1.0 + (normal * 0.5).clamp(-1.0, 1.0) * randomization)

}


#[cfg(test)]
mod tests {

    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use wgtk::res::fs::ResFilesystem;

    use crate::game::collision::ArmorGroup;
    use crate::game::item_defs::VehicleDef;

    use super::super::fixtures::{material, plate, shot, vehicle};
    use super::*;

    /// A shell coming down `z` at this angle from the plates' normal, at full speed,
    /// onto the origin's plate at `z`.
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
        assert_eq!((pierced.outcome, pierced.damage), (ImpactOutcome::Pierced, 400.0));
        let stopped = impact(&ap, 0.0, 1.0, vec![plate(1.0, material(250.0, true))]).unwrap();
        assert_eq!((stopped.outcome, stopped.damage), (ImpactOutcome::NotPierced, 0.0));
    }

    #[test]
    fn pierces_less_slower_but_damages_as_much() {

        let ap = shot(ShellKind::ArmorPiercing, 120.0, 180.0);
        let (collision, transforms) = vehicle(vec![plate(1.0, material(150.0, true))]);
        let mut rng = StdRng::seed_from_u64(1);

        let fast = hit(&ap, 100.0, 1.0, Vec3::Z, -Vec3::Z, &collision, &transforms, &mut rng).unwrap();
        assert_eq!((fast.outcome, fast.damage), (ImpactOutcome::Pierced, 400.0));

        // At 82% of its speed, 135 mm.
        let slow = hit(&ap, 100.0, 0.818, Vec3::Z, -Vec3::Z, &collision, &transforms, &mut rng).unwrap();
        assert_eq!(slow.outcome, ImpactOutcome::NotPierced);

    }

    #[test]
    fn indicates_as_a_players_gun_marker() {

        // 200 mm of penetration, 25% randomized: sure up to 175 mm, unsure up to 225 mm.
        let mut ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        ap.pen_randomization = 0.25;
        let (from, to) = (Vec3::new(0.0, 0.0, 100.0), Vec3::ZERO);
        for (armor, shown) in [
            (170.0, Indication::GreatPierced),
            (200.0, Indication::LittlePierced),
            (230.0, Indication::NotPierced),
        ] {
            let (collision, transforms) = vehicle(vec![plate(1.0, material(armor, true))]);
            assert_eq!(indicate(&ap, from, to, &collision, &transforms), Some(shown), "{armor} mm");
        }

        // Off it at a grazing angle, and nothing past its side.
        let (collision, transforms) = vehicle(vec![plate(1.0, material(50.0, true))]);
        let grazing = Vec3::new(100.0, 0.0, 1.0 + 100.0 * 10f32.to_radians().tan());
        let off = indicate(&ap, grazing, Vec3::new(0.0, 0.0, 1.0), &collision, &transforms);
        assert_eq!(off, Some(Indication::NotPierced));
        let aside = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(indicate(&ap, aside + Vec3::Z * 100.0, aside, &collision, &transforms), None);

    }

    #[test]
    fn angled_armour_is_thicker() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        // 120 mm at 60°, 55° once normalized: about 209 mm.
        let impact = impact(&ap, 60.0, 1.0, vec![plate(1.0, material(120.0, true))]).unwrap();
        assert_eq!(impact.outcome, ImpactOutcome::NotPierced);
        let thickness = penetration_armor(&ap, &material(120.0, true), 60f32.to_radians().cos());
        assert!((thickness - 209.2).abs() < 0.5, "{thickness}");
    }

    #[test]
    fn ricochets_unless_it_overmatches() {

        let ap = shot(ShellKind::ArmorPiercing, 120.0, 400.0);
        let off = impact(&ap, 75.0, 1.0, vec![plate(1.0, material(50.0, true))]).unwrap();
        assert_eq!(off.outcome, ImpactOutcome::Ricochet);

        // A caliber of more than 3 times the armour; exactly 3 times still ricochets.
        let thrice = impact(&ap, 75.0, 1.0, vec![plate(1.0, material(40.0, true))]).unwrap();
        assert_eq!(thrice.outcome, ImpactOutcome::Ricochet);
        let overmatched = impact(&ap, 75.0, 1.0, vec![plate(1.0, material(39.0, true))]).unwrap();
        assert_eq!(overmatched.outcome, ImpactOutcome::Pierced);

        // HEAT checks no caliber.
        let heat = shot(ShellKind::HollowCharge, 120.0, 400.0);
        let off = impact(&heat, 75.0, 1.0, vec![plate(1.0, material(40.0, true))]).unwrap();
        assert_eq!(off.outcome, ImpactOutcome::Ricochet);

    }

    #[test]
    fn a_heat_jet_weakens_past_a_screen() {

        // A 10 mm screen, then 150 mm of main armour 1 m behind: half its penetration
        // lost.
        let heat = shot(ShellKind::HollowCharge, 120.0, 200.0);
        let plates = vec![plate(2.0, material(10.0, false)), plate(1.0, material(150.0, true))];
        assert_eq!(impact(&heat, 0.0, 2.0, plates.clone()).unwrap().outcome, ImpactOutcome::NotPierced);

        // An AP shell loses nothing.
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        assert_eq!(impact(&ap, 0.0, 2.0, plates).unwrap().outcome, ImpactOutcome::Pierced);

    }

    #[test]
    fn misses_the_main_armour_through_screens() {
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let impact = impact(&ap, 0.0, 1.0, vec![plate(1.0, material(10.0, false))]).unwrap();
        assert_eq!((impact.outcome, impact.damage), (ImpactOutcome::Screens, 0.0));
    }

    #[test]
    fn he_explodes_on_armour_it_does_not_pierce() {

        // Half its damage, less 1.3 per millimetre of the 100 mm it stops on.
        let he = shot(ShellKind::HighExplosive, 152.0, 40.0);
        let exploded = impact(&he, 0.0, 1.0, vec![plate(1.0, material(100.0, true))]).unwrap();
        assert_eq!(exploded.outcome, ImpactOutcome::NotPierced);
        assert!(exploded.exploded);
        let falloff = 1.0 - 0.95 * EXPLOSION_STANDOFF / he.explosion_radius;
        assert!((exploded.damage - (200.0 * falloff - 130.0)).abs() < 0.1, "{}", exploded.damage);

        // Pierced, the whole damage.
        let pierced = impact(&he, 0.0, 1.0, vec![plate(1.0, material(30.0, true))]).unwrap();
        assert_eq!((pierced.outcome, pierced.damage), (ImpactOutcome::Pierced, 400.0));

    }

    #[test]
    fn an_explosion_weakens_with_distance_and_screens() {

        // Halfway to its radius, 20 mm absorbing 26.
        let he = shot(ShellKind::HighExplosive, 152.0, 40.0);
        let radius = he.explosion_radius;
        let (collision, transforms) = vehicle(vec![plate(0.0, material(20.0, true))]);
        let half = explosion(&he, Vec3::new(0.0, 0.0, radius / 2.0), &collision, &transforms).unwrap();
        assert!((half.damage - (200.0 * (1.0 - 0.95 * 0.5) - 26.0)).abs() < 0.1, "{half:?}");
        assert!(explosion(&he, Vec3::new(0.0, 0.0, radius * 1.1), &collision, &transforms).is_none());

        // A 10 mm screen in between absorbs 13 more.
        let plates = vec![plate(0.0, material(20.0, true)), plate(radius / 4.0, material(10.0, false))];
        let (screened, transforms) = vehicle(plates);
        let behind = explosion(&he, Vec3::new(0.0, 0.0, radius / 2.0), &screened, &transforms).unwrap();
        assert!((half.damage - behind.damage - 13.0).abs() < 0.1, "{behind:?}");

    }

    /// A vehicle's parts placed at the origin, as its definition puts them.
    fn placed(def: &VehicleDef) -> [Affine3A; 4] {
        [
            Vec3::ZERO,
            def.collision.hull_position,
            def.aiming.turret_position,
            def.aiming.turret_position + def.aiming.gun_position,
        ].map(Affine3A::from_translation)
    }

    /// A shell fired from 100 m ahead at this height onto a vehicle, at full speed.
    fn fire_at(shot: &ShotDef, at: &VehicleDef, height: f32) -> Impact {
        let transforms = placed(at);
        let from = Vec3::new(0.0, height, 100.0);
        let to = Vec3::new(0.0, height, -10.0);
        let first = at.collision.first_hit(&transforms, from, to).unwrap();
        let point = from.lerp(to, first.fraction);
        let mut rng = StdRng::seed_from_u64(1);
        hit(shot, 100.0, 1.0, point, to - from, &at.collision, &transforms, &mut rng).unwrap()
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: the Pz. I's 20 mm against the
    /// Chieftain's front, and the Chieftain's shells against the Pz. I, from 100 m.
    #[test]
    fn real_vehicles_against_each_other() {

        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let chieftain = VehicleDef::load(&fs, "uk", "GB98_T95_FV4201_Chieftain").unwrap();
        let pz1 = VehicleDef::load(&fs, "germany", "G53_PzI").unwrap();

        for shot in &pz1.shots {
            let impact = fire_at(shot, &chieftain, 1.2);
            assert_ne!(impact.outcome, ImpactOutcome::Pierced, "{shot:?}: {impact:?}");
            assert_eq!(impact.damage, 0.0, "{shot:?}: {impact:?}");
        }

        for shot in &chieftain.shots {
            let impact = fire_at(shot, &pz1, 1.0);
            assert!(impact.damage > 0.0, "{:?}: {impact:?}", shot.kind());
        }

    }

}
