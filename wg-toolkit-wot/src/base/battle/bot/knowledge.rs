//! What a bot knows of its battle beyond what it sees this tick, see [`BotKnowledge`]:
//! the enemies it remembers, which of them threaten it, their reloads as reckoned from
//! their shots, the hits it took, and how its team stands.

use glam::Vec3;

use crate::base::physics::{VehicleId, World};
use crate::game::item_defs::VehicleClass;

use super::situation::{BotEnemy, BotHit, BotSituation};
use super::tactics::aims_at;


/// How long the bot remembers an enemy its team no longer spots, in seconds, as a
/// player remembers where one was. Our choice.
const FORGET_AFTER: f64 = 30.0;


/// What a bot knows of its battle, kept from tick to tick, see [`Self::update`].
#[derive(Debug, Clone, Default)]
pub struct BotKnowledge {
    /// The enemies it remembers, spotted or not, see [`KnownEnemy`].
    pub enemies: Vec<KnownEnemy>,
    /// The hits it took lately.
    pub hits: Vec<BotHit>,
    /// How its team stands against the other, see [`BotTeamPicture`].
    pub team: BotTeamPicture,
    /// Its own health left, its share of its full health.
    pub health: f32,
}

/// An enemy a bot remembers, see [`BotKnowledge::enemies`].
#[derive(Debug, Clone)]
pub struct KnownEnemy {
    /// Its vehicle, in the world.
    pub vehicle: VehicleId,
    /// Its class.
    pub class: VehicleClass,
    /// Where its hull's centre was last spotted.
    pub at: Vec3,
    /// The game time it was last spotted at.
    pub seen: f64,
    /// Whether its team spots it now.
    pub spotted: bool,
    /// Its health left when last spotted, and in full.
    pub health: f32,
    pub max_health: f32,
    /// The game time the bot last saw it fire at, see [`Self::loaded`].
    pub fired: Option<f64>,
    /// How long its gun takes to reload, in seconds.
    pub reload: f32,
    /// Whether its gun points at the bot, while spotted.
    pub aims_at_me: bool,
}

/// How a bot's team stands against the other, as the players' lists and team health
/// bars show it.
#[derive(Debug, Clone, Copy, Default)]
pub struct BotTeamPicture {
    /// How many of each team are alive, its own counting the bot.
    pub allies_alive: usize,
    pub enemies_alive: usize,
    /// The health left of each team.
    pub ally_health: f32,
    pub enemy_health: f32,
}

impl BotKnowledge {

    /// Learn what the bot of this vehicle sees this tick, in its `situation`: the
    /// enemies its team spots, see [`Self::remember`], those it no longer does kept for
    /// [`FORGET_AFTER`], none destroyed; the hits it took, how its team stands, and its
    /// health.
    pub fn update(&mut self, world: &World, id: VehicleId, situation: &BotSituation) {

        let now = situation.now;
        let me = world.vehicle(id).hull.position;
        for enemy in situation.enemies {
            self.remember(enemy, now, aims_at(world.vehicle(enemy.vehicle), me));
        }
        for known in self.enemies.iter_mut().filter(|known| known.seen != now) {
            known.spotted = false;
            known.aims_at_me = false;
        }
        let alive = &situation.roster.enemies_alive;
        self.enemies.retain(|known| known.spotted || (alive.contains(&known.vehicle) && now - known.seen <= FORGET_AFTER));

        let roster = situation.roster;
        self.hits = situation.hits.to_vec();
        self.team = BotTeamPicture {
            allies_alive: roster.allies_alive,
            enemies_alive: roster.enemies_alive.len(),
            ally_health: roster.ally_health,
            enemy_health: roster.enemy_health,
        };
        self.health = situation.health / situation.max_health.max(1.0);

    }

    /// Remember an enemy its team spots at game time `now`, whether it aims at the bot:
    /// where it is, its health, and its last shot, but only one it saw fire, spotted the
    /// tick before too, not one it fired out of sight.
    fn remember(&mut self, enemy: &BotEnemy, now: f64, aims_at_me: bool) {

        let known = self.enemies.iter_mut().find(|known| known.vehicle == enemy.vehicle);
        let fired = match &known {
            Some(known) if known.spotted => enemy.fired,
            Some(known) => known.fired,
            None => None,
        };
        let remembered = KnownEnemy {
            vehicle: enemy.vehicle,
            class: enemy.class,
            at: enemy.center,
            seen: now,
            spotted: true,
            health: enemy.health,
            max_health: enemy.max_health,
            fired,
            reload: enemy.reload,
            aims_at_me,
        };

        match known {
            Some(known) => *known = remembered,
            None => self.enemies.push(remembered),
        }

    }

    /// The enemy of this vehicle, if the bot remembers it.
    pub fn enemy(&self, vehicle: VehicleId) -> Option<&KnownEnemy> {
        self.enemies.iter().find(|known| known.vehicle == vehicle)
    }

}

impl KnownEnemy {

    /// Whether its gun is loaded at game time `now`, as the bot reckons it: unless it saw
    /// it fire within its reload.
    pub fn loaded(&self, now: f64) -> bool {
        self.fired.is_none_or(|fired| now - fired >= f64::from(self.reload))
    }

}
