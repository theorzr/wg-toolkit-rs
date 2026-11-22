//! What each team of a battle spots of the other's vehicles, which its clients see, by
//! the game's rules as the community documents them (the wiki's "Battle Mechanics",
//! `Spotting Mechanics`): the client doesn't have them.
//!
//! # Ports and checkpoints
//!
//! A vehicle looks from its view range ports, in turns: its static one, the middle of
//! its top, its turret's roof as it faces ahead, else its hull's, and its dynamic one,
//! where its gun is mounted, turning with its turret. One looks each tick, the other
//! the next, each vehicle from a random one first; the wiki has them take turns every
//! 2 s, our choice is every tick. It is looked at by six
//! checkpoints: its two ports, the middles of its hull's front and rear, and of its
//! turret's left and right sides, halfway up what of the turret rises over its hull,
//! its box reaching down into it, all [`POINT_INSET`] inside its boxes, see [`Viewed`].
//! An observer spots a vehicle once it spots one of its checkpoints: a turret over a
//! hedge is spotted, the hull behind it hidden, and the foliage hides a vehicle only if
//! it covers all its checkpoints.
//!
//! ```text
//!        static port, top ─▶ ●  ● ◀─ dynamic port, gun mount
//!                       left ● ┌┴──┴┐ ● right          ● its checkpoints, its two
//!                        ┌─────┴────┴─────┐             ports among them
//!                 rear ● │      hull      │ ● front
//!                        └────────────────┘
//! ```
//!
//! # Near, else a checkpoint spotted
//!
//! An enemy within MIN_RADIUS of a vehicle, on the ground plane, the middle of its
//! hull's box to the middle of the vehicle's, spots it, whatever is in the way: once for
//! both, rather than from each port to each checkpoint, see [`near`] (Near). Else:
//!
//! ```text
//!   distance d, on the ground plane, from the observer's port to the checkpoint:
//!     d > range(camouflage)                not spotted, nothing looked at    (Far)
//!     the map in the way, World::sees      not spotted                       (Blocked)
//!     else the foliage on the line hides it (Clear):
//!       hidden = camouflage + min(VEGETATION_CAP, Σ density of each plant crossed),
//!         at most 1, but the plants it enters within FOLIAGE_RADIUS of the port,
//!         which the observer sees through; and, the target fired within
//!         SHOT_REVEAL_TIME, of those it leaves within FOLIAGE_RADIUS of the
//!         checkpoint, only the densest counts, by FIRED_FOLIAGE;
//!       spotted if d ≤ range(hidden)
//!
//!   range(c) = MIN_RADIUS + (view range − MIN_RADIUS) · (1 − c), at most VIEW_RANGE
//! ```
//!
//! The camouflage is the target's own, still or moving, times its
//! `camouflage_at_shot` for SHOT_REVEAL_TIME after it fired, see
//! `BattleVehicle::camouflage`: the wiki's
//! `baseCamo · camoAtShot + environmentCamo`, its crew's skill and camouflage pattern
//! and net aside. A plant's density is 0.5 in leaf, 0.25 bare.
//! The foliage is as it stands or lies now, the trees knocked down where they fell,
//! see `World::foliage`.
//!
//! # A team's sightings
//!
//! A team spots what any of its vehicles spots, still for LINGER once none does.

use std::cmp::Ordering;

use rand::Rng;
use glam::{Vec3, Vec4};

use crate::base::debug::{self, DebugChannel, DebugPlace};
use crate::base::physics::World;

use super::vehicle::BattleVehicle;
use super::{TEAMS, VIEW_RANGE};


/// How close an enemy is always spotted, in metres on the ground plane from hull to hull,
/// whatever is in the way: `VISIBILITY.MIN_RADIUS`.
const MIN_RADIUS: f32 = 50.0;

/// How long a team still spots an enemy once none of its vehicles does, in seconds: the
/// wiki's spotting duration.
const LINGER: f64 = 10.0;

/// How near an observer the foliage doesn't hide from it, and how near a vehicle that
/// just fired it hides it less, in metres along the line of sight: the game's 15 m
/// rule, see the module's documentation.
const FOLIAGE_RADIUS: f32 = 15.0;

/// How much the foliage adds to a vehicle's camouflage at most, all its plants
/// together: the wiki's 80 %.
const VEGETATION_CAP: f32 = 0.8;

/// How much of its density the densest plant near a vehicle that just fired still
/// hides it by, the others near it none, see the module's documentation: the wiki's
/// 30 %.
const FIRED_FOLIAGE: f32 = 0.3;

/// How far inside a vehicle's boxes its eye and its points are, in metres, see
/// [`Viewed`]: off their faces, so that the wall it leans on or the ground it stands
/// on doesn't hide them. Our choice.
const POINT_INSET: f32 = 0.1;

/// The colours of a vehicle's text in the debug drawing, of 255, pure for contrast, none
/// a team's, neither red, green, gold nor purple: spotted, magenta, not, cyan, and with
/// no enemy looking, white. See [`Spotting::debug_draw`].
const DEBUG_SPOTTED: Vec4 = Vec4::new(255.0, 64.0, 255.0, 255.0);
const DEBUG_HIDDEN: Vec4 = Vec4::new(64.0, 224.0, 255.0, 255.0);
const DEBUG_UNSEEN: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);
/// How big a vehicle's checkpoints are in the debug drawing, its ports among them, as
/// radii in metres. See [`Spotting::debug_draw`].
const DEBUG_POINT_RADIUS: f32 = 0.08;
const DEBUG_PORT_RADIUS: f32 = 0.15;
/// How its enemies' best line of sight to a vehicle is drawn in the debug drawing: a line
/// in the colour of their team, of 255, the first blue, the second orange, darker by
/// [`DEBUG_HIDDEN_SHADE`] if it doesn't spot it, see [`Spotting::debug_draw`].
const DEBUG_TEAM_COLORS: [Vec4; 2] = [
    Vec4::new(80.0, 150.0, 255.0, 255.0),
    Vec4::new(255.0, 150.0, 40.0, 255.0),
];
const DEBUG_HIDDEN_SHADE: f32 = 0.6;


/// See the module's documentation.
#[derive(Debug)]
pub struct Spotting {
    /// For each of the battle's vehicles, by index, when its enemies last spotted it, the
    /// one team that may: none once destroyed.
    sightings: Vec<Option<Sighting>>,
    /// For each of the battle's vehicles, by index, its enemies' best look at it this
    /// tick, see [`Self::look`].
    looks: Vec<Option<Look>>,
    /// For each of the battle's vehicles, by index, its checkpoints in the world this
    /// tick, its ports first, see [`Viewed`].
    points: Vec<[Vec3; 6]>,
    /// For each of the battle's vehicles, by index, which of its view range ports it
    /// looks from this tick, see the module's documentation.
    ports: Vec<usize>,
}

/// How an observer looks at a checkpoint of an enemy, see [`look`], whether it spots
/// it or not and why.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// The checkpoint of the enemy looked at, none if near, see [`near`].
    pub point: Option<Point>,
    /// How far the checkpoint is from the observer's port, on the ground plane, in
    /// metres, or near, its hull from the observer's.
    pub distance: f32,
    /// What is in the way, as far as it was looked at.
    pub sight: Sight,
    /// The enemy's camouflage, then raised by the foliage in the way, of 1, see the
    /// module's documentation.
    pub camouflage: f32,
    pub hidden: f32,
    /// How many plants in the way hide it.
    pub plants: usize,
    /// How far the observer spots it, so hidden, in metres on the ground plane.
    pub range: f32,
    /// Where the observer looks from, its port, and at, the checkpoint, in the world, or
    /// near, the middles of their hulls' boxes.
    pub eye: Vec3,
    pub at: Vec3,
}

/// What is in the way of a [`Look`], as far as it was looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sight {
    /// Within [`MIN_RADIUS`], hull to hull: nothing was looked at, it is spotted, see
    /// [`near`].
    Near,
    /// Beyond its range before the foliage: nothing was looked at, it isn't spotted.
    Far,
    /// The map is in the way, see `World::sees`.
    Blocked,
    /// Nothing of the map is in the way: the foliage was looked at.
    Clear,
}

/// A checkpoint of a vehicle that its enemies look at, see the module's documentation,
/// in the order they are: its top first, the highest, most often in sight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// The middle of its top, its static view range port.
    Top,
    /// Where its gun is mounted, its dynamic view range port.
    Gun,
    /// The middles of its hull's front and rear.
    Front,
    Rear,
    /// The middles of its turret's left and right sides, halfway up what of it rises
    /// over its hull.
    Left,
    Right,
}

/// Where a vehicle looks from and its checkpoints, in the world this tick, see the
/// module's documentation.
#[derive(Debug, Clone, Copy)]
struct Viewed {
    /// The middle of its hull's box.
    center: Vec3,
    /// Its view range ports, static then dynamic.
    ports: [Vec3; 2],
    /// Its checkpoints, in [`POINTS`] order.
    points: [Vec3; 6],
}

/// The checkpoints of a vehicle, in the order they are looked at.
const POINTS: [Point; 6] = [Point::Top, Point::Gun, Point::Front, Point::Rear, Point::Left, Point::Right];

/// When a team last spotted an enemy.
#[derive(Debug, Clone, Copy)]
struct Sighting {
    /// The team that spotted it, the enemy's enemies.
    by: u8,
    /// The game time it last did at.
    at: f64,
}

impl Spotting {

    /// No vehicle spotted yet, of a battle's `vehicles`, each looking first from a random
    /// one of its ports.
    pub fn new(vehicles: usize) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            sightings: vec![None; vehicles],
            looks: vec![None; vehicles],
            points: vec![[Vec3::ZERO; 6]; vehicles],
            ports: (0..vehicles).map(|_| rng.gen_range(0..2)).collect(),
        }
    }

    /// At game time `now`, what each team spots of the other's vehicles alive, see the
    /// module's documentation:
    ///
    /// 1. Each vehicle's ports and checkpoints in the world, once for the tick.
    /// 2. For each target alive, whether it fired lately, and its camouflage now, cut
    ///    to its share at a shot if it did.
    /// 3. An enemy alive within [`MIN_RADIUS`] of it, hull to hull, spots it, see
    ///    [`near`], the nearest the look kept.
    /// 4. Else each enemy alive looks at its checkpoints from its port of this tick, see
    ///    [`look_at`], until one spots one: then the target is spotted, and the look
    ///    kept is that one. Else the look kept is the nearest to spotting it, see
    ///    [`better`].
    /// 5. Spotted, its enemies' sighting of it is now. Else their last one lingers.
    /// 6. Each vehicle's other port looks the next tick.
    ///
    /// A destroyed vehicle is forgotten.
    pub fn update(&mut self, now: f64, vehicles: &[BattleVehicle], world: &World) {

        // 1. Where each vehicle looks from and is looked at, as it stands now.
        let viewed = vehicles.iter().map(|vehicle| viewed(world, vehicle)).collect::<Vec<_>>();
        for (points, seen) in self.points.iter_mut().zip(&viewed) {
            *points = seen.points;
        }

        for (index, target) in vehicles.iter().enumerate() {

            // 2. A wreck is forgotten: nobody spots it.
            self.looks[index] = None;
            if !target.alive() {
                self.sightings[index] = None;
                continue;
            }

            // 3. How hard it is to see, its foliage aside: still or moving, and for a
            //    while after it fired, only its share at a shot, when the foliage near it
            //    hides it less too.
            let (camouflage, revealed) = target.camouflage(world.vehicle(target.id), now);
            let observers = vehicles.iter()
                .zip(&viewed)
                .zip(&self.ports)
                .filter(|((observer, _), _)| observer.alive() && observer.team != target.team);

            // 4. Near an enemy alive, hull to hull, it is spotted, whatever is in the
            //    way: nothing else is looked at.
            let near = observers.clone()
                .filter_map(|((observer, seen), _)| near(observer.view_range(), seen, &viewed[index], camouflage))
                .min_by(|a, b| a.distance.total_cmp(&b.distance));

            // 5. Else each enemy alive looks at its checkpoints from its port of this tick,
            //    the first spotting one ending it all: spotted once by one, it is by its
            //    team. The look kept, for the debug drawing, is that one, else the
            //    nearest to.
            let mut best = near;
            let looks = observers.clone()
                .filter(|_| near.is_none())
                .map(|((observer, seen), &port)| (observer.view_range(), seen.ports[port]));
            for (view_range, port) in looks {
                let look = look_at(world, port, view_range, &viewed[index], camouflage, revealed);
                if best.is_none_or(|best| better(&look, &best) == Ordering::Less) {
                    best = Some(look);
                }
                if look.spots() {
                    break;
                }
            }
            self.looks[index] = best;

            // 6. Spotted: its enemies' sighting is now. Else their last one lingers,
            //    spotting it still for LINGER, see `Self::spotted`.
            if best.is_some_and(|best| best.spots()) {
                let by = TEAMS.into_iter().find(|&team| team != target.team).unwrap_or_default();
                self.sightings[index] = Some(Sighting { by, at: now });
            }

        }

        // 7. Each vehicle's other port looks the next tick.
        for port in &mut self.ports {
            *port = 1 - *port;
        }

    }

    /// Whether `team` spots the battle's `index`th vehicle at game time `now`: it did
    /// within [`LINGER`]. Its own it doesn't, see [`Self::update`].
    pub fn spotted(&self, team: u8, index: usize, now: f64) -> bool {
        self.sightings[index].is_some_and(|sighting| sighting.by == team && now - sighting.at <= LINGER)
    }

    /// Its enemies' best look at the battle's `index`th vehicle this tick, alive: one
    /// spotting it if any, else the nearest to.
    pub fn look(&self, index: usize) -> Option<&Look> {
        self.looks.get(index)?.as_ref()
    }

    /// Draw, for each of the battle's vehicles alive, how its enemies best looked at it
    /// this tick, see [`Self::update`], in the channels drawn, see the `debug` module:
    ///
    /// - [`DebugChannel::Spotting`]: a text where it stands, following it:
    ///
    ///   ```text
    ///     Bot 3                          its name
    ///     camo 0.31 -> 0.66, 2 plants    its camouflage, then hidden by the foliage
    ///     312 m of 230 m: clear, top     how far, of how far it is spotted, what is in
    ///                                    the way, of which of its checkpoints, or of its
    ///                                    hull if near
    ///   ```
    ///
    ///   magenta if spotted, cyan if not, white if no enemy looks at it.
    /// - [`DebugChannel::SpotPoints`]: its checkpoints, following it, its ports bigger,
    ///   and the line of sight of that look, in the world, in its enemies' colour,
    ///   darker if it doesn't spot it, see [`DEBUG_TEAM_COLORS`].
    pub fn debug_draw(&self, vehicles: &[BattleVehicle], world: &World) {

        for (index, vehicle) in vehicles.iter().enumerate().filter(|(_, vehicle)| vehicle.alive()) {

            let hull = &world.vehicle(vehicle.id).hull;
            let look = self.look(index);

            debug::at(DebugPlace::on(vehicle.id, hull.position, hull.orientation), || {
                if debug::draws(DebugChannel::Spotting) {
                    let (text, color) = debug_spotting_text(vehicle, look);
                    debug::text(DebugChannel::Spotting, hull.position, text, color);
                }
                for (number, &point) in self.points[index].iter().enumerate() {
                    let radius = if number < 2 { DEBUG_PORT_RADIUS } else { DEBUG_POINT_RADIUS };
                    debug::sphere(DebugChannel::SpotPoints, point, radius);
                }
            });

            if let Some(look) = look {
                let enemies = if vehicle.team == TEAMS[0] { 1 } else { 0 };
                let shade = if look.spots() { 1.0 } else { DEBUG_HIDDEN_SHADE };
                debug::line(DebugChannel::SpotPoints, look.eye, look.at, DEBUG_TEAM_COLORS[enemies] * shade);
            }

        }

    }

}

impl Look {

    /// Whether the observer spots the point, see the module's documentation.
    pub fn spots(&self) -> bool {
        match self.sight {
            Sight::Near => true,
            Sight::Far |
            Sight::Blocked => false,
            Sight::Clear => self.distance <= self.range,
        }
    }

}

/// A vehicle's ports and checkpoints in the world, as it stands and its turret is
/// turned now, see the module's documentation.
fn viewed(world: &World, vehicle: &BattleVehicle) -> Viewed {

    let driven = world.vehicle(vehicle.id);
    let mobility = &driven.hull.mobility;
    let pivot = driven.turret.aiming.turret_position;
    let [chassis, _, turret, gun] = driven.part_transforms(vehicle.collision.hull_position);

    // Both boxes in the vehicle's frame, the turret's as it faces ahead.
    let inset = Vec3::splat(POINT_INSET);
    let [hull_min, hull_max] = [mobility.bounds[0] + inset, mobility.bounds[1] - inset];
    let [turret_min, turret_max] = [mobility.turret_bounds[0] + inset, mobility.turret_bounds[1] - inset];
    let hull_middle = (hull_min + hull_max) / 2.0;
    let turret_middle = (turret_min + turret_max) / 2.0;

    // Its static port, on its top, doesn't turn with its turret.
    let top = if turret_max.y >= hull_max.y {
        turret_middle.with_y(turret_max.y)
    } else {
        hull_middle.with_y(hull_max.y)
    };
    let top = chassis.transform_point3(top);
    let mount = Vec3::from(gun.translation);

    // Its turret's sides, halfway up what of it rises over its hull, its box reaching
    // down into it.
    let side_height = if turret_max.y > hull_max.y {
        (turret_min.y.max(hull_max.y) + turret_max.y) / 2.0
    } else {
        turret_middle.y
    };
    let left = turret_middle.with_x(turret_min.x).with_y(side_height);
    let right = turret_middle.with_x(turret_max.x).with_y(side_height);

    Viewed {
        center: chassis.transform_point3(hull_middle),
        ports: [top, mount],
        points: [
            top,
            mount,
            chassis.transform_point3(hull_middle.with_z(hull_max.z)),
            chassis.transform_point3(hull_middle.with_z(hull_min.z)),
            turret.transform_point3(left - pivot),
            turret.transform_point3(right - pivot),
        ],
    }

}

/// How an observer, seeing as far as `view_range`, looks at a target of the given
/// camouflage, if near it: within [`MIN_RADIUS`] on the ground plane, the middle of its
/// hull to the middle of the target's, it spots it, whatever is in the way.
fn near(view_range: f32, observer: &Viewed, target: &Viewed, camouflage: f32) -> Option<Look> {
    let distance = (target.center - observer.center).with_y(0.0).length();
    (distance <= MIN_RADIUS).then_some(Look {
        point: None,
        distance,
        sight: Sight::Near,
        camouflage,
        hidden: camouflage,
        plants: 0,
        range: range(view_range, camouflage),
        eye: observer.center,
        at: target.center,
    })
}

/// How an observer at `eye`, its port, seeing as far as `view_range`, looks at a
/// target's checkpoints, of the given camouflage, `revealed` if it just fired: the
/// first that it spots, else the nearest to, see [`better`].
fn look_at(world: &World, eye: Vec3, view_range: f32, target: &Viewed, camouflage: f32, revealed: bool) -> Look {
    let mut best: Option<Look> = None;
    for (&point, &at) in POINTS.iter().zip(&target.points) {
        let look = look(world, eye, view_range, point, at, camouflage, revealed);
        if look.spots() {
            return look;
        }
        if best.is_none_or(|best| better(&look, &best) == Ordering::Less) {
            best = Some(look);
        }
    }
    best.expect("a vehicle has checkpoints")
}

/// How an observer at `eye`, its port, seeing as far as `view_range`, looks at a
/// target's checkpoint `point` at `at`, of the given camouflage, `revealed` if it just
/// fired, see the module's documentation, beyond [`near`]. The foliage is looked at
/// last, as it only hides more.
fn look(world: &World, eye: Vec3, view_range: f32, point: Point, at: Vec3, camouflage: f32, revealed: bool) -> Look {

    let distance = (at - eye).with_y(0.0).length();
    let mut look = Look {
        point: Some(point),
        distance,
        sight: Sight::Far,
        camouflage,
        hidden: camouflage,
        plants: 0,
        range: range(view_range, camouflage),
        eye,
        at,
    };
    if distance > look.range {
        return look;
    }
    if !world.sees(eye, at) {
        look.sight = Sight::Blocked;
        return look;
    }

    let (foliage, plants) = foliage_camouflage(world, eye, at, revealed);
    look.sight = Sight::Clear;
    look.hidden = (camouflage + foliage).min(1.0);
    look.plants = plants;
    look.range = range(view_range, look.hidden);
    look

}

/// How far an observer seeing as far as `view_range` spots a vehicle of the given
/// camouflage, in metres on the ground plane.
fn range(view_range: f32, camouflage: f32) -> f32 {
    (MIN_RADIUS + (view_range - MIN_RADIUS) * (1.0 - camouflage)).min(VIEW_RANGE)
}

/// A vehicle's text in the debug drawing, of how its enemies best look at it, if any
/// do, and its colour, see [`Spotting::debug_draw`].
fn debug_spotting_text(vehicle: &BattleVehicle, look: Option<&Look>) -> (String, Vec4) {

    let Some(look) = look else {
        return (vehicle.info.name.clone(), DEBUG_UNSEEN);
    };

    let sight = match look.sight {
        Sight::Near => "near",
        Sight::Far => "far",
        Sight::Blocked => "blocked",
        Sight::Clear => "clear",
    };
    let point = match look.point {
        None => "hull",
        Some(Point::Top) => "top",
        Some(Point::Gun) => "gun",
        Some(Point::Front) => "front",
        Some(Point::Rear) => "rear",
        Some(Point::Left) => "left",
        Some(Point::Right) => "right",
    };
    let text = format!(
        "{}\ncamo {:.2} -> {:.2}, {} plants\n{:.0} m of {:.0} m: {sight}, {point}",
        vehicle.info.name,
        look.camouflage,
        look.hidden,
        look.plants,
        look.distance,
        look.range,
    );
    (text, if look.spots() { DEBUG_SPOTTED } else { DEBUG_HIDDEN })

}

/// How much the foliage between an observer's port at `eye` and a checkpoint at `at`,
/// of a target `revealed` if it just fired, adds to the target's camouflage, and how
/// many plants hide it, see the module's documentation.
fn foliage_camouflage(world: &World, eye: Vec3, at: Vec3, revealed: bool) -> (f32, usize) {

    let length = (at - eye).length();
    let mut far = 0.0;
    let mut near: f32 = 0.0;
    let mut plants = 0;
    for crossing in world.foliage(eye, at) {
        if crossing.enter <= FOLIAGE_RADIUS {
            continue;
        }
        plants += 1;
        if revealed && length - crossing.leave <= FOLIAGE_RADIUS {
            near = near.max(crossing.density * FIRED_FOLIAGE);
        } else {
            far += crossing.density;
        }
    }

    ((far + near).min(VEGETATION_CAP), plants)

}

/// Which of two looks at an enemy is the better, see [`Spotting::look`]: one spotting
/// it, else the one nearer to.
fn better(a: &Look, b: &Look) -> Ordering {
    (!a.spots()).cmp(&!b.spots())
        .then((a.distance - a.range).total_cmp(&(b.distance - b.range)))
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use glam::Affine3A;

    use wgtk::space::foliage::{Foliage, Plant};

    use crate::base::physics::fixtures::blocks;
    use super::*;

    #[test]
    fn spots_near_hull_to_hull_only() {

        // Hull to hull 45 m apart on the ground plane, its checkpoints further: near.
        // 55 m apart: not, its checkpoints left to look at.
        let observer = viewed_at(Vec3::new(0.0, 1.0, 0.0));
        let close = near(400.0, &observer, &viewed_at(Vec3::new(0.0, 20.0, 45.0)), 0.3)
            .expect("near");
        assert!(close.spots() && close.point.is_none() && (close.distance - 45.0).abs() < 1e-4, "{close:?}");
        assert!(near(400.0, &observer, &viewed_at(Vec3::new(0.0, 1.0, 55.0)), 0.3).is_none());

    }

    #[test]
    fn spots_within_its_range_by_camouflage_but_behind_walls() {

        // A wall 20 m wide and 5 m high 30 m in front of the observer.
        let space = blocks(&[(Vec3::new(-10.0, 0.0, -71.0), Vec3::new(10.0, 5.0, -69.0))]);
        let world = World::new(Arc::new(space), None);
        let eye = Vec3::new(0.0, 2.0, -100.0);

        // Along the wall, in the open: seeing 400 m, a camouflage of 0.5 is seen within
        // 225 m, none within 400 m, and nothing beyond 445 m.
        assert!(look(&world, eye, 400.0, Point::Top, Vec3::new(220.0, 1.5, -100.0), 0.5, false).spots());
        assert!(!look(&world, eye, 400.0, Point::Top, Vec3::new(240.0, 1.5, -100.0), 0.5, false).spots());
        assert!(look(&world, eye, 400.0, Point::Top, Vec3::new(390.0, 1.5, -100.0), 0.0, false).spots());
        assert!(!look(&world, eye, 500.0, Point::Top, Vec3::new(450.0, 1.5, -100.0), 0.0, false).spots());

        // Behind the wall, never: within 50 m too, which is left to `near`.
        let behind = look(&world, eye, 400.0, Point::Top, Vec3::new(0.0, 1.5, -55.0), 0.0, false);
        assert!(!behind.spots() && behind.sight == Sight::Blocked, "{behind:?}");
        assert!(!look(&world, eye, 400.0, Point::Top, Vec3::new(0.0, 1.5, -40.0), 0.0, false).spots());

    }

    #[test]
    fn foliage_hides_but_near_the_observer_or_a_shooter() {

        // Bushes 6 m wide and 4 m high, hiding 0.5 each, on the line from the observer
        // along +X: one 100 m out, one 10 m from it, one around a vehicle 200 m out.
        let mut space = blocks(&[]);
        space.foliage = Foliage::new(vec![
            bush(1, Vec3::new(100.0, 0.0, -100.0)),
            bush(2, Vec3::new(10.0, 0.0, -100.0)),
            bush(3, Vec3::new(200.0, 0.0, -100.0)),
        ]);
        let world = World::new(Arc::new(space), None);
        let eye = Vec3::new(0.0, 2.0, -100.0);

        // Seeing 400 m, of no camouflage: the bush 100 m out adds 0.5 to it, it is seen
        // within 225 m, at 150 m; the one near the observer it sees through, else both
        // would hide it, seen within 120 m only, their 0.8 at most.
        let open = look(&world, eye, 400.0, Point::Top, Vec3::new(150.0, 1.5, -100.0), 0.0, false);
        assert!(open.spots() && open.plants == 1 && (open.hidden - 0.5).abs() < 1e-4, "{open:?}");

        // In the bush 200 m out too, both hide it, at most 0.8, seen within 120 m.
        let still = look(&world, eye, 400.0, Point::Top, Vec3::new(200.0, 1.5, -100.0), 0.0, false);
        assert!(!still.spots() && (still.hidden - 0.8).abs() < 1e-4, "{still:?}");

        // Having just fired, its own bush hides it by 30 % of its 0.5 only, the one
        // further out by all of it: 0.65.
        let fired = look(&world, eye, 400.0, Point::Top, Vec3::new(200.0, 1.5, -100.0), 0.0, true);
        assert!((fired.hidden - 0.65).abs() < 1e-4, "{fired:?}");

        // Over the bushes, nothing hides it.
        assert!(look(&world, Vec3::new(0.0, 6.0, -100.0), 400.0, Point::Top, Vec3::new(390.0, 6.0, -100.0), 0.0, false).spots());

    }

    #[test]
    fn spots_a_turret_over_a_wall_hiding_its_hull() {

        // A wall 1.5 m high 100 m in front of the observer, a vehicle just behind it, its
        // hull's checkpoints and its turret's sides under the wall's top, its turret's
        // top over it.
        let space = blocks(&[(Vec3::new(-10.0, 0.0, -1.0), Vec3::new(10.0, 1.5, 1.0))]);
        let world = World::new(Arc::new(space), None);
        let eye = Vec3::new(0.0, 2.4, -100.0);
        let mut target = Viewed {
            center: Vec3::new(0.0, 0.9, 4.0),
            ports: [Vec3::new(0.0, 2.4, 4.0), Vec3::new(0.0, 1.3, 5.0)],
            points: [
                Vec3::new(0.0, 2.4, 4.0),
                Vec3::new(0.0, 1.3, 5.0),
                Vec3::new(0.0, 0.9, 7.0),
                Vec3::new(0.0, 0.9, 1.5),
                Vec3::new(-1.2, 1.4, 4.0),
                Vec3::new(1.2, 1.4, 4.0),
            ],
        };

        // Its top is spotted, the first checkpoint looked at.
        let look = look_at(&world, eye, 400.0, &target, 0.0, false);
        assert!(look.spots() && look.point == Some(Point::Top), "{look:?}");

        // Its top down behind the wall too, nothing of it is.
        target.points[0] = Vec3::new(0.0, 1.2, 4.0);
        let look = look_at(&world, eye, 400.0, &target, 0.0, false);
        assert!(!look.spots() && look.sight == Sight::Blocked, "{look:?}");

    }

    /// A vehicle whose hull's middle, ports and checkpoints are all at `at`.
    fn viewed_at(at: Vec3) -> Viewed {
        Viewed {
            center: at,
            ports: [at; 2],
            points: [at; 6],
        }
    }

    /// A bush 6 m wide and 4 m high standing at `at`, hiding half of what is behind it.
    fn bush(tree: u32, at: Vec3) -> Plant {
        Plant {
            tree,
            transform: Affine3A::from_translation(at),
            bounds: [Vec3::new(-3.0, 0.0, -3.0), Vec3::new(3.0, 4.0, 3.0)],
            density: 0.5,
        }
    }

}
