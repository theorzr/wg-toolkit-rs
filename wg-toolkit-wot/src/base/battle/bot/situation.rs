//! What a bot knows of its battle, given each tick, see [`BotSituation`]: what a player
//! sees, the enemies its team spots, its allies, and its own armour.

use glam::Vec3;

use crate::base::ballistics::shot::ShotTarget;
use crate::base::physics::VehicleId;
use crate::game::item_defs::ShotDef;


/// What a bot knows of its battle as it drives, see [`Bot::drive`](super::Bot::drive).
#[derive(Debug, Clone, Copy)]
pub struct BotSituation<'a> {
    /// The enemies its team spots, see [`BotEnemy`].
    pub enemies: &'a [BotEnemy],
    /// Its allies alive, see [`BotAlly`].
    pub allies: &'a [BotAlly],
    /// Its own armour, where it is, which its enemies' gun markers show their shots at.
    pub armor: &'a ShotTarget,
    /// Where it searches while its team spots no enemy.
    pub search: Vec3,
    /// Whether it drives, or only aims.
    pub drives: bool,
}

/// An enemy its team spots, as a bot sees it, see [`BotSituation::enemies`].
#[derive(Debug, Clone)]
pub struct BotEnemy {
    /// Its vehicle, in the world.
    pub vehicle: VehicleId,
    /// Where its hull's centre is.
    pub center: Vec3,
    /// Its health left, as its marker shows it.
    pub health: f32,
    /// Its armour, where it is, which the bot's gun marker shows a shot at, see
    /// `impact::indicate`.
    pub armor: ShotTarget,
    /// The shell it has loaded, as players know a vehicle's gun.
    pub shell: Option<ShotDef>,
    /// Whether its gun is loaded, as players reckon from its last shot.
    pub loaded: bool,
}

/// An ally of a bot, as it drives and fires, see [`BotSituation::allies`].
#[derive(Debug, Clone, Copy)]
pub struct BotAlly {
    /// Where its hull's centre is.
    pub center: Vec3,
    /// How far round its centre a shell hits it, or the bot drives into it: half its
    /// hull's box's diagonal.
    pub radius: f32,
    /// Its velocity on the ground plane, in m/s.
    pub velocity: Vec3,
}
