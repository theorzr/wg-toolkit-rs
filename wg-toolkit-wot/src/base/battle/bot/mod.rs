//! The server driving a vehicle itself, as a player would, knowing what a player knows:
//! the enemies its team spots, their health and where their guns point, where its
//! allies are and how they move, the map and its bushes, and what its gun marker shows
//! of a shot, see `impact::indicate`. Nothing of its allies' plans.
//!
//! Its parts, each its module:
//! - what it knows of its battle, given each tick: [`BotSituation`], `situation`;
//! - what its vehicle's class likes in a spot to fire from: `role`;
//! - what it decides by: its gun marker, what a shot is worth, its best shell, its
//!   sight, its gun's reach: `tactics`;
//! - its target, and leading it: `target`;
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
//!     its target: every CHOOSE_TICKS, or once it no longer sees it, the enemy it sees
//!       worth the most, see tactics::worth, another only if worth SWITCH_MARGIN more;
//!       seeing none, the one it has, else the nearest its team spots
//!     its aim: where its target will be once a shell fired gets there, see
//!       BotTarget::lead, while it sees it
//!     its next shell: the one its gun marker shows best on its target, see
//!       tactics::best_shell
//!     its spot, see FiringSpotSearch::update
//!     its order to its motor: to its spot, within SPOT_ARRIVE, else to its target,
//!       within its role's range seeing it, else within CLOSE_RANGE; else to where it
//!       searches, within SEARCH_RANGE; its hull angled once there, see
//!       BotMotor::stance, BotMotor::controls
//!
//!   it fires once it sees its target, its gun on its aim, see Turret::aim_error,
//!   its shells landing within FIT_RADIUS of it or its aim settled, see Bot::aimed,
//!   and no ally in the way
//! ```

mod situation;
mod tactics;
mod target;
mod motor;
mod role;
mod spot;
#[cfg(test)]
mod fixtures;

use crate::base::physics::{Dispersion, Vehicle, VehicleId, World};
use crate::base::ballistics::gun::GunLoader;
use crate::game::item_defs::VehicleClass;

use self::tactics::{best_shell, in_sight, in_the_way, worth, FIT_RADIUS};
use self::motor::{BotMotor, DriveOrder};
use self::spot::FiringSpotSearch;
use self::target::BotTarget;
use self::role::BotRole;

pub use self::situation::{BotAlly, BotEnemy, BotSituation};


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


/// The server driving a vehicle, as a client would: it sets its controls and its aim.
#[derive(Debug, Clone, Default)]
pub struct Bot {
    /// What it likes in a spot to fire from, by its vehicle's class.
    role: BotRole,
    /// The enemy it aims at, see [`Self::choose_target`].
    target: Option<BotTarget>,
    /// How many ticks since it weighed the enemies it sees, see [`CHOOSE_TICKS`].
    chose: u32,
    /// Where it fires at its target from.
    spot: FiringSpotSearch,
    /// How it drives its vehicle.
    motor: BotMotor,
}

impl Bot {

    /// A bot of the role of its vehicle's class, see [`BotRole`].
    pub fn new(class: VehicleClass) -> Self {
        Self {
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

        self.choose_target(world, id, gun, situation.enemies);
        if let Some(target) = self.target.as_mut() {
            target.lead(world, id);
        }
        self.choose_shell(world, id, gun, situation.enemies);
        if situation.drives {
            self.spot.update(world, id, &self.role, gun, self.target, situation);
        }

        // What it heads for, how close to it it stops, and what it faces there.
        let vehicle = world.vehicle(id);
        let (goal, range, face) = match (self.target, self.spot.spot) {
            (Some(target), Some(spot)) => (spot, SPOT_ARRIVE, target.at),
            (Some(target), None) if target.seen => (target.at, self.role.range(vehicle), target.at),
            (Some(target), None) => (target.at, CLOSE_RANGE, target.at),
            (None, _) => (situation.search, SEARCH_RANGE, situation.search),
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


#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;
    use std::sync::Arc;

    use glam::{Vec2, Vec3};

    use wgtk::space::Space;

    use crate::base::physics::fixtures::{blocks, chieftain_aiming, chieftain_mobility};
    use crate::base::physics::wrap_angle;
    use crate::base::TICK;
    use super::fixtures::{enemy, gun, situation, unarmored, HULL_CENTER};
    use super::motor::{ARMOR_ANGLE, HEADING_TOLERANCE};
    use super::*;

    /// Where a bot's target is, while it sees it.
    fn sighted(bot: &Bot) -> Option<Vec3> {
        bot.target.filter(|target| target.seen).map(|target| target.at)
    }

    /// A bot weighing the enemies it sees again, as it does every [`CHOOSE_TICKS`].
    fn weigh(bot: &mut Bot, world: &World, id: VehicleId, enemies: &[BotEnemy]) {
        for _ in 0..CHOOSE_TICKS {
            bot.choose_target(world, id, &gun(), enemies);
        }
    }

    /// Drive a bot of this class at its enemy, a vehicle standing at `enemy`, for
    /// `ticks`, from `start`, facing +Z.
    fn drive_at(space: Space, start: Vec3, enemy: Vec3, ticks: usize, class: VehicleClass) -> Vec3 {
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(start, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let other = world.add(Vehicle::new(enemy, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemies = [self::enemy(other, enemy + Vec3::Y * HULL_CENTER, 1000.0)];
        let (mut bot, mut gun, armor) = (Bot::new(class), gun(), unarmored());
        for _ in 0..ticks {
            bot.drive(&mut world, id, &mut gun, &situation(&enemies, &armor, enemy, true));
            world.step(0.1);
        }
        let hull = &world.vehicle(id).hull;
        let to = (enemy - hull.position).with_y(0.0);
        let off = wrap_angle(hull.yaw() - to.x.atan2(to.z)).abs();
        assert!((off - ARMOR_ANGLE).abs() <= HEADING_TOLERANCE + 0.05, "{}° off its enemy", off.to_degrees());
        hull.position
    }

    #[test]
    fn targets_the_enemy_worth_most_it_sees_and_keeps_it() {

        // A wall 20 m wide and 5 m high 30 m in front of the bot.
        let space = blocks(&[(Vec3::new(-10.0, 0.0, -71.0), Vec3::new(10.0, 5.0, -69.0))]);
        let mut world = World::new(Arc::new(space), None);
        let id = world.add(Vehicle::new(Vec3::new(0.0, 0.0, -100.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let [hidden, seen, nearer] = [-140.0, -130.0, -120.0].map(|x| {
            world.add(Vehicle::new(Vec3::new(x, 0.0, 140.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()))
        });
        let mut bot = Bot::default();
        let behind_wall = Vec3::new(0.0, HULL_CENTER, -40.0);
        let in_the_open = Vec3::new(60.0, HULL_CENTER, 0.0);
        let near_open = Vec3::new(-30.0, HULL_CENTER, -80.0);
        let far = Vec3::new(0.0, HULL_CENTER, 400.0);

        // The nearest is behind the wall, so it targets the one in the open; one past
        // its view range it doesn't see.
        let enemies = [
            enemy(hidden, behind_wall, 1000.0),
            enemy(seen, in_the_open, 1000.0),
            enemy(nearer, far, 1000.0),
        ];
        bot.choose_target(&world, id, &gun(), &enemies);
        assert_eq!(sighted(&bot), Some(in_the_open));
        // It keeps it while it sees it, against a nearer one that takes more shots.
        weigh(&mut bot, &world, id, &[enemy(seen, in_the_open, 1000.0), enemy(nearer, near_open, 2000.0)]);
        assert_eq!(sighted(&bot), Some(in_the_open));
        // But not against one a shot from destroyed.
        weigh(&mut bot, &world, id, &[enemy(seen, in_the_open, 1000.0), enemy(nearer, near_open, 100.0)]);
        assert_eq!(sighted(&bot), Some(near_open));
        // Behind the wall, still spotted, with no other in sight, it stays its target,
        // where its team spots it, but isn't fired at.
        bot.choose_target(&world, id, &gun(), &[enemy(nearer, behind_wall, 100.0)]);
        assert_eq!(sighted(&bot), None);
        assert_eq!(bot.target.map(|target| target.at), Some(behind_wall));
        // Once gone from those spotted, it is forgotten, and the nearest spotted, which
        // it doesn't see, is its target; none spotted, it has none.
        bot.choose_target(&world, id, &gun(), &[enemy(hidden, far, 1000.0), enemy(seen, behind_wall, 1000.0)]);
        assert_eq!(sighted(&bot), None);
        assert_eq!(bot.target.map(|target| target.vehicle), Some(seen));
        bot.choose_target(&world, id, &gun(), &[]);
        assert!(bot.target.is_none());

    }

    #[test]
    fn searches_where_its_team_last_saw_an_enemy() {
        let space = blocks(&[]);
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(Vec3::new(-120.0, 0.0, -120.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let search = Vec3::new(100.0, 0.0, 100.0);
        let (mut bot, mut gun, armor) = (Bot::default(), gun(), unarmored());
        for _ in 0..900 {
            bot.drive(&mut world, id, &mut gun, &situation(&[], &armor, search, true));
            world.step(0.1);
        }
        let distance = (search - world.vehicle(id).hull.position).with_y(0.0).length();
        assert!(distance <= SEARCH_RANGE + 5.0, "stopped {distance} m away");
    }

    #[test]
    fn leads_a_moving_target() {

        // An enemy 300 m ahead, crossing at 10 m/s to the right.
        let mut world = World::new(Arc::new(blocks(&[])), None);
        let id = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemy = world.add(Vehicle::new(
            Vec3::new(0.0, 0.0, 300.0),
            Vec3::new(0.0, 0.0, FRAC_PI_2),
            chieftain_mobility(),
            chieftain_aiming(),
        ));
        world.vehicle_mut(enemy).hull.speed = 10.0;
        let at = Vec3::new(0.0, HULL_CENTER, 300.0);
        let (mut bot, mut gun, armor) = (Bot::default(), gun(), unarmored());
        bot.drive(&mut world, id, &mut gun, &situation(&[self::enemy(enemy, at, 1000.0)], &armor, at, false));

        // It aims ahead of it by as far as it goes in a tick and the shell's flight.
        let velocity = world.vehicle(enemy).hull.velocity();
        let target = bot.target.unwrap();
        let (eye, _, _) = world.vehicle(id).turret.gun_marker(&world.vehicle(id).hull);
        let flight = TICK.as_secs_f32() + target.aim.distance(eye) / chieftain_aiming().shot_speed;
        assert!((target.aim - (at + velocity * flight)).length() < 0.01, "{}", target.aim);
        assert!(target.aim.x > 4.0, "{}", target.aim);

    }

    #[test]
    fn waits_for_its_aim_to_settle() {

        // An enemy 300 m off to its right: its turret turns to it, its aim spreading.
        let mut world = World::new(Arc::new(blocks(&[])), None);
        let id = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let at = Vec3::new(300.0, 0.0, 0.0);
        let enemy = world.add(Vehicle::new(at, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemies = [self::enemy(enemy, at + Vec3::Y * HULL_CENTER, 1000.0)];
        let (mut bot, mut gun, armor) = (Bot::default(), gun(), unarmored());

        let mut on_target = None;
        let mut fired = None;
        for tick in 0..200 {
            bot.drive(&mut world, id, &mut gun, &situation(&enemies, &armor, at, false));
            world.step(TICK.as_secs_f32());
            let vehicle = world.vehicle(id);
            if on_target.is_none() && vehicle.turret.aim_error(&vehicle.hull) < AIM_TOLERANCE {
                on_target = Some(tick);
            }
            if bot.fires(vehicle, &[]) {
                fired = Some(tick);
                break;
            }
        }

        // On it after turning a quarter, it fires only seconds later, settled.
        let (on_target, fired) = (on_target.unwrap(), fired.unwrap());
        assert!(fired > on_target + 20, "on target at {on_target}, fired at {fired}");
        let (_, _, dispersion) = world.vehicle(id).turret.gun_marker(&world.vehicle(id).hull);
        assert!(dispersion.angle <= chieftain_aiming().dispersion_angle * SETTLED_AIM);

    }

    #[test]
    fn drives_round_an_ally_standing_in_its_way() {

        let space = blocks(&[]);
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let [id, standing] = [-100.0, -50.0].map(|z| {
            world.add(Vehicle::new(Vec3::new(0.0, 0.0, z), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()))
        });
        let [min, max] = chieftain_mobility().bounds;
        let ally = BotAlly {
            center: world.vehicle(standing).hull.position,
            radius: (max - min).length() / 2.0,
            velocity: Vec3::ZERO,
        };
        let search = Vec3::ZERO;
        let (mut bot, mut gun, armor) = (Bot::default(), gun(), unarmored());

        let mut nearest = f32::INFINITY;
        for _ in 0..600 {
            let situation = BotSituation {
                allies: &[ally],
                ..situation(&[], &armor, search, true)
            };
            bot.drive(&mut world, id, &mut gun, &situation);
            world.step(0.1);
            let at = world.vehicle(id).hull.position;
            nearest = nearest.min((at - ally.center).with_y(0.0).length());
        }

        // Round it, never against it, and on to where it searches.
        let end = world.vehicle(id).hull.position;
        assert!(nearest > ally.radius, "{nearest} m from its ally");
        assert!(end.with_y(0.0).length() <= SEARCH_RANGE + 5.0, "stopped at {end:?}");

    }

    #[test]
    fn closes_in_to_its_roles_range_then_angles_its_armour() {
        // A heavy closes in, a tank destroyer stops far off, each about its range: a spot
        // nearer it isn't worth leaving its own for, see SPOT_MARGIN.
        let chieftain = Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        let enemy = Vec3::new(120.0, 0.0, 120.0);
        for class in [VehicleClass::Heavy, VehicleClass::TankDestroyer] {
            let end = drive_at(blocks(&[]), Vec3::new(-120.0, 0.0, -120.0), enemy, 600, class);
            let distance = (enemy - end).with_y(0.0).length();
            let range = BotRole::of(class).range(&chieftain);
            assert!((distance - range).abs() < 30.0, "{class:?} stopped {distance} m away, not {range} m");
        }
    }

    #[test]
    fn drives_around_a_block_in_its_way() {
        // A block 40 m wide and 5 m high across its way, hiding its enemy within its
        // range: it goes round, until it sees it past the block's side.
        let space = blocks(&[(Vec3::new(-20.0, 0.0, -62.0), Vec3::new(20.0, 5.0, -58.0))]);
        let enemy = Vec3::new(0.0, 0.0, 140.0);
        let end = drive_at(space, Vec3::new(0.0, 0.0, -120.0), enemy, 900, VehicleClass::Medium);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(end.x.abs() > 20.0 || end.z > -58.0, "stopped at {end:?}, {distance} m away");
    }

    #[test]
    fn drives_on_until_it_sees_its_enemy() {
        // A wall 60 m wide and 5 m high just in front of its enemy, well within its
        // range: it drives round it, closer than its range, and stops once it sees it.
        let space = blocks(&[(Vec3::new(-30.0, 0.0, 98.0), Vec3::new(30.0, 5.0, 102.0))]);
        let enemy = Vec3::new(0.0, 0.0, 110.0);
        let end = drive_at(space, Vec3::new(0.0, 0.0, -100.0), enemy, 900, VehicleClass::Medium);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(distance < 150.0 && distance >= CLOSE_RANGE - 5.0, "stopped at {end:?}, {distance} m away");
        assert!(end.x.abs() > 25.0 || end.z > 102.0, "stopped at {end:?}, still behind the wall");
    }

}
