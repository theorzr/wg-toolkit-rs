//! A shell fired, flying through a map as the battle goes until it lands, see
//! [`Shot::fly`], then going on off what it ricochets off, see [`Shot::ricochet`].
//!
//! It flies the parabola the client draws, in steps of [`FLIGHT_STEP`], each a segment
//! tested against all that may stop it, the nearest along it winning: the vehicles, as
//! they are when it gets there, the static models and the terrain. Destructibles weak
//! enough it flies through, breaking them, see [`ShotPiece`]. It flies at the speed it
//! has, and only loses what the air's drag and what it flew through took of it as it
//! hits something, see [`Shot::impact`] and [`super::speed`].
//!
//! ```text
//!   fired           Shot::new, at its gun
//!   each tick       Shot::fly, up to the battle's time
//!     each step     its next segment, gravity bending its way
//!       first met   a vehicle, but its shooter's, a static model, or the terrain
//!       before      the destructibles it flies through
//!       else        on, until its range
//!   landed          Shot::impact, losing its speed for what it met on the way
//!   off it          Shot::ricochet, flying on slower from where it hit
//! ```

use std::sync::Arc;

use glam::{Affine3A, Vec3, Vec4};

use wgtk::space::statics::BodyKind;
use wgtk::space::Space;

use crate::game::collision::{segment_triangle, VehicleCollision, VehicleIntersection};
use crate::base::physics::{DestructiblePiece, Rubble};
use crate::game::item_defs::{ShellKind, ShotDef};
use crate::base::debug::{self, DebugChannel};

use super::speed;


/// The flight is followed in steps of this many seconds.
const FLIGHT_STEP: f32 = 0.005;

/// The ricochets a shell goes on after, at most.
const MAX_RICOCHETS: u32 = 3;
/// How far before the point it ricochets off a shell goes on from, in metres, as the
/// server shows ricochets: for it not to meet what it ricochets off again.
const RICOCHET_STANDOFF: f32 = 0.001;

/// How a shell's flight is drawn to be debugged, in [`DebugChannel::Shells`], see
/// [`Shot::fly`]: for this long, in seconds of game time, a line every this many of its
/// steps, yellow from its gun, orange off a ricochet; a cross where it broke through a
/// destructible, violet, and where it ends, red on a vehicle, green on the terrain, blue
/// on a static model, white out of range. Of 255.
pub const DEBUG_SHOT_SECONDS: f32 = 10.0;
const DEBUG_PATH_STEPS: u32 = 5;
const DEBUG_FIRED: Vec4 = Vec4::new(255.0, 230.0, 60.0, 255.0);
const DEBUG_RICOCHET: Vec4 = Vec4::new(255.0, 140.0, 30.0, 255.0);
const DEBUG_THROUGH: Vec4 = Vec4::new(190.0, 110.0, 255.0, 255.0);
const DEBUG_VEHICLE: Vec4 = Vec4::new(255.0, 60.0, 60.0, 255.0);
const DEBUG_TERRAIN: Vec4 = Vec4::new(120.0, 220.0, 70.0, 255.0);
const DEBUG_STATIC: Vec4 = Vec4::new(70.0, 150.0, 255.0, 255.0);
const DEBUG_RANGE: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);


/// A shell fired, flying from `start`: from its gun, or off what it last ricocheted off,
/// see the module's documentation.
#[derive(Debug, Clone)]
pub struct Shot {
    /// Its id, which the clients tell its tracer by: the same off its ricochets.
    pub id: i32,
    /// Its shell.
    pub def: ShotDef,
    /// The shells fired at once with it when it is the first of them, it included: the
    /// client shows them all from the first one, see `showShooting`. None for the others
    /// of a burst.
    pub burst: Option<u32>,
    /// The times it ricocheted.
    pub ricochets: u32,
    /// Where it flies from: its gun's muzzle, or just before where it ricocheted off.
    pub start: Vec3,
    /// Its velocity at `start`, in m/s, which its tracer is shown with.
    pub start_velocity: Vec3,
    /// How far it flew before `start`, in metres: 0 unless it ricocheted.
    pub flown: f32,
    /// How far from `start` it flies at most: its shell's maximum distance, less what it
    /// flew.
    pub range: f32,
    /// Where it got to: where it is while it flies, where it landed once it did.
    pub end: Vec3,
    /// The velocity at `end`.
    pub end_velocity: Vec3,
    /// When it got to `end`, in seconds after its start.
    pub flight_time: f32,
    /// The steps it flew, of [`FLIGHT_STEP`] each.
    steps: u32,
    /// The destructibles it flew through on its way to `end`, in order: each breaks as
    /// it gets there, see [`World::break_shot`](crate::base::physics::World::break_shot).
    pub through: Vec<ShotPiece>,
    /// Its speed, as a share of its speed from its gun, which its penetration follows,
    /// see [`speed::pen_at_speed`]: what it lost as it hit, see [`Self::impact`], and
    /// off its ricochets. It flies on at its speed, but loses some more, virtually, see
    /// [`Self::speed_at`], until it next hits.
    pub speed_factor: f32,
    /// The share of its speed the air's drag left it at its last impact, see
    /// [`speed::drag_speed`].
    drag: f32,
    /// The share of its speed what it flew through since its last impact took, see
    /// [`ShotPiece`]: it loses it as it next hits.
    slowed: f32,
    /// Where its path is drawn from next to be debugged, see [`DEBUG_PATH_STEPS`].
    debug_from: Vec3,
}

/// What a shell ends its flight on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotLanding {
    /// A vehicle, see [`ShotTarget`].
    Vehicle {
        /// Its entity's id.
        entity_id: u32,
        /// Where on its collision models the shell hit it first.
        hit: VehicleIntersection,
        /// Each of its parts' transform as it got hit.
        transforms: [Affine3A; 4],
    },
    /// The terrain, which it may skim off, see [`super::ricochet::off_ground`].
    Terrain,
    /// A static model: a wall, a rock, a house's module too strong to fly through.
    Static {
        /// Its triangle's normal there, toward where the shell came from.
        normal: Vec3,
        /// The destructible it is of, if any, which the shell damages, see
        /// [`World::shoot`](crate::base::physics::World::shoot).
        piece: Option<ShotPiece>,
    },
    /// Nothing, at its maximum distance.
    Range,
}

/// A destructible a shell meets. One of no more health than
/// [`Destructibles::shoot_through_health`] is flown through, breaking, the shell losing
/// some of its penetration, see [`Destructibles::shoot_through_loss`], as speed, see
/// [`Shot::slowed`]; a stronger one stops it, see [`ShotLanding::Static`]. An HE, APHE or
/// HEAT shell explodes on whatever it meets, its fuze set off by a fence too, and flies
/// through nothing.
///
/// [`Destructibles::shoot_through_health`]: wgtk::space::destructibles::Destructibles::shoot_through_health
/// [`Destructibles::shoot_through_loss`]: wgtk::space::destructibles::Destructibles::shoot_through_loss
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotPiece {
    /// The placed model of the triangle it met, its index in `BSMI`.
    pub instance: u32,
    /// The destructible, and its module for a house's.
    pub piece: DestructiblePiece,
    /// When the shell gets there, in seconds after its start, as [`Shot::flight_time`].
    pub time: f32,
}

/// What of a map a shell can hit besides vehicles: its terrain and its static models,
/// but for what broke of them.
#[derive(Debug, Clone, Copy)]
pub struct ShotObstacles<'a> {
    /// The map: its terrain and its static models.
    pub space: &'a Space,
    /// What broke of its destructibles, which shells fly through.
    pub rubble: &'a Rubble,
}

/// A vehicle shells can hit: its collision models, placed in the world.
#[derive(Debug, Clone)]
pub struct ShotTarget {
    /// Its entity's id.
    pub entity_id: u32,
    /// Its armour and modules, by part.
    pub collision: Arc<VehicleCollision>,
    /// Each part's transform, see
    /// [`Vehicle::part_transforms`](crate::base::physics::Vehicle::part_transforms).
    pub transforms: [Affine3A; 4],
}

/// Where a shell's flight starts, see [`Shot::launch`].
#[derive(Debug, Clone, Copy)]
struct Launch {
    /// Where it flies from, see [`Shot::start`].
    start: Vec3,
    /// Its velocity there, see [`Shot::start_velocity`].
    velocity: Vec3,
    /// How far it flew before `start`, in metres, see [`Shot::flown`].
    flown: f32,
    /// How far from `start` it flies at most, in metres.
    range: f32,
    /// Its speed, as a share of its speed from its gun, see [`Shot::speed_factor`].
    speed_factor: f32,
    /// The share of its speed the air's drag left it at its last impact, see
    /// [`Shot::drag`].
    drag: f32,
}

impl Shot {

    // -- Its flight --

    /// A shell of this id fired from its gun at `start`, at `velocity`: at its full
    /// speed, for its whole range.
    pub fn new(id: i32, def: ShotDef, start: Vec3, velocity: Vec3) -> Self {
        let launch = Launch {
            start,
            velocity,
            flown: 0.0,
            range: def.max_distance,
            speed_factor: 1.0,
            drag: 1.0,
        };
        Self::launch(id, def, launch, 0)
    }

    /// Fly it on until `time` seconds after its start, step by step, against the
    /// vehicles where they are now, but its shooter's, of this entity id, and the map's
    /// obstacles, if any, see the module's documentation. What it lands on, once it
    /// does. Drawn to be debugged as it goes, lasting, see [`DEBUG_SHOT_SECONDS`].
    pub fn fly(
        &mut self,
        time: f32,
        obstacles: Option<ShotObstacles>,
        vehicles: &[ShotTarget],
        shooter: u32,
    ) -> Option<ShotLanding> {

        debug::lasting(DEBUG_SHOT_SECONDS, || {

            while (self.steps + 1) as f32 * FLIGHT_STEP <= time {
                if let Some(landing) = self.step(obstacles, vehicles, shooter) {
                    return Some(landing);
                }
            }

            None

        })

    }

    /// Fly it a step on, see [`Self::fly`]: what it lands on, if it does on the way.
    fn step(
        &mut self,
        obstacles: Option<ShotObstacles>,
        vehicles: &[ShotTarget],
        shooter: u32,
    ) -> Option<ShotLanding> {

        // Its next segment, gravity bending its way.
        let position = self.end;
        let velocity = self.end_velocity;
        let next = position + velocity * FLIGHT_STEP;
        let next_velocity = velocity - Vec3::Y * self.def.gravity * FLIGHT_STEP;

        // The first thing the segment meets, and what it flies through before, in order.
        let mut crossed = Vec::new();
        let first = self.first_met(obstacles, vehicles, shooter, position, next, &mut crossed);
        let stop = first.as_ref().map_or(1.0, |&(t, _)| t);
        crossed.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(obstacles) = obstacles {
            for &(t, piece) in crossed.iter().filter(|&&(t, _)| t < stop) {
                self.fly_through(obstacles, piece, position.lerp(next, t));
            }
        }

        // Landed where it met it.
        if let Some((t, landing)) = first {
            self.end = position.lerp(next, t);
            self.end_velocity = velocity.lerp(next_velocity, t);
            self.flight_time += FLIGHT_STEP * t;
            debug::line(DebugChannel::Shells, self.debug_from, self.end, self.debug_color());
            debug_landing(self.end, &landing);
            return Some(landing);
        }

        // Else on to the next step, unless out of range.
        self.steps += 1;
        self.end = next;
        self.end_velocity = next_velocity;
        self.flight_time = self.steps as f32 * FLIGHT_STEP;

        if (next - self.start).length() >= self.range {
            debug::line(DebugChannel::Shells, self.debug_from, next, self.debug_color());
            debug_landing(next, &ShotLanding::Range);
            return Some(ShotLanding::Range);
        }

        if self.steps % DEBUG_PATH_STEPS == 0 {
            debug::line(DebugChannel::Shells, self.debug_from, next, self.debug_color());
            self.debug_from = next;
        }
        None

    }

    /// The first thing a segment of its flight from `from` to `to` meets, as a share of
    /// it, and what it lands on there: a vehicle, but its shooter's, a static model, or
    /// the terrain, a vehicle first, then a static model, when as near. The
    /// destructibles it would fly through on the way go to `crossed`, see
    /// [`meet_statics`].
    fn first_met(
        &self,
        obstacles: Option<ShotObstacles>,
        vehicles: &[ShotTarget],
        shooter: u32,
        from: Vec3,
        to: Vec3,
        crossed: &mut Vec<(f32, ShotPiece)>,
    ) -> Option<(f32, ShotLanding)> {

        let vehicle = meet_vehicles(vehicles, shooter, from, to);
        let Some(obstacles) = obstacles else {
            return vehicle
        };

        let statics = meet_statics(obstacles, &self.def, from, to, self.flight_time, crossed);
        let terrain = cross_terrain(obstacles, from, to).map(|t| (t, ShotLanding::Terrain));
        [vehicle, statics, terrain].into_iter()
            .flatten()
            .min_by(|a, b| a.0.total_cmp(&b.0))

    }

    /// It flies through a destructible it meets at `at`, breaking it, once: the
    /// penetration the space says it loses there, it loses as speed as it next hits,
    /// see [`Self::slowed`].
    fn fly_through(&mut self, obstacles: ShotObstacles, piece: ShotPiece, at: Vec3) {

        if self.through.iter().any(|done| done.piece == piece.piece) {
            return;
        }

        // The speed it would pierce what it has left at, of the speed it has there.
        let distance = self.flown + self.start.distance(at);
        let pen = speed::pen_at_speed(&self.def, self.speed_at(distance));
        let left = obstacles.space.destructibles.shoot_through_loss.apply(pen);
        let kept = speed::speed_for_pen(&self.def, left)
            / speed::speed_for_pen(&self.def, pen).max(f32::EPSILON);

        self.slowed *= kept.min(1.0);
        self.through.push(piece);
        debug::cross(DebugChannel::Shells, at, DEBUG_THROUGH);

    }

    /// The colour its path is drawn in to be debugged, see [`DEBUG_FIRED`].
    fn debug_color(&self) -> Vec4 {
        if self.flown > 0.0 { DEBUG_RICOCHET } else { DEBUG_FIRED }
    }

    // -- Its impacts --

    /// It hits something at `end`: it loses what the air's drag and what it flew
    /// through took of its speed since its last impact, which it only lost virtually
    /// until then, see [`Self::speed_at`]. Its penetration there follows that speed, see
    /// [`speed::pen_at_speed`].
    pub fn impact(&mut self) {

        let drag = speed::drag_speed(&self.def, self.distance());
        let speed_factor = self.speed_dragged(drag);
        if self.speed_factor > 0.0 {
            self.end_velocity *= speed_factor / self.speed_factor;
        }

        self.speed_factor = speed_factor;
        self.drag = drag;
        self.slowed = 1.0;

    }

    /// The shell going on off what it hit at `end`, ricocheting at `velocity`, as the
    /// server shows ricochets, for the rest of its range: it starts flying again, as
    /// much slower as `velocity` is than the one it hit at. None once it ricocheted
    /// [`MAX_RICOCHETS`] times, or out of range.
    pub fn ricochet(&self, velocity: Vec3) -> Option<Shot> {

        if self.ricochets >= MAX_RICOCHETS {
            return None;
        }

        let range = self.range - self.start.distance(self.end);
        if range <= 0.0 {
            return None;
        }

        // Just before where it hit, not to meet it again.
        let start = self.end - self.end_velocity.normalize_or_zero() * RICOCHET_STANDOFF;
        let kept = velocity.length() / self.end_velocity.length().max(f32::EPSILON);
        let launch = Launch {
            start,
            velocity,
            flown: self.distance(),
            range,
            speed_factor: self.speed_factor * kept,
            drag: self.drag,
        };

        Some(Self::launch(self.id, self.def.clone(), launch, self.ricochets + 1))

    }

    /// A shell of this id launched, at its start, after ricocheting this many times.
    fn launch(id: i32, def: ShotDef, launch: Launch, ricochets: u32) -> Self {
        let Launch { start, velocity, flown, range, speed_factor, drag } = launch;
        Self {
            id,
            def,
            burst: None,
            ricochets,
            start,
            start_velocity: velocity,
            flown,
            range,
            end: start,
            end_velocity: velocity,
            flight_time: 0.0,
            steps: 0,
            through: Vec::new(),
            speed_factor,
            drag,
            slowed: 1.0,
            debug_from: start,
        }
    }

    /// Its speed, as [`Self::speed_factor`], had it hit something after flying
    /// `distance` metres: less what the air's drag and what it flew through took since
    /// its last impact.
    fn speed_at(&self, distance: f32) -> f32 {
        self.speed_dragged(speed::drag_speed(&self.def, distance))
    }

    /// Its speed, as [`Self::speed_factor`], had the air's drag left it at `drag` of its
    /// speed, see [`speed::drag_speed`]: less what the drag and what it flew through
    /// took since its last impact.
    fn speed_dragged(&self, drag: f32) -> f32 {
        let dragged = drag / self.drag.max(f32::EPSILON);
        self.speed_factor * self.slowed * dragged.min(1.0)
    }

    /// How far it flew from its gun to `end`, in metres.
    pub fn distance(&self) -> f32 {
        self.flown + self.start.distance(self.end)
    }

}

/// The first of these vehicles a segment from `from` to `to` meets, but its shooter's,
/// of this entity id, as a share of the segment.
fn meet_vehicles(
    vehicles: &[ShotTarget],
    shooter: u32,
    from: Vec3,
    to: Vec3,
) -> Option<(f32, ShotLanding)> {
    vehicles.iter()
        .filter(|vehicle| vehicle.entity_id != shooter)
        .filter_map(|vehicle| Some((vehicle, vehicle.collision.first_hit(&vehicle.transforms, from, to)?)))
        .min_by(|a, b| a.1.fraction.total_cmp(&b.1.fraction))
        .map(|(vehicle, hit)| {
            let landing = ShotLanding::Vehicle {
                entity_id: vehicle.entity_id,
                hit,
                transforms: vehicle.transforms,
            };
            (hit.fraction, landing)
        })
}

/// The static triangles a step of a shell from `from` to `to`, starting `time` after its
/// start, meets: the first that stops it, as a share of the step, and in `crossed`, the
/// destructibles it flies through, each with the share of the step it meets them at. A
/// destructible already broken, or that the client doesn't number, isn't met, nor is a
/// triangle shells pass, see [`StaticTriangle::shells_pass`].
///
/// [`StaticTriangle::shells_pass`]: wgtk::space::statics::StaticTriangle::shells_pass
fn meet_statics(
    obstacles: ShotObstacles,
    def: &ShotDef,
    from: Vec3,
    to: Vec3,
    time: f32,
    crossed: &mut Vec<(f32, ShotPiece)>,
) -> Option<(f32, ShotLanding)> {

    let ShotObstacles { space, rubble } = obstacles;
    let statics = &space.statics;
    let min = from.min(to);
    let max = from.max(to);
    let mut stop: Option<(f32, ShotLanding)> = None;

    // HE, APHE and HEAT shells go off on whatever they meet.
    let explodes = matches!(
        def.kind(),
        ShellKind::HighExplosive | ShellKind::ArmorPiercingHe | ShellKind::HollowCharge,
    );

    for kind in [BodyKind::Solid, BodyKind::Ramp, BodyKind::Destructible] {
        for triangle in statics.triangles_in(kind, min, max) {

            if triangle.shells_pass {
                continue;
            }
            let Some((t, normal)) = segment_triangle(from, to - from, &triangle.vertices) else { continue };

            // A destructible weak enough it flies through, else it stops on it.
            let mut met = None;
            if kind == BodyKind::Destructible {
                let Some((piece, destructible)) = rubble.standing(statics, triangle) else { continue };
                let piece_met = ShotPiece {
                    instance: triangle.instance,
                    piece,
                    time: time + FLIGHT_STEP * t,
                };
                let health = destructible.health_of(piece.module).unwrap_or(f32::INFINITY);
                if !explodes && health <= space.destructibles.shoot_through_health {
                    crossed.push((t, piece_met));
                    continue;
                }
                met = Some(piece_met);
            }

            if stop.as_ref().is_none_or(|&(first, _)| t < first) {
                let landing = ShotLanding::Static {
                    normal,
                    piece: met,
                };
                stop = Some((t, landing));
            }

        }
    }

    stop

}

/// Where a step from `from` to `to` goes into the terrain, as a share of it: the ground
/// taken as flat between the two ends.
fn cross_terrain(obstacles: ShotObstacles, from: Vec3, to: Vec3) -> Option<f32> {

    let terrain = &obstacles.space.terrain;
    let ground = terrain.height_at(to.x, to.z)?;
    if to.y > ground {
        return None;
    }

    // Between its height above the ground at its start and its depth under it at its
    // end.
    let above = from.y - terrain.height_at(from.x, from.z).unwrap_or(ground);
    let below = ground - to.y;
    Some((above / (above + below)).clamp(0.0, 1.0))

}

/// Draw where a shell ends to be debugged, a cross of the colour of what it lands on,
/// see [`DEBUG_VEHICLE`].
fn debug_landing(at: Vec3, landing: &ShotLanding) {
    let color = match landing {
        ShotLanding::Vehicle { .. } => DEBUG_VEHICLE,
        ShotLanding::Terrain => DEBUG_TERRAIN,
        ShotLanding::Static { .. } => DEBUG_STATIC,
        ShotLanding::Range => DEBUG_RANGE,
    };
    debug::cross(DebugChannel::Shells, at, color);
}


#[cfg(test)]
mod tests {

    use wgtk::space::destructibles::{DestructibleKind, Module, PenetrationLoss};
    use wgtk::space::statics::{PlacedDestructible, Statics, StaticTriangle};

    use crate::game::collision::{ArmorGroup, CollisionModel, VehiclePart};
    use crate::base::physics::World;

    use super::super::fixtures::shot;
    use super::*;

    /// An AP shell of 120 mm, piercing 250 mm, down to 240 mm from 50 to 500 m.
    fn shell() -> ShotDef {
        ShotDef {
            pen_far: 240.0,
            ..shot(ShellKind::ArmorPiercing, 120.0, 250.0)
        }
    }

    /// A shell fired from the origin straight along +Z.
    fn fired(def: ShotDef) -> Shot {
        let velocity = Vec3::Z * def.speed;
        Shot::new(1, def, Vec3::ZERO, velocity)
    }

    #[test]
    fn flies_its_range() {
        let mut shot = fired(shell());
        assert_eq!(shot.fly(f32::INFINITY, None, &[], 0), Some(ShotLanding::Range));
        assert!((shot.end.length() - 720.0).abs() < 5.0, "{}", shot.end);
    }

    #[test]
    fn ricochets_at_its_speed_for_the_rest_of_its_range() {

        // As if it hit something 100 m on.
        let mut shot = fired(shell());
        shot.end = Vec3::Z * 100.0;
        shot.end_velocity = Vec3::Z * 800.0;

        let mut off = shot.ricochet(Vec3::X * 600.0).unwrap();
        assert_eq!((off.id, off.ricochets, off.flown), (shot.id, 1, 100.0));
        assert!(off.start.distance(shot.end) < 0.01 && off.start.z < 100.0, "{}", off.start);
        assert!((off.start_velocity - Vec3::X * 600.0).length() < 1e-3, "{}", off.start_velocity);
        assert!((off.range - 620.0).abs() < 0.1, "{}", off.range);
        assert_eq!(off.speed_factor, 0.75);
        assert_eq!(off.fly(f32::INFINITY, None, &[], 0), Some(ShotLanding::Range));

        // Out of range where it ends, else at most `MAX_RICOCHETS` times.
        assert!(off.ricochet(-Vec3::X * 600.0).is_none());
        let mut last = off;
        loop {
            last.end = last.start + Vec3::X;
            let Some(next) = last.ricochet(-Vec3::X * 600.0) else { break };
            last = next;
        }
        assert_eq!(last.ricochets, MAX_RICOCHETS);

    }

    /// A part's collision model of these triangles, in an armour group of its own.
    fn model(triangles: Vec<[Vec3; 3]>) -> CollisionModel {
        CollisionModel {
            groups: vec![ArmorGroup {
                name: "armor_1".into(),
                material_kind: 1,
                material: None,
                triangles,
            }],
            radius: 5.0,
            bounds: [Vec3::splat(-5.0), Vec3::splat(5.0)],
        }
    }

    /// A vehicle of entity id 7 at `x`, 100 m ahead, its hull a plate 3 m in front of
    /// its origin.
    fn vehicle_at(x: f32) -> ShotTarget {
        let a = Vec3::new(-2.0, -1.0, -3.0);
        let b = Vec3::new(2.0, -1.0, -3.0);
        let c = Vec3::new(2.0, 1.0, -3.0);
        let d = Vec3::new(-2.0, 1.0, -3.0);
        let collision = VehicleCollision {
            parts: [model(vec![]), model(vec![[a, b, c], [a, c, d]]), model(vec![]), model(vec![])],
            hull_position: Vec3::ZERO,
        };
        let hull = Affine3A::from_translation(Vec3::new(x, 0.0, 100.0));
        ShotTarget {
            entity_id: 7,
            collision: Arc::new(collision),
            transforms: [hull; 4],
        }
    }

    #[test]
    fn hits_a_vehicle_in_its_way() {
        let mut shot = fired(shell());
        let landing = shot.fly(f32::INFINITY, None, &[vehicle_at(0.0)], 0);
        let Some(ShotLanding::Vehicle { entity_id: 7, hit, .. }) = landing else { panic!("missed: {landing:?}") };
        assert_eq!(hit.part, VehiclePart::Hull);
        assert_eq!(hit.material_kind, 1);
        assert_eq!(hit.normal, -Vec3::Z);
        assert!((shot.end.z - 97.0).abs() < 0.01 && hit.local_point.z == -3.0);
    }

    #[test]
    fn hits_vehicles_where_they_are_as_it_flies() {

        // It gets 97 m on in 0.12 s: 40 m on by 0.05 s.
        let ahead = vehicle_at(0.0);
        let away = vehicle_at(50.0);

        // One that drove out of its way it misses.
        let mut shot = fired(shell());
        assert_eq!(shot.fly(0.05, None, std::slice::from_ref(&ahead), 0), None);
        assert!((shot.end.z - 40.0).abs() < 0.1, "{}", shot.end);
        assert_eq!(shot.fly(f32::INFINITY, None, std::slice::from_ref(&away), 0), Some(ShotLanding::Range));

        // One that drove into its way it hits.
        let mut shot = fired(shell());
        assert_eq!(shot.fly(0.05, None, std::slice::from_ref(&away), 0), None);
        let landing = shot.fly(f32::INFINITY, None, std::slice::from_ref(&ahead), 0);
        assert!(matches!(landing, Some(ShotLanding::Vehicle { entity_id: 7, .. })), "{landing:?}");

        // Its shooter's own it flies through.
        let mut shot = fired(shell());
        assert_eq!(shot.fly(f32::INFINITY, None, &[ahead], 7), Some(ShotLanding::Range));

    }

    #[test]
    fn slows_as_it_hits_as_its_penetration_falls() {

        // 400 m on, its penetration from 250 down to 240 mm from 50 to 500 m.
        let mut shot = fired(shell());
        assert_eq!(shot.fly(0.5, None, &[], 0), None);
        let distance = shot.distance();
        let before = shot.end_velocity;

        // Its whole flight at its speed, which only drops as it hits.
        assert!((before.length() - 800.0).abs() < 5.0, "{before}");
        shot.impact();
        let speed = speed::drag_speed(&shell(), distance);
        assert!(speed < 1.0);
        assert!((shot.speed_factor - speed).abs() < 1e-5, "{}", shot.speed_factor);
        assert!((shot.end_velocity.length() - before.length() * speed).abs() < 1e-2, "{}", shot.end_velocity);

        // Piercing there what the game says it pierces, once.
        let pen = speed::pen_at_speed(&shell(), shot.speed_factor);
        assert!((pen - shell().pen_at(distance)).abs() < 0.01, "{pen}");
        shot.impact();
        assert!((shot.speed_factor - speed).abs() < 1e-5, "{}", shot.speed_factor);

    }

    /// A wall 4 m wide across the way at z = 50, of this kind, as two triangles of
    /// the placed model 0; destructible with this health when it is one, the 2nd of its
    /// chunk, a fence or a house's module; one shells pass if `shells_pass`.
    fn wall(kind: BodyKind, health: f32, house: bool, shells_pass: bool) -> Space {

        let a = Vec3::new(-2.0, -2.0, 50.0);
        let b = Vec3::new(2.0, -2.0, 50.0);
        let c = Vec3::new(2.0, 2.0, 50.0);
        let d = Vec3::new(-2.0, 2.0, 50.0);
        let material_kind = if house { 73 } else { 0 };
        let triangles = [[a, b, c], [a, c, d]]
            .map(|vertices| StaticTriangle {
                shells_pass,
                ..StaticTriangle::new(vertices, 0, material_kind, kind)
            })
            .to_vec();

        let mut statics = Statics::from_triangles(triangles);
        let modules = if house {
            vec![Module {
                material_kind: 73,
                health,
                depends: Vec::new(),
            }]
        } else {
            Vec::new()
        };
        statics.add_destructible(0, PlacedDestructible {
            chunk_id: 0x7f7f,
            index: 1,
            kind: if house { DestructibleKind::Structure } else { DestructibleKind::Fragile },
            health,
            modules,
        });

        let mut space = Space {
            statics,
            ..Space::default()
        };
        space.destructibles.shoot_through_health = 19.0;
        space.destructibles.shoot_through_loss = PenetrationLoss {
            factor: 0.0,
            min: 25.0,
        };
        space

    }

    /// Fire a shell at a space straight along +Z, flying it to its end: what it lands on.
    fn fire_at(space: &Space, rubble: &Rubble, def: ShotDef) -> (Shot, ShotLanding) {
        let obstacles = ShotObstacles {
            space,
            rubble,
        };
        let mut shot = fired(def);
        let landing = shot.fly(f32::INFINITY, Some(obstacles), &[], 0).unwrap();
        (shot, landing)
    }

    #[test]
    fn static_models_stop_shells_but_what_they_fly_through() {

        // A wall, or a ramp, stops it where it meets it, facing it.
        let rubble = Rubble::default();
        for kind in [BodyKind::Solid, BodyKind::Ramp] {
            let (shot, landed) = fire_at(&wall(kind, 0.0, false, false), &rubble, shell());
            let landing = ShotLanding::Static {
                normal: -Vec3::Z,
                piece: None,
            };
            assert_eq!(landed, landing);
            assert!((shot.end.z - 50.0).abs() < 0.01, "{}", shot.end);
            assert!(shot.through.is_empty());
        }

        // A fence of 10 health it flies through, breaking it, as it gets there.
        let (shot, landed) = fire_at(&wall(BodyKind::Destructible, 10.0, false, false), &rubble, shell());
        assert_eq!(landed, ShotLanding::Range);
        assert_eq!(shot.through.len(), 1);
        assert_eq!((shot.through[0].instance, shot.through[0].piece.index), (0, 1));
        let at = 50.0 / shell().speed;
        assert!((shot.through[0].time - at).abs() < 0.01, "{} s, not {at} s", shot.through[0].time);

        // The 25 mm of the 250 it pierces there it loses as speed, only as it next hits.
        let kept = (225.0f32 / 250.0).powf(1.0 / 1.43);
        assert!((shot.slowed - kept).abs() < 1e-3, "{}", shot.slowed);
        assert_eq!(shot.speed_factor, 1.0);

        // An HE or a HEAT shell explodes on that fence instead, hitting it.
        let he = ShotDef {
            kind_index: ShellKind::HighExplosive as u8,
            ..shell()
        };
        let heat = ShotDef {
            kind_index: ShellKind::HollowCharge as u8,
            ..shell()
        };
        for def in [he.clone(), heat.clone()] {
            let (shot, landed) = fire_at(&wall(BodyKind::Destructible, 10.0, false, false), &rubble, def);
            let ShotLanding::Static { piece: Some(hit), .. } = landed else { panic!("{:?}", landed) };
            assert_eq!(hit.piece.index, 1);
            assert!(shot.through.is_empty());
        }

        // A light fence shells pass, neither stops nor breaks, whatever the shell.
        let light = wall(BodyKind::Destructible, 1.0, false, true);
        for def in [shell(), he, heat] {
            let (shot, landed) = fire_at(&light, &rubble, def);
            assert_eq!(landed, ShotLanding::Range);
            assert!(shot.through.is_empty());
        }

        // A house's module of 50 health stops it, hit.
        let (shot, landed) = fire_at(&wall(BodyKind::Destructible, 50.0, true, false), &rubble, shell());
        let ShotLanding::Static { piece: Some(hit), .. } = landed else { panic!("{:?}", landed) };
        assert_eq!((hit.instance, hit.piece.module), (0, Some(73)));
        assert!(shot.through.is_empty());

        // Shells' damage adds up on it: 30 then 30 of its 50 break it, once.
        let space = wall(BodyKind::Destructible, 50.0, true, false);
        let mut world = World::new(Arc::new(space), None);
        assert!(world.shoot(0, hit.piece, 30.0, 0.0).is_empty());
        assert_eq!(world.shoot(0, hit.piece, 30.0, 0.0).len(), 1);
        assert!(world.shoot(0, hit.piece, 30.0, 0.0).is_empty());

        // Once broken, it isn't there.
        let (_, landed) = fire_at(world.space(), world.rubble(), shell());
        assert_eq!(landed, ShotLanding::Range);
        assert_eq!(world.take_breakages().count(), 1);

    }

}
