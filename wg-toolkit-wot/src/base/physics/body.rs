//! A hull's rigid body and its tracks: how gravity, its suspension and its tracks move
//! it in 3D ([`Hull::begin_step`], [`Hull::solve_pass`], [`Hull::end_step`]).
//!
//! A hull is a rigid body: a position and an orientation, the velocity of its centre of
//! mass and its spin about each axis of the world, its weight as its mass and a box's
//! inertia. Its two tracks each have their own speed, their belt's, and a little mass,
//! which the client animates. Each tick moves them in [`SUBSTEPS`] steps of 10 ms:
//!
//! ```text
//!   for each step:
//!     1. its weight, at its centre of mass
//!     2. where its contacts touch the space, its terrain and its static models alike
//!          (see the contacts module), and its springs other vehicles' boxes: its
//!          springs and its tracks' ends push it up, those past their travel are
//!          stopped; its boxes are held off what they touch
//!     3. gravity and the springs accelerate it, against its mass and inertia
//!     4. its contacts touching the space are held, as impulses (solve): no deeper,
//!          its tracks hold it across and pass its drive's push and turning on to the
//!          ground (see the drive module), the rest scrapes
//!     5. it moves
//!   then its speeds on the ground plane are read back from it
//! ```
//!
//! In a world, its vehicles' steps go together, see `World::step`: in the same passes
//! as each is held off the space, they are held apart where they touch, see the
//! `collision` module.
//!
//! # Its suspension
//!
//! Its suspension pushes it up under each of its road wheels: a spring and a damper at
//! each, sharing its weight, sinking `SAG` at rest so that its origin is on the ground.
//! Each spring finds what is under it by a ray down its hull from its wheel's centre,
//! the space's terrain and static models alike. Its tracks' ends, around its idlers and
//! sprockets, give too: spheres down to the nearest road wheel, which meet a rock or a
//! step before its first road wheel does. Each pushes as a spring from where it is
//! deepest in what it touches, along its normal, and gives as the road wheels do, up
//! its hull: as much of their travel as what it touches faces up its hull. A bump the
//! track's front meets lifts it softly, as the game's sprung road wheels do, rather than
//! throwing it up; a wall, facing across its hull, gets no give and stops it at once.
//! The rest of it is rigid: its hull's boxes and its turret's, which stop it on what is
//! higher than its tracks, and on the ground when it rolls over. See the `contacts`
//! module for their shapes. The pushes' torques roll and pitch it, and its spin carries
//! on from a step to the next: it bounces, rocks, and tips.
//!
//! ```text
//!   back                                           front
//!           _______________________________________
//!          |                   x                   |           x   centre of mass
//!          |_______________________________________|           |   a spring, from the hull
//!       (O)     |     |     |     |     |     |     (O)            down to its road wheel
//!        #\     o     o     o     o     o     o     /#         o   road wheels
//!          #\___*_____*_____*_____*_____*_____*___/#           *   where it pushes: the track
//!     ===================================================          on the ground under it
//!                          ground                             (O)  sprocket, idler
//!                                                              #   the track around them,
//!                                                                  sprung up the hull
//!
//!   for a spring, its ray down from its wheel's centre, h above *, hitting at distance r:
//!                 depth = h + SAG - r                    (no hit: it is in the air)
//!                 push  = k * depth + d * (how fast depth grows), up, at the hit
//!                 SAG   = g / w^2,  w = 2 pi * SUSPENSION_FREQUENCY
//!
//!   for a track's end, deepest d in what it touches, of normal n, the hull's up u:
//!                 travel = SUSPENSION_TRAVEL * max(n . u, 0)
//!                 push   = k * min(d, travel) + d * (how fast it sinks along n), along n
//! ```
//!
//! A spring gives [`SUSPENSION_TRAVEL`] past where it rests, then stops hard: its contact
//! sinks no deeper, an impulse stopping it as the ground does, and a hull landing from a
//! ledge stops on its tracks rather than going into the ground. A track's end stops hard
//! past its travel too, along its normal. What sank past is moved
//! back out without the hull keeping any speed from it (a split impulse, see
//! `Hull::solve_pass`), or it would be thrown up.
//!
//! # Its tracks
//!
//! Its tracks pass its drive on to the ground, see the `drive` module: a push along its
//! heading toward a target speed, within a range of force, and a torque toward a turning
//! speed, `steer`. Across, they hold it from sliding sideways. All of it is solved in
//! each step, after gravity and the springs, as impulses over its contacts touching the
//! ground ([`Hull::solve_pass`]), [`CONTACT_PASSES`] passes of them, sequential impulses:
//! each is held in turn, those held moving the others, and each's sum over the passes
//! is bounded, not each pass's. A step:
//!
//! ```text
//!   before the passes   rolling friction at each contact of its tracks: against its
//!                       motion along them, within rolling friction · its load
//!                       its drive's target and force range, at its speed at p,
//!                       see Drive::along
//!   each pass:        its tracks together, first:
//!                       1. along: pushed at p toward the target, within the force
//!                          range · h and what of their grip turning leaves
//!                       2. turning: about n toward steer, within what of their grip
//!                          pushing along leaves
//!                     then at each contact, back and forth, so that nothing pushes it
//!                     into what it touches last:
//!                       3. held off: a spring past its travel sinks no further, up; a
//!                          track's end or a box gets no deeper along its normal
//!                       4. a track's: held across, at under, within its side grip;
//!                          its end, climbing what it meets, see below
//!                          any other: scrapes, within SCRAPE_FRICTION · how hard pressed
//!                       5. in what it touches: moved back out, split
//!   after the passes    its belts (see the end of this section)
//! ```
//!
//! where, for its tracks' contacts touching the ground, each pressed with `Nᵢ` in the
//! step (its spring's push, and how hard it is held off so far), at `armᵢ` from the
//! centre of mass, `xᵢ` of it across its tracks, `h` the step:
//!
//! ```text
//!   n      their ground's normal, all their contacts' together; along, across: its axes
//!   N      = Σ Nᵢ                                     how hard they are pressed in all
//!   p      = Σ Nᵢ · armᵢ / N, its part across taken out, raised    where they push it
//!                                                                     along
//!   under  = armᵢ − along · (armᵢ · along), raised    where a contact holds it across
//!   raised r = r − n · (r · n) · FRICTION_HEIGHT      toward its centre of mass
//!   G      = TRACK_GRIP · grip(slope along) · N       their grip along
//!   C      = TRACK_GRIP · grip(slope along) · Σ Nᵢ · |xᵢ|   their grip turning it
//!
//!   along     Δ = (target − (v + ω × p) · along) / k(p, along)
//!   turning   Δ = (steer − ω · n) / (n · I⁻¹ n)
//!   across    Δ = −((v + ω × under) · across) / k(under, across)
//!             k(r, a) = 1/m + (r × a) · I⁻¹ (r × a), how much an impulse moves r along a
//!
//!   shared    |along| / G + |turning| / C ≤ 1
//!   across    within side grip · grip(slope across) · Nᵢ · √(1 − used²),
//!             used = |along| / G + |turning| / C
//! ```
//!
//! The grips are shared as two tracks would: each carries half the push along, and the
//! torque as opposite pushes, `along / 2 ± turning / (2x)`, so the most both can take
//! together is a diamond. Across, a contact holds what grip their push along leaves it,
//! an ellipse: pushing hard, they slide out of turns. Grip falls with the ground's slope
//! as the game's slope grip says ([`SLOPE_GRIP_ALONG`], [`SLOPE_GRIP_ACROSS`]): on a
//! slope too steep, gravity, which the springs and the body bear, takes the hull back
//! down it, its tracks spinning.
//!
//! Each push is where it moves only what it is for. Along, at `p`, its part across
//! taken out: it doesn't turn the hull, even pressed harder on one track on a slope
//! across. Across, at `under`, a contact's arm without its part along: it holds what
//! slides the hull, not what turns it, so the hull turns only as its steering says,
//! as the game's do. Both are under the centre of mass, raised from the ground toward
//! it as the game's are, [`FRICTION_HEIGHT`]: the hull squats a little as it speeds up
//! and leans a little out of turns, and pushing another vehicle it doesn't rear up.
//!
//! A contact's side grip is [`TRACK_GRIP`] while it rolls, and [`SIDE_GRIP`], less,
//! once it slides across faster than [`SLIDE_SPEED`] as the step starts: a turning hull
//! holds its line, and a drifting one slides on. On its handbrake, its blocked tracks
//! slide along the ground rather than roll: the force range along is their sliding grip,
//! [`SLIDE_GRIP`] of how hard they press, and the grips aren't shared, its steering
//! swinging it as it slides.
//!
//! After the passes, each belt's speed is set: blocked, 0; gripping, if the grip didn't
//! cut the push along, the ground's speed under it, the middle of its contacts'; else
//! it spins, from the ground's speed if it touches it, toward what its drive asks of it
//! ([`Drive::track_goal`]), as hard as half the engine's torque, or of its brakes, can
//! spin [`TRACK_MASS`] of the vehicle's weight.
//!
//! A track's end meeting a step or a rock face head on climbs it, its belt gripping
//! the face: pushed up it, along the hull's heading, toward the belt's speed, within
//! [`CLIMB_GRIP`] of how hard it is held off it. Only held off, the hull would lose its
//! speed to every step its tracks' fronts meet, where the game's ride up.
//!
//! Its tracks lie on the ground under all of their contacts: their normal `n`, which
//! their springs push along too, is theirs together, so that a bump shorter than they
//! are lifts them but doesn't throw them back.
//!
//! A contact in the air, or barely touching, holds nothing: over a crest its front
//! tracks unload, and over a ledge the hull tips off it rather than being held on. With
//! no contact of its tracks on the ground, or tilted past [`TRACKS_UPRIGHT`], it is
//! `airborne`: its tracks push nothing and run free, and it keeps going, turning and
//! tumbling as it went, until it lands.
//!
//! Its speeds, `speed` and `slide`, are then read back from its body, at its origin:
//! rolling about its tracks, its centre of mass swings, but its tracks don't slide.
//! Off a ledge it falls; with its centre of mass past an edge, its last contacts on it
//! tip it over, nose first. On its side or its roof, its rigid contacts scrape on the
//! ground, held back by [`SCRAPE_FRICTION`] of how hard they press on it, and it slides
//! to a stop there. Where nothing is under it at all, off the terrain, its tracks drive
//! it as if they gripped without slipping.

use glam::{Mat3, Quat, Vec3};
use smallvec::SmallVec;

use wgtk::space::Cuboid;
use wgtk::space::statics::{BodyKind, StaticTriangle};
use wgtk::space::Space;

use super::contacts::{Contact, Shape, MAX_CONTACTS};
use super::collision::Solid;
use super::drive::{Drive, Side};
use super::{approach, Hull, Rubble, GRAVITY};


/// How many steps a tick's motion of a hull's body is integrated in: 10 ms each, short
/// enough for its suspension's springs.
pub const SUBSTEPS: usize = 10;
/// How fast a hull bounces on its suspension, in Hz, and how much of the damping that
/// would stop the bounce at once (critical) it has. Our choice, by feel: at 1.5 Hz the
/// hull sinks 11 cm into its springs at rest.
const SUSPENSION_FREQUENCY: f32 = 1.5;
const SUSPENSION_DAMPING: f32 = 0.7;
/// How much further than at rest a contact's spring gives, in metres, before it stops
/// hard, and a track's end from where it touches, on what faces up its hull: past it,
/// the contact sinks no deeper, the hull's body stopped there as if rigid. Our choice:
/// the game's client springs give about as much before they harden, half their length
/// (`hardRatio`), and less, 15 cm, felt stiff, small bumps throwing a light tank up.
pub const SUSPENSION_TRAVEL: f32 = 0.25;
/// How far, in metres, a contact's depth in the ground may change in a step of a hull's
/// body for its damper to take it as the contact sinking: more is the ground jumping
/// under it, off an edge or onto a step. 5 m/s.
const DEPTH_JUMP: f32 = 0.05;
/// How near the space, in metres, a hull's tracks' ends and boxes touch it already, at a negative
/// depth: they close no more than the gap in a step, so they stop on what they meet
/// rather than in it. Our choice.
pub const CONTACT_MARGIN: f32 = 0.05;
/// How far below its centre of mass the space is looked for, in metres, when none of
/// its contacts touches anything: none at all, it is off the terrain.
const GROUND_SEARCH: f32 = 1000.0;
/// The most of a hull's contacts touching the space in a step that stay on the stack.
const MAX_TOUCHES: usize = 64;
/// How a contact past its travel is pushed back out: this share of how far past it is,
/// a step, no faster than this, in m/s. A contact is past it on landing, or meeting a
/// step: pushed out at once, the hull would be thrown off it. Its ground's rise under it
/// counts no faster either. Our choice.
pub const STOP_RECOVERY: f32 = 0.2;
pub const STOP_SPEED: f32 = 2.0;
/// How many times its tracks and its contacts touching the ground are each held in a
/// step, those held moving the others, see `Hull::solve_pass`: a hull landing on its nose
/// stops on its front contacts, and turns onto its back ones.
pub const CONTACT_PASSES: usize = 4;
/// How high its centre of mass is, as a share of its collision box's height. Our choice:
/// a hull's mass is low, its turret's higher.
const CENTER_OF_MASS_HEIGHT: f32 = 0.35;
/// How much each of its tracks weighs, with what turns with it, its sprocket and its
/// wheels, as a share of the vehicle's weight: how fast its engine spins it off the
/// ground, and how hard landing pulls it to the ground's speed. Our choice.
const TRACK_MASS: f32 = 0.05;
/// How upright a hull must be for its tracks to grip, as its up axis' height: past 60°
/// of tilt, on its side or its roof, they hold nothing. Our choice.
const TRACKS_UPRIGHT: f32 = 0.5;
/// How much its tracks hold the ground along them where they touch it, rolling, as a
/// share of how hard each contact presses on it: what passes its engine's and brakes'
/// push on to the hull. Fitted loosely in part 2a, when its tracks pushed it toward the
/// drive model's motion: the drive model asks up to 1.4 g of them, braking at 14 m/s².
const TRACK_GRIP: f32 = 2.5;
/// How much its tracks hold the ground across them sliding on it, as a share of how hard
/// each contact presses on it: 11 m/s². Fitted, as `SIDE_FRICTION` was before its tracks
/// were driven: on the Type 64's two drifts at 20 and 16.7 m/s, its slide across its
/// tracks slowed by 9.5 to 12 m/s². Not sliding, they hold it as much as along them,
/// [`TRACK_GRIP`]: turning at 20 m/s, the Type 64 holds 14.6 m/s² across.
const SIDE_GRIP: f32 = 11.0 / GRAVITY;
/// How fast a contact of its tracks must slide across them, in m/s, as a step starts, to
/// be held by [`SIDE_GRIP`] only. Our choice.
const SLIDE_SPEED: f32 = 0.5;
/// How much a blocked track holds the ground along it, sliding on it, as a share of how
/// hard each contact presses on it: 3 m/s². Fitted, as `HANDBRAKE_FRICTION` was: on the
/// same drifts, its slide along its tracks slowed by 2 to 3.5 m/s².
const SLIDE_GRIP: f32 = 3.0 / GRAVITY;
/// How its tracks grip the terrain the steeper it is, along them and across: fully up to
/// the first angle, in degrees, least, the share given, from the second, in between as the
/// cosines go. From the game: `slopeGripLngTerrain` and `slopeGripSdwTerrain`, the
/// defaults of `physics_shared.py`, which the server's vehicle sections may override.
/// Each contact's slope is the ground's under it, along its track and across it.
const SLOPE_GRIP_ALONG: (f32, f32, f32) = (27.5, 32.0, 0.1);
const SLOPE_GRIP_ACROSS: (f32, f32, f32) = (24.5, 29.0, 0.1);
/// How much a contact sliding on the ground is held back, as a share of how hard it
/// presses on it: a hull on its side or its roof scrapes to a stop. Our choice.
const SCRAPE_FRICTION: f32 = 0.6;
/// How high its tracks' grip acts, pushing it along and holding it across, as a share of
/// how high its centre of mass is over the ground: not at the ground, which would rear a
/// hull pushing another, its nose up, and lean it out of turns as much as they hold.
/// From the game: `comFrictionYOffs` of the tanks' defaults in `physics_shared.py`, read
/// as this share. At the ground, two tanks driving into each other reared up to 42°, and
/// one pushing a parked one tipped both, pushing high on each other.
const FRICTION_HEIGHT: f32 = 0.7;
/// How steep, in radians, what a track's end touches may be for it to take it as flat,
/// and be lifted up it rather than held back: 35°. From the game:
/// `ROLLER_HORIZONTAL_SURFACE_ANGLE` in `physics_shared.py`, which its native physics
/// takes; that it means this is our reading. A debris pile's ramp on Ruinberg, its foot
/// 26 to 32° steep, stopped the T110E4 dead from 8 m/s, where the real one climbed it.
const ROLLER_FLAT: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// How much a track's end climbs what it meets head on, as a share of how hard it is
/// held off it: its belt, gripping the face, pulls it up. From the game:
/// `ROLLER_FRICTION_GAIN_MAX` in `physics_shared.py`, read as this share. At 1, a Type 64
/// meeting a house's step on Live Oaks at 15 m/s rolled over; at 0.25, the T110E4 climbs
/// the Ruinberg debris it lost a third of its speed to.
const CLIMB_GRIP: f32 = 0.25;

impl Hull {

    /// Give its body the speeds and turning on the ground plane its hull has, which hits
    /// and walls change between its ticks: the start of a tick. Its origin's velocity on
    /// the ground plane becomes `speed` and `slide`, its spin about the vertical
    /// `yaw_rate`; its vertical velocity and its roll and pitch spin are its body's own.
    pub fn speeds_to_body(&mut self) {
        self.angular_velocity.y = self.yaw_rate;
        // Its centre of mass moves as its origin does, less what turning adds there.
        let turning = self.origin_velocity() - self.linear_velocity;
        let velocity = self.velocity() - turning;
        self.linear_velocity.x = velocity.x;
        self.linear_velocity.z = velocity.z;
    }

    /// What holds of its body over a tick of `dt` seconds, its tracks pushed by `drive`,
    /// see [`Tick`].
    pub fn tick(&self, dt: f32, drive: Drive) -> Tick {
        Tick {
            mass: self.mobility.weight,
            center: self.center_of_mass(),
            inertia: self.inertia(),
            suspension: Suspension::of(self.mobility.weight, &self.contacts),
            drive,
            step: dt / SUBSTEPS as f32,
        }
    }

    /// Begin a step of its body, one of [`SUBSTEPS`] in a tick, see `World::step`: where
    /// it touches the space and the `others`' bodies, but its `own` among them, its weight
    /// and its springs accelerating it, and how its contacts are to be held, see [`Step`],
    /// then [`Self::solve_pass`] and [`Self::end_step`]. Its body is under its weight at
    /// its centre of mass and its suspension at its contacts, see the `contacts` module: a
    /// spring and a damper under each road wheel, pushing it up from what its ray down
    /// finds, as far as it sinks. Its velocity and its spin carry on from a step to the
    /// next, changed only by those forces and their torques, against its mass and its
    /// inertia (with the gyroscopic term, `ω × Iω`), and by its tracks' grip on the
    /// ground, see [`Self::solve_pass`].
    ///
    /// ```text
    ///   each spring, its ray from h above its contact, hitting at distance r, at p:
    ///     depth  = h + SAG − r                 > 0: it touches, and pushes
    ///     push   = k · depth + d · (how fast depth grows),   along n, never pulling
    ///   force    = Σ push − m · g              torque = Σ (p − centre of mass) × push
    ///   n        the normal of what its springs touch, all of theirs together
    ///   v += force / m · h                     ω += I⁻¹ (torque − ω × Iω) · h
    /// ```
    ///
    /// The springs share its weight, so at rest each sinks `SAG = g / ω²` and its origin
    /// is on the ground; `ω` is [`SUSPENSION_FREQUENCY`]'s. A spring gives
    /// [`SUSPENSION_TRAVEL`] more at most: past it, its contact is stopped, see
    /// [`Self::solve_pass`]. Its tracks' ends and its boxes are held off what they touch,
    /// the terrain and the static models alike, its tracks' ends giving as its springs
    /// do, up its hull: a step met by its tracks' front lifts its nose, and it climbs it.
    ///
    /// Upright, on the ground, its tracks grip; with its tracks off the ground, or on its
    /// side or its roof ([`TRACKS_UPRIGHT`]), it is `airborne`, its tracks run free, and
    /// it keeps going, turning and tumbling as it went, until it lands. Where no contact
    /// finds any ground at all, off the terrain, nothing holds it up and it doesn't fall
    /// either: its tracks drive it as if they gripped, see [`Self::drive_unheld`]. What it
    /// touches is the space's, but for the destructibles: only those standing that
    /// block it, see `Rubble::blocks`. After a tick's steps, its speeds are its body's,
    /// read back, see [`Self::speeds_from_body`].
    pub fn begin_step(
        &mut self,
        tick: &Tick,
        space: &Space,
        keep: &impl Fn(&StaticTriangle) -> bool,
        others: &[Solid],
        own: Option<usize>,
    ) -> Step {

        // 1. Where its centre of mass is as the step starts.
        let rotation = self.orientation;
        let mass_center = self.position + rotation * tick.center;

        // 2. Its weight, and where its contacts touch the space and the others: its
        //    springs' and its tracks' ends' pushes, and how they and its boxes are held
        //    off it.
        let loads = self.loads(space, keep, &tick.suspension, mass_center, tick.step, others, own);
        let world_inertia = Mat3::from_quat(rotation) * Mat3::from_diagonal(tick.inertia) * Mat3::from_quat(rotation.inverse());
        let inverse_inertia = world_inertia.inverse();
        let body = Mass {
            mass: tick.mass,
            inverse_inertia,
        };

        // Off the terrain, nothing under it at all: held as its tracks drive it,
        // neither falling nor tilting.
        let unheld = loads.touches.is_empty()
            && space.ray(mass_center, Vec3::NEG_Y, GROUND_SEARCH, keep).is_none();
        let gripping = loads.tracks_touching && (rotation * Vec3::Y).y > TRACKS_UPRIGHT;
        let traction = self.traction(&tick.drive, gripping, loads.tracks_normal);
        let mut step = Step {
            mass_center,
            rotation,
            body,
            unheld,
            axes: ground_axes(traction.heading, traction.normal),
            traction,
            driving: false,
            push: (0.0, 0.0, 0.0),
            stops: SmallVec::from_elem(0.0, loads.touches.len()),
            scrapes: SmallVec::from_elem(Vec3::ZERO, loads.touches.len()),
            holds: SmallVec::from_elem(0.0, loads.touches.len()),
            climbs: SmallVec::from_elem(0.0, loads.touches.len()),
            touches: loads.touches,
            pushed: Pushed::default(),
            push_velocity: Vec3::ZERO,
            push_spin: Vec3::ZERO,
            reactions: loads.reactions,
        };

        if unheld {
            self.drive_unheld(&tick.drive, mass_center, tick.step);
            self.airborne = false;
            self.touching = true;
            return step;
        }

        // 3. Gravity and its springs accelerate it.
        let gyroscopic = self.angular_velocity.cross(world_inertia * self.angular_velocity);
        self.linear_velocity += loads.force / tick.mass * tick.step;
        self.angular_velocity += inverse_inertia * (loads.torque - gyroscopic) * tick.step;
        self.airborne = !gripping;
        self.touching = !step.touches.is_empty();

        // 4. The ground's rolling friction holds back what its tracks press on it; then
        //    how its engine and brakes push it, at the speed it goes as the step starts,
        //    its tracks blocked sliding as hard as they are pressed. Upright only: on its
        //    side or its roof it scrapes, its tracks free.
        step.driving = gripping && step.touches.iter().any(|touch| touch.side.is_some());
        if step.driving {
            for touch in step.touches.iter().filter(|touch| touch.side.is_some() && touch.load > 0.0) {
                self.roll_on(touch, traction.heading, tick.drive.rolling, &body);
            }
            let (along, across) = step.axes;
            let (point, load, _) = track_point(&step.touches, &step.stops, (along, across));
            let speed = self.velocity_at(point).dot(along);
            let sliding = SLIDE_GRIP * grip(along.y.asin(), SLOPE_GRIP_ALONG) * load / tick.step;
            step.push = tick.drive.along(speed, sliding);
        }

        step

    }

    /// End a step of its body: its belts' speeds set, see [`Self::spin_tracks`], it moves
    /// at its velocity, and by what pushed its contacts out.
    pub fn end_step(&mut self, tick: &Tick, step: &Step) {
        if !step.unheld {
            self.spin_tracks(&step.touches, &step.traction, &step.pushed, tick.step);
        }
        let mass_center = step.mass_center + (self.linear_velocity + step.push_velocity) * tick.step;
        self.orientation = (Quat::from_scaled_axis((self.angular_velocity + step.push_spin) * tick.step) * step.rotation).normalize();
        self.position = mass_center - self.orientation * tick.center;
    }

    /// How fast the point at `arm` from its centre of mass moves, in m/s.
    pub fn velocity_at(&self, arm: Vec3) -> Vec3 {
        self.linear_velocity + self.angular_velocity.cross(arm)
    }

    /// Take an impulse, in kg·m/s, at `arm` from its centre of mass: its velocity and its
    /// spin change, as its `body` says.
    pub fn take(&mut self, impulse: Vec3, arm: Vec3, body: &Mass) {
        self.linear_velocity += impulse / body.mass;
        self.angular_velocity += body.inverse_inertia * arm.cross(impulse);
    }

    /// The forces on its body in a step, and where its contacts touch the space, see
    /// [`Loads`]: its weight, at its centre of mass; its springs' rays down to what
    /// `keep` keeps of the space, or to the `others`' boxes, but its `own`, and the
    /// pushes of those in it, those on the others pushing them back; and its tracks'
    /// ends' and its boxes' contacts with the space, its tracks' ends pushing as springs
    /// as far as they give. Each spring's depth is kept for the next step, its damper
    /// resisting how fast it sinks.
    fn loads(
        &mut self,
        space: &Space,
        keep: &impl Fn(&StaticTriangle) -> bool,
        suspension: &Suspension,
        mass_center: Vec3,
        step: f32,
        others: &[Solid],
        own: Option<usize>,
    ) -> Loads {

        let mut loads = Loads {
            force: Vec3::NEG_Y * GRAVITY * self.mobility.weight,
            torque: Vec3::ZERO,
            tracks_touching: false,
            tracks_normal: Vec3::Y,
            touches: SmallVec::new(),
            reactions: SmallVec::new(),
        };
        let heading = self.orientation * Vec3::Z;
        let down = self.orientation * Vec3::NEG_Y;
        let motion = (self.linear_velocity, self.angular_velocity);

        // 1. Its springs' rays, down its hull from each road wheel's centre, to what is
        //    under it, no further than its spring would reach: the space, or another
        //    vehicle, whichever is nearer.
        let hits = self.contacts.iter()
            .map(|contact| match contact.shape {
                Shape::Spring { point, height } => {
                    let from = self.position + self.orientation * (point + Vec3::Y * height);
                    let length = height + suspension.sag;
                    let ground = space.ray(from, down, length, keep).map(|hit| Ground {
                        point: hit.point,
                        normal: hit.normal,
                        distance: hit.distance,
                        on: None,
                    });
                    let vehicle = others.iter()
                        .enumerate()
                        .filter(|&(index, _)| Some(index) != own)
                        .filter_map(|(index, solid)| solid.ray(from, down, length).map(|(distance, normal)| Ground {
                            point: from + down * distance,
                            normal,
                            distance,
                            on: Some(index),
                        }))
                        .min_by(|a, b| a.distance.total_cmp(&b.distance));
                    let nearest = [ground, vehicle].into_iter()
                        .flatten()
                        .min_by(|a, b| a.distance.total_cmp(&b.distance));
                    nearest.map(|ground| (ground, height))
                }
                _ => None,
            })
            .collect::<SmallVec<[_; MAX_CONTACTS]>>();

        // Its tracks lie on the ground under all of them, its slope theirs: a bump shorter
        // than they are lifts them, but doesn't throw them back.
        loads.tracks_normal = hits.iter()
            .flatten()
            .map(|(hit, _)| hit.normal)
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let normal = loads.tracks_normal;

        for ((contact, previous), hit) in self.contacts.iter().zip(&mut self.depths).zip(&hits) {

            let Some((hit, height)) = hit else {
                *previous = f32::NAN;
                continue;
            };

            // 2. How deep it is, sinking into the ground at rest, and how much deeper than
            //    in the last step.
            let depth = height + suspension.sag - hit.distance;
            let sank = depth - *previous;
            *previous = depth;
            if depth <= 0.0 {
                continue;
            }

            // How fast it sinks: into the ground as it follows it, not down; but off an
            // edge the ground jumps, and only the contact's own speed counts.
            let arm = hit.point - mass_center;
            let velocity = self.linear_velocity + self.angular_velocity.cross(arm);
            let sinking = if sank.abs() <= DEPTH_JUMP { sank / step } else { -velocity.dot(normal) };

            // 3. Past its travel, it is stopped: it sinks no further, rising as fast as the
            //    ground rises under it, no faster than STOP_SPEED. Up, not along the
            //    ground's normal: meeting a rise, it climbs it rather than being thrown
            //    back. The ground rises as its slope under the contact and the contact's
            //    speed across it say: from how its depth changed instead, a hull rocking
            //    over the terrain's triangles was kicked up, and never rested. On another
            //    vehicle, its boxes stop it, see the `collision` module.
            let past = depth - suspension.stop;
            let slope = hit.normal;
            let rising = -(slope.x * velocity.x + slope.z * velocity.z) / slope.y.max(0.1);
            let hold = if past > 0.0 && hit.on.is_none() {
                Hold::Up {
                    rising: rising.min(STOP_SPEED),
                    past,
                }
            } else {
                Hold::Free
            };

            // 4. It pushes, as far as it sank and as fast as it sinks, never pulling, out
            //    of the ground: on a slope, gravity takes the hull down it unless its
            //    tracks hold it. On another vehicle, it pushes it back as much.
            let push = (suspension.stiffness * depth.min(suspension.stop) + suspension.damping * sinking).max(0.0);
            loads.force += normal * push;
            loads.torque += arm.cross(normal * push);
            loads.tracks_touching = true;
            loads.touches.push(Touch {
                arm,
                normal,
                side: contact.side,
                load: push * step,
                hold,
                sliding: sliding(motion, arm, heading, normal),
            });
            if let Some(other) = hit.on {
                loads.reactions.push(Reaction {
                    other,
                    impulse: -normal * push * step,
                    point: hit.point,
                });
            }

        }

        // 5. Its tracks' ends and its boxes: held off what they touch of the space along
        //    its normal, its tracks' gripping it too. Other vehicles are the world's to
        //    hold them off, see the `collision` module.
        for contact in &self.contacts {
            let touched = match contact.shape {
                Shape::Spring { .. } => continue,
                Shape::Sphere { center, radius } => {
                    space.sphere(self.position + self.orientation * center, radius, CONTACT_MARGIN, keep)
                }
                Shape::Box { center, half } => {
                    let cuboid = Cuboid {
                        center: self.position + self.orientation * center,
                        rotation: self.orientation,
                        half,
                    };
                    space.cuboid(&cuboid, CONTACT_MARGIN, keep)
                }
            };
            let deepest = touched.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.depth.total_cmp(&b.depth))
                .map(|(index, _)| index);
            for (index, touch) in touched.iter().enumerate() {

                // A track's end on what is flat enough takes it as flat, as the game's
                // rollers do: it is lifted up it, not held back, as deep as it is in it
                // straight down.
                let arm = touch.point - mass_center;
                let (normal, depth) = match contact.shape {
                    Shape::Sphere { .. } if touch.normal.y >= ROLLER_FLAT.cos() => (Vec3::Y, touch.depth / touch.normal.y),
                    _ => (touch.normal, touch.depth),
                };

                // It gives as its road wheels do, up its hull: as much of its travel as
                // what it touches faces up its hull, none against a wall or with the hull
                // on its side. Its deepest touch pushes as a spring, as far as it gives;
                // each is stopped past it. Its boxes don't give.
                let travel = match contact.shape {
                    Shape::Sphere { .. } => SUSPENSION_TRAVEL * normal.dot(-down).max(0.0),
                    _ => 0.0,
                };
                let mut load = 0.0;
                if Some(index) == deepest && depth > 0.0 && travel > 0.0 {
                    let velocity = self.linear_velocity + self.angular_velocity.cross(arm);
                    let sinking = -velocity.dot(normal);
                    let push = (suspension.stiffness * depth.min(travel) + suspension.damping * sinking).max(0.0);
                    loads.force += normal * push;
                    loads.torque += arm.cross(normal * push);
                    load = push * step;
                }

                loads.tracks_touching |= contact.side.is_some() && depth > 0.0;
                loads.touches.push(Touch {
                    arm,
                    normal,
                    side: contact.side,
                    load,
                    hold: Hold::Out {
                        depth: depth - travel,
                    },
                    sliding: sliding(motion, arm, heading, normal),
                });

            }
        }

        loads

    }

    /// How its tracks hold the ground in a step, see [`Traction`]: as `drive` pushes
    /// them, if `gripping` at all, on the ground's plane under them, its `normal`.
    fn traction(&self, drive: &Drive, gripping: bool, normal: Vec3) -> Traction {
        Traction {
            drive: *drive,
            gripping,
            heading: self.orientation * Vec3::Z,
            normal,
            track_mass: TRACK_MASS * self.mobility.weight,
        }
    }

    /// One pass of holding its tracks and its contacts touching the space in a step, see
    /// [`Self::begin_step`]: [`CONTACT_PASSES`] of them, those held moving the others, as
    /// impulses, each's sum over the passes bounded, not each pass's.
    ///
    /// At each contact, an impulse holding it off what it touches, up for a spring past
    /// its travel, along the normal for a track's end past its own or a box; one along
    /// what it touches; and where it is in it, one moving it out, split. Never a pull: a
    /// contact leaving goes. The first stops it; the last only moves it back out, as fast
    /// as [`STOP_RECOVERY`] says, for this step: pushed out, the hull keeps no speed from
    /// it, which would throw it up. That last is kept in the step, as a velocity and a
    /// spin to move by.
    ///
    /// Along the ground, a contact of its tracks, as they grip, holds the hull from
    /// sliding across them, see [`Self::hold_across`]; any other scrapes, held back to a
    /// stop by [`SCRAPE_FRICTION`] of how hard it is pressed. Its tracks together push it
    /// along and turn it first, see [`Self::drive_tracks`]; its belts are set as the step
    /// ends, see [`Self::spin_tracks`]: the order, the formulas and the grips are in "Its
    /// tracks", in the module's documentation.
    ///
    /// ```text
    ///   up      short   = rising − (v + ω × arm).y         > 0: it sinks too fast
    ///           impulse = short / (1/m + (arm × Y) · I⁻¹ (arm × Y))
    ///           rising  = min(how fast its ground rises under it, STOP_SPEED)
    ///   off     impulse = −(max(−d, 0) / h + (v + ω × arm) · n) / k(arm, n), its sum ≥ 0
    ///           d its depth past its give, negative short of it: it closes no more
    ///           than its gap
    ///   out     wanted  = min(STOP_RECOVERY · past / step, STOP_SPEED)
    /// ```
    pub fn solve_pass(&mut self, tick: &Tick, step: &mut Step, pass: usize) {

        let Step {
            body,
            touches,
            traction,
            axes,
            driving,
            push,
            stops,
            scrapes,
            holds,
            climbs,
            pushed,
            push_velocity,
            push_spin,
            ..
        } = step;
        let body = *body;
        let Traction { drive, gripping, heading, normal, .. } = *traction;
        let length = tick.step;

        // Its tracks push it along and turn it, as its drive says, first: what it
        // touches holds it after, so that nothing pushes it into a wall last.
        if *driving {
            self.drive_tracks(touches, stops, *push, &drive, *axes, normal, &body, length, pushed);
        }

        // Back and forth, so that neither of its sides is held first every time: what
        // is held last takes the rest, and it would lean that way.
        for index in 0..touches.len() {
            let index = if pass % 2 == 0 { index } else { touches.len() - 1 - index };
            let touch = &touches[index];

            let arm = touch.arm;
            let out = touch.hold.way(touch.normal);
            let lever = arm.cross(out);
            let effective = body.per_impulse(arm, out);

            // 1. Held off: a spring past its travel sinks no further, rising with its
            //    ground; a track's end or a box gets no deeper past what it gives,
            //    closing no more than its gap.
            let velocity = self.velocity_at(arm);
            let held = match touch.hold {
                Hold::Free => 0.0,
                Hold::Up { rising, .. } => stops[index] + (rising - velocity.y).max(0.0) / effective,
                Hold::Out { depth } => {
                    let closing = (-depth).max(0.0) / length;
                    (stops[index] - (closing + velocity.dot(out)) / effective).max(0.0)
                }
            };
            let impulse = held - stops[index];
            stops[index] = held;
            self.linear_velocity += out * (impulse / body.mass);
            self.angular_velocity += body.inverse_inertia * lever * impulse;
            let pressed = touch.load + stops[index];

            // 2. Along the ground, in its plane: its tracks hold it from sliding
            //    across, as much as their push along leaves them, else it scrapes.
            if touch.side.is_some() && gripping {
                let used = if drive.handbrake() { 0.0 } else { pushed.used() };
                let left = pressed * (1.0 - used * used).max(0.0).sqrt();
                self.hold_across(touch, *axes, &body, left, &mut holds[index]);
                if let Hold::Out { .. } = touch.hold {
                    self.climb(touch, heading, &body, stops[index], &mut climbs[index]);
                }
            } else {
                let (along, across) = ground_axes(heading, touch.normal);
                let velocity = self.velocity_at(arm);
                let mut impulse = Vec3::ZERO;
                for axis in [along, across] {
                    impulse -= axis * (velocity.dot(axis) / body.per_impulse(arm, axis));
                }
                let friction = (scrapes[index] + impulse).clamp_length_max(SCRAPE_FRICTION * pressed);
                let change = friction - scrapes[index];
                scrapes[index] = friction;
                self.take(change, arm, &body);
            }

            // 3. In what it touches: moved back out, without keeping the speed it took.
            let past = touch.hold.past();
            if past > 0.0 {
                let moved = (*push_velocity + push_spin.cross(arm)).dot(out);
                let short = (STOP_RECOVERY * past / length).min(STOP_SPEED) - moved;
                if short > 0.0 {
                    let impulse = short / effective;
                    *push_velocity += out * (impulse / body.mass);
                    *push_spin += body.inverse_inertia * lever * impulse;
                }
            }

        }

    }

    /// The ground's rolling friction at a contact of a track, for a step: against the
    /// hull's motion along the track there, as much as `rolling` of how hard the contact
    /// is pressed, never past stopping it.
    fn roll_on(&mut self, touch: &Touch, heading: Vec3, rolling: f32, body: &Mass) {
        let (along, _) = ground_axes(heading, touch.normal);
        let rolled = (self.linear_velocity + self.angular_velocity.cross(touch.arm)).dot(along);
        let stopping = rolled / body.per_impulse(touch.arm, along);
        let impulse = along * -stopping.clamp(-rolling * touch.load, rolling * touch.load);
        self.linear_velocity += impulse / body.mass;
        self.angular_velocity += body.inverse_inertia * touch.arm.cross(impulse);
    }

    /// One pass of a contact of a track holding the ground across it, on the ground's
    /// plane `axes`, along the track then across it, pressed on it with `pressed`, less
    /// what pushing along takes of its grip: where the track passes under the centre of
    /// mass, so that it holds what slides the hull, not what turns it, which its steering
    /// does. Its impulse so far is `so_far`. Rolling, it holds as much as its tracks grip
    /// along, [`TRACK_GRIP`]; sliding, [`SIDE_GRIP`]; both falling with the ground's slope
    /// across, as the game's slope grip says.
    fn hold_across(
        &mut self,
        touch: &Touch,
        (along, across): (Vec3, Vec3),
        body: &Mass,
        pressed: f32,
        so_far: &mut f32,
    ) {
        let under = raised(touch.arm - along * touch.arm.dot(along), along.cross(across));
        let slip = (self.linear_velocity + self.angular_velocity.cross(under)).dot(across);
        let side_grip = if touch.sliding { SIDE_GRIP } else { TRACK_GRIP };
        let limit = side_grip * grip(across.y.asin(), SLOPE_GRIP_ACROSS) * pressed;
        let held = (*so_far - slip / body.per_impulse(under, across)).clamp(-limit, limit);
        let impulse = across * (held - *so_far);
        *so_far = held;
        self.linear_velocity += impulse / body.mass;
        self.angular_velocity += body.inverse_inertia * under.cross(impulse);
    }

    /// One pass of a track's end climbing what it meets head on, its belt gripping it: up
    /// what it touches, along its `heading`, toward its belt's speed, within
    /// [`CLIMB_GRIP`] of how hard it is held off it, `held`. Its impulse so far is
    /// `so_far`. Without, a track's end meeting a step would only be held back off it.
    fn climb(&mut self, touch: &Touch, heading: Vec3, body: &Mass, held: f32, so_far: &mut f32) {
        let Some(side) = touch.side else { return };
        let up = (heading - touch.normal * heading.dot(touch.normal)).normalize_or_zero();
        if heading.dot(touch.normal) >= 0.0 || up.y <= 0.0 {
            return;
        }
        let belt = self.tracks[side.index()].max(0.0);
        let short = belt - self.velocity_at(touch.arm).dot(up);
        let limit = CLIMB_GRIP * held;
        let climbed = (*so_far + short / body.per_impulse(touch.arm, up)).clamp(-limit, limit);
        self.take(up * (climbed - *so_far), touch.arm, body);
        *so_far = climbed;
    }

    /// One pass of its tracks pushing it along and turning it, as its drive says: along
    /// its heading toward `target` within the force `low` to `high`, see [`Drive::along`],
    /// under its centre of mass where they touch the ground, so that it squats as it
    /// speeds up; and about the ground's `normal` toward its steering, as a torque, as the
    /// game's "gimlet" does. Their pushes so far are `pushed`.
    ///
    /// Both are bounded by their grip, [`TRACK_GRIP`] of how hard they are pressed,
    /// falling with the ground's slope along, as the game's slope grip says, which they
    /// share as a diamond: pushing as hard as they grip along, they turn nothing.
    /// Blocked, they slide, as hard as their drive says, and turn as their steering says.
    /// These are steps 4 and 5 of "Its tracks", in the module's documentation, with the
    /// formulas.
    fn drive_tracks(
        &mut self,
        touches: &[Touch],
        stops: &[f32],
        (target, low, high): (f32, f32, f32),
        drive: &Drive,
        (along, across): (Vec3, Vec3),
        normal: Vec3,
        body: &Mass,
        step: f32,
        pushed: &mut Pushed,
    ) {

        // How hard they are pressed, where, and how far from their middle.
        let (point, pressed, couple) = track_point(touches, stops, (along, across));
        if pressed <= 0.0 {
            return;
        }
        let grip_along = TRACK_GRIP * grip(along.y.asin(), SLOPE_GRIP_ALONG);
        pushed.grip = grip_along * pressed;
        pushed.couple = grip_along * couple;
        let handbrake = drive.handbrake();

        // 1. Along: toward its target, within its drive's force, and what of their grip
        //    turning it leaves.
        let speed = (self.linear_velocity + self.angular_velocity.cross(point)).dot(along);
        let wanted = pushed.along + (target - speed) / body.per_impulse(point, along);
        let driven = wanted.clamp(low * step, high * step);
        let room = if handbrake {
            f32::INFINITY
        } else {
            (pushed.grip * (1.0 - pushed.turned.abs() / pushed.couple.max(1e-6))).max(0.0)
        };
        let held = driven.clamp(-room, room);
        pushed.slipping |= held.abs() < driven.abs() * 0.999;
        let impulse = along * (held - pushed.along);
        pushed.along = held;
        self.linear_velocity += impulse / body.mass;
        self.angular_velocity += body.inverse_inertia * point.cross(impulse);

        // 2. Turning: toward its steering, within what of their grip pushing it along
        //    leaves.
        let turning = self.angular_velocity.dot(normal);
        let per_impulse = normal.dot(body.inverse_inertia * normal);
        let wanted = pushed.turned + (drive.steer - turning) / per_impulse;
        let room = if handbrake {
            pushed.couple
        } else {
            (pushed.couple * (1.0 - pushed.along.abs() / pushed.grip)).max(0.0)
        };
        let held = wanted.clamp(-room, room);
        let impulse = held - pushed.turned;
        pushed.turned = held;
        self.angular_velocity += body.inverse_inertia * normal * impulse;

    }

    /// The ground's speed under a track, along it, the middle of its contacts' touching
    /// it: none if none does.
    fn ground_speed(&self, touches: &[Touch], side: Side, heading: Vec3) -> Option<f32> {
        let (count, sum) = touches.iter()
            .filter(|touch| touch.side == Some(side))
            .fold((0, 0.0), |(count, sum), touch| {
                let (along, _) = ground_axes(heading, touch.normal);
                let velocity = self.linear_velocity + self.angular_velocity.cross(touch.arm);
                (count + 1, sum + velocity.dot(along))
            });
        if count > 0 { Some(sum / count as f32) } else { None }
    }

    /// Its tracks' speeds after a step: rolling at the ground's speed where they touch it
    /// and grip, blocked at rest, and where they slipped or touch nothing, spinning toward
    /// what their drive asks, as hard as their engine or their brakes do, against their
    /// own mass.
    fn spin_tracks(&mut self, touches: &[Touch], traction: &Traction, pushed: &Pushed, step: f32) {

        let Traction { drive, gripping, heading, track_mass, .. } = *traction;

        for side in Side::BOTH {
            let index = side.index();
            let ground = self.ground_speed(touches, side, heading).filter(|_| gripping);
            if drive.blocked[index] {
                self.tracks[index] = 0.0;
            } else if let Some(ground) = ground.filter(|_| !pushed.slipping) {
                self.tracks[index] = ground;
            } else {
                // Spinning, from the ground's speed if it touches it.
                let from = ground.unwrap_or(self.tracks[index]);
                let goal = drive.track_goal(side);
                let speeding = goal.abs() > from.abs() && goal * from >= 0.0;
                let force = if speeding { drive.torque } else { drive.braking } / 2.0;
                self.tracks[index] = approach(from, goal, force / track_mass * step);
            }
        }

    }

    /// Its tracks driving it with nothing under it, off the terrain, for a step: as if
    /// they gripped without slipping, on its ground's rolling friction, its body following
    /// them on the ground plane, turning as its steering says. Gripping so, they take its
    /// motion first: what hits and walls gave it.
    fn drive_unheld(&mut self, drive: &Drive, mass_center: Vec3, step: f32) {

        let mass = self.mobility.weight;
        let (heading, _) = self.axes();
        let origin = self.position - mass_center;
        let speed = (self.linear_velocity + self.angular_velocity.cross(origin)).dot(heading);
        let speed = approach(speed, 0.0, drive.rolling * GRAVITY * step);

        let (target, low, high) = drive.along(speed, SLIDE_GRIP * GRAVITY * mass);
        let speed = speed + ((target - speed) * mass).clamp(low * step, high * step) / mass;
        self.angular_velocity = Vec3::Y * drive.steer;
        self.linear_velocity = heading * speed - self.angular_velocity.cross(origin);
        self.tracks = Side::BOTH.map(|side| {
            if drive.blocked[side.index()] { 0.0 } else { speed - side.way() * drive.steer * drive.half_gauge }
        });

    }

    /// Read its speeds on the ground plane back from its body, at its origin, see
    /// [`Self::origin_velocity`]: the end of a tick.
    pub fn speeds_from_body(&mut self) {
        let (heading, right) = self.axes();
        let velocity = self.origin_velocity();
        self.speed = velocity.dot(heading);
        self.slide = velocity.dot(right);
        self.yaw_rate = self.angular_velocity.y;
    }

    /// How fast its origin moves, in m/s, on the ground under its tracks: its centre of
    /// mass's velocity, and how it turns about it. Its speeds are its origin's: rolling
    /// over, its centre of mass swings over its tracks, but they don't slide.
    fn origin_velocity(&self) -> Vec3 {
        let origin = -(self.orientation * self.center_of_mass());
        self.linear_velocity + self.angular_velocity.cross(origin)
    }

    /// Its centre of mass, in its own frame: over its origin, the middle of what its
    /// tracks carry it on, at [`CENTER_OF_MASS_HEIGHT`] of its collision box's height. Over
    /// its tracks' middle, it rests level on flat ground, as the real ones do, rather than
    /// tilted toward its box's middle.
    fn center_of_mass(&self) -> Vec3 {
        let [min, max] = self.mobility.bounds;
        Vec3::new(0.0, min.y + (max.y - min.y) * CENTER_OF_MASS_HEIGHT, 0.0)
    }

    /// Its inertia about its centre of mass, along its own axes (right, up, heading), in
    /// kg·m²: a box's of its weight over its collision box, `m (b² + c²) / 12` for each
    /// axis and the box's two other sides.
    fn inertia(&self) -> Vec3 {
        let [min, max] = self.mobility.bounds;
        let size = max - min;
        let share = self.mobility.weight / 12.0;
        Vec3::new(
            share * (size.y * size.y + size.z * size.z),
            share * (size.x * size.x + size.z * size.z),
            share * (size.x * size.x + size.y * size.y),
        )
    }

}

/// What of a space stops a vehicle: all but what vehicles drive through, and the
/// destructibles down, see `Rubble::blocks`.
pub fn blocking<'a>(space: &'a Space, rubble: &'a Rubble) -> impl Fn(&StaticTriangle) -> bool + 'a {
    |triangle: &StaticTriangle| {
        !triangle.vehicles_pass && (triangle.kind != BodyKind::Destructible || rubble.blocks(&space.statics, triangle))
    }
}

/// What holds of a hull's body over a tick, see [`Hull::tick`]: its mass, its centre
/// of mass and its inertia in its own frame, its springs, how its drive pushes its
/// tracks, and how long each of its steps is, in seconds.
#[derive(Debug, Clone, Copy)]
pub struct Tick {
    mass: f32,
    center: Vec3,
    inertia: Vec3,
    suspension: Suspension,
    drive: Drive,
    pub step: f32,
}

/// A step of a hull's body under way, from [`Hull::begin_step`] to [`Hull::end_step`]:
/// where its centre of mass and its orientation were as it began, its mass, whether
/// nothing at all is under it, its contacts touching the space, how its tracks hold the
/// ground and push it, the impulses held at each contact so far, what moves it out of
/// what it is in, and its springs' pushes on other vehicles, for the world to give them.
#[derive(Debug)]
pub struct Step {
    pub mass_center: Vec3,
    rotation: Quat,
    pub body: Mass,
    unheld: bool,
    touches: SmallVec<[Touch; MAX_TOUCHES]>,
    traction: Traction,
    axes: (Vec3, Vec3),
    driving: bool,
    push: (f32, f32, f32),
    stops: SmallVec<[f32; MAX_TOUCHES]>,
    scrapes: SmallVec<[Vec3; MAX_TOUCHES]>,
    holds: SmallVec<[f32; MAX_TOUCHES]>,
    climbs: SmallVec<[f32; MAX_TOUCHES]>,
    pushed: Pushed,
    pub push_velocity: Vec3,
    pub push_spin: Vec3,
    pub reactions: SmallVec<[Reaction; 8]>,
}

/// A spring's push on another vehicle in a step, which pushes it back: the other, by
/// its place among the bodies the step was given, the impulse, and where.
#[derive(Debug, Clone, Copy)]
pub struct Reaction {
    pub other: usize,
    pub impulse: Vec3,
    pub point: Vec3,
}

/// What a spring's ray hits: the space, or another vehicle, by its place among the
/// bodies its step was given.
#[derive(Debug, Clone, Copy)]
struct Ground {
    point: Vec3,
    normal: Vec3,
    distance: f32,
    on: Option<usize>,
}

/// A hull's springs, for a tick: shared by its springs, its weight sinks each `sag` at
/// rest, and one stops `stop` deep, past its travel.
#[derive(Debug, Clone, Copy)]
struct Suspension {
    stiffness: f32,
    damping: f32,
    sag: f32,
    stop: f32,
}

impl Suspension {

    /// The springs of a hull of this mass, on these contacts, at [`SUSPENSION_FREQUENCY`]
    /// and [`SUSPENSION_DAMPING`].
    fn of(mass: f32, contacts: &[Contact]) -> Self {
        let springs = contacts.iter().filter(|contact| matches!(contact.shape, Shape::Spring { .. })).count().max(1) as f32;
        let omega = std::f32::consts::TAU * SUSPENSION_FREQUENCY;
        let sag = GRAVITY / (omega * omega);
        Self {
            stiffness: mass * omega * omega / springs,
            damping: 2.0 * SUSPENSION_DAMPING * mass * omega / springs,
            sag,
            stop: sag + SUSPENSION_TRAVEL,
        }
    }

}

/// The forces on a hull's body in a step, its weight and its springs' pushes, their
/// torque about its centre of mass, and its contacts touching the space: whether any of
/// its tracks' does, and the ground's normal under its springs, all of theirs together.
/// And its springs' pushes on other vehicles.
#[derive(Debug)]
struct Loads {
    force: Vec3,
    torque: Vec3,
    tracks_touching: bool,
    tracks_normal: Vec3,
    touches: SmallVec<[Touch; MAX_TOUCHES]>,
    reactions: SmallVec<[Reaction; 8]>,
}

/// A [`Contact`] touching the space in a step, see [`Hull::solve_pass`]: where it is from the
/// hull's centre of mass, the normal of what it touches, its springs' together for a
/// spring, which track it is of, if any, how hard its spring pushed it out in the step,
/// as an impulse, how it is held off what it touches, and whether it slides across its
/// track as the step starts.
#[derive(Debug, Clone, Copy)]
struct Touch {
    arm: Vec3,
    normal: Vec3,
    side: Option<Side>,
    load: f32,
    hold: Hold,
    sliding: bool,
}

/// How a [`Touch`] is held off what it touches.
#[derive(Debug, Clone, Copy)]
enum Hold {
    /// A spring within its travel: its push is all.
    Free,
    /// A spring past its travel, this far: it sinks no further, rising as fast as its
    /// ground rises under it, up.
    Up {
        rising: f32,
        past: f32,
    },
    /// A track's end or a box, this deep in what it touches past what it gives, negative
    /// short of it: it gets no deeper, along its normal.
    Out {
        depth: f32,
    },
}

impl Hold {

    /// The way it is held off, for a touch of this normal.
    fn way(self, normal: Vec3) -> Vec3 {
        match self {
            Self::Out { .. } => normal,
            _ => Vec3::Y,
        }
    }

    /// How far into what it touches it is, to be moved back out of.
    fn past(self) -> f32 {
        match self {
            Self::Free => 0.0,
            Self::Up { past, .. } => past,
            Self::Out { depth } => depth,
        }
    }

}

/// How a hull's tracks hold the ground in a step, see [`Hull::solve_pass`]: how its drive
/// pushes them, whether they grip at all, the hull's heading, which they run along, the
/// ground's normal under them, and each track's mass.
#[derive(Debug, Clone, Copy)]
struct Traction {
    drive: Drive,
    gripping: bool,
    heading: Vec3,
    normal: Vec3,
    track_mass: f32,
}

/// Its tracks' pushes on the ground in a step, see [`Hull::drive_tracks`]: along its
/// heading and turning it, as impulses so far, how much they grip along and about their
/// middle over the step, and whether they slipped, their grip short of their drive.
#[derive(Debug, Clone, Copy, Default)]
struct Pushed {
    along: f32,
    turned: f32,
    grip: f32,
    couple: f32,
    slipping: bool,
}

impl Pushed {

    /// The share of their grip their pushes take, from 0 to 1.
    fn used(&self) -> f32 {
        if self.grip <= 0.0 || self.couple <= 0.0 {
            return 0.0;
        }
        (self.along.abs() / self.grip + self.turned.abs() / self.couple).min(1.0)
    }

}

/// A hull's body against impulses, in a step: its mass, and its inertia's inverse in the
/// world.
#[derive(Debug, Clone, Copy)]
pub struct Mass {
    pub mass: f32,
    pub inverse_inertia: Mat3,
}

impl Mass {

    /// How much an impulse along `axis` at `arm` from its centre of mass changes the
    /// velocity there along it, per unit: the inverse of its effective mass there.
    pub fn per_impulse(&self, arm: Vec3, axis: Vec3) -> f32 {
        let lever = arm.cross(axis);
        1.0 / self.mass + lever.dot(self.inverse_inertia * lever)
    }

}

/// Whether a hull's contact at `arm` from its centre of mass slides across its tracks,
/// running `heading` on a ground of this `normal`, faster than [`SLIDE_SPEED`], the
/// hull's body moving at its velocity and spin: where they pass under its centre of mass.
fn sliding((velocity, spin): (Vec3, Vec3), arm: Vec3, heading: Vec3, normal: Vec3) -> bool {
    let (along, across) = ground_axes(heading, normal);
    let under = arm - along * arm.dot(along);
    (velocity + spin.cross(under)).dot(across).abs() > SLIDE_SPEED
}

/// Where a hull's tracks push it, from its centre of mass, with its contacts `stops` so
/// far, on the ground's plane `along` and `across`: under it, the middle of their
/// contacts as hard as each is pressed, its offset across taken out, so that pushing
/// there doesn't turn it, raised toward it, see [`raised`]; how hard they are pressed,
/// as an impulse; and that each times how far it is from their middle.
fn track_point(touches: &[Touch], stops: &[f32], (along, across): (Vec3, Vec3)) -> (Vec3, f32, f32) {
    let mut point = Vec3::ZERO;
    let mut pressed = 0.0;
    let mut couple = 0.0;
    for (touch, stop) in touches.iter().zip(stops).filter(|(touch, _)| touch.side.is_some()) {
        let load = touch.load + stop;
        point += touch.arm * load;
        pressed += load;
        couple += touch.arm.dot(across).abs() * load;
    }
    if pressed <= 0.0 {
        return (Vec3::ZERO, 0.0, 0.0);
    }
    let point = point / pressed;
    (raised(point - across * point.dot(across), along.cross(across)), pressed, couple)
}

/// A point under a hull's centre of mass, `arm` from it, raised toward it along the
/// ground's `normal` as its tracks' grip acts, see [`FRICTION_HEIGHT`].
fn raised(arm: Vec3, normal: Vec3) -> Vec3 {
    arm - normal * (arm.dot(normal) * FRICTION_HEIGHT)
}

/// The axes of the ground's plane, its `normal`'s, along a track running `heading` and
/// across it, to its right.
fn ground_axes(heading: Vec3, normal: Vec3) -> (Vec3, Vec3) {
    let along = (heading - normal * heading.dot(normal)).normalize_or(heading);
    (along, normal.cross(along))
}

/// How much the tracks grip on a slope of this angle, in radians, by one of the curves
/// [`SLOPE_GRIP_ALONG`] and [`SLOPE_GRIP_ACROSS`].
fn grip(angle: f32, (full, least, least_grip): (f32, f32, f32)) -> f32 {
    let (full, least) = (full.to_radians().cos(), least.to_radians().cos());
    let cos = angle.cos();
    if cos >= full {
        1.0
    } else if cos <= least {
        least_grip
    } else {
        least_grip + (1.0 - least_grip) * (cos - least) / (full - least)
    }
}


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use wgtk::space::statics::Statics;
    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, ground_under, quad};
    use super::super::{movement_flag, Rubble};
    use super::*;

    #[test]
    fn drives_onto_decks_within_a_step() {
        // A step up, 0.5 m, its tracks' front meets it, and it climbs onto it, nose first,
        // and rests on it.
        let step = 0.5;
        let space = ground_under(Some(step), 0.0);
        let mut hull = chieftain();
        hull.position.z = -6.0;
        hull.flags = movement_flag::FORWARD;
        let mut steepest: f32 = 0.0;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            steepest = steepest.min(hull.direction().y);
        }
        assert!(hull.position.z > 10.0 && (hull.position.y - step).abs() < 0.05, "{}", hull.position);
        assert!(steepest < -3f32.to_radians(), "{}°", steepest.to_degrees());
        // Higher, a bridge it drives under, it stays below.
        let space = ground_under(Some(3.8), 0.0);
        let mut hull = chieftain();
        hull.position.z = -6.0;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.position.z > 10.0 && hull.position.y.abs() < 0.01, "{}", hull.position);
    }

    #[test]
    fn meets_steps_with_its_tracks_front() {
        // Driven at a step 0.5 m high, its track's front, rising to its idler, meets it
        // before its front road wheel does, and lifts its nose.
        let mut triangles = Vec::new();
        quad(&mut triangles, -50.0, 100.0, 0.0, 0.0);
        quad(&mut triangles, 4.0, 60.0, 0.5, 0.5);
        let space = Space {
            statics: Statics::from_triangles(triangles),
            ..Space::default()
        };
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        let mut pitch_before_wheels = None;
        for _ in 0..60 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            if hull.position.z > 1.3 && pitch_before_wheels.is_none() {
                pitch_before_wheels = Some(hull.direction().y);
            }
        }
        let pitch = pitch_before_wheels.expect("got to the step");
        assert!(pitch < -1f32.to_radians(), "{}°", pitch.to_degrees());
        // And climbs onto it.
        assert!(hull.position.z > 10.0 && (hull.position.y - 0.5).abs() < 0.05, "{}", hull.position);
    }

    #[test]
    fn lands_on_its_roof_or_its_side() {
        let mut triangles = Vec::new();
        quad(&mut triangles, -50.0, 50.0, 0.0, 0.0);
        let space = Space {
            statics: Statics::from_triangles(triangles),
            ..Space::default()
        };
        // Dropped on its roof from 2 m, spinning, it lands on its turret and its hull's
        // top, and scrapes to a stop there, never in the ground, its tracks up and
        // holding nothing.
        let mut hull = chieftain();
        hull.position.y = 5.0;
        hull.set_direction(Vec3::new(std::f32::consts::PI, 0.0, 0.0));
        hull.slide = 3.0;
        hull.yaw_rate = 0.5;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..60 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            let deepest = -lowest_contact(&hull);
            assert!(deepest < 0.15, "a contact {deepest} m deep at {}", hull.position);
        }
        assert!(hull.airborne && (hull.orientation * Vec3::Y).y < -0.9, "{}", hull.direction());
        assert!(hull.linear_velocity.length() < 0.1, "{}", hull.linear_velocity);
        // Laid on its side, it rests there, its tracks holding nothing either. Dropped onto
        // it, or rolled past it, it rolls on over the edge of its tracks, back onto them: a
        // box this wide, its centre of mass this low, barely rests on its side.
        let mut hull = chieftain();
        hull.set_direction(Vec3::new(90f32.to_radians(), 0.0, 0.0));
        hull.position.y = -lowest_contact(&hull) + 0.05;
        for _ in 0..60 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            let deepest = -lowest_contact(&hull);
            assert!(deepest < 0.15, "a contact {deepest} m deep at {}", hull.position);
        }
        let up = hull.orientation * Vec3::Y;
        assert!(hull.airborne && up.y.abs() < TRACKS_UPRIGHT, "up {up}");
    }

    #[test]
    fn falls_off_ledges() {
        // Off a deck 5 m high at full speed, it falls, gravity taking it no faster than
        // `g · t`, for most of a second in the air, keeping its speed, and lands where it
        // went on, its suspension stopping it hard rather than letting it into the
        // ground.
        let space = ground_under(Some(5.0), 0.0);
        let mut hull = chieftain();
        hull.position = Vec3::new(0.0, 5.0, 14.0);
        hull.speed = hull.mobility.speed_forward;
        hull.flags = movement_flag::FORWARD;
        let mut ticks_in_air = 0;
        let mut height = hull.position.y;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            assert!(height - hull.position.y < 1.5, "dropped from {height} to {}", hull.position.y);
            height = hull.position.y;
            let lowest = lowest_contact(&hull);
            assert!(lowest > -(SUSPENSION_TRAVEL + 0.1), "a contact {lowest} m deep, at {}", hull.position);
            assert!(hull.position.y > -0.1, "{}", hull.position);
            if hull.airborne {
                ticks_in_air += 1;
            }
        }
        assert!((6..=11).contains(&ticks_in_air), "{ticks_in_air} ticks");
        assert!(!hull.airborne && hull.position.y.abs() < 0.1, "{}", hull.position);
        assert!(hull.position.z > 40.0, "{}", hull.position);
        // Its nose over the edge, its centre of mass on the deck, it stays on it, all but
        // level.
        let mut hull = chieftain();
        hull.position = Vec3::new(0.0, 5.0, 18.5);
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let direction = hull.direction();
        assert!(!hull.airborne && (hull.position.y - 5.0).abs() < 0.1, "{}", hull.position);
        assert!(direction.y.abs() < 3f32.to_radians(), "{direction}");
        // Its centre of mass past the edge, it tips over it nose first, down onto the
        // ground, its rear track sliding off the edge, which holds it only as much as it
        // presses on it, and it stands on its nose, leaning on the deck's face.
        let mut hull = chieftain();
        hull.position = Vec3::new(0.0, 5.0, 21.0);
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let pitch = hull.direction().y;
        assert!(pitch > 60f32.to_radians(), "{}°", pitch.to_degrees());
        let lowest = lowest_contact(&hull);
        assert!(lowest < 0.2, "its nose {lowest} m up, at {}", hull.position);
        let mass_center = hull.position + hull.orientation * hull.center_of_mass();
        assert!(mass_center.z > 20.5 && mass_center.y < 5.0, "{mass_center}");
    }

    #[test]
    fn follows_slopes_down() {
        // Over the crest of a 10° slope down at full speed, it is light for a moment,
        // then its tracks keep to the slope, its nose down with it.
        let space = ground_under(None, 40.0 * 10f32.to_radians().tan());
        let mut hull = chieftain();
        hull.position.z = -10.0;
        hull.speed = hull.mobility.speed_forward;
        hull.flags = movement_flag::FORWARD;
        let mut ticks_in_air = 0;
        for _ in 0..50 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            if hull.airborne {
                ticks_in_air += 1;
                assert!(hull.position.z < 10.0, "{}", hull.position);
            }
            if (12.0..28.0).contains(&hull.position.z) {
                let pitch = hull.direction().y.to_degrees();
                assert!((pitch - 10.0).abs() < 2.0, "{pitch}° at {}", hull.position);
            }
        }
        assert!(ticks_in_air <= 3, "{ticks_in_air} ticks");
        assert!(hull.position.z > 40.0, "{}", hull.position);
    }

    /// How high its lowest contact is, over a flat ground at 0.
    fn lowest_contact(hull: &Hull) -> f32 {
        let place = |point: Vec3| hull.position + hull.orientation * point;
        hull.contacts
            .iter()
            .map(|contact| match contact.shape {
                Shape::Spring { point, .. } => place(point).y,
                Shape::Sphere { center, radius } => place(center).y - radius,
                Shape::Box { center, half } => {
                    let axes = Mat3::from_quat(hull.orientation);
                    let reach = half.x * axes.x_axis.y.abs() + half.y * axes.y_axis.y.abs() + half.z * axes.z_axis.y.abs();
                    place(center).y - reach
                }
            })
            .fold(f32::INFINITY, f32::min)
    }

}
