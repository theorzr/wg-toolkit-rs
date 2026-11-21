//! The server driving a vehicle itself, as a player would, knowing what a player knows:
//! the enemies its team spots, their health and where their guns point, where its
//! allies are and how they move, the map and its bushes, and what its gun marker shows
//! of a shot, see [`impact::indicate`]. Nothing of its allies' plans.
//!
//! It picks its target among the enemies it sees, by what a shot at each is worth, see
//! [`worth`], finds its way to it over the map, see `World::path`, round its allies,
//! or, while its team spots none, searches where they were last seen, else their team's
//! start. It drives with its controls until it sees its target within its gun's reach,
//! see [`effective_range`], or gets close to it, then stops, its armour angled to it,
//! keeps its turret on it, leading it, loads the shell its gun marker shows best, and
//! says when it fires: it sees it itself, its gun on it, its aim settled, and no ally in
//! the way.
//!
//! ```text
//!   each tick:
//!     0. its target: every CHOOSE_TICKS, or once it no longer sees it, the enemy it
//!          sees worth the most, see worth, another only if worth SWITCH_MARGIN more;
//!          seeing none, the one it has, else the nearest its team spots
//!        its aim: where its target will be once a shell fired gets there, see
//!          Bot::lead, while it sees it
//!        its next shell: the one its gun marker shows best on its target, see
//!          best_shell
//!     1. seeing its target within its effective range, or within CLOSE_RANGE of
//!          it: it stops and turns its hull ARMOR_ANGLE off it, to the side it is
//!          already turned to, see armor_angle; close to where it searches, it stops
//!     2. else, its way, round its allies: planned again once it has none, its goal
//!          moved off its end, or it has been some time on it; points it got close to
//!          dropped
//!     3. it drives to the next point: turning toward it, and driving on only when
//!          headed near enough it, and no ally drives just ahead its way, see
//!          gives_way
//!     4. stuck, driving and not moving: it backs off a little, then plans again
//!
//!   it fires once it sees its target, its gun on its aim, see Turret::aim_error,
//!   its shells landing within FIT_RADIUS of it or its aim settled, see Bot::aimed,
//!   and no ally in the way
//! ```

use glam::Vec3;

use crate::base::physics::{movement_flag, wrap_angle, Dispersion, Vehicle, VehicleId, World};
use crate::base::ballistics::impact::{self, Indication};
use crate::game::item_defs::{ShellKind, ShotDef};
use crate::base::ballistics::shot::ShotTarget;
use crate::base::ballistics::gun::GunLoader;
use crate::base::TICK;

use super::VIEW_RANGE;


/// How close to its target the bot drives at most, in metres on the ground plane, seeing
/// it or not: not to push into it. Our choice.
const CLOSE_RANGE: f32 = 30.0;
/// How far off its target its hull turns once stopped, in radians: its front armour
/// thicker along a shell's way, its side not yet bare. Our choice, see [`armor_angle`].
const ARMOR_ANGLE: f32 = 25.0f32.to_radians();
/// How far within its turret's yaw limits the bot keeps its target, angling its hull,
/// in radians. Our choice.
const TURRET_LIMIT_MARGIN: f32 = 3.0f32.to_radians();
/// How close to where it searches the bot drives, in metres on the ground plane. Our
/// choice: its team gets there within `spotting::MIN_RADIUS`, which ends the search.
const SEARCH_RANGE: f32 = 10.0;
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
const HEADING_TOLERANCE: f32 = 0.1;
/// How close the gun must point to its aim to fire, in radians, see
/// [`Turret::aim_error`](crate::base::physics::Turret::aim_error).
const AIM_TOLERANCE: f32 = 0.002;
/// How far from its aim the bot's shells may stray for it to fire, in metres: about half
/// a hull's height. Our choice.
const FIT_RADIUS: f32 = 1.0;
/// How settled the bot's aim must be, its dispersion of its gun's best, for it to fire at
/// a target too far for [`FIT_RADIUS`]. Our choice.
const SETTLED_AIM: f32 = 1.1;
/// How many times the bot works out where its target will be, from where the last time
/// said, see [`Bot::lead`].
const LEAD_PASSES: u32 = 3;
/// How near ahead an ally must be for the bot to give way to it, in metres between their
/// centres on the ground plane, and how far off its heading, in radians, see
/// [`gives_way`]. Our choice: a hull's length and some.
const GIVE_WAY_DISTANCE: f32 = 12.0;
const GIVE_WAY_ANGLE: f32 = 35.0f32.to_radians();
/// How fast an ally must drive the bot's way, in m/s, for it to give way: one standing
/// or coming at it is driven round instead.
const GIVE_WAY_SPEED: f32 = 1.0;
/// How many ticks the bot keeps its target before weighing the enemies it sees again,
/// see [`worth`], and how much more another must be worth for it to switch. Our choice:
/// half a second, as a player looks round.
const CHOOSE_TICKS: u32 = 5;
const SWITCH_MARGIN: f32 = 1.3;
/// What a shot is worth by what the gun marker shows, red, yellow and green, see
/// [`worth`]: a red one still hits, a yellow one pierces about half the time. Our choice.
const INDICATION_WORTH: [f32; 3] = [0.1, 0.5, 1.0];
/// How close an enemy's gun must point to the bot for it to be a threat, in radians, and
/// how much more it is then worth. Our choice.
const AIMED_AT: f32 = 3.0f32.to_radians();
const THREAT_WORTH: f32 = 1.5;
/// How far an enemy is for its worth to halve, in metres. Our choice.
const HALF_WORTH_DISTANCE: f32 = 200.0;
/// How close to a point of its way the bot gets before heading for the next, in metres.
/// Our choice: as close as it can turn short of it.
const ARRIVE: f32 = 4.0;
/// How far its enemy may move off its way's end before the bot plans again, in metres.
const REPLAN_DISTANCE: f32 = 25.0;
/// How many ticks the bot keeps a way before planning again anyway: what moved, a
/// vehicle in its way say, is planned around.
const REPLAN_TICKS: u32 = 50;
/// How much clearance the bot keeps past its hull's half width, in metres: a cell's is
/// measured from the middle of the nearest one it can't enter, half a cell off the wall.
const WIDTH_MARGIN: f32 = 1.0;
/// How slow the bot must go, in m/s, driving, for how many ticks, to be stuck.
const STUCK_SPEED: f32 = 0.5;
const STUCK_TICKS: u32 = 20;
/// How many ticks the bot backs off for once stuck.
const BACK_OFF_TICKS: u32 = 15;


/// The server driving a vehicle, as a client would: it sets its controls and its aim.
#[derive(Debug, Clone, Default)]
pub struct Bot {
    /// The enemy it aims at, see [`Self::spot`].
    target: Option<Target>,
    /// How many ticks since it weighed the enemies it sees, see [`CHOOSE_TICKS`].
    chose: u32,
    /// The points of its way it hasn't reached, the next first.
    way: Vec<Vec3>,
    /// Where its goal was when it planned its way.
    goal: Option<Vec3>,
    /// How many ticks since it planned its way.
    planned: u32,
    /// How many ticks it has driven without moving.
    stuck: u32,
    /// How many ticks it still backs off for.
    backing: u32,
    /// The side it angles its hull to once stopped, see [`ARMOR_ANGLE`]: 1 to the right
    /// of its target, -1 to the left, 0 while it drives.
    side: f32,
}

/// What a bot knows of its battle as it drives, see [`Bot::drive`].
#[derive(Debug, Clone, Copy)]
pub struct Situation<'a> {
    /// The enemies its team spots, see [`Enemy`].
    pub enemies: &'a [Enemy],
    /// Its allies alive, see [`Ally`].
    pub allies: &'a [Ally],
    /// Where it searches while its team spots no enemy.
    pub search: Vec3,
    /// Whether it drives, or only aims.
    pub drives: bool,
}

/// An enemy its team spots, as a bot sees it, see [`Situation::enemies`].
#[derive(Debug, Clone)]
pub struct Enemy {
    /// Its vehicle, in the world.
    pub vehicle: VehicleId,
    /// Where its hull's centre is.
    pub center: Vec3,
    /// Its health left, as its marker shows it.
    pub health: f32,
    /// Its armour, where it is, which the bot's gun marker shows a shot at, see
    /// [`impact::indicate`].
    pub armor: ShotTarget,
}

/// An ally of a bot, as it drives and fires, see [`Situation::allies`].
#[derive(Debug, Clone, Copy)]
pub struct Ally {
    /// Where its hull's centre is.
    pub center: Vec3,
    /// How far round its centre a shell hits it, or the bot drives into it: half its
    /// hull's box's diagonal.
    pub radius: f32,
    /// Its velocity on the ground plane, in m/s.
    pub velocity: Vec3,
}

impl Bot {

    /// Set a vehicle's controls and aim for its next step, in its `situation`, and the
    /// shell its `gun` loads next: it picks its target, see [`Self::spot`], and heads
    /// for it, else for where it searches, its turret on it; if it drives, finding its
    /// way over its world's map round its allies until it sees its target within its
    /// [`effective_range`], or within [`CLOSE_RANGE`] of it, then turning its armour to
    /// it, or within [`SEARCH_RANGE`] of where it searches. See the module's
    /// documentation.
    pub fn drive(&mut self, world: &mut World, id: VehicleId, gun: &mut GunLoader, situation: &Situation) {

        self.spot(world, id, gun, situation.enemies);
        self.lead(world, id);
        self.choose_shell(world, id, gun, situation.enemies);

        // What it heads for, and how close to it it stops.
        let vehicle = world.vehicle(id);
        let (goal, range, angle) = match self.target {
            Some(target) if target.seen => (
                target.at,
                effective_range(vehicle),
                armor_angle(vehicle),
            ),
            Some(target) => (target.at, CLOSE_RANGE, armor_angle(vehicle)),
            None => (situation.search, SEARCH_RANGE, 0.0),
        };
        let aim = self.target.map_or(goal, |target| target.aim);
        world.vehicle_mut(id).turret.track(aim);

        let flags = if situation.drives {
            self.controls(world, id, goal, range, angle, situation.allies)
        } else {
            0
        };
        world.vehicle_mut(id).hull.flags = flags;

    }

    /// Pick its target among the `enemies` its team spots, see the module's
    /// documentation: every [`CHOOSE_TICKS`], or once it no longer sees it, the one it
    /// sees worth the most, see [`worth`], another than the one it has only if worth
    /// [`SWITCH_MARGIN`] more; seeing none, the one it has, else the nearest. One its
    /// team no longer spots, or destroyed, is forgotten.
    fn spot(&mut self, world: &World, id: VehicleId, gun: &GunLoader, enemies: &[Enemy]) {

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
                self.target = Some(Target {
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
            self.target = nearest.map(|enemy| Target {
                vehicle: enemy.vehicle,
                at: enemy.center,
                aim: enemy.center,
                seen: false,
            });
        }

    }

    /// Load next the shell its gun marker shows best on its target, while it sees it, as
    /// it weighs the enemies it sees, see [`best_shell`].
    fn choose_shell(&self, world: &World, id: VehicleId, gun: &mut GunLoader, enemies: &[Enemy]) {

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

    /// Aim where its target will be once a shell it fires gets there, while it sees it:
    /// its hull's centre moved on at its velocity for a tick, the battle firing after
    /// its step, then for the shell's flight to there, worked out again from there
    /// [`LEAD_PASSES`] times. The flight is the distance at the shell's speed, as near
    /// as its low arc is to a line.
    fn lead(&mut self, world: &World, id: VehicleId) {

        let Some(target) = self.target.as_mut().filter(|target| target.seen) else { return };
        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);
        let speed = vehicle.turret.aiming.shot_speed.max(f32::EPSILON);
        let velocity = world.vehicle(target.vehicle).hull.velocity();

        for _ in 0..LEAD_PASSES {
            let flight = TICK.as_secs_f32() + target.aim.distance(eye) / speed;
            target.aim = target.at + velocity * flight;
        }

    }

    /// The controls driving a vehicle to its goal, round its `allies`, until within
    /// `range` of it, then turning it `angle` off it, see [`Self::drive`].
    fn controls(
        &mut self,
        world: &mut World,
        id: VehicleId,
        goal: Vec3,
        range: f32,
        angle: f32,
        allies: &[Ally],
    ) -> u8 {

        let hull = &world.vehicle(id).hull;
        let (position, yaw) = (hull.position, hull.yaw());

        // 4. Stuck, it backs off first.
        if self.backing > 0 {
            self.backing -= 1;
            return movement_flag::BACKWARD;
        }
        if hull.flags & movement_flag::FORWARD != 0 && hull.speed.abs() < STUCK_SPEED {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_TICKS {
            self.stuck = 0;
            self.backing = BACK_OFF_TICKS;
            self.way.clear();
            return movement_flag::BACKWARD;
        }

        // 1. Close, it stops, turned off its goal to the side it first stopped on.
        let to_goal = (goal - position).with_y(0.0);
        if to_goal.length() <= range {
            self.way.clear();
            let toward = to_goal.x.atan2(to_goal.z);
            if self.side == 0.0 {
                self.side = if wrap_angle(yaw - toward) < 0.0 { -1.0 } else { 1.0 };
            }
            return steer(wrap_angle(toward + self.side * angle - yaw), false);
        }
        self.side = 0.0;

        // 2. Its way, round its allies, planned again as needed, the points it got close
        //    to dropped.
        self.planned += 1;
        let moved = self.goal.is_none_or(|planned| planned.distance(goal) > REPLAN_DISTANCE);
        if self.way.is_empty() || moved || self.planned >= REPLAN_TICKS {
            let [min, max] = hull.mobility.bounds;
            let radius = max.x.max(-min.x) + WIDTH_MARGIN;
            let around: Vec<_> = allies.iter()
                .map(|ally| (ally.center, ally.radius))
                .collect();
            self.way = world.path(position, goal, radius, &around).unwrap_or_default();
            self.goal = Some(goal);
            self.planned = 0;
        }
        while self.way.first().is_some_and(|point| (*point - position).with_y(0.0).length() < ARRIVE) {
            self.way.remove(0);
        }

        // 3. It drives to the next point, its goal if none, but behind an ally its way.
        let next = self.way.first().copied().unwrap_or(goal);
        let to = (next - position).with_y(0.0);
        let off = wrap_angle(to.x.atan2(to.z) - yaw);
        steer(off, off.abs() < TURN_WHILE_DRIVING && !gives_way(position, yaw, allies))

    }

    /// Whether it fires its vehicle's gun now: at its target, which it sees, its gun on
    /// its aim, see [`Self::lead`], its aim settled, see [`Self::aimed`], and none of
    /// its `allies` in the way, see [`in_the_way`].
    pub fn fires(&self, vehicle: &Vehicle, allies: &[Ally]) -> bool {

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

/// An enemy a bot aims at, see [`Bot::spot`]: its vehicle, where its hull's centre is,
/// where the bot aims, and whether the bot sees it itself.
#[derive(Debug, Clone, Copy)]
struct Target {
    /// Its vehicle, in the world.
    vehicle: VehicleId,
    /// Where its hull's centre is, as the bot's team spots it.
    at: Vec3,
    /// Where the bot aims: where it will be once a shell fired gets there while the bot
    /// sees it, see [`Bot::lead`], else where it is.
    aim: Vec3,
    /// Whether the bot sees it itself, which it fires at only then.
    seen: bool,
}

/// How far a vehicle's gun reaches, as a bot fires it, in metres on the ground plane:
/// as far as its best dispersion lands its shells within [`FIT_RADIUS`] of its aim, see
/// [`Bot::aimed`], no closer than [`CLOSE_RANGE`], no farther than it sees, see
/// [`VIEW_RANGE`].
fn effective_range(vehicle: &Vehicle) -> f32 {
    let best = vehicle.turret.aiming.dispersion_angle.tan().max(f32::EPSILON);
    (FIT_RADIUS / best).clamp(CLOSE_RANGE, VIEW_RANGE)
}

/// How far off its target a vehicle's hull turns once stopped, see [`ARMOR_ANGLE`]: as
/// far as its turret's yaw limits keep it on its target, [`TURRET_LIMIT_MARGIN`] within
/// them, none if they don't.
fn armor_angle(vehicle: &Vehicle) -> f32 {
    match vehicle.turret.aiming.yaw_limits {
        Some((min, max)) => (max.min(-min) - TURRET_LIMIT_MARGIN).clamp(0.0, ARMOR_ANGLE),
        None => ARMOR_ANGLE,
    }
}

/// Whether a bot's gun at `eye` sees a point: within [`VIEW_RANGE`] on the ground plane,
/// and nothing of the map in the way, see [`World::sees`].
fn in_sight(world: &World, eye: Vec3, at: Vec3) -> bool {
    (at - eye).with_y(0.0).length() <= VIEW_RANGE && world.sees(eye, at)
}

/// What a shot at an enemy it sees is worth to a bot at `eye`, its vehicle of this id:
/// by what its gun marker shows of its best shell there, see [`INDICATION_WORTH`], over
/// the shots it takes to destroy it, more if its gun points at the bot, see
/// [`THREAT_WORTH`], less the farther, see [`HALF_WORTH_DISTANCE`].
fn worth(world: &World, id: VehicleId, eye: Vec3, gun: &GunLoader, enemy: &Enemy) -> f32 {

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
fn best_shell<'a>(
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
fn aims_at(vehicle: &Vehicle, point: Vec3) -> bool {
    let (position, velocity, _) = vehicle.turret.gun_marker(&vehicle.hull);
    velocity.angle_between(point - position) < AIMED_AT
}

/// Whether a bot at `position`, heading `yaw`, gives way to one of its `allies`: one
/// driving its way, faster than [`GIVE_WAY_SPEED`], within [`GIVE_WAY_DISTANCE`] ahead of
/// it and [`GIVE_WAY_ANGLE`] of its heading.
fn gives_way(position: Vec3, yaw: f32, allies: &[Ally]) -> bool {
    let heading = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    allies.iter().any(|ally| {
        let to = (ally.center - position).with_y(0.0);
        ally.velocity.dot(heading) > GIVE_WAY_SPEED
            && to.length() < GIVE_WAY_DISTANCE
            && wrap_angle(to.x.atan2(to.z) - yaw).abs() < GIVE_WAY_ANGLE
    })
}

/// The controls turning toward a way `off` the heading, in radians, positive to the
/// right, driving forward if `forward`.
fn steer(off: f32, forward: bool) -> u8 {
    let mut flags = 0;
    if off > HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_RIGHT;
    } else if off < -HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_LEFT;
    }
    if forward {
        flags |= movement_flag::FORWARD;
    }
    flags
}

/// Whether something within `radius` of `center` is in the way from a gun at `from` to
/// `to`: between them, the line passing within its radius.
fn in_the_way(from: Vec3, to: Vec3, center: Vec3, radius: f32) -> bool {
    let line = to - from;
    let along = (center - from).dot(line) / line.length_squared().max(f32::EPSILON);
    along > 0.0 && along < 1.0 && (from + line * along).distance(center) < radius
}


#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;
    use std::sync::Arc;

    use glam::{Affine3A, Vec2};

    use wgtk::space::Space;

    use crate::base::ballistics::fixtures::{material, plate, shot, vehicle as plated};
    use crate::base::physics::fixtures::{blocks, chieftain_aiming, chieftain_mobility};
    use super::*;

    /// How high an enemy's hull's centre is over the ground, about, in metres.
    const HULL_CENTER: f32 = 1.5;

    /// Where a bot's target is, while it sees it.
    fn sighted(bot: &Bot) -> Option<Vec3> {
        bot.target.filter(|target| target.seen).map(|target| target.at)
    }

    /// What a bot knows driving among no allies.
    fn situation(enemies: &[Enemy], search: Vec3, drives: bool) -> Situation<'_> {
        Situation {
            enemies,
            allies: &[],
            search,
            drives,
        }
    }

    /// An enemy of this vehicle, with its hull's centre at `center`, its health, and a
    /// plate of 100 mm across its centre, facing along z.
    fn enemy(vehicle: VehicleId, center: Vec3, health: f32) -> Enemy {
        let (collision, _) = plated(vec![plate(0.0, material(100.0, true))]);
        Enemy {
            vehicle,
            center,
            health,
            armor: ShotTarget {
                entity_id: 0,
                collision: Arc::new(collision),
                transforms: [Affine3A::from_translation(center); 4],
            },
        }
    }

    /// A gun of AP shells piercing 200 mm, dealing 400 damage.
    fn gun() -> GunLoader {
        GunLoader::new(vec![(shot(ShellKind::ArmorPiercing, 120.0, 200.0), 10)], 10.0)
    }

    /// A bot weighing the enemies it sees again, as it does every [`CHOOSE_TICKS`].
    fn weigh(bot: &mut Bot, world: &World, id: VehicleId, enemies: &[Enemy]) {
        for _ in 0..CHOOSE_TICKS {
            bot.spot(world, id, &gun(), enemies);
        }
    }

    /// Drive a bot at its enemy, a vehicle standing at `enemy`, for `ticks`, from
    /// `start`, facing +Z.
    fn drive_at(space: Space, start: Vec3, enemy: Vec3, ticks: usize) -> Vec3 {
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(start, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let other = world.add(Vehicle::new(enemy, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let enemies = [self::enemy(other, enemy + Vec3::Y * HULL_CENTER, 1000.0)];
        let (mut bot, mut gun) = (Bot::default(), gun());
        for _ in 0..ticks {
            bot.drive(&mut world, id, &mut gun, &situation(&enemies, enemy, true));
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
        bot.spot(&world, id, &gun(), &enemies);
        assert_eq!(sighted(&bot), Some(in_the_open));
        // It keeps it while it sees it, against a nearer one that takes more shots.
        weigh(&mut bot, &world, id, &[enemy(seen, in_the_open, 1000.0), enemy(nearer, near_open, 2000.0)]);
        assert_eq!(sighted(&bot), Some(in_the_open));
        // But not against one a shot from destroyed.
        weigh(&mut bot, &world, id, &[enemy(seen, in_the_open, 1000.0), enemy(nearer, near_open, 100.0)]);
        assert_eq!(sighted(&bot), Some(near_open));
        // Behind the wall, still spotted, with no other in sight, it stays its target,
        // where its team spots it, but isn't fired at.
        bot.spot(&world, id, &gun(), &[enemy(nearer, behind_wall, 100.0)]);
        assert_eq!(sighted(&bot), None);
        assert_eq!(bot.target.map(|target| target.at), Some(behind_wall));
        // Once gone from those spotted, it is forgotten, and the nearest spotted, which
        // it doesn't see, is its target; none spotted, it has none.
        bot.spot(&world, id, &gun(), &[enemy(hidden, far, 1000.0), enemy(seen, behind_wall, 1000.0)]);
        assert_eq!(sighted(&bot), None);
        assert_eq!(bot.target.map(|target| target.vehicle), Some(seen));
        bot.spot(&world, id, &gun(), &[]);
        assert!(bot.target.is_none());

    }

    #[test]
    fn searches_where_its_team_last_saw_an_enemy() {
        let space = blocks(&[]);
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(Vec3::new(-120.0, 0.0, -120.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let search = Vec3::new(100.0, 0.0, 100.0);
        let (mut bot, mut gun) = (Bot::default(), gun());
        for _ in 0..900 {
            bot.drive(&mut world, id, &mut gun, &situation(&[], search, true));
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
        let (mut bot, mut gun) = (Bot::default(), gun());
        bot.drive(&mut world, id, &mut gun, &situation(&[self::enemy(enemy, at, 1000.0)], at, false));

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
        let (mut bot, mut gun) = (Bot::default(), gun());

        let mut on_target = None;
        let mut fired = None;
        for tick in 0..200 {
            bot.drive(&mut world, id, &mut gun, &situation(&enemies, at, false));
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
    fn drives_round_an_ally_standing_in_its_way() {

        let space = blocks(&[]);
        let mut world = World::new(Arc::new(space), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let [id, standing] = [-100.0, -50.0].map(|z| {
            world.add(Vehicle::new(Vec3::new(0.0, 0.0, z), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()))
        });
        let [min, max] = chieftain_mobility().bounds;
        let ally = Ally {
            center: world.vehicle(standing).hull.position,
            radius: (max - min).length() / 2.0,
            velocity: Vec3::ZERO,
        };
        let search = Vec3::ZERO;
        let (mut bot, mut gun) = (Bot::default(), gun());

        let mut nearest = f32::INFINITY;
        for _ in 0..600 {
            let situation = Situation {
                allies: &[ally],
                ..situation(&[], search, true)
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
    fn gives_way_to_an_ally_driving_its_way() {

        let ally = Ally {
            center: Vec3::new(1.0, HULL_CENTER, 8.0),
            radius: 4.0,
            velocity: Vec3::Z * 5.0,
        };

        // Just ahead, driving its way: it waits.
        assert!(gives_way(Vec3::ZERO, 0.0, &[ally]));
        // Not standing, coming at it, far, or aside.
        let standing = Ally {
            velocity: Vec3::ZERO,
            ..ally
        };
        let coming = Ally {
            velocity: -Vec3::Z * 5.0,
            ..ally
        };
        let far = Ally {
            center: Vec3::new(0.0, HULL_CENTER, 20.0),
            ..ally
        };
        assert!(!gives_way(Vec3::ZERO, 0.0, &[standing, coming, far]));
        assert!(!gives_way(Vec3::ZERO, FRAC_PI_2, &[ally]));

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

    #[test]
    fn closes_in_then_angles_its_armour() {
        let space = blocks(&[]);
        let enemy = Vec3::new(120.0, 0.0, 120.0);
        let end = drive_at(space, Vec3::new(-120.0, 0.0, -120.0), enemy, 600);
        let distance = (enemy - end).with_y(0.0).length();
        let chieftain = Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        let range = effective_range(&chieftain);
        assert!(distance <= range && distance > range - 20.0, "stopped {distance} m away");
    }

    #[test]
    fn drives_around_a_block_in_its_way() {
        // A block 40 m wide and 5 m high across its way, hiding its enemy within its
        // range: it goes round, until it sees it past the block's side.
        let space = blocks(&[(Vec3::new(-20.0, 0.0, -62.0), Vec3::new(20.0, 5.0, -58.0))]);
        let enemy = Vec3::new(0.0, 0.0, 140.0);
        let end = drive_at(space, Vec3::new(0.0, 0.0, -120.0), enemy, 900);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(end.x.abs() > 20.0 || end.z > -58.0, "stopped at {end:?}, {distance} m away");
    }

    #[test]
    fn drives_on_until_it_sees_its_enemy() {
        // A wall 60 m wide and 5 m high just in front of its enemy, well within its
        // range: it drives round it, closer than its range, and stops once it sees it.
        let space = blocks(&[(Vec3::new(-30.0, 0.0, 98.0), Vec3::new(30.0, 5.0, 102.0))]);
        let enemy = Vec3::new(0.0, 0.0, 110.0);
        let end = drive_at(space, Vec3::new(0.0, 0.0, -100.0), enemy, 900);
        let distance = (enemy - end).with_y(0.0).length();
        assert!(distance < 150.0 && distance >= CLOSE_RANGE - 5.0, "stopped at {end:?}, {distance} m away");
        assert!(end.x.abs() > 25.0 || end.z > 102.0, "stopped at {end:?}, still behind the wall");
    }

}
