//! A shell's speed, and the armour it pierces at it. Its speed and its penetration go
//! together by De Marre's formula, whatever the shell: the game's penetration, falling
//! with the distance it flew, is the air's drag slowing it, and what it loses of its
//! penetration flying through something is speed it loses too. Only an AP or APCR shell
//! pierces by its speed, though, as it hits: HE, HEAT and APHE shells pierce as much
//! however fast they fly, their penetration the same at any distance in the game, so
//! that the air's drag doesn't slow them.
//!
//! ```text
//!   speed   a share of its speed from its gun, 1 as it leaves it
//!   drag    = (pen_at(distance) / pen_near)^(1/1.43)     1 over the first 50 m
//!   pen     = pen_near · speed^1.43                      AP, APCR
//!           = pen_near                                   HE, HEAT, APHE
//! ```
//!
//! A shell flies its parabola at the speed it has, as the client draws it: what it
//! loses on the way, to the air's drag and to what it flies through, it only loses as it
//! hits something, see [`Shot::impact`]. Gravity is left out, as the game leaves it out
//! of its penetration.
//!
//! [`Shot::impact`]: super::shot::Shot::impact

use crate::game::item_defs::{ShellKind, ShotDef};


/// How a kinetic shell's penetration follows its speed, by De Marre's formula: as its
/// speed to this power, 1 / 0.7.
const DE_MARRE_SPEED_EXPONENT: f32 = 1.43;


/// The penetration of a shell flying at `speed`, a share of its speed from its gun:
/// for a kinetic shell, its penetration near its gun times its speed to
/// [`DE_MARRE_SPEED_EXPONENT`]; else that penetration, as fixed.
pub fn pen_at_speed(shot: &ShotDef, speed: f32) -> f32 {
    if is_kinetic(shot) {
        shot.pen_near * speed.max(0.0).powf(DE_MARRE_SPEED_EXPONENT)
    } else {
        shot.pen_near
    }
}

/// The share of its speed from its gun the air's drag leaves a shell flying `distance`
/// metres: the speed at which it pierces what the game says it pierces there, see
/// [`speed_for_pen`].
pub fn drag_speed(shot: &ShotDef, distance: f32) -> f32 {
    speed_for_pen(shot, shot.pen_at(distance))
}

/// The share of its speed from its gun at which a shell's penetration is `pen`, by De
/// Marre's formula, whatever the shell, see the module's documentation: all of it for a
/// shell that pierces nothing.
pub fn speed_for_pen(shot: &ShotDef, pen: f32) -> f32 {
    if shot.pen_near > 0.0 {
        (pen.max(0.0) / shot.pen_near).powf(1.0 / DE_MARRE_SPEED_EXPONENT)
    } else {
        1.0
    }
}

/// Whether a shell pierces by its speed: AP and APCR.
pub fn is_kinetic(shot: &ShotDef) -> bool {
    matches!(shot.kind(), ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr)
}


#[cfg(test)]
mod tests {

    use super::super::fixtures::shot;
    use super::*;

    #[test]
    fn pierces_less_slower() {

        // At 82% of its speed, 75% of its penetration.
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let pen = pen_at_speed(&ap, 0.818);
        assert!((pen - 150.0).abs() < 0.2, "{pen}");
        assert!((speed_for_pen(&ap, pen) - 0.818).abs() < 1e-4);

        // A HEAT shell's jet pierces as much however fast it flies, though it slows as
        // any shell does.
        let heat = shot(ShellKind::HollowCharge, 120.0, 180.0);
        assert_eq!(pen_at_speed(&heat, 0.5), 180.0);
        assert!((speed_for_pen(&heat, 90.0) - 0.5f32.powf(1.0 / 1.43)).abs() < 1e-5);

    }

    #[test]
    fn drag_slows_it_as_its_penetration_falls() {

        let ap = ShotDef {
            pen_far: 200.0,
            ..shot(ShellKind::ArmorPiercing, 120.0, 250.0)
        };

        // None over its first 50 m.
        assert_eq!(drag_speed(&ap, 20.0), 1.0);
        assert_eq!(drag_speed(&ap, 50.0), 1.0);

        // Then as slow as it pierces what the game says.
        for distance in [200.0, 400.0, 720.0] {
            let speed = drag_speed(&ap, distance);
            assert!(speed < 1.0);
            assert!((pen_at_speed(&ap, speed) - ap.pen_at(distance)).abs() < 0.01);
        }

    }

}
