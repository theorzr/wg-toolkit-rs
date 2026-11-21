//! The server driving a vehicle itself, as a player would, knowing what a player knows:
//! the enemies its team spots, their health and where their guns point, where its
//! allies are and how they move, the map and its bushes, and what its gun marker shows
//! of a shot, see `impact::indicate`. Nothing of its allies' plans.
//!
//! Its parts, each its module:
//! - what it knows of its battle, given each tick: [`BotSituation`], `situation`;
//! - what it remembers of it from tick to tick: the enemies it saw, which threaten it,
//!   their reloads as reckoned from their shots, the hits it took, how its team stands:
//!   `knowledge`;
//! - what its vehicle's class does: the posts it holds, what it likes in a spot to fire
//!   from: `role`;
//! - what it decides by: its gun marker, what a shot is worth, its best shell, its
//!   sight, its gun's reach: `tactics`;
//! - its target, and leading it: `target`;
//! - the point of interest of its map it holds, for its team, where what its role does
//!   goes on: `post`;
//! - where it fires from, looked for round it: `spot`;
//! - driving where it is ordered, round its allies, backing up, getting unstuck, its
//!   hull angled once there: `motor`.
//!
//! It picks its target among the enemies it sees, by what a shot at each is worth, and
//! looks round it for a spot to fire at it from, by what its role likes. Far from its
//! target, or seeing it from nowhere near, it drives at it instead, and while its team
//! spots none, searches where they were last seen, else their team's start. Stopped, it
//! angles its armour to its target, keeps its turret on it, leading it, loads the shell
//! its gun marker shows best, and says when it fires: it sees it itself, its gun on it,
//! its aim settled, and no ally in the way.
//!
//! ```text
//!   each tick:
//!     what it knows, see BotKnowledge::update
//!     its target: every CHOOSE_TICKS, or once it no longer sees it, the enemy it sees
//!       worth the most, see tactics::worth, another only if worth SWITCH_MARGIN more;
//!       seeing none, the one it has, else the nearest its team spots
//!     its aim: where its target will be once a shell fired gets there, see
//!       BotTarget::lead, while it sees it
//!     its next shell: the one its gun marker shows best on its target, see
//!       tactics::best_shell
//!     its post, chosen at its first tick, kept while it has something to do there,
//!       see BotPost::choose, BotPost::update
//!     its spot, see FiringSpotSearch::update
//!     its order to its motor, within SPOT_ARRIVE of it unless said otherwise:
//!       with a target: to its spot; else to its post, facing it; else to it, within
//!         its role's range seeing it, else within CLOSE_RANGE
//!       without one: to its post, facing the way it faces; else to where it
//!         searches, within SEARCH_RANGE
//!     its hull angled once there, see BotMotor::stance, BotMotor::controls
//!
//!   it fires once it sees its target, its gun on its aim, see Turret::aim_error,
//!   its shells landing within FIT_RADIUS of it or its aim settled, see Bot::aimed,
//!   and no ally in the way
//! ```

mod situation;
mod knowledge;
mod tactics;
mod target;
mod motor;
mod post;
mod role;
mod spot;

use glam::{Vec3, Vec4};

use crate::base::physics::{Dispersion, Vehicle, VehicleId, World};
use crate::base::debug::{self, DebugChannel, DebugPlace};
use crate::base::ballistics::gun::GunLoader;
use crate::game::item_defs::VehicleClass;

use self::tactics::{best_shell, in_sight, in_the_way, worth, FIT_RADIUS};
use self::motor::{BotMotor, DriveOrder};
use self::post::{BotPost, POST_LOOK};
use self::knowledge::BotKnowledge;
use self::spot::FiringSpotSearch;
use self::target::BotTarget;
use self::role::BotRole;

pub use self::situation::{BotAlly, BotEnemy, BotHit, BotRoster, BotSituation};


/// How close to its target the bot drives at most, in metres on the ground plane, seeing
/// it or not: not to push into it. Our choice.
const CLOSE_RANGE: f32 = 30.0;
/// How close to its spot the bot gets before stopping, in metres: farther than it drops
/// the points of its way, see [`motor::ARRIVE`], not to circle round it.
const SPOT_ARRIVE: f32 = 6.0;
/// How close to where it searches the bot drives, in metres on the ground plane. Our
/// choice: its team gets there within `spotting::MIN_RADIUS`, which ends the search.
const SEARCH_RANGE: f32 = 10.0;
/// How close the gun must point to its aim to fire, in radians, see
/// [`Turret::aim_error`](crate::base::physics::Turret::aim_error).
const AIM_TOLERANCE: f32 = 0.002;
/// How settled the bot's aim must be, its dispersion of its gun's best, for it to fire at
/// a target too far for [`FIT_RADIUS`]. Our choice.
const SETTLED_AIM: f32 = 1.1;
/// How many ticks the bot keeps its target before weighing the enemies it sees again,
/// see [`tactics::worth`], and how much more another must be worth for it to switch. Our
/// choice: half a second, as a player looks round.
const CHOOSE_TICKS: u32 = 5;
const SWITCH_MARGIN: f32 = 1.3;
/// How the bot's knowledge is drawn, see [`Bot::debug_draw`]: its text, where it
/// remembers enemies, the hits it took, its spot and its post, and how high over its
/// hull its text is, in metres.
const DEBUG_TEXT: Vec4 = Vec4::new(255.0, 255.0, 160.0, 255.0);
const DEBUG_REMEMBERED: Vec4 = Vec4::new(160.0, 160.0, 160.0, 255.0);
const DEBUG_HIT: Vec4 = Vec4::new(255.0, 64.0, 64.0, 255.0);
const DEBUG_SPOT: Vec4 = Vec4::new(64.0, 255.0, 160.0, 255.0);
const DEBUG_POST: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);
const DEBUG_TEXT_HEIGHT: f32 = 4.0;


/// The server driving a vehicle, as a client would: it sets its controls and its aim.
#[derive(Debug, Clone, Default)]
pub struct Bot {
    /// Its vehicle's class.
    class: VehicleClass,
    /// What it likes in a spot to fire from, by its vehicle's class.
    role: BotRole,
    /// What it remembers of its battle.
    knowledge: BotKnowledge,
    /// The enemy it aims at, see [`Self::choose_target`].
    target: Option<BotTarget>,
    /// How many ticks since it weighed the enemies it sees, see [`CHOOSE_TICKS`].
    chose: u32,
    /// The point of interest of its map it holds.
    post: BotPost,
    /// Where it fires at its target from.
    spot: FiringSpotSearch,
    /// How it drives its vehicle.
    motor: BotMotor,
}

impl Bot {

    /// A bot of the role of its vehicle's class, see [`BotRole`].
    pub fn new(class: VehicleClass) -> Self {
        Self {
            class,
            role: BotRole::of(class),
            ..Self::default()
        }
    }

    /// Set a vehicle's controls and aim for its next step, in its `situation`, and the
    /// shell its `gun` loads next: it picks its target, see [`Self::choose_target`], and
    /// heads for its spot to fire at it from, see [`FiringSpotSearch::update`], else for
    /// it, else for where it searches, its turret on it; if it drives, its motor finding
    /// its way over its world's map round its allies, then turning its armour to its
    /// target, see [`BotMotor`]. See the module's documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, gun: &mut GunLoader, situation: &BotSituation) {

        self.knowledge.update(world, id, situation);
        self.post.choose(&self.role, situation.poi_map, situation.team);
        self.choose_target(world, id, gun, situation.enemies);
        if let Some(target) = self.target.as_mut() {
            target.lead(world, id);
        }
        self.choose_shell(world, id, gun, situation.enemies);
        if situation.drives {
            self.spot.update(world, id, &self.role, &self.knowledge, gun, self.target, situation);
        }

        // What it heads for, how close to it it stops, and what it faces there.
        let vehicle = world.vehicle(id);
        self.post.update(situation.now, vehicle.hull.position, SPOT_ARRIVE, self.target.is_some());
        let post = self.post.poi.as_ref();
        let (goal, range, face) = match (self.target, self.spot.spot, post) {
            (Some(target), Some(spot), _) => (spot, SPOT_ARRIVE, target.at),
            (Some(target), None, Some(post)) => (post.position(), SPOT_ARRIVE, target.at),
            (Some(target), None, None) if target.seen => (target.at, self.role.range(vehicle), target.at),
            (Some(target), None, None) => (target.at, CLOSE_RANGE, target.at),
            (None, _, Some(post)) => (post.position(), SPOT_ARRIVE, post.position() + post.heading() * POST_LOOK),
            (None, _, None) => (situation.search, SEARCH_RANGE, situation.search),
        };
        let aim = self.target.map_or(goal, |target| target.aim);
        world.vehicle_mut(id).turret.track(aim);

        if !situation.drives {
            world.vehicle_mut(id).hull.flags = 0;
            return;
        }

        // Its armour angled to its target once there, looked at again as it weighs the
        // enemies it sees.
        let order = DriveOrder {
            goal,
            range,
            face,
            reverse: self.target.is_some(),
        };
        let against = self.target
            .and_then(|target| situation.enemies.iter().find(|enemy| enemy.vehicle == target.vehicle));
        self.motor.stance(world, id, &order, against, situation.armor, self.chose == 0);

        let flags = self.motor.controls(world, id, &order, situation.allies);
        world.vehicle_mut(id).hull.flags = flags;

        if debug::draws(DebugChannel::Bots) {
            self.debug_draw(world, id, situation);
        }

    }

    /// Draw what it knows of its battle, see [`DebugChannel::Bots`]: over its vehicle,
    /// its class, health and post, how its team stands, the threats to it, the hits it
    /// took and its allies; where it remembers enemies its team no longer spots, from
    /// where the hits it took came, its spot, and a line to its post.
    fn debug_draw(&self, world: &World, id: VehicleId, situation: &BotSituation) {

        let knowledge = &self.knowledge;
        let team = &knowledge.team;
        let hull = &world.vehicle(id).hull;
        let threats = knowledge.enemies.iter().filter(|known| known.aims_at_me).count();
        let damage: f32 = knowledge.hits.iter().map(|hit| hit.damage).sum();
        let ally_health: f32 = situation.allies.iter().map(|ally| ally.health).sum();
        let ally_max_health: f32 = situation.allies.iter().map(|ally| ally.max_health).sum();
        let allies: Vec<String> = situation.allies.iter().map(|ally| format!("{:?}", ally.class)).collect();
        let post = self.post.poi.as_ref().map_or("none".to_string(), |post| post.to_string());
        let text = format!(
            "{:?} {:.0}%, post {post}\nteams {} v {}, health {:.0} v {:.0}\nthreats {threats}, hits {} ({damage:.0})\nallies {:.0}%: {}",
            self.class,
            knowledge.health * 100.0,
            team.allies_alive,
            team.enemies_alive,
            team.ally_health,
            team.enemy_health,
            knowledge.hits.len(),
            100.0 * ally_health / ally_max_health.max(1.0),
            allies.join(", "),
        );
        debug::at(DebugPlace::on(id, hull.position, hull.orientation), || {
            debug::text(DebugChannel::Bots, hull.position + Vec3::Y * DEBUG_TEXT_HEIGHT, text, DEBUG_TEXT);
        });

        for known in knowledge.enemies.iter().filter(|known| !known.spotted) {
            let text = format!(
                "{:?} {:.0}/{:.0}, {:.0} s ago",
                known.class,
                known.health,
                known.max_health,
                situation.now - known.seen,
            );
            debug::cross(DebugChannel::Bots, known.at, DEBUG_REMEMBERED);
            debug::text(DebugChannel::Bots, known.at, text, DEBUG_REMEMBERED);
        }
        for hit in &knowledge.hits {
            debug::line(DebugChannel::Bots, hit.from, hull.position, DEBUG_HIT);
        }
        if let Some(spot) = self.spot.spot {
            debug::line(DebugChannel::Bots, hull.position, spot, DEBUG_SPOT);
            debug::cross(DebugChannel::Bots, spot, DEBUG_SPOT);
        }
        if let Some(post) = &self.post.poi {
            debug::line(DebugChannel::Bots, hull.position, post.position(), DEBUG_POST);
        }

    }

    /// Pick its target among the `enemies` its team spots, see the module's
    /// documentation: every [`CHOOSE_TICKS`], or once it no longer sees it, the one it
    /// sees worth the most, see [`tactics::worth`], another than the one it has only if
    /// worth [`SWITCH_MARGIN`] more; seeing none, the one it has, else the nearest. One
    /// its team no longer spots, or destroyed, is forgotten.
    fn choose_target(&mut self, world: &World, id: VehicleId, gun: &GunLoader, enemies: &[BotEnemy]) {

        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);

        // 1. Its target where its team spots it, forgotten once it no longer does.
        let current = self.target
            .and_then(|target| enemies.iter().find(|enemy| enemy.vehicle == target.vehicle));
        match (&mut self.target, current) {
            (Some(target), Some(enemy)) => {
                target.at = enemy.center;
                target.aim = enemy.center;
                target.seen = in_sight(world, eye, enemy.center);
            }
            _ => self.target = None,
        }

        // 2. Now and then, or with its target out of sight, the one it sees worth most.
        self.chose += 1;
        let seen = self.target.is_some_and(|target| target.seen);
        if seen && self.chose < CHOOSE_TICKS {
            return;
        }
        self.chose = 0;

        let best = enemies.iter()
            .filter(|enemy| in_sight(world, eye, enemy.center))
            .map(|enemy| (enemy, worth(world, id, eye, gun, enemy)))
            .max_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((enemy, best)) = best {
            let kept = current
                .filter(|_| seen)
                .map(|current| worth(world, id, eye, gun, current));
            if kept.is_none_or(|kept| best > kept * SWITCH_MARGIN) {
                self.target = Some(BotTarget {
                    vehicle: enemy.vehicle,
                    at: enemy.center,
                    aim: enemy.center,
                    seen: true,
                });
            }
            return;
        }

        // 3. Else the one it has, or the nearest its team spots.
        if self.target.is_none() {
            let nearest = enemies.iter()
                .min_by(|a, b| a.center.distance_squared(eye).total_cmp(&b.center.distance_squared(eye)));
            self.target = nearest.map(|enemy| BotTarget {
                vehicle: enemy.vehicle,
                at: enemy.center,
                aim: enemy.center,
                seen: false,
            });
        }

    }

    /// Load next the shell its gun marker shows best on its target, while it sees it, as
    /// it weighs the enemies it sees, see [`tactics::best_shell`].
    fn choose_shell(&self, world: &World, id: VehicleId, gun: &mut GunLoader, enemies: &[BotEnemy]) {

        if self.chose != 0 {
            return;
        }
        let Some(target) = self.target.filter(|target| target.seen) else { return };
        let Some(enemy) = enemies.iter().find(|enemy| enemy.vehicle == target.vehicle) else { return };
        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);

        let Some((shell, _)) = best_shell(gun, eye, target.aim, &enemy.armor) else { return };
        let compact_descr = shell.compact_descr.to_int();
        gun.select_next(compact_descr);

    }

    /// Whether it fires its vehicle's gun now: at its target, which it sees, its gun on
    /// its aim, see [`BotTarget::lead`], its aim settled, see [`Self::aimed`], and none
    /// of its `allies` in the way, see [`tactics::in_the_way`].
    pub fn fires(&self, vehicle: &Vehicle, allies: &[BotAlly]) -> bool {

        let Some(target) = self.target.filter(|target| target.seen) else { return false };
        let (position, _, dispersion) = vehicle.turret.gun_marker(&vehicle.hull);
        if vehicle.turret.aim_error(&vehicle.hull) >= AIM_TOLERANCE {
            return false;
        }

        Self::aimed(vehicle, dispersion, target.aim.distance(position))
            && !allies.iter().any(|ally| in_the_way(position, target.aim, ally.center, ally.radius))

    }

    /// Whether its vehicle's aim is settled enough to fire at `distance` metres with this
    /// `dispersion`: its shells landing within [`FIT_RADIUS`] of its aim, or, too far for
    /// that, its dispersion within [`SETTLED_AIM`] of its gun's best.
    fn aimed(vehicle: &Vehicle, dispersion: Dispersion, distance: f32) -> bool {
        let best = vehicle.turret.aiming.dispersion_angle;
        dispersion.angle.tan() * distance <= FIT_RADIUS || dispersion.angle <= best * SETTLED_AIM
    }

}
