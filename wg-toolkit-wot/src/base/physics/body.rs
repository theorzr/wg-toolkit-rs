//! A hull's rigid body: how gravity, its suspension and its tracks move it in 3D
//! ([`Hull::integrate`]).
//!
//! A hull is a rigid body: a position and an orientation, the velocity of its centre of
//! mass and its spin about each axis of the world, its weight as its mass and a box's
//! inertia. Each tick moves it in [`SUBSTEPS`] steps of 10 ms:
//!
//! ```text
//!   for each step:
//!     1. its weight, at its centre of mass
//!     2. its contacts on the ground (see the contacts and ground modules):
//!          its springs push it up, those past their travel are stopped
//!     3. gravity and the springs accelerate it, against its mass and inertia
//!     4. the contacts touching the ground are held, as impulses (solve):
//!          those stopped don't sink further, its tracks push it along the ground
//!          toward the drive model's motion, the rest scrapes
//!     5. it moves
//!   then its speeds on the ground plane are read back from it
//! ```
//!
//! # Its suspension
//!
//! Its suspension pushes it up under each of its road wheels: a spring and a damper at
//! each, sharing its weight, sinking `SAG` at rest so that its origin is on the ground.
//! Its idlers and sprockets, at its tracks' ends, have no spring: the track around them
//! is rigid, and stops the hull as soon as it touches the ground, a rock or a step
//! meeting the track's front before its first road wheel. The rest of it is rigid too:
//! points on its hull's box and its turret's, which stop it on the ground when it rolls
//! over. See the `contacts` module for where they are. The pushes' torques roll and
//! pitch it, and its spin carries on from a step to the next: it bounces, rocks, and
//! tips.
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
//!                          ground                             (O)  sprocket, idler, no spring
//!                                                              #   rigid, on the track
//!
//!   for a spring, at its contact *:
//!                 depth = ground + SAG - the contact's height   (none: it is in the air)
//!                 push  = k * depth + d * (how fast depth grows), up, at *
//!                 SAG   = g / w^2,  w = 2 pi * SUSPENSION_FREQUENCY
//! ```
//!
//! A spring gives [`SUSPENSION_TRAVEL`] past where it rests, then stops hard: its contact
//! sinks no deeper, an impulse stopping it as the ground does, and a hull landing from a
//! ledge stops on its tracks rather than going into the ground. What sank past is moved
//! back out without the hull keeping any speed from it (a split impulse, see
//! `Hull::solve`), or it would be thrown up.
//!
//! # Its tracks
//!
//! Its tracks push it along the ground where they touch it, through their friction: each
//! of their contacts toward the motion the drive model gives the hull there, as much as
//! [`TRACK_GRIP`] of how hard it presses on the ground allows. A contact in the air, or
//! barely touching, drives nothing: over a crest its front tracks unload, and over a
//! ledge the hull tips off it rather than being held on. The pushes are at the ground,
//! under its centre of mass, so it squats as it speeds up and leans out of turns. With
//! no contact of its tracks on the ground, or tilted past [`TRACKS_UPRIGHT`], it is
//! `airborne`: nothing grips it, and it keeps going, turning and tumbling as it went,
//! until it lands.
//!
//! ```text
//!   at a contact of its tracks, arm from the centre of mass, pressed with N:
//!     wanted = drive velocity + drive spin * (Y x arm from its origin)    the drive model's
//!     slip   = (v + w x arm) - wanted, along the ground
//!     push   = against the slip, no more than TRACK_GRIP * N
//! ```
//!
//! Its speeds, `speed` and `slide`, are then read back from its body, at its origin:
//! rolling about its tracks, its centre of mass swings, but its tracks don't slide.
//! Off a ledge it falls; with its centre of mass past an edge, its last contacts on it
//! tip it over, nose first. On its side or its roof, its rigid contacts scrape on the
//! ground, held back by [`SCRAPE_FRICTION`] of how hard they press on it, and it slides
//! to a stop there. Where no contact finds any ground at all, off the terrain, it is held
//! as the drive model says.

use glam::{Mat3, Quat, Vec3, Vec3Swizzles};
use smallvec::SmallVec;

use wgtk::space::Space;

use super::contacts::{Contact, ContactKind, MAX_CONTACTS};
use super::ground::{ground_height, STEP_HEIGHT};
use super::{movement_flag, Hull, GRAVITY};


/// How many steps a tick's motion of a hull's body is integrated in: 10 ms each, short
/// enough for its suspension's springs.
const SUBSTEPS: usize = 10;
/// How fast a hull bounces on its suspension, in Hz, and how much of the damping that
/// would stop the bounce at once (critical) it has. Our choice, by feel: at 1.5 Hz the
/// hull sinks 11 cm into its springs at rest.
const SUSPENSION_FREQUENCY: f32 = 1.5;
const SUSPENSION_DAMPING: f32 = 0.7;
/// How much further than at rest a contact's spring gives, in metres, before it stops
/// hard: past it, the contact sinks no deeper, the hull's body stopped there as if
/// rigid. Our choice: a real track's road wheels travel about a quarter of a metre,
/// 11 cm of it sunk at rest.
pub const SUSPENSION_TRAVEL: f32 = 0.15;
/// How far, in metres, a contact's depth in the ground may change in a step of a hull's
/// body for its damper to take it as the contact sinking: more is the ground jumping
/// under it, off an edge or onto a step. 5 m/s.
const DEPTH_JUMP: f32 = 0.05;
/// How a contact past its travel is pushed back out: this share of how far past it is,
/// a step, no faster than this, in m/s. A contact is past it on landing, or meeting a
/// step: pushed out at once, the hull would be thrown off it. Its ground's rise under it
/// counts no faster either. Our choice.
const STOP_RECOVERY: f32 = 0.2;
const STOP_SPEED: f32 = 2.0;
/// How many times the contacts touching the ground are each held in a step, those held
/// moving the others, see `Hull::solve`: a hull landing on its nose stops on its front
/// contacts, and turns onto its back ones.
const CONTACT_PASSES: usize = 4;
/// How high its centre of mass is, as a share of its collision box's height. Our choice:
/// a hull's mass is low, its turret's higher.
const CENTER_OF_MASS_HEIGHT: f32 = 0.35;
/// How upright a hull must be for its tracks to grip, as its up axis' height: past 60°
/// of tilt, on its side or its roof, they hold nothing. Our choice.
const TRACKS_UPRIGHT: f32 = 0.5;
/// How much its tracks hold where they touch the ground, as a share of how hard each
/// contact presses on it, see `Hull::solve`. Fitted loosely, until its engine drives
/// them: the drive model asks up to 1.4 g of them, braking at 14 m/s², and the Type 64
/// turning at speed about 1.5 g sideways. At 1.5, its worst yaw error on its drive grew
/// from 35° to 59° where the real one held; from 2.5 on, the drives follow the real
/// server as well as tracks that never slip, and only a contact in the air or barely
/// touching falls short.
const TRACK_GRIP: f32 = 2.5;
/// How much a contact sliding on the ground is held back, as a share of how hard it
/// presses on it: a hull on its side or its roof scrapes to a stop. Our choice.
const SCRAPE_FRICTION: f32 = 0.6;

impl Hull {

    /// Give its body the speeds and turning on the ground plane its hull has, which the
    /// drive model reads and hits and walls change between its ticks: the start of a
    /// tick. Its origin's velocity on the ground plane becomes `speed` and `slide`, its
    /// spin about the vertical `yaw_rate + spin`; its vertical velocity and its roll and
    /// pitch spin are its body's own.
    pub fn speeds_to_body(&mut self) {
        self.angular_velocity.y = self.yaw_rate + self.spin;
        // Its centre of mass moves as its origin does, less what turning adds there.
        let turning = self.origin_velocity() - self.linear_velocity;
        let velocity = self.velocity() - turning;
        self.linear_velocity.x = velocity.x;
        self.linear_velocity.z = velocity.z;
    }

    /// Move its body by `dt` seconds, in [`SUBSTEPS`] steps, each under its weight at its
    /// centre of mass and its suspension at its contacts, see the `contacts` module: a
    /// spring and a damper at each, pushing it up from the ground under it as far as the
    /// contact sinks into it. Its velocity and its spin carry on from a step to the next,
    /// changed only by those forces and their torques, against its mass and its inertia
    /// (with the gyroscopic term, `ω × Iω`).
    ///
    /// ```text
    ///   each contact c, its point p in the world, the ground under it g:
    ///     depth  = g + SAG − p.y               > 0: it touches, and pushes
    ///     push   = k · depth + d · (how fast depth grows),   upward, never pulling
    ///   force    = Σ push − m · g              torque = Σ (p − centre of mass) × push
    ///   v += force / m · h                     ω += I⁻¹ (torque − ω × Iω) · h
    /// ```
    ///
    /// The springs share its weight, so at rest each sinks `SAG = g / ω²` and its origin
    /// is on the ground; `ω` is [`SUSPENSION_FREQUENCY`]'s. A spring gives
    /// [`SUSPENSION_TRAVEL`] more at most: past it, its contact is stopped, see
    /// [`Self::solve`]. A contact finds the ground up to [`STEP_HEIGHT`] above itself,
    /// where it is on the tilted hull: a nose raised by a rock's face finds the face
    /// further up, and the hull climbs it.
    ///
    /// Its tracks push it along the ground where they touch it, toward the motion the
    /// drive model gave it, `speed` along its heading and `slide` across it, carried
    /// round as it turns, or, its tracks blocked, kept in the world as its hull swings
    /// under it, and `yaw_rate + spin` about the vertical. Upright, on the ground, it is
    /// driven as the drive model says; with its tracks off the ground, or on its side or
    /// its roof ([`TRACKS_UPRIGHT`]), it is `airborne`, and keeps going, turning and
    /// tumbling as it went, until it lands. Where no contact finds any ground at all, off
    /// the terrain, nothing holds it up and it doesn't fall either: it is driven as the
    /// drive model says.
    ///
    /// After, its speeds are its body's, read back, see [`Self::speeds_from_body`].
    pub fn integrate(&mut self, dt: f32, space: &Space) {

        // What holds over the whole tick: its mass and its contacts in its own frame,
        // and, its tracks blocked, the velocity they hold it going at in the world.
        let mass = self.mobility.weight;
        let center = self.center_of_mass();
        let inertia = self.inertia();
        let suspension = Suspension::of(mass, &self.contacts);
        let handbrake = self.flags & movement_flag::BLOCK_TRACKS != 0;
        let step = dt / SUBSTEPS as f32;
        let held = self.velocity();

        for _ in 0..SUBSTEPS {

            // 1. Where its centre of mass is as the step starts.
            let rotation = self.orientation;
            let mass_center = self.position + rotation * center;

            // 2. Its weight, and its contacts on the ground: their springs' pushes, and
            //    which are stopped.
            let loads = self.loads(space, &suspension, mass_center, step);

            // 3. The motion its tracks drive it to, as the drive model said.
            let target = DriveTarget {
                velocity: if handbrake { held } else { self.velocity() },
                spin: self.yaw_rate + self.spin,
                origin: self.position - mass_center,
            };

            let (push_velocity, push_spin) = if loads.any_ground {

                // 4. Gravity and its springs accelerate it, then its contacts on the
                //    ground are held: its tracks grip only upright, on its side or its
                //    roof it scrapes.
                let gripping = loads.tracks_touching && (rotation * Vec3::Y).y > TRACKS_UPRIGHT;
                let world_inertia = Mat3::from_quat(rotation) * Mat3::from_diagonal(inertia) * Mat3::from_quat(rotation.inverse());
                let inverse_inertia = world_inertia.inverse();
                let gyroscopic = self.angular_velocity.cross(world_inertia * self.angular_velocity);
                self.linear_velocity += loads.force / mass * step;
                self.angular_velocity += inverse_inertia * (loads.torque - gyroscopic) * step;
                let target = if gripping { Some(target) } else { None };
                let body = Mass {
                    mass,
                    inverse_inertia,
                };
                let pushed = self.solve(&loads.touches, target, &body, step);
                self.airborne = !gripping;
                self.touching = !loads.touches.is_empty();
                pushed

            } else {

                // 4. No ground under any contact, off the terrain: it is held as driven,
                //    neither falling nor tilting.
                self.angular_velocity = Vec3::Y * target.spin;
                self.linear_velocity = target.velocity - self.angular_velocity.cross(target.origin);
                self.airborne = false;
                self.touching = true;
                (Vec3::ZERO, Vec3::ZERO)

            };

            // 5. It moves, at its velocity and by what pushed its contacts out.
            let mass_center = mass_center + (self.linear_velocity + push_velocity) * step;
            self.orientation = (Quat::from_scaled_axis((self.angular_velocity + push_spin) * step) * rotation).normalize();
            self.position = mass_center - self.orientation * center;

        }

        // 6. Its speeds on the ground plane, read back from its body.
        self.speeds_from_body();

    }

    /// The forces on its body in a step, see [`Loads`]: its weight, at its centre of mass,
    /// and the pushes of its springs whose contacts are in the ground, with each contact
    /// touching it. Each contact's depth is kept for the next step, its damper resisting
    /// how fast it sinks.
    fn loads(&mut self, space: &Space, suspension: &Suspension, mass_center: Vec3, step: f32) -> Loads {

        let mut loads = Loads {
            force: Vec3::NEG_Y * GRAVITY * self.mobility.weight,
            torque: Vec3::ZERO,
            any_ground: false,
            tracks_touching: false,
            touches: SmallVec::new(),
        };

        for (contact, previous) in self.contacts.iter().zip(&mut self.depths) {

            // Where it is in the world, and the ground under it, up to a step above.
            let point = self.position + self.orientation * contact.point;
            let Some(ground) = ground_height(space, point.xz(), point.y + STEP_HEIGHT) else {
                *previous = f32::NAN;
                continue;
            };
            loads.any_ground = true;

            // How deep it is in the ground, a spring sinking into it at rest, a rigid
            // contact never, and how much deeper than in the last step.
            let spring = contact.kind == ContactKind::Spring;
            let depth = if spring { ground + suspension.sag - point.y } else { ground - point.y };
            let sank = depth - *previous;
            *previous = depth;
            if depth <= 0.0 {
                continue;
            }
            loads.tracks_touching |= contact.kind != ContactKind::Body;

            // How fast it sinks: into the ground as it follows it, not down; but off an
            // edge the ground jumps, and only the contact's own speed counts.
            let arm = point - mass_center;
            let velocity = self.linear_velocity + self.angular_velocity.cross(arm);
            let sinking = if sank.abs() <= DEPTH_JUMP { sank / step } else { -velocity.y };

            // Past its travel, or rigid, it is stopped: as fast as the ground rises under
            // it, at most, off a rough triangle's edges it would throw it.
            let past = if spring { depth - suspension.stop } else { depth };
            let stopped = if past > 0.0 { Some(((sinking + velocity.y).min(STOP_SPEED), past)) } else { None };

            // A spring pushes, as far as it sank and as fast as it sinks, never pulling.
            let mut load = 0.0;
            if spring {
                let push = (suspension.stiffness * depth.min(suspension.stop) + suspension.damping * sinking).max(0.0);
                loads.force.y += push;
                loads.torque += arm.cross(Vec3::Y * push);
                load = push * step;
            }

            loads.touches.push(Touch {
                arm,
                kind: contact.kind,
                load,
                stopped,
            });

        }

        loads

    }

    /// Hold its contacts touching the ground, see [`Self::integrate`], [`CONTACT_PASSES`]
    /// times each, those held moving the others, as impulses: at those past their travel
    /// one up, one along the ground at all, and at those past their travel again one
    /// moving them out, split. Never a pull: a contact leaving the ground goes.
    ///
    /// Up, the first stops it sinking, its velocity and spin rising with its ground; the
    /// last only moves it back out, as fast as [`STOP_RECOVERY`] says, for this step:
    /// pushed out, the hull keeps no speed from it, which would throw it up. That last is
    /// returned, as a velocity and a spin to move by.
    ///
    /// Along the ground, a contact of its tracks, as they grip, takes the velocity the
    /// `target` gives it, as much as [`TRACK_GRIP`] of how hard it presses on the ground
    /// allows: its spring's push, or how hard it was stopped. Any other scrapes, held back
    /// to a stop by [`SCRAPE_FRICTION`] of it.
    ///
    /// ```text
    ///   up      short   = wanted − (v + ω × arm).y           > 0: it sinks too fast
    ///           impulse = short / (1/m + (arm × Y) · I⁻¹ (arm × Y))
    ///           wanted  = rising                             its velocity and spin
    ///                   = min(STOP_RECOVERY · past / step, STOP_SPEED)   the move out
    ///   along   slip    = (v + ω × arm) − (target.v + target.spin · Y × (arm − origin)),
    ///                     along X and Z, the origin's arm from the centre of mass
    ///           impulse = −slip / their effective masses,
    ///                     all a contact's together no longer than grip · its push
    /// ```
    fn solve(&mut self, touches: &[Touch], target: Option<DriveTarget>, body: &Mass, step: f32) -> (Vec3, Vec3) {

        let Mass { mass, inverse_inertia } = *body;

        // What moves it out, and each contact's impulses so far: up to stop it, along the
        // ground, so that their sums are what is bounded, not each pass's.
        let mut push_velocity = Vec3::ZERO;
        let mut push_spin = Vec3::ZERO;
        let mut stops = [0.0; MAX_CONTACTS];
        let mut frictions = [Vec3::ZERO; MAX_CONTACTS];

        for _ in 0..CONTACT_PASSES {
            for (index, touch) in touches.iter().enumerate() {

                let arm = touch.arm;
                let lever = arm.cross(Vec3::Y);
                let effective = body.per_impulse(arm, Vec3::Y);

                // 1. Stopped: it sinks no faster than its ground rises under it.
                if let Some((rising, _)) = touch.stopped {
                    let velocity = self.linear_velocity + self.angular_velocity.cross(arm);
                    let short = rising - velocity.y;
                    if short > 0.0 {
                        let impulse = short / effective;
                        stops[index] += impulse;
                        self.linear_velocity.y += impulse / mass;
                        self.angular_velocity += inverse_inertia * lever * impulse;
                    }
                }

                // 2. Along the ground: its tracks toward their target as they grip, else
                //    scraping to a stop, as much as it is pressed.
                let (wanted, grip) = match target {
                    Some(target) if touch.kind != ContactKind::Body => {
                        (target.velocity + Vec3::Y.cross(arm - target.origin) * target.spin, TRACK_GRIP)
                    }
                    _ => (Vec3::ZERO, SCRAPE_FRICTION),
                };
                let slip = self.linear_velocity + self.angular_velocity.cross(arm) - wanted;
                let mut impulse = Vec3::ZERO;
                for axis in [Vec3::X, Vec3::Z] {
                    impulse -= axis * (slip.dot(axis) / body.per_impulse(arm, axis));
                }
                let friction = (frictions[index] + impulse).clamp_length_max(grip * (touch.load + stops[index]));
                let change = friction - frictions[index];
                frictions[index] = friction;
                self.linear_velocity += change / mass;
                self.angular_velocity += inverse_inertia * arm.cross(change);

                // 3. Past its travel: moved back out, without keeping the speed it took.
                if let Some((_, past)) = touch.stopped {
                    let pushed = push_velocity + push_spin.cross(arm);
                    let short = (STOP_RECOVERY * past / step).min(STOP_SPEED) - pushed.y;
                    if short > 0.0 {
                        let impulse = short / effective;
                        push_velocity.y += impulse / mass;
                        push_spin += inverse_inertia * lever * impulse;
                    }
                }

            }
        }

        (push_velocity, push_spin)

    }

    /// Read its speeds on the ground plane back from its body, at its origin, see
    /// [`Self::origin_velocity`]: the end of a tick. Gripping, `spin` is what turns it
    /// besides its tracks; airborne, `yaw_rate` is, so that it lands turning as it went.
    fn speeds_from_body(&mut self) {
        let (heading, right) = self.axes();
        let velocity = self.origin_velocity();
        self.speed = velocity.dot(heading);
        self.slide = velocity.dot(right);
        if self.airborne {
            self.yaw_rate = self.angular_velocity.y - self.spin;
        } else {
            self.spin = self.angular_velocity.y - self.yaw_rate;
        }
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
        let springs = contacts.iter().filter(|contact| contact.kind == ContactKind::Spring).count().max(1) as f32;
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
/// torque about its centre of mass, and its contacts touching the ground: whether any
/// contact finds ground at all, and any of its tracks' touches it.
#[derive(Debug)]
struct Loads {
    force: Vec3,
    torque: Vec3,
    any_ground: bool,
    tracks_touching: bool,
    touches: SmallVec<[Touch; MAX_CONTACTS]>,
}

/// A [`Contact`] touching the ground in a step, see [`Hull::solve`]: where it is from
/// the hull's centre of mass, how hard its spring pushed it up in the step, as an
/// impulse, and, past its travel, how fast the ground under it rises and how far past it
/// is.
#[derive(Debug, Clone, Copy)]
struct Touch {
    arm: Vec3,
    kind: ContactKind,
    load: f32,
    stopped: Option<(f32, f32)>,
}

/// The motion a hull's tracks drive it to, see [`Hull::solve`]: its origin's velocity on
/// the ground plane, its spin about the vertical, and where its origin is from its centre
/// of mass.
#[derive(Debug, Clone, Copy)]
struct DriveTarget {
    velocity: Vec3,
    spin: f32,
    origin: Vec3,
}

/// A hull's body against impulses, in a step: its mass, and its inertia's inverse in the
/// world.
#[derive(Debug, Clone, Copy)]
struct Mass {
    mass: f32,
    inverse_inertia: Mat3,
}

impl Mass {

    /// How much an impulse along `axis` at `arm` from its centre of mass changes the
    /// velocity there along it, per unit: the inverse of its effective mass there.
    fn per_impulse(&self, arm: Vec3, axis: Vec3) -> f32 {
        let lever = arm.cross(axis);
        1.0 / self.mass + lever.dot(self.inverse_inertia * lever)
    }

}


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use wgtk::space::statics::Statics;
    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, ground_under, quad};
    use super::super::Rubble;
    use super::*;

    #[test]
    fn drives_onto_decks_within_a_step() {
        // A step up, it climbs onto it, nose first, and rests on it.
        let space = ground_under(Some(STEP_HEIGHT - 0.1), 0.0);
        let mut hull = chieftain();
        hull.position.z = -6.0;
        hull.flags = movement_flag::FORWARD;
        let mut steepest: f32 = 0.0;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            steepest = steepest.min(hull.direction().y);
        }
        assert!(hull.position.z > 10.0 && (hull.position.y - (STEP_HEIGHT - 0.1)).abs() < 0.05, "{}", hull.position);
        assert!(steepest < -3f32.to_radians(), "{}°", steepest.to_degrees());
        // Higher, a bridge it drives under, it stays below.
        let space = ground_under(Some(STEP_HEIGHT + 3.0), 0.0);
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
        hull.spin = 0.5;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..60 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
            let deepest = -lowest_contact(&hull);
            assert!(deepest < 0.15, "a contact {deepest} m deep at {}", hull.position);
        }
        assert!(hull.airborne && (hull.orientation * Vec3::Y).y < -0.9, "{}", hull.direction());
        assert!(hull.linear_velocity.length() < 0.1, "{}", hull.linear_velocity);
        // Rolled past its side, its edge 10 cm off the ground, it tips over onto it, rocks
        // back toward its tracks, and rests on its side, its tracks holding nothing
        // either. Dropped from higher, it rolls on over the edge of its tracks, back onto
        // them: a box this wide, its centre of mass this low, barely rests on its side.
        let mut hull = chieftain();
        hull.position.y = 2.3;
        hull.set_direction(Vec3::new(100f32.to_radians(), 0.0, 0.0));
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
        hull.contacts
            .iter()
            .map(|contact| (hull.position + hull.orientation * contact.point).y)
            .fold(f32::INFINITY, f32::min)
    }

}
