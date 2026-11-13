//! The physics of a battle: how its vehicles drive, push each other and stop against the
//! map. The real server runs each vehicle as a rigid body on springs (`WGTankPhysics`),
//! with server-only tuning; the client doesn't predict anything and only shows what the
//! server sends. This module is a simpler model, fitted on drives captured from the real
//! server (see "Where the numbers come from" below), so that driving feels the same.
//!
//! # The big picture
//!
//! A [`World`] holds the [`Vehicle`]s of one battle, on its map's [`Space`]: terrain,
//! static models and bounds, and the [`Rubble`] of what they broke in it. A vehicle is a
//! [`Hull`] (where it is, how fast it goes and turns) and a [`Turret`] (where its gun
//! points). Whoever drives a vehicle, a client or a bot, only sets its `hull.flags` (the
//! `vehicle_moveWith` flags) and its turret's aim before a tick, then [`World::step`]
//! does everything else, each part in its own module:
//!
//! ```text
//!   controllers set flags and aim
//!                │
//!                ▼
//!   World::step(dt)                                   dt = one tick, 0.1 s    (here)
//!   │
//!   ├─ 1. remember()           pose ← where each vehicle is, to tell jumps
//!   │
//!   ├─ 2. for every vehicle, on its own                                      (here)
//!   │     ├─ start: drive      the drive model: how hard its engine,          drive
//!   │     │                    brakes and steering push it                    ground
//!   │     ├─ rests?            still for long enough: it rests, unless one
//!   │     │                    moving may reach it, and the rest is skipped
//!   │     └─ set_off: ram      damage and break the destructibles it is       rubble
//!   │                          about to get into; its body takes what         body
//!   │                          hits and rams left it
//!   │
//!   ├─ 3. their bodies together, 10 steps of each                     (here), body
//!   │     ├─ each one's boxes and its tracks' ends' spheres, where       collision
//!   │     │  they touch each other's
//!   │     ├─ each one's gravity, its suspension and its tracks           contacts
//!   │     │  driven, all it touches of the terrain and static models     (wgtk::space)
//!   │     │  holding it off; its springs on another, pushing it back
//!   │     ├─ 4 passes: each one held off the space, then the vehicles
//!   │     │  held apart where they touch, sharing their impulses
//!   │     └─ each moves
//!   │
//!   ├─ 4. for every vehicle: finish, its speeds read back, stopped at        walls
//!   │     the map's edge; Turret::step, turn toward the aim, update the     turret
//!   │     dispersion
//!   │
//!   └─ then the battle tells its clients what moved (client: engine and aux data)
//!      and what broke, World::take_breakages
//! ```
//!
//! A vehicle alone, the drive harness's or a test's, moves through a world of it alone,
//! `Hull::advance`.
//!
//! Bots find their way over the world's map with [`World::path`], outside of its tick:
//! where a tank can drive, asked of its space as their ways look at it, and forgotten
//! once something breaks, see the `nav` module.
//!
//! The modules, in the order a tick uses them:
//!
//! ```text
//!   (here)      the world: the vehicles of a battle, its tick, the passes of
//!               collisions between them, what breaks; a vehicle; its hull's state,
//!               the steps of its tick, its motion on the ground plane
//!   drive       the drive model: how hard its engine, brakes and steering push it
//!   body        its rigid body: gravity, suspension, its tracks gripping the ground
//!   ground      the kind of ground under it, firm, medium or soft
//!   rubble      destructibles: what it rams and breaks, what shells break
//!   contacts    where its body touches the space: its springs, its tracks' ends,
//!               its hull's boxes and its turret's
//!   collision   vehicles' bodies against each other: where they touch, and the
//!               impulses holding them apart
//!   walls       the map's edge, what it rams
//!   turret      the turret and its gun: turning, aiming, dispersion
//!   client      what the client is told beyond positions: engine and tracks
//!
//!   nav         where tanks can drive on the map, and bots' ways over it
//! ```
//!
//! Nothing in a tick allocates, once the world's lists of bodies and of where they touch
//! have grown: boxes, outlines and polygons live on the stack
//! ([`SmallVec`](smallvec::SmallVec)), and vehicles are in a slab
//! ([`Slots`](wgtk::app::slots::Slots)).
//!
//! # Conventions
//!
//! Everything is in metres, seconds, radians and kilograms. The world is Y up. Seen from
//! above with +Z up the page, a vehicle at yaw 0 faces +Z, and turning right *increases*
//! its yaw:
//!
//! ```text
//!                 +Z  (yaw 0)
//!                  ▲
//!                  │      heading = (sin ψ, cos ψ)
//!                  │    ╱
//!                  │  ╱  ψ = yaw, grows clockwise
//!                  │╱
//!      ────────────┼────────────▶ +X   right = (cos ψ, −sin ψ)
//!                  │
//! ```
//!
//! A hull's `direction` is `(roll, pitch, yaw)`, in radians, all 0 for a vehicle standing
//! level and facing +Z. What the signs mean ([`Hull::rotation`] composes them as the
//! client's matrices do):
//!
//! ```text
//!   angle   sign   it means                                       when it happens
//!   ─────   ────   ─────────────────────────────────────────────  ───────────────────────
//!   pitch    +     nose DOWN: the front is lower than the back    driving down a slope
//!            −     nose UP: the front is higher than the back     driving up a slope
//!   roll     +     right side UP: the right track is higher       the right track is on
//!                  than the left one                              a bump or a slope
//!            −     right side DOWN: the right track is lower
//!   yaw      +     turned to the right (clockwise from above)     pressing "turn right"
//! ```
//!
//! For example, a vehicle whose front tracks stand 0.5 m lower than its back tracks,
//! 4 m apart, has `pitch = atan(0.5 / 4) ≈ +7°`; with its right track 0.3 m higher than
//! its left one, 3 m apart, `roll = atan(0.3 / 3) ≈ +6°`, about where its suspension
//! settles it. The hull's truth is its `orientation`, a quaternion; these angles are read
//! from it, [`Hull::direction`].
//!
//! # Two views of the same motion
//!
//! A hull's motion is kept twice, and each tick goes from one to the other:
//!
//! ```text
//!   on the ground plane          speed, slide, yaw_rate             what walls and hits
//!   (the hull's fields)                                             change
//!            │  speeds_to_body, as a tick starts          ▲
//!            ▼                                            │  speeds_from_body, as
//!   in 3D (its body)             linear_velocity,                   its steps end
//!                                angular_velocity                   what gravity, its
//!                                                                   springs and tracks move
//! ```
//!
//! The ground plane's speeds are its origin's (the middle of its tracks, on the ground),
//! along its heading (`speed`) and across it (`slide`), and its turning about the
//! vertical (`yaw_rate`). Its body's are its centre of mass's, and its spin about each
//! axis of the world. Its tracks have their own speeds besides, `tracks`, its belts',
//! which the client animates: on the ground they roll with it, off it they run free.
//!
//! # Where the numbers come from
//!
//! Each constant says whether it comes from the game's sources, was fitted on drives
//! captured from the real server (a T110E4 on Ruinberg, a stock Type 64 on Live Oaks,
//! and a story-mode Pershing on Graf Zeppelin, see `drive_replay.rs`), or is our own
//! choice until a capture can fit it. The drive harness replays those captures through
//! this module in windows, each starting from the real state, and reports the errors:
//! about 0.25 m median over 1 s windows. Collisions between vehicles were checked on the
//! Pershing's pushes of its allies, where the harness puts them as the real server had
//! them each tick; how high its tracks grip is the game's own (`comFrictionYOffs`), but
//! the rest of their model is ours: a vehicle not driven holds against a push with its
//! brakes, so that one pushes another as heavy at a crawl, and two not at all.
//!
//! # What isn't modelled
//!
//! Its collision shapes are boxes, not its hull's mesh: a rock catches its box's corner
//! where the real hull is narrower, and another vehicle meets its box's face, as high as
//! its hull, where the real ones meet their sloped plates. Its tracks on another vehicle
//! drive it, but don't push that one back. Its gun touches nothing. A house's floor, not
//! one of its destructible modules, stays when it breaks, and the hull drives up onto it,
//! where the real one went on below it on Live Oaks. It takes no damage from a fall, and
//! doesn't float or sink in water. Its tracks don't scrub sideways as it turns: its
//! steering turns it as the game's turning speeds say, and holds back a hull spun by a
//! push off its middle only as fast as it would stop turning. Vehicles take no damage
//! from ramming, each other or destructibles. Trees don't fall: the client numbers them
//! among the speed trees, which aren't decoded. What a destroyed module leaves
//! (`BodyKind::Remains`) is never collided with. Shells are flown in `gun.rs`, against
//! the same statics and rubble.

mod drive;
mod body;
mod contacts;
mod ground;
mod rubble;
mod walls;
mod collision;
mod nav;
mod turret;
mod client;
#[cfg(test)]
pub mod fixtures;
#[cfg(test)]
mod drive_replay;

use std::vec::Drain;
use std::sync::Arc;
use std::ops::Deref;

use glam::{Affine3A, EulerRot, Quat, Vec2, Vec3, Vec3Swizzles};
use tracing::warn;

use wgtk::app::slots::{SlotKey, Slots};
use wgtk::space::terrain::Ground;
use wgtk::space::Space;

use crate::game::item_defs::{AimingDef, MobilityDef};

use self::body::{blocking, Step, Tick, CONTACT_MARGIN, CONTACT_PASSES, SUBSTEPS};
use self::contacts::{contacts_of, Contacts, MAX_CONTACTS};
use self::collision::{hold_apart, touching, Pose, Solid, Touching};
use self::ground::ground_at;
use self::nav::NavGrid;

pub use self::rubble::{Breakage, DestructiblePiece, Rubble};
pub use self::turret::{Dispersion, Turret};


/// The `vehicle_moveWith` flags, `MOVEMENT_FLAGS` in the client's `Avatar.py`.
pub mod movement_flag {
    pub const FORWARD: u8 = 1;
    pub const BACKWARD: u8 = 2;
    pub const ROTATE_LEFT: u8 = 4;
    pub const ROTATE_RIGHT: u8 = 8;
    pub const CRUISE_CONTROL50: u8 = 16;
    pub const CRUISE_CONTROL25: u8 = 32;
    /// The handbrake.
    pub const BLOCK_TRACKS: u8 = 64;
}

/// Gravity, in m/s². From the game: `G` in `physics_shared.py`, which the server's
/// physics multiplies by the vehicle's `gravityFactor`, 1 when its definitions don't set
/// it, as the client's never do.
const GRAVITY: f32 = 9.81;
/// How many ticks in a row a hull is still on the ground before it rests, its body no
/// longer moved, and what still is: under this speed, in m/s, and this turning speed, in
/// rad/s. Resting, it sends no position updates for nothing. Our choice.
const REST_TICKS: u8 = 5;
const REST_SPEED: f32 = 0.02;
const REST_SPIN: f32 = 0.01;

/// A vehicle of a [`World`]. It is stale once the vehicle is removed, even if another one
/// takes its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VehicleId(SlotKey);

/// A vehicle in a [`World`], where it was when last remembered, see
/// [`World::remember`], and in a tick, what holds of its body over it, whether it moves,
/// and its step under way.
#[derive(Debug)]
struct Placed {
    vehicle: Vehicle,
    pose: Pose,
    tick: Option<Tick>,
    moving: bool,
    step: Option<Step>,
}

impl Placed {

    /// Its hull and its step under way, if it moves.
    fn body(&mut self) -> Option<(&mut Hull, &mut Step)> {
        Some((&mut self.vehicle.hull, self.step.as_mut()?))
    }

}

/// The vehicles of a battle on its map, which the map's statics and bounds hold in and
/// which push each other. See the module's documentation for what a step does, in
/// order. A step allocates nothing, once its lists of bodies and of where they touch
/// have grown.
///
/// Vehicles are in a slab with generational keys ([`Slots`]): a [`VehicleId`] never
/// reaches the vehicle that took the place of a removed one. Its space is shared with
/// the battle, an `Arc`, or borrowed, for a test's vehicle alone, see `Hull::advance`.
#[derive(Debug)]
pub struct World<S = Arc<Space>> {
    space: S,
    bounds: Option<(Vec2, Vec2)>,
    vehicles: Slots<Placed>,
    rubble: Rubble,
    /// The bodies of the vehicles moving, in a step, for each other to touch.
    solids: Vec<Solid>,
    /// Where they touch, in a step.
    touching: Vec<Touching>,
    /// Where tanks drive within its bounds, see [`Self::path`]; none without bounds.
    nav: Option<NavGrid>,
    /// How many pieces its rubble had broken when its way grid last looked: once more
    /// broke, it forgets what it learnt.
    nav_broken: usize,
}

impl<S: Deref<Target = Space>> World<S> {

    pub fn new(space: S, bounds: Option<(Vec2, Vec2)>) -> Self {
        Self {
            space,
            bounds,
            vehicles: Slots::default(),
            rubble: Rubble::default(),
            solids: Vec::new(),
            touching: Vec::new(),
            nav: bounds.map(NavGrid::new),
            nav_broken: 0,
        }
    }

    /// The map's space.
    pub fn space(&self) -> &Space {
        &self.space
    }

    /// What its vehicles and shells broke of the map, see [`Rubble`].
    pub fn rubble(&self) -> &Rubble {
        &self.rubble
    }

    /// What its vehicles and shells broke, to change: the drive harness keeps it from a
    /// window to the next.
    #[allow(dead_code)]
    pub fn rubble_mut(&mut self) -> &mut Rubble {
        &mut self.rubble
    }

    /// A way for a tank `radius` wide from one point to another over its map, or to as
    /// near it as it can get: the points it turns at, then its end. None without bounds,
    /// or starting out of them. See the `nav` module.
    pub fn path(&mut self, from: Vec3, to: Vec3, radius: f32) -> Option<Vec<Vec3>> {
        let nav = self.nav.as_mut()?;
        let broken = self.rubble.broken_count();
        if broken != self.nav_broken {
            nav.forget();
            self.nav_broken = broken;
        }
        nav.path(from, to, radius, &self.space, &blocking(&self.space, &self.rubble))
    }

    // -- Its vehicles --

    /// Put a vehicle in the world.
    pub fn add(&mut self, vehicle: Vehicle) -> VehicleId {
        VehicleId(self.vehicles.insert(Placed {
            pose: Pose::of(&vehicle.hull),
            vehicle,
            tick: None,
            moving: false,
            step: None,
        }))
    }

    /// Take a vehicle out of the world: the drive harness's other vehicles come and go.
    #[allow(dead_code)]
    pub fn remove(&mut self, id: VehicleId) -> Option<Vehicle> {
        self.vehicles.remove(id.0).map(|placed| placed.vehicle)
    }

    /// A vehicle of the world. Panics if it was removed: whoever removes a vehicle
    /// drops its id.
    pub fn vehicle(&self, id: VehicleId) -> &Vehicle {
        &self.vehicles.get(id.0).expect("a vehicle of the world").vehicle
    }

    pub fn vehicle_mut(&mut self, id: VehicleId) -> &mut Vehicle {
        &mut self.vehicles.get_mut(id.0).expect("a vehicle of the world").vehicle
    }

    // -- Its tick --

    /// Advance every vehicle by `dt` seconds on its controls, their bodies together, so
    /// that they push each other.
    pub fn step(&mut self, dt: f32) {
        self.step_on(dt, None);
    }

    /// Advance every vehicle by `dt` seconds, see [`Self::step`], on this `ground` if any,
    /// whatever the terrain's: the tests'.
    fn step_on(&mut self, dt: f32, ground: Option<Ground>) {

        // 1. Where each vehicle starts the tick, to tell jumps.
        self.remember();

        // 2. Each on its own: its drive model, on the ground under it, then it rests or
        //    sets off.
        for (_, placed) in self.vehicles.iter_mut() {
            let hull = &mut placed.vehicle.hull;
            let under = ground.unwrap_or_else(|| ground_at(&self.space.terrain, hull.position));
            placed.tick = Some(hull.start(dt, under));
            placed.moving = !hull.rests();
            if placed.moving {
                hull.set_off(dt, &self.space, &mut self.rubble);
            }
        }
        self.wake(dt);

        // 3. Their bodies, together.
        let step = dt / SUBSTEPS as f32;
        for _ in 0..SUBSTEPS {
            self.step_bodies(step);
        }

        // 4. Each: its speeds read back from its body, held within the map, its turret
        //    turned on it.
        for (_, placed) in self.vehicles.iter_mut() {
            if placed.moving {
                placed.vehicle.hull.finish(self.bounds);
            }
            placed.vehicle.turret.step(dt, &placed.vehicle.hull);
            placed.tick = None;
        }

        self.report_jumps(dt);

    }

    /// Remember where every vehicle is, as a step starts, to tell jumps after.
    pub fn remember(&mut self) {
        for (_, placed) in self.vehicles.iter_mut() {
            placed.pose = Pose::of(&placed.vehicle.hull);
        }
    }

    /// Wake the vehicles at rest that one moving may reach in a tick of `dt` seconds:
    /// they move with it, pushed.
    fn wake(&mut self, dt: f32) {
        let (space, rubble) = (&*self.space, &mut self.rubble);
        self.vehicles.for_each_pair_mut(|placed, other| {
            if placed.moving == other.moving {
                return;
            }
            let (moving, resting) = if placed.moving { (placed, other) } else { (other, placed) };
            let (center, radius) = moving.vehicle.hull.reach();
            let (other_center, other_radius) = resting.vehicle.hull.reach();
            let speed = moving.vehicle.hull.linear_velocity.length() + moving.vehicle.hull.angular_velocity.length() * radius;
            if center.distance(other_center) < radius + other_radius + speed * dt + CONTACT_MARGIN {
                resting.moving = true;
                resting.vehicle.hull.wake();
                resting.vehicle.hull.set_off(dt, space, rubble);
            }
        });
    }

    /// A step of the moving vehicles' bodies, `step` seconds long, see the `body` and
    /// `collision` modules:
    ///
    /// ```text
    ///   1. each one's body where it starts, for the others to touch
    ///   2. where they touch each other
    ///   3. each begins its step: where it touches the space and its springs the others,
    ///      its weight and its springs accelerating it
    ///   4. its springs on the others push them back
    ///   5. CONTACT_PASSES passes: each one's contacts with the space held, then where
    ///      they touch each other, held apart
    ///   6. each moves
    /// ```
    fn step_bodies(&mut self, step: f32) {

        let space = &*self.space;
        let keep = blocking(space, &self.rubble);

        // 1. Their bodies.
        self.solids.clear();
        for (key, placed) in self.vehicles.iter() {
            if placed.moving {
                self.solids.push(Solid::of(key, &placed.vehicle.hull));
            }
        }

        // 2. Where they touch each other.
        self.touching.clear();
        for (index, solid) in self.solids.iter().enumerate() {
            for other in &self.solids[index + 1..] {
                touching(solid, other, step, &mut self.touching);
            }
        }

        // 3. Each begins its step.
        for (index, solid) in self.solids.iter().enumerate() {
            let placed = self.vehicles.get_mut(solid.key).expect("a vehicle of the world");
            let tick = placed.tick.as_ref().expect("a tick under way");
            placed.step = Some(placed.vehicle.hull.begin_step(tick, space, &keep, &self.solids, Some(index)));
        }

        // 4. Its springs on the others push them back.
        for solid in &self.solids {
            let placed = self.vehicles.get_mut(solid.key).expect("a vehicle of the world");
            let Some(step) = placed.step.as_mut() else { continue };
            let reactions = std::mem::take(&mut step.reactions);
            for reaction in reactions {
                let other = self.vehicles.get_mut(self.solids[reaction.other].key).expect("a vehicle of the world");
                let Some((hull, step)) = other.body() else { continue };
                hull.take(reaction.impulse, reaction.point - step.mass_center, &step.body);
            }
        }

        // 5. Held off the space, and apart.
        for pass in 0..CONTACT_PASSES {
            for (_, placed) in self.vehicles.iter_mut() {
                let Placed { vehicle, tick, step: Some(step), .. } = placed else { continue };
                vehicle.hull.solve_pass(tick.as_ref().expect("a tick under way"), step, pass);
            }
            for touching in &mut self.touching {
                let Some((placed, other)) = self.vehicles.get_pair_mut(touching.first, touching.second) else { continue };
                let (Some(body), Some(other_body)) = (placed.body(), other.body()) else { continue };
                hold_apart(touching, body, other_body, step);
            }
        }

        // 6. Each moves.
        for (_, placed) in self.vehicles.iter_mut() {
            let Some(step) = placed.step.take() else { continue };
            placed.vehicle.hull.end_step(placed.tick.as_ref().expect("a tick under way"), &step);
        }

    }

    /// Warn about the vehicles that moved further in a step than their speed allows, or
    /// that turned or rose abruptly: what the client sees as a jump. And about those still
    /// in each other after the step.
    fn report_jumps(&self, dt: f32) {

        for solid in &self.solids {
            for other in self.solids.iter().skip_while(|other| other.key != solid.key).skip(1) {
                let Some(depth) = solid.depth_in(other) else { continue };
                let first = &self.vehicles.get(solid.key).expect("a vehicle of the world").vehicle.hull;
                let second = &self.vehicles.get(other.key).expect("a vehicle of the world").vehicle.hull;
                if depth > 0.1 {
                    warn!(
                        first = ?first.position,
                        second = ?second.position,
                        depth,
                        "Left two vehicles in each other after a step",
                    );
                }
            }
        }

        for (_, placed) in self.vehicles.iter() {
            let hull = &placed.vehicle.hull;
            let moved = hull.position - placed.pose.position;
            let allowed = (hull.speed.abs() + hull.slide.abs()) * dt * 1.5 + 0.5;
            // Its yaw only: on its suspension, it pitches and rolls fast over bumps.
            let yaw_from = placed.pose.orientation.to_euler(EulerRot::YXZ).0;
            let turned = wrap_angle(hull.yaw() - yaw_from).abs();
            let rose = moved.y.abs() - moved.xz().length();
            if moved.length() > allowed || rose > 0.4 || turned > 0.25 {
                warn!(
                    from = ?placed.pose.position,
                    to = ?hull.position,
                    yaw_from,
                    yaw_to = hull.yaw(),
                    speed = hull.speed,
                    "Jumped a vehicle in a step",
                );
            }
        }

    }

    // -- What breaks --

    /// Break a piece a shell flew through, see [`Rubble::break_shot`].
    pub fn break_shot(&mut self, instance: u32, piece: DestructiblePiece, yaw: f32) {
        self.rubble.break_shot(&self.space.statics, instance, piece, yaw);
    }

    /// Damage a piece a shell hit and stopped on, see [`Rubble::shoot`]. What broke.
    pub fn shoot(&mut self, instance: u32, piece: DestructiblePiece, damage: f32, yaw: f32) -> &[Breakage] {
        self.rubble.shoot(&self.space.statics, instance, piece, damage, yaw)
    }

    /// The destructibles broken since last taken, for the battle to tell its clients.
    pub fn take_breakages(&mut self) -> Drain<'_, Breakage> {
        self.rubble.take_breakages()
    }

}

/// A live vehicle: its hull and its turret.
#[derive(Debug, Clone)]
pub struct Vehicle {
    pub hull: Hull,
    pub turret: Turret,
}

impl Vehicle {

    pub fn new(position: Vec3, direction: Vec3, mobility: MobilityDef, aiming: AimingDef) -> Self {
        Self {
            hull: Hull::new(position, direction, mobility),
            turret: Turret::new(aiming),
        }
    }

    /// Where its chassis, hull, turret and gun are in the world, as transforms from
    /// each part's frame, the hull sitting at `hull_position` on the chassis.
    pub fn part_transforms(&self, hull_position: Vec3) -> [Affine3A; 4] {
        let aiming = &self.turret.aiming;
        let turret_rotation = Quat::from_rotation_y(self.turret.yaw);
        let gun_rotation = Quat::from_rotation_x(self.turret.pitch);
        let chassis = Affine3A::from_rotation_translation(self.hull.rotation(), self.hull.position);
        let hull = chassis * Affine3A::from_translation(hull_position);
        let turret = chassis * Affine3A::from_rotation_translation(turret_rotation, aiming.turret_position);
        let gun = turret * Affine3A::from_rotation_translation(gun_rotation, aiming.gun_position);
        [chassis, hull, turret, gun]
    }

}

/// A vehicle's hull, moving as the client drives it: a rigid body, held up by its
/// suspension, see [`Self::begin_step`]. Its motion is kept both on the ground plane and
/// in 3D, see "Two views of the same motion" in the module's documentation.
#[derive(Debug, Clone)]
pub struct Hull {
    /// Its origin: the middle of its tracks' footprint, on the ground at rest.
    pub position: Vec3,
    /// How it is turned in the world, see [`Self::direction`].
    pub orientation: Quat,
    /// Signed speed of its origin along the vehicle's heading, in m/s: its body's, read
    /// back as its tick ends, and between its ticks, what hits and walls leave it, for
    /// its body to take as the next starts, see [`Self::speeds_to_body`].
    pub speed: f32,
    /// Signed speed of its origin across its heading, positive to the right, in m/s, as
    /// `speed` is: it slides with its tracks blocked, then its tracks hold it again.
    pub slide: f32,
    /// Turning speed about the vertical, in rad/s, positive to the right, as `speed` is.
    pub yaw_rate: f32,
    /// Each of its tracks' speed, a pair by `drive::Side`, in m/s, positive forward: its
    /// belts', rolling with the ground under them, or spinning where they slip or touch
    /// nothing, see [`Self::solve_pass`].
    pub tracks: [f32; 2],
    /// The turning speed its steering pushes it toward, in rad/s, positive to the right,
    /// see [`Self::drive`].
    steer: f32,
    /// The velocity of its centre of mass, in m/s: its body's, see [`Self::begin_step`].
    pub linear_velocity: Vec3,
    /// How fast it turns, about each axis of the world, in rad/s: its body's.
    pub angular_velocity: Vec3,
    /// Whether its tracks don't grip the ground: none of their contacts touches it, or
    /// it is on its side or its roof, see `body::TRACKS_UPRIGHT`. They run free then.
    pub airborne: bool,
    /// Whether any of its contacts touches the ground, its body's too: it can rest.
    touching: bool,
    /// How many ticks in a row it has been still on the ground, see [`REST_TICKS`].
    still: u8,
    /// How deep each of its contacts was in the ground at the end of the last step of
    /// its body, NaN where none was found: their dampers resist how fast they sink, see
    /// [`Self::begin_step`].
    depths: [f32; MAX_CONTACTS],
    /// Where its suspension and its body touch the ground, in its own frame, made once
    /// from its mobility, see [`contacts::contacts_of`].
    contacts: Contacts,
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub mobility: MobilityDef,
}

impl Hull {

    pub fn new(position: Vec3, direction: Vec3, mobility: MobilityDef) -> Self {
        Self {
            position,
            orientation: rotation_of(direction),
            speed: 0.0,
            slide: 0.0,
            yaw_rate: 0.0,
            tracks: [0.0; 2],
            steer: 0.0,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            airborne: false,
            touching: true,
            still: 0,
            depths: [f32::NAN; MAX_CONTACTS],
            contacts: contacts_of(&mobility),
            flags: 0,
            mobility,
        }
    }

    // -- Its tick --

    /// Advance by `dt` seconds in its space alone, within the map's `bounds` if any, as a
    /// world of it alone does, see [`World::step`], what it breaks in `rubble`: the tests'
    /// and the drive harness's way to move one. Returns whether it moved, not resting.
    #[cfg(test)]
    pub fn advance(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &mut Rubble) -> bool {
        self.advance_on(dt, space, bounds, rubble, None)
    }

    /// Advance by `dt` seconds alone, see [`Self::advance`], on this `ground` if any,
    /// whatever the terrain's. Its turret, which it doesn't have, is a Chieftain's.
    #[cfg(test)]
    pub fn advance_on(
        &mut self,
        dt: f32,
        space: &Space,
        bounds: Option<(Vec2, Vec2)>,
        rubble: &mut Rubble,
        ground: Option<Ground>,
    ) -> bool {
        let mut world = World::new(space, bounds);
        std::mem::swap(&mut world.rubble, rubble);
        let id = world.add(Vehicle {
            hull: self.clone(),
            turret: Turret::new(fixtures::chieftain_aiming()),
        });
        world.step_on(dt, ground);
        std::mem::swap(&mut world.rubble, rubble);
        *self = world.remove(id).expect("a vehicle of the world").hull;
        self.still <= REST_TICKS
    }

    /// The start of a tick of `dt` seconds, on this `ground`: the drive model says how
    /// hard its engine, its brakes and its steering push it, for its tracks to pass on
    /// where they grip, and what holds of its body over the tick.
    pub fn start(&mut self, dt: f32, ground: Ground) -> Tick {
        let drive = self.drive(dt, ground);
        self.tick(dt, drive)
    }

    /// Whether it rests this tick, still on the ground long enough: nothing to move,
    /// nothing to send.
    pub fn rests(&mut self) -> bool {
        self.still = if self.is_still() { self.still.saturating_add(1) } else { 0 };
        self.still > REST_TICKS
    }

    /// Whether it is still on the ground, its controls released: no speed or turning
    /// to speak of, on the ground plane or in 3D, see [`REST_SPEED`] and [`REST_SPIN`].
    fn is_still(&self) -> bool {
        self.touching
            && self.flags == 0
            && self.speed.abs() < REST_SPEED
            && self.slide.abs() < REST_SPEED
            && self.yaw_rate.abs() < REST_SPIN
            && self.steer.abs() < REST_SPIN
            && self.tracks.iter().all(|track| track.abs() < REST_SPEED)
            && self.linear_velocity.length() < REST_SPEED
            && self.angular_velocity.length() < REST_SPIN
    }

    /// Wake it from rest: another vehicle may push it.
    pub fn wake(&mut self) {
        self.still = 0;
    }

    /// Set off for a tick of `dt` seconds: what it is about to ram damaged, and broken out
    /// of its way before it moves, or stopping it as walls do; then its body takes what
    /// hits and rams left on the ground plane since its last tick.
    pub fn set_off(&mut self, dt: f32, space: &Space, rubble: &mut Rubble) {
        self.ram(dt, space, rubble);
        self.speeds_to_body();
    }

    /// The end of a tick: its speeds on the ground plane read back from its body, and
    /// the map's edge.
    pub fn finish(&mut self, bounds: Option<(Vec2, Vec2)>) {
        self.speeds_from_body();
        if let Some(bounds) = bounds {
            self.keep_within(bounds);
        }
    }

    // -- Its orientation --

    /// Its roll, pitch and yaw, in radians, the order position updates carry them: rolled
    /// positively right side up, pitched positively nose down. A yaw of 0 faces +Z, and
    /// turning right increases it.
    pub fn direction(&self) -> Vec3 {
        let (yaw, pitch, roll) = self.orientation.to_euler(EulerRot::YXZ);
        Vec3::new(roll, pitch, yaw)
    }

    /// Turn it to a roll, pitch and yaw, see [`Self::direction`].
    #[cfg(test)]
    pub fn set_direction(&mut self, direction: Vec3) {
        self.orientation = rotation_of(direction);
    }

    /// Its yaw, see [`Self::direction`].
    pub fn yaw(&self) -> f32 {
        self.orientation.to_euler(EulerRot::YXZ).0
    }

    /// The hull's orientation: yaw about Y, then pitch about X (nose down), then roll
    /// about Z (right side up), as the client's matrices compose them.
    pub fn rotation(&self) -> Quat {
        self.orientation
    }

    // -- Its motion on the ground plane --

    /// Set its speeds along and across its heading from a velocity on the ground plane.
    fn set_velocity(&mut self, velocity: Vec3) {
        let (heading, right) = self.axes();
        self.speed = velocity.dot(heading);
        self.slide = velocity.dot(right);
    }

    /// Its heading and its right on the ground plane.
    fn axes(&self) -> (Vec3, Vec3) {
        let yaw = self.yaw();
        let heading = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        (heading, right)
    }

    /// How fast the point at `arm` from its middle moves on the ground plane, in m/s: its
    /// velocity and its yaw rate's.
    fn point_velocity(&self, arm: Vec2) -> Vec2 {
        self.velocity().xz() + Vec2::new(arm.y, -arm.x) * self.yaw_rate
    }

    /// Its velocity on the ground plane, in m/s.
    pub fn velocity(&self) -> Vec3 {
        let (heading, right) = self.axes();
        heading * self.speed + right * self.slide
    }

    /// Its collision box on the ground plane, upright: where it touches destructibles.
    fn footprint(&self) -> Footprint {
        let (sin, cos) = self.yaw().sin_cos();
        let heading = Vec2::new(sin, cos);
        let right = Vec2::new(cos, -sin);
        let [min, max] = self.mobility.bounds;
        let center = (min + max) / 2.0;
        let half = (max - min) / 2.0;
        Footprint {
            middle: self.position.xz() + heading * center.z + right * center.x,
            heading,
            right,
            half: Vec2::new(half.x, half.z),
        }
    }

}

/// A hull's collision box on the ground plane, upright: its middle, its heading and right
/// axes, and its half sizes across then along.
#[derive(Debug, Clone, Copy)]
struct Footprint {
    middle: Vec2,
    heading: Vec2,
    right: Vec2,
    half: Vec2,
}

impl Footprint {

    /// Its corners.
    fn corners(&self) -> [Vec2; 4] {
        [(1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (-1.0, 1.0)]
            .map(|(along, across)| self.middle + self.heading * along * self.half.y + self.right * across * self.half.x)
    }

}

/// A value moved toward a goal by at most `step`, without going past it.
fn approach(value: f32, goal: f32, step: f32) -> f32 {
    value + (goal - value).clamp(-step, step)
}

/// The orientation of a roll, pitch and yaw, see [`Hull::rotation`].
fn rotation_of(direction: Vec3) -> Quat {
    Quat::from_rotation_y(direction.z) * Quat::from_rotation_x(direction.y) * Quat::from_rotation_z(direction.x)
}

/// An angle in `(-π, π]`.
pub fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI { wrapped - TAU } else { wrapped }
}
