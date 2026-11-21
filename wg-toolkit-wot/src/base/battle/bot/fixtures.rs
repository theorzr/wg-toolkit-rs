//! What the bot's tests build on: what it knows, its enemies, its gun, its armour.

use std::sync::Arc;

use glam::{Affine3A, Vec3};

use crate::base::ballistics::fixtures::{material, plate, shot, vehicle as plated};
use crate::base::ballistics::shot::ShotTarget;
use crate::base::ballistics::gun::GunLoader;
use crate::game::item_defs::ShellKind;
use crate::base::physics::VehicleId;

use super::situation::{BotEnemy, BotSituation};


/// How high an enemy's hull's centre is over the ground, about, in metres.
pub const HULL_CENTER: f32 = 1.5;


/// What a bot of this armour knows driving among no allies.
pub fn situation<'a>(enemies: &'a [BotEnemy], armor: &'a ShotTarget, search: Vec3, drives: bool) -> BotSituation<'a> {
    BotSituation {
        enemies,
        allies: &[],
        armor,
        search,
        drives,
    }
}

/// The armour of a bot no shell can hit.
pub fn unarmored() -> ShotTarget {
    let (collision, transforms) = plated(vec![]);
    ShotTarget {
        entity_id: 0,
        collision: Arc::new(collision),
        transforms,
    }
}

/// An enemy of this vehicle, with its hull's centre at `center`, its health, and a
/// plate of 100 mm across its centre, facing along z.
pub fn enemy(vehicle: VehicleId, center: Vec3, health: f32) -> BotEnemy {
    let (collision, _) = plated(vec![plate(0.0, material(100.0, true))]);
    BotEnemy {
        vehicle,
        center,
        health,
        armor: ShotTarget {
            entity_id: 0,
            collision: Arc::new(collision),
            transforms: [Affine3A::from_translation(center); 4],
        },
        shell: None,
        loaded: false,
    }
}

/// A gun of AP shells piercing 200 mm, dealing 400 damage.
pub fn gun() -> GunLoader {
    GunLoader::new(vec![(shot(ShellKind::ArmorPiercing, 120.0, 200.0), 10)], 10.0)
}
