//! A shell ricocheting off what it hits, a vehicle's armour or the ground: the way it
//! goes off and the speed it keeps, its penetration following it, see
//! [`super::speed`].
//!
//! Whether it ricochets off armour is the game's rule, see [`super::impact`]. How it
//! goes off is an oblique impact, see [`off_armor`]: what of its velocity goes into the
//! plate bounces back at a small share of itself, and the plate's friction slows what
//! goes along it as much as the shell pushed into it. The more it grazes the plate, the
//! less it loses, and it goes off flatter than it came:
//!
//! ```text
//!   into the plate    v·cos θ   ->  bounced back at ARMOR_RESTITUTION of it
//!   along the plate   v·sin θ   ->  less ARMOR_FRICTION · (1 + restitution) · v·cos θ
//!
//!   from the normal   70°    75°    80°    85°
//!   speed kept        82%    87%    92%    97%
//!   penetration       75%    82%    89%    95%     the game's 75% at its 70°
//! ```
//!
//! Off the ground, which the game never does, AP and APCR shells skim at a grazing
//! angle, by the ground's material, see [`GroundMaterial`], after what was measured of
//! projectiles off sand, soil and concrete:
//!
//! ```text
//!   softer ground   lower ricochet angles, steeper off it, slower
//!   firmer ground   higher ricochet angles, flatter off it, less slowed
//! ```

use std::f32::consts::FRAC_PI_2;

use glam::{Quat, Vec3};
use rand::Rng;

use wgtk::space::terrain::Ground;

use crate::game::item_defs::{ShellKind, ShotDef};


/// The share of what of its velocity goes into armour a shell bounces back at, see
/// [`off_armor`]: steel barely gives it back.
const ARMOR_RESTITUTION: f32 = 0.2;
/// How much armour slows a shell along it, of what the shell pushes into it, see
/// [`off_armor`]: so that a shell ricocheting at the game's 70° from the normal keeps
/// 75% of its penetration, as all of the game's ricochets leave it.
const ARMOR_FRICTION: f32 = 0.3;

/// The grounds of the terrain, as it is painted, see [`GroundMaterial::of`] and the
/// module's documentation. Our choices.
const FIRM_GROUND: GroundMaterial = GroundMaterial {
    ricochet_angle: 15.0f32.to_radians(),
    sure_ricochet_angle: 8.0f32.to_radians(),
    exit_angle_factor: [0.8, 1.2],
    speed_kept: [0.7, 0.85],
    spread: 1.0f32.to_radians(),
};

const MEDIUM_GROUND: GroundMaterial = GroundMaterial {
    ricochet_angle: 12.0f32.to_radians(),
    sure_ricochet_angle: 6.0f32.to_radians(),
    exit_angle_factor: [1.0, 1.4],
    speed_kept: [0.6, 0.8],
    spread: 1.5f32.to_radians(),
};

const SOFT_GROUND: GroundMaterial = GroundMaterial {
    ricochet_angle: 9.0f32.to_radians(),
    sure_ricochet_angle: 4.0f32.to_radians(),
    exit_angle_factor: [1.2, 1.6],
    speed_kept: [0.5, 0.7],
    spread: 2.0f32.to_radians(),
};


/// What a ground does to the AP and APCR shells meeting it, see [`off_ground`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundMaterial {
    /// The largest angle between a shell's path and the surface it ricochets off at.
    pub ricochet_angle: f32,
    /// The angle below which a shell always ricochets: ever less likely from there to
    /// its ricochet angle.
    pub sure_ricochet_angle: f32,
    /// Its angle off the surface, of its angle onto it, at least and at most.
    pub exit_angle_factor: [f32; 2],
    /// The share of its speed it keeps, at least and at most.
    pub speed_kept: [f32; 2],
    /// How far aside of its way it goes off, at most, either way.
    pub spread: f32,
}

impl GroundMaterial {

    /// The material of this kind of ground, see [`FIRM_GROUND`]: unpainted, firm, as
    /// vehicles drive it.
    pub fn of(ground: Ground) -> Self {
        match ground {
            Ground::None |
            Ground::Firm => FIRM_GROUND,
            Ground::Medium => MEDIUM_GROUND,
            Ground::Soft => SOFT_GROUND,
        }
    }

}


/// The velocity a shell flying at `velocity` ricochets off armour of this `normal` at,
/// once it does, see [`super::impact::hit`]: bounced back off it at
/// [`ARMOR_RESTITUTION`], slowed along it by [`ARMOR_FRICTION`], see the module's
/// documentation.
pub fn off_armor(velocity: Vec3, normal: Vec3) -> Vec3 {

    // Its velocity into the plate, whichever way the normal points, and along it.
    let into = normal * velocity.dot(normal);
    let along = velocity - into;

    // Bounced back, then slowed along it as much as it pushed into it, never back.
    let bounced = -into * ARMOR_RESTITUTION;
    let friction = ARMOR_FRICTION * (1.0 + ARMOR_RESTITUTION) * into.length();
    let slid = along.normalize_or_zero() * (along.length() - friction).max(0.0);
    slid + bounced

}

/// The velocity a shell flying at `velocity` ricochets off the ground at, of this
/// material and this upward `normal` where it meets it, if it does: AP and APCR shells
/// only, HE and HEAT going off on it.
pub fn off_ground(
    shot: &ShotDef,
    velocity: Vec3,
    normal: Vec3,
    ground: &GroundMaterial,
    rng: &mut impl Rng,
) -> Option<Vec3> {

    if !matches!(shot.kind(), ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr) {
        return None;
    }

    // Its grazing angle, between its way and the surface.
    let speed = velocity.length();
    let direction = velocity.normalize_or_zero();
    let grazing = (-direction.dot(normal)).clamp(-1.0, 1.0).asin();
    if !(0.0..=ground.ricochet_angle).contains(&grazing) {
        return None;
    }

    // Sure below the sure angle, then ever less likely up to the ricochet angle.
    let chance = if grazing <= ground.sure_ricochet_angle {
        1.0
    } else {
        (ground.ricochet_angle - grazing) / (ground.ricochet_angle - ground.sure_ricochet_angle)
    };
    if rng.r#gen::<f32>() >= chance {
        return None;
    }

    // Along the surface, turned a little aside around its normal.
    let along = (direction - normal * direction.dot(normal)).normalize_or_zero();
    if along == Vec3::ZERO {
        return None;
    }
    let aside = rng.gen_range(-ground.spread..=ground.spread);
    let along = Quat::from_axis_angle(normal, aside) * along;

    // Then up off it, steeper or flatter than it came, slower.
    let [low, high] = ground.exit_angle_factor;
    let exit = (grazing * rng.gen_range(low..=high)).min(FRAC_PI_2);
    let [least, most] = ground.speed_kept;
    let speed = speed * rng.gen_range(least..=most);
    Some((along * exit.cos() + normal * exit.sin()) * speed)

}


#[cfg(test)]
mod tests {

    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use super::super::speed::pen_at_speed;
    use super::super::fixtures::shot;
    use super::*;

    /// A velocity coming down onto flat ground at this grazing angle, in degrees.
    fn grazing(angle: f32) -> Vec3 {
        Vec3::new(angle.to_radians().cos(), -angle.to_radians().sin(), 0.0) * 1000.0
    }

    #[test]
    fn grazes_off_armour_losing_less_the_flatter_it_hits() {

        // At the game's 70° from the normal, 75% of its penetration, as the game's.
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let off = off_armor(grazing(20.0), Vec3::Y);
        let pen = pen_at_speed(&ap, off.length() / 1000.0);
        assert!((pen - 150.0).abs() < 1.0, "{pen}");

        // Flatter off it than it came, on its way, whichever way the normal points.
        let exit = off.normalize().y.asin().to_degrees();
        assert!((4.0..6.0).contains(&exit), "{exit}°");
        assert!(off.x > 0.0 && off.z == 0.0, "{off}");
        assert_eq!(off_armor(grazing(20.0), -Vec3::Y), off);

        // The flatter it hits, the more it keeps.
        let kept = [20.0, 15.0, 10.0, 5.0]
            .map(|angle| off_armor(grazing(angle), Vec3::Y).length() / 1000.0);
        assert!(kept.windows(2).all(|pair| pair[0] < pair[1]), "{kept:?}");
        assert!((kept[3] - 0.97).abs() < 0.01, "{kept:?}");

    }

    #[test]
    fn skims_off_the_ground_at_a_grazing_angle() {

        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let mut rng = StdRng::seed_from_u64(1);

        // Always within its sure angle: steeper, a little aside, slower. Never past its
        // ricochet angle.
        for _ in 0..100 {
            let off = off_ground(&ap, grazing(5.0), Vec3::Y, &MEDIUM_GROUND, &mut rng).unwrap();
            let exit = off.normalize().y.asin().to_degrees();
            let aside = off.z.atan2(off.x).to_degrees().abs();
            assert!((5.0 - 1e-3..=7.0 + 1e-3).contains(&exit), "{exit}°");
            assert!(aside <= 1.5 + 1e-3, "{aside}°");
            assert!((600.0 - 0.1..=800.0 + 0.1).contains(&off.length()), "{off}");
            assert!(off_ground(&ap, grazing(13.0), Vec3::Y, &MEDIUM_GROUND, &mut rng).is_none());
        }

        // Halfway between, half the time.
        let skimmed = (0..1000)
            .filter(|_| off_ground(&ap, grazing(9.0), Vec3::Y, &MEDIUM_GROUND, &mut rng).is_some())
            .count();
        assert!((400..600).contains(&skimmed), "{skimmed}");

        // Neither HE nor HEAT, which go off on it.
        let he = shot(ShellKind::HighExplosive, 120.0, 40.0);
        let heat = shot(ShellKind::HollowCharge, 120.0, 200.0);
        assert!(off_ground(&he, grazing(5.0), Vec3::Y, &MEDIUM_GROUND, &mut rng).is_none());
        assert!(off_ground(&heat, grazing(5.0), Vec3::Y, &MEDIUM_GROUND, &mut rng).is_none());

    }

}
