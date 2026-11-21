//! What a bot knows of its battle, given each tick, see [`BotSituation`]: what a player
//! sees, the enemies its team spots, its allies, the teams' roster, the hits it takes,
//! and its own armour and health.

use glam::Vec3;

use crate::game::item_defs::{ShotDef, VehicleClass};
use crate::base::ballistics::shot::ShotTarget;
use crate::base::physics::VehicleId;
use crate::base::poi::PoiMap;


/// What a bot knows of its battle as it drives, see [`Bot::drive`](super::Bot::drive).
#[derive(Debug, Clone, Copy)]
pub struct BotSituation<'a> {
    /// The game time, in seconds.
    pub now: f64,
    /// Its team, 1 or 2.
    pub team: u8,
    /// Its map's points of interest, see [`PoiMap`].
    pub poi_map: &'a PoiMap,
    /// The enemies its team spots, see [`BotEnemy`].
    pub enemies: &'a [BotEnemy],
    /// Its allies alive, see [`BotAlly`].
    pub allies: &'a [BotAlly],
    /// Both teams as the players' lists and health bars show them, see [`BotRoster`].
    pub roster: &'a BotRoster,
    /// The hits it took lately, as its damage indicator shows them, see [`BotHit`].
    pub hits: &'a [BotHit],
    /// Its own armour, where it is, which its enemies' gun markers show their shots at.
    pub armor: &'a ShotTarget,
    /// Its own health left, and in full.
    pub health: f32,
    pub max_health: f32,
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
    /// Its class, as the players' lists show it.
    pub class: VehicleClass,
    /// Where its hull's centre is.
    pub center: Vec3,
    /// Its health left, as its marker shows it, and in full.
    pub health: f32,
    pub max_health: f32,
    /// Its armour, where it is, which the bot's gun marker shows a shot at, see
    /// `impact::indicate`.
    pub armor: ShotTarget,
    /// The shell it has loaded, as players know a vehicle's gun.
    pub shell: Option<ShotDef>,
    /// The game time it last fired at, as its shot's tracer shows it while spotted.
    pub fired: Option<f64>,
    /// How long its gun takes to reload, in seconds, as players know a vehicle's gun.
    pub reload: f32,
}

/// An ally of a bot, as it drives and fires, see [`BotSituation::allies`].
#[derive(Debug, Clone, Copy)]
pub struct BotAlly {
    /// Its class, as the players' lists show it.
    pub class: VehicleClass,
    /// Where its hull's centre is.
    pub center: Vec3,
    /// How far round its centre a shell hits it, or the bot drives into it: half its
    /// hull's box's diagonal.
    pub radius: f32,
    /// Its velocity on the ground plane, in m/s.
    pub velocity: Vec3,
    /// Its health left, and in full.
    pub health: f32,
    pub max_health: f32,
}

/// Both teams of a bot's battle, as the players' lists and team health bars show them:
/// which enemies are alive, how many of each team, and their health left.
#[derive(Debug, Clone, Default)]
pub struct BotRoster {
    /// The vehicles of the enemy team alive, spotted or not.
    pub enemies_alive: Vec<VehicleId>,
    /// How many of its own team are alive, itself included.
    pub allies_alive: usize,
    /// The health left of its own team, itself included, and of the enemy team.
    pub ally_health: f32,
    pub enemy_health: f32,
}

/// A shell that hit a bot, as its damage indicator shows it.
#[derive(Debug, Clone, Copy)]
pub struct BotHit {
    /// Where it was fired from.
    pub from: Vec3,
    /// The game time it hit at.
    pub at: f64,
    /// The damage it dealt, none if it didn't pierce.
    pub damage: f32,
}
