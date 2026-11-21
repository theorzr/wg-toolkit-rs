//! What a bot decides by: what its gun marker shows, what a shot is worth, which shell,
//! whether it sees a point, whether a gun aims at it, how far its gun reaches.

use glam::Vec3;

use crate::base::ballistics::impact::{self, Indication};
use crate::base::physics::{Vehicle, VehicleId, World};
use crate::game::item_defs::{ShellKind, ShotDef};
use crate::base::ballistics::shot::ShotTarget;
use crate::base::ballistics::gun::GunLoader;
use crate::game::collision::VehiclePart;

use super::situation::BotEnemy;
use super::super::VIEW_RANGE;
use super::CLOSE_RANGE;


/// How far from its aim the bot's shells may stray for it to fire, in metres: about half
/// a hull's height. Our choice.
pub const FIT_RADIUS: f32 = 1.0;
/// What a shot is worth by what the gun marker shows, red, yellow and green, see
/// [`worth`]: a red one still hits, a yellow one pierces about half the time. Our choice.
pub const INDICATION_WORTH: [f32; 3] = [0.1, 0.5, 1.0];
/// How close an enemy's gun must point to the bot for it to be a threat, in radians, and
/// how much more it is then worth. Our choice.
const AIMED_AT: f32 = 3.0f32.to_radians();
const THREAT_WORTH: f32 = 1.5;
/// How far an enemy is for its worth to halve, in metres. Our choice.
const HALF_WORTH_DISTANCE: f32 = 200.0;
/// How far within its turret's yaw limits the bot keeps its target, angling its hull,
/// in radians. Our choice.
const TURRET_LIMIT_MARGIN: f32 = 3.0f32.to_radians();


/// What a shot at an enemy it sees is worth to a bot at `eye`, its vehicle of this id:
/// by what its gun marker shows of its best shell there, see [`INDICATION_WORTH`], over
/// the shots it takes to destroy it, more if its gun points at the bot, see
/// [`THREAT_WORTH`], less the farther, see [`HALF_WORTH_DISTANCE`].
pub fn worth(world: &World, id: VehicleId, eye: Vec3, gun: &GunLoader, enemy: &BotEnemy) -> f32 {

    let Some((shell, shown)) = best_shell(gun, eye, enemy.center, &enemy.armor) else { return 0.0 };
    let distance = eye.distance(enemy.center);
    let shots = (enemy.health / shell.damage_at(distance).max(1.0)).ceil().max(1.0);

    let me = world.vehicle(id).hull.position;
    let threat = if aims_at(world.vehicle(enemy.vehicle), me) { THREAT_WORTH } else { 1.0 };
    INDICATION_WORTH[shown as usize] * threat / shots / (1.0 + distance / HALF_WORTH_DISTANCE)

}

/// The shell a gun has left whose marker shows best fired from `eye` at `aim` on this
/// `armor`, see [`impact::indicate`], and what it shows: the loaded one first among
/// equals, and HE only where it shows green, as players fire it, into a rear say. None
/// without shells.
pub fn best_shell<'a>(
    gun: &'a GunLoader,
    eye: Vec3,
    aim: Vec3,
    armor: &ShotTarget,
) -> Option<(&'a ShotDef, Indication)> {
    let left = gun.ammo.iter()
        .filter(|(_, count)| *count > 0)
        .map(|(def, _)| def);
    gun.shells().0.into_iter()
        .chain(left)
        .map(|shell| {
            let shown = impact::indicate(shell, eye, aim, &armor.collision, &armor.transforms)
                .unwrap_or(Indication::NotPierced);
            (shell, shown)
        })
        .min_by_key(|&(shell, shown)| {
            let sure = shell.kind() != ShellKind::HighExplosive || shown == Indication::GreatPierced;
            std::cmp::Reverse((sure, shown))
        })
}

/// Whether a vehicle's gun points at a point, within [`AIMED_AT`].
pub fn aims_at(vehicle: &Vehicle, point: Vec3) -> bool {
    let (position, velocity, _) = vehicle.turret.gun_marker(&vehicle.hull);
    velocity.angle_between(point - position) < AIMED_AT
}

/// Whether a bot's gun at `eye` sees a point: within [`VIEW_RANGE`] on the ground plane,
/// and nothing of the map in the way, see [`World::sees`].
pub fn in_sight(world: &World, eye: Vec3, at: Vec3) -> bool {
    (at - eye).with_y(0.0).length() <= VIEW_RANGE && world.sees(eye, at)
}

/// Whether something within `radius` of `center` is in the way from a gun at `from` to
/// `to`: between them, the line passing within its radius.
pub fn in_the_way(from: Vec3, to: Vec3, center: Vec3, radius: f32) -> bool {
    let line = to - from;
    let along = (center - from).dot(line) / line.length_squared().max(f32::EPSILON);
    along > 0.0 && along < 1.0 && (from + line * along).distance(center) < radius
}

/// How far a vehicle's gun reaches, as a bot fires it, in metres on the ground plane:
/// as far as its best dispersion lands its shells within [`FIT_RADIUS`] of its aim, see
/// [`Bot::fires`](super::Bot::fires), no closer than [`CLOSE_RANGE`], no farther than it
/// sees, see [`VIEW_RANGE`].
pub fn effective_range(vehicle: &Vehicle) -> f32 {
    let best = vehicle.turret.aiming.dispersion_angle.tan().max(f32::EPSILON);
    (FIT_RADIUS / best).clamp(CLOSE_RANGE, VIEW_RANGE)
}

/// How far off its target a vehicle's hull may turn once stopped: as far as its turret's
/// yaw limits keep it on its target, [`TURRET_LIMIT_MARGIN`] within them, none if they
/// don't; any way without.
pub fn turn_limit(vehicle: &Vehicle) -> f32 {
    match vehicle.turret.aiming.yaw_limits {
        Some((min, max)) => (max.min(-min) - TURRET_LIMIT_MARGIN).max(0.0),
        None => f32::INFINITY,
    }
}

/// Where the centre of a vehicle's hull is, its armour placed in the world.
pub fn hull_center(armor: &ShotTarget) -> Vec3 {
    Vec3::from(armor.transforms[VehiclePart::Hull as usize].translation)
}


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use crate::base::ballistics::fixtures::{material, plate, shot, vehicle as plated};
    use super::*;

    /// The shell a gun of these shells, the first loaded, fires from 100 m at a plate
    /// this thick, and what its gun marker shows, see [`best_shell`].
    fn picked(ammo: &[&ShotDef], armor: f32) -> Option<(ShellKind, Indication)> {
        let gun = GunLoader::new(ammo.iter().map(|&def| (def.clone(), 10)).collect(), 10.0);
        let (collision, transforms) = plated(vec![plate(0.0, material(armor, true))]);
        let armor = ShotTarget {
            entity_id: 0,
            collision: Arc::new(collision),
            transforms,
        };
        best_shell(&gun, Vec3::new(0.0, 0.0, -100.0), Vec3::ZERO, &armor)
            .map(|(shell, shown)| (shell.kind(), shown))
    }

    #[test]
    fn loads_what_its_marker_shows_best() {

        // AP piercing 200 mm, HE 40 mm.
        let ap = shot(ShellKind::ArmorPiercing, 120.0, 200.0);
        let mut he = shot(ShellKind::HighExplosive, 120.0, 40.0);
        he.compact_descr.id = 2;
        let (green, red) = (Indication::GreatPierced, Indication::NotPierced);

        // Through 30 mm both are green: the loaded one. Through 250 mm both are red: AP,
        // HE only fired where it shows green.
        assert_eq!(picked(&[&ap, &he], 30.0), Some((ShellKind::ArmorPiercing, green)));
        assert_eq!(picked(&[&he, &ap], 30.0), Some((ShellKind::HighExplosive, green)));
        assert_eq!(picked(&[&he, &ap], 250.0), Some((ShellKind::ArmorPiercing, red)));

    }

    #[test]
    fn holds_fire_with_an_ally_in_the_way() {
        let (from, to) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 100.0));
        // In the line, or near enough it.
        assert!(in_the_way(from, to, Vec3::new(0.0, 0.0, 30.0), 3.0));
        assert!(in_the_way(from, to, Vec3::new(2.0, 0.0, 30.0), 3.0));
        // Off it, behind the gun, or past the target.
        assert!(!in_the_way(from, to, Vec3::new(5.0, 0.0, 30.0), 3.0));
        assert!(!in_the_way(from, to, Vec3::new(0.0, 0.0, -2.0), 3.0));
        assert!(!in_the_way(from, to, Vec3::new(0.0, 0.0, 120.0), 3.0));
    }

}
