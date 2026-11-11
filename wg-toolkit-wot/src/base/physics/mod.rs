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
//!   ├─ 1. remember()           pose ← where each vehicle is, for the sweep in 3
//!   │
//!   ├─ 2. for every vehicle: Vehicle::step
//!   │     ├─ Hull::advance                                                   (here)
//!   │     │    ├─ drive            the drive model: how hard its engine,       drive
//!   │     │    │                   brakes and steering push it                ground
//!   │     │    ├─ (still for long enough: it rests, and the rest is skipped)
//!   │     │    ├─ ram              damage and break the destructibles it is   rubble
//!   │     │    │                   about to get into
//!   │     │    ├─ speeds_to_body   its body takes what hits and rams left it  body
//!   │     │    ├─ integrate        the rigid body, 10 steps: gravity, its      body
//!   │     │    │                   suspension, its tracks driven, all it      contacts
//!   │     │    │                   touches of the terrain and static models   (wgtk::space)
//!   │     │    │                   holding it off
//!   │     │    └─ keep_within      stop at the map's edge                     walls
//!   │     └─ Turret::step          turn toward the aim, update the dispersion  turret
//!   │
//!   ├─ 3. collide(dt), up to 3 passes over every pair of vehicles       (here), collision
//!   │     for a pair whose 3D boxes overlap:
//!   │     ├─ find where they first touched in the tick, put both back there
//!   │     ├─ push them apart, each by the other's share of their weight
//!   │     ├─ exchange an impulse at the contact: speeds and spin
//!   │     ├─ let both go on for what was left of the tick at their new speeds
//!   │     └─ keep_on_map for both: walls, edge, ground again
//!   │
//!   └─ then the battle tells its clients what moved (client: engine and aux data)
//!      and what broke, World::take_breakages
//! ```
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
//!   walls       the map's edge, shoves out of the static models, what it rams
//!   collision   two vehicles' boxes, where they first touch, the impulse between
//!   turret      the turret and its gun: turning, aiming, dispersion
//!   client      what the client is told beyond positions: engine and tracks
//! ```
//!
//! Nothing in a tick allocates: boxes, outlines and polygons live on the stack
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
//!   in 3D (its body)             linear_velocity,                   integrate ends
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
//! Pershing's pushes of its allies, but their model (the impulse without bounce or
//! friction) is ours.
//!
//! # What isn't modelled
//!
//! Vehicles still hit each other on the ground plane, not in 3D: that is next. Its
//! collision shapes are boxes, not its hull's mesh: a rock catches its box's corner
//! where the real hull is narrower. A house's floor, not one of its destructible modules,
//! stays when it breaks, and the hull drives up onto it, where the real one went on
//! below it on Live Oaks. It takes no damage from a fall, and doesn't float or sink in
//! water. Its tracks don't scrub sideways as it turns: its steering turns it as the
//! game's turning speeds say, and holds back a hull spun by a push off its middle only
//! as fast as it would stop turning. Vehicles have no friction between them and
//! take no damage from ramming, each other or destructibles. Trees don't fall: the
//! client numbers them among the speed trees, which aren't decoded. What a destroyed
//! module leaves (`BodyKind::Remains`) is never collided with. Shells are flown in
//! `gun.rs`, against the same statics and rubble.

mod drive;
mod body;
mod contacts;
mod ground;
mod rubble;
mod walls;
mod collision;
mod turret;
mod client;
#[cfg(test)]
pub mod fixtures;

use std::sync::Arc;
use std::vec::Drain;

use glam::{Affine3A, EulerRot, Quat, Vec2, Vec3, Vec3Swizzles};
use tracing::warn;

use wgtk::app::slots::{SlotKey, Slots};
use wgtk::space::Space;

use crate::wot::item_defs::{AimingDef, MobilityDef};

use self::contacts::{contacts_of, Contacts, MAX_CONTACTS};
use self::collision::{collide, Pose};
use self::ground::ground_at;

pub use self::rubble::{Breakage, Piece, Rubble};
pub use self::turret::Turret;


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
/// How many times a step pushes vehicles apart, those pushed into a third again.
const COLLISION_PASSES: usize = 3;

/// A vehicle of a [`World`]. It is stale once the vehicle is removed, even if another one
/// takes its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VehicleId(SlotKey);

/// A vehicle in a [`World`] and where it was when last remembered, see
/// [`World::remember`].
#[derive(Debug)]
struct Placed {
    vehicle: Vehicle,
    pose: Pose,
}

/// The vehicles of a battle on its map, which the map's statics and bounds hold in and
/// which push each other. A step allocates nothing. See the module's documentation for
/// what a step does, in order.
///
/// Vehicles are in a slab with generational keys ([`Slots`]): a [`VehicleId`] never
/// reaches the vehicle that took the place of a removed one.
#[derive(Debug)]
pub struct World {
    space: Arc<Space>,
    bounds: Option<(Vec2, Vec2)>,
    vehicles: Slots<Placed>,
    rubble: Rubble,
}

impl World {

    pub fn new(space: Arc<Space>, bounds: Option<(Vec2, Vec2)>) -> Self {
        Self {
            space,
            bounds,
            vehicles: Slots::default(),
            rubble: Rubble::default(),
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

    // -- Its vehicles --

    /// Put a vehicle in the world.
    pub fn add(&mut self, vehicle: Vehicle) -> VehicleId {
        VehicleId(self.vehicles.insert(Placed {
            pose: Pose::of(&vehicle.hull),
            vehicle,
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

    /// Advance every vehicle by `dt` seconds on its controls, then push apart those that
    /// ran into each other, see [`Self::collide`].
    pub fn step(&mut self, dt: f32) {

        // 1. Where each vehicle starts the tick, for the collisions to sweep from.
        self.remember();

        // 2. Each vehicle on its own, see `Vehicle::step`.
        for (_, placed) in self.vehicles.iter_mut() {
            placed.vehicle.step(dt, &self.space, self.bounds, &mut self.rubble);
        }

        // 3. Those that ran into each other, pushed apart.
        self.collide(dt);

        self.report_jumps(dt);

    }

    /// Remember where every vehicle is, as a step starts: [`Self::collide`] finds from
    /// there where vehicles first touched, so that fast ones don't get into each other.
    pub fn remember(&mut self) {
        for (_, placed) in self.vehicles.iter_mut() {
            placed.pose = Pose::of(&placed.vehicle.hull);
        }
    }

    /// Push apart the vehicles whose collision boxes overlap, see [`collide`]: where they
    /// first touched since they were remembered, a step of `dt` seconds ago, the vehicles
    /// pushed into others apart again, and those that touched back out of the static
    /// models and within the map's bounds, their suspension catching them: a vehicle isn't
    /// shoved through a wall.
    ///
    /// ```text
    ///   pass 0   every pair, swept from the remembered poses to where the step left them
    ///   pass 1   every pair again, from where pass 0 left them (nothing to sweep: they
    ///   pass 2   were put there), if any pair touched, up to COLLISION_PASSES passes
    /// ```
    ///
    /// The extra passes are for three vehicles in a row: pushing the middle one off the
    /// first can push it into the third, which pass 0 had already checked.
    pub fn collide(&mut self, dt: f32) {
        let (space, bounds, rubble) = (&*self.space, self.bounds, &self.rubble);
        for pass in 0..COLLISION_PASSES {
            let mut touched = false;
            self.vehicles.for_each_pair_mut(|placed, other| {
                // Past the first pass, they were pushed to where they are.
                let (from, other_from) = match pass {
                    0 => (placed.pose, other.pose),
                    _ => (Pose::of(&placed.vehicle.hull), Pose::of(&other.vehicle.hull)),
                };
                if collide(&mut placed.vehicle.hull, from, &mut other.vehicle.hull, other_from, dt) {
                    touched = true;
                    other.vehicle.hull.keep_on_map(space, bounds, rubble);
                    placed.vehicle.hull.keep_on_map(space, bounds, rubble);
                }
            });
            if !touched {
                break;
            }
        }
    }

    /// Warn about the vehicles that moved further in a step than their speed allows, or
    /// that turned or rose abruptly: what the client sees as a jump. And about those still
    /// in each other after the step.
    fn report_jumps(&self, dt: f32) {

        for (key, placed) in self.vehicles.iter() {
            let obb = placed.vehicle.hull.obb();
            for (_, other) in self.vehicles.iter().skip_while(|&(other_key, _)| other_key != key).skip(1) {
                let depth = match obb.separation(&other.vehicle.hull.obb()) {
                    Some((_, depth)) => depth,
                    None => continue,
                };
                if depth > 0.05 {
                    warn!(
                        first = ?placed.vehicle.hull.position,
                        second = ?other.vehicle.hull.position,
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
    pub fn break_shot(&mut self, instance: u32, piece: Piece, yaw: f32) {
        self.rubble.break_shot(&self.space.statics, instance, piece, yaw);
    }

    /// Damage a piece a shell hit and stopped on, see [`Rubble::shoot`]. What broke.
    pub fn shoot(&mut self, instance: u32, piece: Piece, damage: f32, yaw: f32) -> &[Breakage] {
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

    /// Advance by `dt` seconds in its space, within the map's bounds if any: the hull
    /// moves, see [`Hull::advance`], then the turret turns on it. Returns whether the
    /// hull moved or turned.
    pub fn step(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &mut Rubble) -> bool {
        let moved = self.hull.advance(dt, space, bounds, rubble);
        self.turret.step(dt, &self.hull);
        moved
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
/// suspension, see [`Self::integrate`]. Its motion is kept both on the ground plane and
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
    /// nothing, see [`Self::integrate`].
    pub tracks: [f32; 2],
    /// The turning speed its steering pushes it toward, in rad/s, positive to the right,
    /// see [`Self::drive`].
    steer: f32,
    /// The velocity of its centre of mass, in m/s: its body's, see [`Self::integrate`].
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
    /// [`Self::integrate`].
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

    /// Advance by `dt` seconds in its space. Returns whether the hull moved or turned.
    ///
    /// ```text
    ///   drive(ground under it)     the drive model: how hard its engine, brakes and
    ///        ▼                     steering push it
    ///   (still? it rests)          nothing below happens
    ///        ▼
    ///   ram                        the destructibles it is about to get into, damaged,
    ///        ▼                     broken ones out of its way, speed lost to them
    ///   speeds_to_body             its body takes the speeds hits and rams left it
    ///        ▼
    ///   integrate                  the body and its tracks move: gravity, its
    ///        ▼                     suspension, its tracks gripping the ground, walls
    ///   keep_within(bounds)        inside the map, speed into the edge lost
    /// ```
    ///
    /// A hull still on the ground for [`REST_TICKS`] rests: it isn't moved until its
    /// controls or a hit move it again.
    pub fn advance(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &mut Rubble) -> bool {

        // 1. The drive model says how hard its engine, its brakes and its steering push
        //    it, for its tracks to pass on where they grip.
        let drive = self.drive(dt, ground_at(&space.terrain, self.position));

        // 2. Still on the ground long enough, it rests: nothing to move, nothing to send.
        self.still = if self.is_still() { self.still.saturating_add(1) } else { 0 };
        if self.still > REST_TICKS {
            return false;
        }

        // 3. What it is about to ram: damaged, and broken out of its way before it moves,
        //    or stopping it as walls do.
        self.ram(dt, space, rubble);

        // 4. Its body takes what hits and rams left on the ground plane since its last
        //    tick, then it moves, its tracks pushing it where they grip, walls holding it.
        self.speeds_to_body();
        self.integrate(dt, space, rubble, &drive);

        // 5. The map's edge.
        if let Some(bounds) = bounds {
            self.keep_within(bounds);
        }

        true

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

    /// Put it back where the map holds it after it was shoved by another vehicle: out of
    /// the static models and within the map's bounds, as the end of [`Self::advance`]
    /// does. A vehicle is never shoved through a wall or off the map, but can be off a
    /// ledge: its suspension finds out on the next tick.
    pub fn keep_on_map(&mut self, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &Rubble) {
        self.still = 0;
        self.push_out(space, rubble);
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

    /// Its heading and its right on the ground plane.
    fn axes(&self) -> (Vec3, Vec3) {
        let yaw = self.yaw();
        let heading = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        (heading, right)
    }

    /// Its velocity on the ground plane, in m/s.
    pub fn velocity(&self) -> Vec3 {
        let (heading, right) = self.axes();
        heading * self.speed + right * self.slide
    }

    /// Set its speeds along and across its heading from a velocity on the ground plane.
    fn set_velocity(&mut self, velocity: Vec3) {
        let (heading, right) = self.axes();
        self.speed = velocity.dot(heading);
        self.slide = velocity.dot(right);
    }

    /// How fast the point at `arm` from its middle moves on the ground plane, in m/s: its
    /// velocity and its yaw rate's.
    fn point_velocity(&self, arm: Vec2) -> Vec2 {
        self.velocity().xz() + Vec2::new(arm.y, -arm.x) * self.yaw_rate
    }

    /// Its inertia against turning on the ground plane, in kg·m²: that of a box of its
    /// weight over its collision box's width and length.
    fn yaw_inertia(&self) -> f32 {
        let [min, max] = self.mobility.bounds;
        let size = max.xz() - min.xz();
        self.mobility.weight * size.length_squared() / 12.0
    }

    /// Take an impulse, in kg·m/s, at `arm` from its middle on the ground plane: its
    /// velocity and its spin change. What walls and other vehicles do to it.
    fn hit(&mut self, impulse: Vec2, arm: Vec2, mass: f32, inertia: f32) {
        let velocity = self.velocity() + Vec3::new(impulse.x, 0.0, impulse.y) / mass;
        self.set_velocity(velocity);
        self.yaw_rate -= arm.perp_dot(impulse) / inertia;
    }

    /// Its collision box on the ground plane, upright: where it touches walls,
    /// destructibles and other vehicles.
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
