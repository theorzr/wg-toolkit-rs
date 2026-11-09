//! The physics of a battle: how its vehicles drive, push each other and stop against the
//! map. The real server runs each vehicle as a rigid body on springs (`WGTankPhysics`),
//! with server-only tuning; the client doesn't predict anything and only shows what the
//! server sends. This module is a much simpler model that is fitted on drives captured
//! from the real server (see "Where the numbers come from" below), so that driving feels
//! the same.
//!
//! # The big picture
//!
//! A [`World`] holds the [`Vehicle`]s of one battle, on its map's [`Space`]: terrain,
//! static models, and bounds. A vehicle is a [`Hull`] (where it is, how fast it goes and
//! turns) and a [`Turret`] (where its gun points). Whoever drives a vehicle, a client or a
//! bot, only sets its `hull.flags` (the `vehicle_moveWith` flags) and its turret's aim
//! before a tick, then [`World::step`] does everything else:
//!
//! ```text
//!   controllers set flags and aim
//!                │
//!                ▼
//!   World::step(dt)                                        dt = one tick, 0.1 s
//!   │
//!   ├─ 1. remember()        pose ← where each vehicle is, for the sweep in 4
//!   │
//!   ├─ 2. for every vehicle: Vehicle::step
//!   │     ├─ Hull::advance
//!   │     │    ├─ Hull::step        drive model: engine, ground, slope, turning
//!   │     │    ├─ push_out          stop against walls                  (if it moved)
//!   │     │    ├─ keep_within       stop at the map's edge
//!   │     │    └─ settle            rest on the ground, roll and pitch  (if it moved)
//!   │     └─ Turret::step     turn toward the aim, update the dispersion
//!   │
//!   └─ 3. collide(dt), up to 3 passes over every pair of vehicles
//!         for a pair whose 3D boxes overlap:
//!         ├─ 4. find where they first touched in the tick, put both back there
//!         ├─ 5. push them apart, each by the other's share of their weight
//!         ├─ 6. exchange an impulse at the contact: speeds and spin
//!         ├─ 7. let both go on for what was left of the tick at their new speeds
//!         └─ 8. keep_on_map for both: walls, edge, ground again
//! ```
//!
//! Nothing in a step allocates: boxes, outlines and polygons live on the stack
//! ([`SmallVec`]), and vehicles are in a slab ([`Slots`]).
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
//! its left one, 3 m apart, `roll = atan(0.3 / 3) ≈ +6°`. That is exactly what
//! [`Hull::settle`] computes from the ground under the vehicle.
//!
//! # The drive model (`Hull::step`)
//!
//! A hull has two speeds on the ground plane, along its heading (`speed`) and across it
//! (`slide`), so its velocity is `heading · speed + right · slide`. It moves by
//! `position += velocity · dt`. The tracks hold `slide` at zero (at `SIDE_FRICTION`)
//! unless the handbrake is on.
//!
//! The speed along the heading follows the flags, in one of four cases:
//!
//! ```text
//!   handbrake (BLOCK_TRACKS) ──▶ slows toward 0 (or the pivot speed) at HANDBRAKE_FRICTION
//!   no direction asked ───────▶ slows toward 0 at COASTING
//!   going against the way ────▶ slows toward 0 at BRAKING            (then reverses)
//!   going the way asked ──────▶ the engine accelerates it, capped at the target speed:
//!
//!        power      = POWER_FACTOR · P / m
//!        drive      = min( power / |v| , MAX_ACCELERATION ) · grip_along
//!        resistance = μ(ground) · g + (TURNING_RESISTANCE while turning)
//!        a          = drive − resistance + g · sin(pitch) · way
//!        |v|′       = min( |v| + a · dt , target )
//! ```
//!
//! so the engine pushes hard from a standstill (the cap), less and less as the speed
//! rises (`power / |v|`), and stalls on a slope too steep for its power. `grip_along`
//! and `grip_across` fall from 1 to 0.1 as a slope steepens ([`SLOPE_GRIP_ALONG`]): the
//! tracks hold less, and gravity wins.
//!
//! The turning speed follows a goal with a rise and a fall time, so it doesn't jump:
//!
//! ```text
//!   goal       = turn · rotation_speed(ground) · (ROTATION_ON_MOVE if moving)
//!   yaw_rate  →  goal   at   rotation_speed / ROTATION_RISE   (or ROTATION_FALL)
//!   yaw       +=  (yaw_rate + spin) · dt
//!   spin      →  0      at   what its tracks' friction leaves it (track_friction)
//! ```
//!
//! `spin` is turning that hits gave the hull (see below): unlike `yaw_rate` the tracks
//! don't drive it, they only hold it. To turn, a hull skids its tracks sideways, against
//! the friction that also holds its slide (`SIDE_FRICTION`, about 1.1 g), which the two
//! share: a hull sliding sideways turns freely, as a rammed one does, and one at rest
//! holds the turn a weak push would give it, see `Hull::track_friction`.
//!
//! # The ground (`settle`, `ground_height`)
//!
//! A vehicle doesn't fall: after it moves it is put back on the ground, tilted with it.
//! Five points are looked up, the centre and the four corners of the area its tracks
//! carry it on (`MobilityDef::carrying_point`), and the hull takes the centre's height
//! and the tilt that the corners' heights give:
//!
//! ```text
//!          front
//!     FL ●─────────● FR            roll  = atan( (FR+BR − FL−BL) / (4 · cx) )
//!        │     ● C  │              pitch = −atan( (FR+FL − BR−BL) / (4 · cy) )
//!     BL ●─────────● BR            y     = C
//!          back                    cx, cy: the carrying point's half width and length
//! ```
//!
//! The ground at a point is the *highest* of the terrain and of the static triangles that
//! are ground-like (flatter than [`WALL_ANGLE`] from vertical, or ramps) *no higher than
//! `y + STEP_HEIGHT`*. That last bound is what makes a bridge work from both sides:
//!
//! ```text
//!    height                                         the deck is above y + 0.8 m, so it
//!      ▲                      ═════════════════     isn't ground: the tank drives under
//!      │         ┌──────┐
//!      │ y+0.8 ┈ │ tank │ ┈┈┈┈┈┈┈┈┈┈┈ top ┈┈┈┈     ground = highest surface below this
//!      │ y     ──┴──────┴────────── terrain
//!      ┴───────────────────────────────────▶
//!
//!      on the ramp, y rises with the highest surface below `top`, and once the tank is on
//!      the deck, the deck is below `top` and holds it, not the terrain 5 m under it:
//!
//!      │                          ╱═════════════════ deck
//!      │       ┌──────┐        ╱   (ramp)
//!      │ ──────┴──────┴─────╱
//! ```
//!
//! If any of the five points has no ground, the hull stays where it is.
//!
//! # Static geometry (`push_out`, `overlaps`)
//!
//! The map's solid models are triangles in a grid ([`StaticScene`]). The hull's box is
//! tested against the triangles that are *walls* (within [`WALL_ANGLE`], 50°, of
//! vertical; flatter ones are ground), but only against the part of each triangle in a
//! *band* of heights, in the hull's own frame (so a tilted hull's nose clears higher):
//!
//! ```text
//!    height in the hull's frame
//!      ▲       ┌────────────────┐ ◀── top of the hull's box
//!      │       │ walls in this   │
//!      │       │ band count      │
//!      │ ┈┈┈┈┈ │┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈┈│ ◀── bottom + STEP_HEIGHT (0.8 m)
//!      │       │ steps and debris│     lower than this, the tank drives over
//!      │ ──────┴────────────────┴────  the ground
//! ```
//!
//! Each triangle is clipped to the band ([`clip_to_heights`]), giving a convex outline
//! seen from above, and the box is separated from it along the least overlapping axis
//! of a 2D separating-axis test ([`box_push`], below). The deepest overlap is applied
//! first, at most [`WALL_PASSES`] times. The wall stops the hull's speed into it with an
//! impulse at its corner deepest in the wall (or the middle of an edge flat against it),
//! as a vehicle would that doesn't move: a corner swings the hull round. The rest slows
//! by [`WALL_FRICTION`] of it, so a tank scrapes along a wall instead of sticking.
//!
//! The ground under the corners can tilt the hull no more than [`MAX_TILT`]: a corner on
//! a rock's facet would tip it over, and so tilted, the walls would push it metres aside.
//!
//! The map's edge is the same idea with 4 planes ([`Hull::keep_within`]): the centre
//! stays [`BOUNDS_MARGIN`] inside, the speed into the edge is removed, and the rest
//! slows by [`EDGE_FRICTION`].
//!
//! # Vehicles against each other (`World::collide`, `collide`)
//!
//! Each vehicle has a box ([`MobilityDef::bounds`]) oriented as the hull is, with its
//! roll, pitch and yaw: an [`Obb`]. A tank on a bridge ramp is tilted, a tank on the
//! deck is above one on the road. Two tilted boxes are tested with the separating axis
//! theorem in 3D. Two convex shapes don't overlap if some axis exists on which their
//! projections don't overlap, and for two boxes it is enough to try 15 axes: the 3 faces'
//! normals of each, and the 9 cross products of an edge of one with an edge of the other.
//! The axis with the *least* overlap is the cheapest way to separate them:
//!
//! ```text
//!   on one axis n:      A ├────────────────┤
//!                                    B ├────────────────┤
//!                                      ├────┤ depth d      ◀ least over the 15 axes
//!                                        ⇒ push A by d along n
//! ```
//!
//! Vertical axes don't push (`|n.y| > STACKED_NORMAL`): a box on top of another is held
//! by its ground, not shoved. Otherwise the push is the horizontal part of the normal,
//! scaled so that the 3D depth is cleared (`d / |n.xz|`), because vehicles only move on
//! the ground plane.
//!
//! **Where they first touched.** At 75 km/h a vehicle moves 2 m in a tick, which is
//! about as far as its box is deep: testing only at the end of the tick finds a deep overlap
//! and the least axis can be a sideways one, throwing the vehicle through the other.
//! So the world remembers each vehicle's pose at the start of the tick and sweeps both
//! boxes over the tick, halving the interval to find the first overlap:
//!
//! ```text
//!   start of tick        first touch  t*             end of tick (deep overlap)
//!   ┌─────┐   ┌─────┐    ┌─────┬─────┐                ┌─────┬▒▒▒┬─────┐
//!   │  A  │ → │  B  │    │  A  │  B  │                │  A  │▒▒▒│  B  │
//!   └─────┘   └─────┘    └─────┴─────┘                └─────┴▒▒▒┴─────┘
//!                          ▲ both are put back here (6 halvings of the tick)
//! ```
//!
//! **The push and the impulse.** From there the overlap is shallow and the axis can be
//! trusted. The push is split by weight, `A` moves by `m_B / (m_A + m_B)` of it, so
//! the heavier one barely moves. Then the speeds along the normal `n` are made equal
//! *at the contact point*, with the usual rigid-body impulse for a collision that
//! doesn't bounce, which includes the turning each vehicle takes:
//!
//! ```text
//!   v_close = (v_A + ω_A × r_A − v_B − ω_B × r_B) · n           (< 0 when closing in)
//!
//!                      − v_close
//!   j = ─────────────────────────────────────────────────────
//!        1/m_A + 1/m_B + (r_A × n)²/I_A + (r_B × n)²/I_B
//!
//!   A: v += j·n/m_A     spin −= (r_A × j·n) / I_A        B: the opposite
//!   I = m · (width² + length²) / 12                           (a box's yaw inertia)
//! ```
//!
//! `r_A` and `r_B` are the arms from each box's middle to the contact point. A hit
//! through the middle (`r × n = 0`) doesn't turn anything; a hit near a corner swings
//! both. The contact point is the *centroid of the overlap* of the two boxes seen from
//! above, taken at the end of the tick, where a whole face is in contact (at the very
//! moment of first touch, boxes a hair off square meet at one corner, which would spin
//! them wildly):
//!
//! ```text
//!     ┌───────────┐
//!     │     A     │            ▒ the overlap seen from above (clipped polygon)
//!     │      ┌────┼──────┐     ● its centroid: the contact point
//!     │      │▒▒●▒│      │     r_A: from A's middle to ●     r_B: from B's middle to ●
//!     └──────┼────┘      │
//!            │     B     │
//!            └───────────┘
//! ```
//!
//! Then both go on for what was left of the tick (`(1 − t*) · dt`) at their new speeds
//! and turning, are put back out of walls and within the map, and the whole pair loop
//! runs again (up to [`COLLISION_PASSES`] times) because a vehicle pushed into a third
//! must be resolved too.
//!
//! # The turret
//!
//! [`Turret::step`] turns the turret toward the aim at its rotation speed, within its
//! yaw limits, and raises the gun within its pitch limits (which depend on the yaw). The
//! aim is either a point in the world (the gun is raised so that the shell *drops onto*
//! it, [`Turret::ballistic_direction`]) or fixed angles on the hull. It also keeps the
//! gun's dispersion: it jumps up when the hull moves or turns or the turret turns, and
//! shrinks back over the aiming time.
//!
//! # Where the numbers come from
//!
//! Each constant says whether it comes from the game's sources, was fitted on drives
//! captured from the real server (a T110E4 on Ruinberg, a stock Type 64 on Live Oaks,
//! and a story-mode Pershing on Graf Zeppelin, see `drive_replay.rs`), or is our own
//! choice until a capture can fit it. The drive harness replays those captures through
//! this module in windows, each starting from the real state, and reports the errors:
//! about 0.25 m median over 1 s windows. Collisions between vehicles were checked on the
//! Pershing's pushes of its allies, but their model (the tracks' friction against a
//! spin, the impulse without bounce or friction) is ours.
//!
//! # What isn't modelled
//!
//! A vehicle doesn't fall, flip, climb on debris taller than `STEP_HEIGHT`, or sink in
//! water: that is the real server's spring suspension, a future step. Vehicles have no
//! friction between them and take no damage from ramming. Trees don't fall: the client
//! numbers them among the speed trees, which aren't decoded.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::vec::Drain;

use glam::{Affine3A, Mat3, Quat, Vec2, Vec3, Vec3Swizzles};
use smallvec::SmallVec;
use tracing::warn;

use wgtk::space::statics::{BodyKind, PlacedDestructible, StaticScene, StaticTriangle};
use wgtk::space::destructibles::DestructibleKind;
use wgtk::space::terrain::{Ground, Terrain};
use wgtk::app::slots::{SlotKey, Slots};
use wgtk::space::Space;

use crate::wot::item_defs::{AimingDef, MobilityDef};


/// How far inside the map's bounds the hull's centre stays, in metres. Fitted: the real
/// server holds it 0.3 m inside driven square on, 0.0 to 0.1 m inside at 30°.
const BOUNDS_MARGIN: f32 = 0.2;
/// How much of the speed an edge stops it also takes along it: their friction. Fitted
/// on the real server's drag along an edge driven into at 30°.
const EDGE_FRICTION: f32 = 0.25;
/// How high above the ground under its centre the hull's box starts, for the static
/// models it collides with: what is lower it drives over. Fitted roughly: the drives'
/// real hulls went over debris and porches up to 0.76 m high.
const STEP_HEIGHT: f32 = 0.8;
/// How much of the speed a wall stops it also takes along it. Our choice, an edge's
/// until fitted.
const WALL_FRICTION: f32 = EDGE_FRICTION;
/// How far from vertical a solid static triangle stops the hull, in radians: flatter,
/// it is ground the hull would climb. Our choice, before fitting.
const WALL_ANGLE: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// How many times a step pushes the hull out of the static models it overlaps, the
/// deepest overlap first.
const WALL_PASSES: usize = 4;
/// How vertical a contact normal may be, as its height share, before one vehicle is on
/// top of the other, which its ground holds, not a push: about 45°. Our choice.
const STACKED_NORMAL: f32 = 0.7;
/// How much the overlap along an edge pair's axis counts for more than along a face's,
/// in metres: crossed edges of near-parallel boxes give noisy axes.
const EDGE_AXIS_BIAS: f32 = 0.01;
/// How many times a step pushes vehicles apart, those pushed into a third again.
const COLLISION_PASSES: usize = 3;
/// How many times the moment two vehicles first touched is halved: the step's length over
/// `2^FIRST_TOUCH_HALVINGS`.
const FIRST_TOUCH_HALVINGS: usize = 6;

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

// Where each constant comes from: the game's sources, the drives captured from the real
// server the hull is fitted on (a T110E4, 950 hp for 66 t, on Ruinberg's mostly firm
// ground, captured with its setup; a stock Type 64, 528 hp for 23 t, on Live Oaks'
// mostly medium ground), or our own choice.

/// Gravity, in m/s². From the game: `G` in `physics_shared.py`, which the server's
/// physics multiplies by the vehicle's `gravityFactor`, 1 when its definitions don't set
/// it, as the client's never do.
const GRAVITY: f32 = 9.81;
/// How many times its engine's power over its weight drives the vehicle, against its
/// ground's rolling friction: `a = POWER_FACTOR · P / (m · v) - μ · g`. Fitted: on the
/// samples driving straight forward from 4 m/s to near the top speed, forward held for
/// 0.4 s before, the acceleration plus gravity along the slope, on the ground under
/// them, gives a median of 1.125 on the T110E4 (559 samples) and 1.137 on the Type 64
/// (1040). Fitting the friction too trades it against the power (1.2 to 1.36 for 1.25 to
/// 1.3 times the friction), so the game's friction is kept.
const POWER_FACTOR: f32 = 1.13;
/// The ground the vehicle drives on where the terrain gives none, or off it. Our choice:
/// unpainted cells are mostly beyond the battle's bounds, and medium is most of the
/// painted ones.
const DEFAULT_GROUND: Ground = Ground::Medium;
/// The most the tracks accelerate the vehicle, in m/s²: the engine's power, over the
/// speed, would be endless from a standstill. Fitted: the drives' errors are lowest from
/// 10 to 14, both vehicles starting up to about 6 m/s² in the captured ticks.
const MAX_ACCELERATION: f32 = 10.0;
/// How fast the vehicle slows down without controls, and brakes against them, in m/s²,
/// whatever the vehicle and its ground. Measured: released from forward, both vehicles
/// lose 1.2 to 1.6 m/s a tick (the T110E4 9.8, 8.7, 7.1, 5.7; the Type 64 11.6, 10.0,
/// 8.6, 7.0 on medium ground, 17.0, 15.8, 14.3, 12.7 on soft), and reversing slows them
/// as fast. The drives' errors keep falling up to 20, beyond what they show, so make up
/// for something else.
const COASTING: f32 = 14.0;
const BRAKING: f32 = 14.0;
/// The share of its turning speed on the spot it turns at while moving. Fitted: the
/// T110E4 turns at 26.8°/s on the spot and 22.8°/s while moving, medians, and the Type
/// 64 at 49.3°/s and 41.9°/s on medium ground.
const ROTATION_ON_MOVE: f32 = 0.85;
/// The share of its top speed it drives at while turning. Fitted: the 90th percentile
/// speed while turning against straight, 9.0 and 9.9 m/s for the T110E4, 18.1 and
/// 20.0 m/s for the Type 64, 0.91 both.
const SPEED_WHILE_TURNING: f32 = 0.91;
/// How much turning while moving slows the vehicle on top of its ground's resistance, in
/// m/s². Fitted: the power its acceleration shows while turning falls short of driving
/// straight by about 0.45 to 1 m/s², on both drives and at every speed.
const TURNING_RESISTANCE: f32 = 0.8;
/// How much a ram damages a destructible: this share of the vehicle's kinetic energy as
/// it closes on it, `½ · w · v²` of its weight `w` in [`UNIT_VEHICLE_MASS`] and its
/// speed in m/s, adding up from a tick to the next; it breaks once that reaches its
/// health, and keeps the energy left. Fitted on the captured drives: the Type 64, 0.66
/// such units, drove through a house's modules of 50 health at 13 m/s, 55; the Pershing,
/// 1.23, felled a lamp post of 10 at 5.5 m/s, 19, and broke a barrel of 10 pushing it at
/// 2.7 m/s, 4.5 a tick; the Type 64 drove through two houses' modules of 35 at
/// 15.8 m/s, 82.
const RAM_DAMAGE: f32 = 1.0;
/// How much of its energy breaking a destructible costs the vehicle, as a share of the
/// health it took. Fitted: the real Type 64 drove through four house modules of 35
/// health at 15.6 m/s, all but keeping its speed.
const RAM_COST: f32 = 0.05;
/// How much a vehicle damages a destructible it presses against, its tracks driving it
/// in, per second, of its weight in [`UNIT_VEHICLE_MASS`], on top of [`RAM_DAMAGE`]: one
/// stopped by a house's wall breaks it in time, a module of 50 health in 2 s at 35 t.
/// Our choice, by feel.
const RAM_PRESS: f32 = 25.0;
/// The weight of a vehicle that destructibles' health is measured against, in kg:
/// `unitVehicleMass` in the game's `scripts/destructibles.xml`.
const UNIT_VEHICLE_MASS: f32 = 35_000.0;
/// How far the hull tilts at most, in radians, either way along it or across: its tracks
/// grip least from 32° along and 29° across, see [`SLOPE_GRIP_ALONG`]. Our choice: with a
/// corner on a rock's facet, the ground under its corners tilted it up to 66°, and so
/// tilted, the walls pushed it metres aside; it waits for a suspension.
const MAX_TILT: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// How long its turning speed takes to grow from nothing to its turning speed on the
/// spot, and to fall back, in seconds: the faster it turns, the faster it gets there.
/// Fitted: turning from rest, the T110E4 (26.8°/s) and the Type 64 (49.3°/s) both get
/// there in about 0.3 s; released, the Type 64 stops in about 0.2 s, the T110E4 within
/// two ticks.
const ROTATION_RISE: f32 = 0.3;
const ROTATION_FALL: f32 = 0.2;
/// How fast its blocked tracks slow the vehicle sliding along them, and its tracks
/// sideways, in m/s². Fitted: on the Type 64's two drifts at 20 and 16.7 m/s, its
/// velocity kept in the world as its hull swings under it, slowed by each separately,
/// follows the real one within 0.2 to 0.8 m/s for 2 to 3.5 along, 9.5 to 12 across; a
/// friction on the whole velocity, weighed by the slip angle, fits twice as badly.
const HANDBRAKE_FRICTION: f32 = 3.0;
const SIDE_FRICTION: f32 = 11.0;
/// The share of the torque its tracks would hold, skidding sideways about their middle,
/// that holds a hull against a hit's spin, see `Hull::track_friction`. Our choice, by
/// feel: in full, a steady push off its middle never turned the pusher, and one 30 km/h
/// ram in a side turned it 13°; at 0.3, the pusher turns 9° in 5 s and the ram 22°.
const SPIN_GRIP: f32 = 0.3;
/// How fast the vehicle turns with its tracks blocked: this many times its turning speed
/// on the spot, the less it slides, `1 - e^(-v / HANDBRAKE_ROTATION_SPEED)`. Fitted: the
/// Type 64 (49.3°/s on the spot) drifts up to 124°/s, then turns at 121, 115, 101, 90,
/// 74 and 35°/s as it slides at 10, 7.6, 5, 3.8, 2.7 and 1 m/s.
const HANDBRAKE_ROTATION: f32 = 2.5;
const HANDBRAKE_ROTATION_SPEED: f32 = 3.0;
/// How far from its centre the vehicle pivots, in metres, on its blocked inner track as
/// its outer one drives it. Fitted: backing up into a turn on its handbrake, the Type 64
/// settles at 1.8 m/s turning at its 41.9°/s while moving, on medium ground.
const PIVOT_RADIUS: f32 = 2.5;
/// How long its turning speed takes to grow by its turning speed on the spot with its
/// tracks blocked, in seconds. Fitted: the Type 64's drifts turn 72°/s faster each
/// second.
const HANDBRAKE_ROTATION_RISE: f32 = 0.7;
/// How fast the vehicle must slide, in m/s, for a drift to swing up as fast as
/// [`HANDBRAKE_ROTATION_RISE`] says, slower below as the square of its share of it: from
/// a crawl, its tracks blocked and turning, it hardly turns, while a drift started fast
/// keeps its swing down to a crawl, see [`HANDBRAKE_ROTATION`]. Fitted: the Type 64's
/// swing grows 1.3 to 1.5 rad/s² whether it slides at 13 or 20 m/s, and it turns 0.07 rad
/// in its first second at 1.85 m/s; the square is our choice between.
const HANDBRAKE_SWING_SPEED: f32 = 6.0;
/// How its tracks grip the terrain the steeper it is, along them and across: fully up to
/// the first angle, in degrees, least, the share given, from the second, in between as the
/// cosines go. From the game: `slopeGripLngTerrain` and `slopeGripSdwTerrain`, the
/// defaults of `physics_shared.py`, which the server's vehicle sections may override.
const SLOPE_GRIP_ALONG: (f32, f32, f32) = (27.5, 32.0, 0.1);
const SLOPE_GRIP_ACROSS: (f32, f32, f32) = (24.5, 29.0, 0.1);
/// Under this speed, in m/s, the vehicle turns as on the spot. Our choice, the split the
/// fit above uses between turning on the spot and moving.
const ON_SPOT_SPEED: f32 = 0.5;

/// The engine's relative RPM at idle, and just after shifting up. Captured: the RPM
/// byte of `ownVehicleAuxPhysicsData` in the battle capture of 2026-10-31, whose 5th
/// percentile is 0.20.
const IDLE_RPM: f32 = 0.2;
const SHIFT_RPM: f32 = 0.3;
/// The shares of the top speed the gearbox shifts up at: 4 gears. Captured, roughly: in
/// the same capture, `ownVehicleGear` shifts up to the 2nd gear at a median 0.31 of the
/// top speed and to the 4th at 0.82, while the 3rd comes anywhere (median 0.29): the
/// real gearbox (`fakegearbox`, server-only) shifts on more than the speed. The 3rd's
/// share is our choice, between the others.
const GEAR_SHIFTS: [f32; 3] = [0.27, 0.52, 0.8];

/// The ground at a position on the terrain, else [`DEFAULT_GROUND`].
pub fn ground_at(terrain: &Terrain, position: Vec3) -> Ground {
    terrain.ground_at(position.x, position.z)
        .filter(|&ground| ground != Ground::None)
        .unwrap_or(DEFAULT_GROUND)
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

/// A ground's index in a vehicle's per-ground mobility, firm, medium then soft.
fn ground_index(ground: Ground) -> usize {
    match ground {
        Ground::Firm => 0,
        Ground::Medium | Ground::None => 1,
        Ground::Soft => 2,
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
        Self { hull: Hull::new(position, direction, mobility), turret: Turret::new(aiming) }
    }

    /// Advance by `dt` seconds in its space, within the map's bounds if any: the hull
    /// moves, see [`Hull::advance`], then the turret turns. Returns whether the hull
    /// moved or turned.
    pub fn step(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &mut Rubble) -> bool {
        let moved = self.hull.advance(dt, space, bounds, rubble);
        self.turret.step(dt, &self.hull);
        moved
    }

    /// Where its chassis, hull, turret and gun are in the world, as transforms from
    /// each part's frame, the hull sitting at `hull_position` on the chassis.
    pub fn part_transforms(&self, hull_position: Vec3) -> [Affine3A; 4] {
        let aiming = &self.turret.aiming;
        let chassis = Affine3A::from_rotation_translation(self.hull.rotation(), self.hull.position);
        let hull = chassis * Affine3A::from_translation(hull_position);
        let turret = chassis * Affine3A::from_rotation_translation(Quat::from_rotation_y(self.turret.yaw), aiming.turret_position);
        let gun = turret * Affine3A::from_rotation_translation(Quat::from_rotation_x(self.turret.pitch), aiming.gun_position);
        [chassis, hull, turret, gun]
    }

}

/// A vehicle's hull, moving as the client drives it.
#[derive(Debug, Clone)]
pub struct Hull {
    pub position: Vec3,
    /// Roll, pitch and yaw, in radians, the order position updates carry them. A yaw
    /// of 0 faces +Z, and turning right increases it.
    pub direction: Vec3,
    /// Signed speed along the vehicle's heading, in m/s.
    pub speed: f32,
    /// Signed speed across its heading, positive to the right, in m/s: it slides with
    /// its tracks blocked, then its tracks hold it again.
    pub slide: f32,
    /// Turning speed, in rad/s, positive to the right.
    pub yaw_rate: f32,
    /// Turning speed hits gave it on top, in rad/s: its tracks don't drive it but hold it,
    /// see [`Self::spin_friction`].
    pub spin: f32,
    /// The latest `vehicle_moveWith` flags.
    pub flags: u8,
    pub mobility: MobilityDef,
}

impl Hull {

    pub fn new(position: Vec3, direction: Vec3, mobility: MobilityDef) -> Self {
        Self { 
            position, 
            direction, 
            speed: 0.0, 
            slide: 0.0, 
            yaw_rate: 0.0, 
            spin: 0.0, 
            flags: 0, 
            mobility,
        }
    }

    /// The speed the flags ask for: the top speed in their direction, reduced by cruise
    /// control, and by turning.
    fn target_speed(&self) -> f32 {
        
        use movement_flag::*;
        
        let share = if self.flags & CRUISE_CONTROL25 != 0 {
            0.25
        } else if self.flags & CRUISE_CONTROL50 != 0 {
            0.5
        } else {
            1.0
        };

        let share = if self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0 { share * SPEED_WHILE_TURNING } else { share };
        if self.flags & FORWARD != 0 {
            self.mobility.speed_forward * share
        } else if self.flags & BACKWARD != 0 {
            -self.mobility.speed_backward * share
        } else {
            0.0
        }

    }

    /// Advance by `dt` seconds in its space. Returns whether the hull moved or turned.
    ///
    /// ```text
    ///   step(ground under it)      the drive model: speeds, turning, new position
    ///        ▼   (only if it moved)
    ///   push_out                   out of the solid static models, speed into them lost
    ///        ▼
    ///   keep_within(bounds)        inside the map, speed into the edge lost
    ///        ▼   (only if it moved)
    ///   settle                     back on the ground, rolled and pitched with it
    /// ```
    ///
    /// The order matters: a wall can push the hull off the terrain it was on, so the
    /// ground is looked up last.
    pub fn advance(&mut self, dt: f32, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &mut Rubble) -> bool {
        let moved = self.step(dt, ground_at(&space.terrain, self.position));
        if moved {
            self.ram(dt, &space.statics, rubble);
            self.push_out(&space.statics, rubble);
        }
        if let Some(bounds) = bounds {
            self.keep_within(bounds);
        }
        if moved {
            self.settle(space);
        }
        moved
    }

    /// Put it back where the map holds it after it was shoved by another vehicle: out of
    /// the static models, within the map's bounds, and on the ground, as the end of
    /// [`Self::advance`] does. A vehicle is never shoved through a wall or off the map.
    pub fn keep_on_map(&mut self, space: &Space, bounds: Option<(Vec2, Vec2)>, rubble: &Rubble) {
        self.push_out(&space.statics, rubble);
        if let Some(bounds) = bounds {
            self.keep_within(bounds);
        }
        self.settle(space);
    }

    /// Push its collision box out of the solid static models it overlaps, on the ground
    /// plane, see [`Self::overlaps`]: their triangles within [`WALL_ANGLE`] of vertical,
    /// the others being ground, between [`STEP_HEIGHT`] above its bottom and its top,
    /// the deepest overlap first, [`WALL_PASSES`] at most. What of its velocity goes into
    /// a wall stops, at its corner in the wall, which turns it, and slows the rest by
    /// [`WALL_FRICTION`].
    pub fn push_out(&mut self, statics: &StaticScene, rubble: &Rubble) {

        for _ in 0..WALL_PASSES {

            let push = self.overlaps(statics, rubble)
                .map(|(_, push)| push)
                .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
            let Some(push) = push else { break };

            self.position += Vec3::new(push.x, 0.0, push.y);
            let normal = push.normalize();

            // Hit where it pushed into the wall: a corner swings it round, an edge flat
            // against it doesn't. The wall doesn't move, as if infinitely heavy.
            let footprint = self.footprint_at(Pose::of(self));
            let arm = wall_contact(&footprint.corners(), normal) - footprint.middle;
            let into = self.point_velocity(arm).dot(normal);
            if into < 0.0 {
                let mass = self.mobility.weight;
                let inertia = self.yaw_inertia();
                let lever = arm.perp_dot(normal);
                let impulse = -into / (1.0 / mass + lever * lever / inertia);
                self.hit(normal * impulse, arm, mass, inertia);
                // It scrapes along the wall.
                let normal = Vec3::new(normal.x, 0.0, normal.y);
                let mut velocity = self.velocity();
                let along = velocity - normal * velocity.dot(normal);
                velocity -= along.clamp_length_max(WALL_FRICTION * -into);
                self.set_velocity(velocity);
            }

        }

    }

    /// The solid static triangles its collision box overlaps, see [`Self::push_out`],
    /// each with how far to push it out of it on the ground plane. They are tested in its
    /// own frame, tilted with it: nose up, its front clears higher.
    ///
    /// ```text
    ///   for each wall-like triangle near the hull (found in the grid by the box's reach):
    ///     1. move its 3 vertices into the hull's frame            (inverse rotation)
    ///     2. clip it to the band [bottom + STEP_HEIGHT, top]      (clip_to_heights)
    ///     3. what is left seen from above is a convex polygon     (≤ 5 vertices)
    ///     4. separate the hull's rectangle from it, least axis    (box_push)
    ///     5. bring the push back out of the hull's frame
    /// ```
    pub fn overlaps<'a>(&self, statics: &'a StaticScene, rubble: &'a Rubble) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {
        let (min, max) = self.reach();
        let solid = statics.triangles_in(BodyKind::Solid, min, max);
        let standing = statics.triangles_in(BodyKind::Destructible, min, max)
            .filter(move |triangle| rubble.blocks(statics, triangle));
        self.overlapping(solid.chain(standing))
    }

    /// The destructible triangles still standing that its collision box overlaps, see
    /// [`Self::overlaps`].
    fn standing_overlaps<'a>(&self, statics: &'a StaticScene, rubble: &'a Rubble) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {
        let (min, max) = self.reach();
        let standing = statics.triangles_in(BodyKind::Destructible, min, max)
            .filter(move |triangle| rubble.standing(statics, triangle).is_some());
        self.overlapping(standing)
    }

    /// The box around it its collision box may reach, however it is tilted.
    fn reach(&self) -> (Vec3, Vec3) {
        let [min, max] = self.mobility.bounds;
        let reach = Vec3::splat(min.abs().max(max.abs()).length());
        (self.position - reach, self.position + reach)
    }

    /// Those of these triangles its collision box overlaps, see [`Self::overlaps`].
    fn overlapping<'a>(&self, triangles: impl Iterator<Item = &'a StaticTriangle> + 'a) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {

        let (position, rotation) = (self.position, self.rotation());
        let inverse = rotation.inverse();
        let [min, max] = self.mobility.bounds;
        let center = (min + max) / 2.0;
        let half = (max - min) / 2.0;
        let low = STEP_HEIGHT.max(min.y);
        let high = max.y;

        triangles
            .filter(|triangle| triangle.normal.y.abs() < WALL_ANGLE.sin())
            .filter_map(move |triangle| {
                let local = triangle.vertices.map(|vertex| inverse * (vertex - position));
                let outline = clip_to_heights(&local, low, high)?;
                let push = box_push(center.xz(), Vec2::Y, Vec2::X, half.xz(), &outline)?;
                Some((triangle, (rotation * Vec3::new(push.x, 0.0, push.y)).xz()))
            })

    }

    /// Damage the destructibles it drives into, see [`RAM_DAMAGE`], and break those
    /// whose health it took: it goes on through, less the speed their health took; those
    /// still standing stop it as walls do, see [`Self::push_out`]. Its damage adds up
    /// from a tick to the next, so a crawl through a fence takes a few, and so does
    /// pressing against one, see [`RAM_PRESS`].
    fn ram(&mut self, dt: f32, statics: &StaticScene, rubble: &mut Rubble) {

        // Each piece once, its first triangle telling the way into it: against its push
        // out, else, a small one it drove onto within a tick having no way out of it, the
        // way from its middle to the triangle's.
        let footprint = self.footprint_at(Pose::of(self));
        let middle = footprint.middle;
        let mut hits: SmallVec<[(Piece, &PlacedDestructible, Vec2, Vec2); 8]> = SmallVec::new();
        for (triangle, push) in self.standing_overlaps(statics, rubble) {
            let Some((piece, destructible)) = rubble.standing(statics, triangle) else { continue };
            let center = (triangle.vertices[0] + triangle.vertices[1] + triangle.vertices[2]).xz() / 3.0;
            if !hits.iter().any(|&(hit, _, _, _)| hit == piece) {
                hits.push((piece, destructible, -push, center - middle));
            }
        }

        let weight = self.mobility.weight / UNIT_VEHICLE_MASS;
        for (piece, destructible, against, center) in hits {
            let velocity = self.velocity();
            // Where it touches it: its corner deepest in it, else, one it drove onto, its
            // point nearest the triangle's middle. Turning on the spot swings it in too.
            let arm = match against.try_normalize() {
                Some(against) => wall_contact(&footprint.corners(), -against) - middle,
                None => {
                    let local = Vec2::new(center.dot(footprint.right), center.dot(footprint.heading));
                    let local = local.clamp(-footprint.half, footprint.half);
                    footprint.right * local.x + footprint.heading * local.y
                }
            };
            let point = self.point_velocity(arm);
            let closing = [against, center].into_iter()
                .filter_map(Vec2::try_normalize)
                .any(|toward| point.dot(toward) > 0.0);
            if !closing {
                continue;
            }
            // Grazing one breaks it too: the real T110E4 drove on along a fence.
            let speed = point.length();
            let energy = RAM_DAMAGE * weight * speed * speed / 2.0;
            let Some((health, done)) = rubble.damage(destructible, piece, energy + RAM_PRESS * weight * dt) else { continue };
            if done < health {
                continue;
            }
            // It loses a little of its energy: see `RAM_COST`.
            let left = (health - (done - energy)).max(0.0);
            let kept = (speed * speed - 2.0 * RAM_COST * left / weight).max(0.0).sqrt();
            if speed > 0.0 {
                self.set_velocity(velocity * (kept / speed));
            }
            rubble.break_piece(destructible, piece, velocity.x.atan2(velocity.z), speed);
        }

    }

    /// Keep its centre within the map's bounds, [`BOUNDS_MARGIN`] inside them: what of
    /// its velocity goes into an edge stops, and slows the rest by [`EDGE_FRICTION`].
    /// Driven into an edge at an angle, it drags along it: its tracks keep pushing it in,
    /// and lose that share again each step.
    pub fn keep_within(&mut self, (min, max): (Vec2, Vec2)) {
        let mut velocity = self.velocity();
        let mut stopped = Vec3::ZERO;
        for (axis, (least, most)) in [(0, (min.x, max.x)), (2, (min.y, max.y))] {
            let (least, most) = (least + BOUNDS_MARGIN, most - BOUNDS_MARGIN);
            let into = if self.position[axis] < least {
                self.position[axis] = least;
                velocity[axis].min(0.0)
            } else if self.position[axis] > most {
                self.position[axis] = most;
                velocity[axis].max(0.0)
            } else {
                continue;
            };
            stopped[axis] = into;
            velocity[axis] -= into;
        }
        if stopped == Vec3::ZERO {
            return;
        }
        // Along the edges, slowed as much as their friction takes of what they stopped.
        let along = velocity - velocity.project_onto(stopped);
        velocity -= along.clamp_length_max(EDGE_FRICTION * stopped.length());
        self.set_velocity(velocity);
    }

    /// Advance by `dt` seconds on this ground. Returns whether the hull moved or turned.
    ///
    /// Driving its way, the engine's power accelerates it, as much as it is slow, up to
    /// [`MAX_ACCELERATION`], against the ground's resistance and the slope, up to the
    /// speed the flags ask for. Without controls it stops at [`COASTING`], and against
    /// them it brakes at [`BRAKING`]. It turns at its ground's turning speed on the spot,
    /// less while moving, reaching it in [`ROTATION_RISE`].
    ///
    /// With its tracks blocked, it slides where it was going as its hull swings under it,
    /// slowed by [`HANDBRAKE_FRICTION`] along its tracks and [`SIDE_FRICTION`] across
    /// them, and turns faster the faster it slides, see [`HANDBRAKE_ROTATION`]. Driven
    /// into a turn, it pivots on its blocked inner track, see [`PIVOT_RADIUS`].
    pub fn step(&mut self, dt: f32, ground: Ground) -> bool {

        use movement_flag::*;

        let ground = ground_index(ground);
        let rotation_speed = self.mobility.rotation_speed[ground];
        let handbrake = self.flags & BLOCK_TRACKS != 0;

        let mut turn: f32 = match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
            ROTATE_LEFT => -1.0,
            ROTATE_RIGHT => 1.0,
            _ => 0.0,
        };
        // Backing up steers like a car: left swings the rear to the left.
        if self.flags & BACKWARD != 0 {
            turn = -turn;
        }

        // On its handbrake, driving into a turn pivots it on its blocked inner track,
        // its outer one carrying it around at this speed.
        let drive_way = match self.flags & (FORWARD | BACKWARD) {
            FORWARD => 1.0,
            BACKWARD => -1.0,
            _ => 0.0,
        };
        let pivot = drive_way * turn.abs() * rotation_speed * ROTATION_ON_MOVE * PIVOT_RADIUS;

        let target = if handbrake { 0.0 } else { self.target_speed() };
        let way = target.signum();

        // Gravity down the slope, pitched positively nose down and rolled positively
        // right side up, which the tracks hold against as much as they grip it.
        let roll = self.direction.x;
        let pitch = self.direction.y;
        let grip_along = grip(pitch, SLOPE_GRIP_ALONG);
        let grip_across = grip(roll, SLOPE_GRIP_ACROSS);
        let slope = GRAVITY * pitch.sin();
        let (slide_friction, spin_friction) = self.track_friction(grip_across);
        self.slide = approach(self.slide - GRAVITY * roll.sin() * dt, 0.0, slide_friction * dt);
        self.speed = 
            if handbrake {
                approach(self.speed + slope * dt, pivot, HANDBRAKE_FRICTION * grip_along * dt)
            } else if target == 0.0 {
                approach(self.speed + slope * dt, 0.0, COASTING * grip_along * dt)
            } else if self.speed * way < 0.0 {
                approach(self.speed + slope * dt, 0.0, BRAKING * grip_along * dt)
            } else {
                // Along its way: the drive, less the ground's resistance, and the slope.
                let mobility = &self.mobility;
                let along = self.speed.abs();
                let power = POWER_FACTOR * mobility.engine_power / mobility.weight;
                let drive = (power / along.max(1e-3)).min(MAX_ACCELERATION) * grip_along;
                let turning = if self.flags & (ROTATE_LEFT | ROTATE_RIGHT) != 0 { TURNING_RESISTANCE } else { 0.0 };
                let resistance = mobility.rolling_friction[ground] * GRAVITY + turning;
                let along = if along > target.abs() {
                    approach(along, target.abs(), BRAKING * grip_along * dt)
                } else {
                    // Short of power it stalls, unless the slope takes it back down past
                    // what its tracks hold.
                    let floor = if slope * way + BRAKING * grip_along >= 0.0 { 0.0 } else { f32::NEG_INFINITY };
                    (along + (drive - resistance + slope * way) * dt).clamp(floor, target.abs())
                };
                along * way
            };

        let (goal, rise) = if handbrake {
            // Sliding faster than it pivots swings it round faster than it drives.
            let sliding = (self.speed.hypot(self.slide) - pivot.abs()).max(0.0);
            let share = 1.0 - (-sliding / HANDBRAKE_ROTATION_SPEED).exp();
            let drifting = rotation_speed * HANDBRAKE_ROTATION * share;
            let pivoting = if pivot != 0.0 { rotation_speed * ROTATION_ON_MOVE } else { 0.0 };
            // A drift swings up only as fast as it slides, a pivot as its outer track drives.
            let swing = (sliding / HANDBRAKE_SWING_SPEED).powi(2).min(1.0);
            let rise = if drifting > pivoting { HANDBRAKE_ROTATION_RISE / swing.max(1e-3) } else { HANDBRAKE_ROTATION_RISE };
            (turn * drifting.max(pivoting), rise)
        } else {
            let moving = self.speed.abs() >= ON_SPOT_SPEED || target != 0.0;
            (turn * rotation_speed * if moving { ROTATION_ON_MOVE } else { 1.0 }, ROTATION_RISE)
        };
        let time = if goal.abs() > self.yaw_rate.abs() && goal * self.yaw_rate >= 0.0 { rise } else { ROTATION_FALL };
        let rate = rotation_speed / time;

        let velocity = self.velocity();
        self.yaw_rate = approach(self.yaw_rate, goal, rate * dt);
        // Its tracks hold a spin before it turns: one too weak to skid them doesn't.
        self.spin = approach(self.spin, 0.0, spin_friction * dt);
        self.direction.z = wrap_angle(self.direction.z + (self.yaw_rate + self.spin) * dt);
        // Its tracks blocked, it keeps going where it went as its hull turns, else its
        // tracks carry its speed around.
        if handbrake {
            let (heading, right) = self.axes();
            self.speed = velocity.dot(heading);
            self.slide = velocity.dot(right);
        }
        self.position += self.velocity() * dt;

        self.speed != 0.0 || self.slide != 0.0 || self.yaw_rate != 0.0 || self.spin != 0.0

    }

    /// Its heading and its right on the ground plane.
    fn axes(&self) -> (Vec3, Vec3) {
        let yaw = self.direction.z;
        (Vec3::new(yaw.sin(), 0.0, yaw.cos()), Vec3::new(yaw.cos(), 0.0, -yaw.sin()))
    }

    /// Its velocity on the ground plane, in m/s.
    pub fn velocity(&self) -> Vec3 {
        let (heading, right) = self.axes();
        heading * self.speed + right * self.slide
    }

    /// How fast the point at `arm` from its middle moves on the ground plane, in m/s: its
    /// velocity and its yaw rate's.
    fn point_velocity(&self, arm: Vec2) -> Vec2 {
        self.velocity().xz() + Vec2::new(arm.y, -arm.x) * (self.yaw_rate + self.spin)
    }

    /// Its inertia against turning on the ground plane, in kg·m²: that of a box of its
    /// weight over its collision box's width and length.
    fn yaw_inertia(&self) -> f32 {
        let [min, max] = self.mobility.bounds;
        let size = max.xz() - min.xz();
        self.mobility.weight * size.length_squared() / 12.0
    }

    /// How fast its tracks stop its slide, in m/s², and its spin, in rad/s², as much as
    /// they grip. Each point of its tracks, `z` along them from their middle, skids
    /// sideways at `slide + spin · z` against `friction`, [`SIDE_FRICTION`]: the real
    /// server stops a hull sliding along the map's edge, its tracks free, as fast as one
    /// drifting on its handbrake, see `stops_at_the_map_edge`.
    ///
    /// ```text
    ///   turning point   z₀ = −slide / spin,   r = |z₀| / (l/2)
    ///
    ///   r ≥ 1   all the tracks skid one way:   slide  →  0  at  friction
    ///   ┃▶▶▶▶▶┃                                 spin   free
    ///   r < 1   they skid both ways:           slide  →  0  at  friction · r
    ///   ┃◀◀·▶▶┃                                 spin   →  0  at  spin_max · (1 − r²)
    ///
    ///   spin_max = SPIN_GRIP · friction · 3·l / (w² + l²)
    /// ```
    ///
    /// `spin_max` is the torque `μ·m·g·l/4` of tracks of length `l` skidding about their
    /// middle against its yaw inertia `m (w² + l²) / 12` (see [`Self::yaw_inertia`]), of
    /// which [`SPIN_GRIP`] holds: about 1.1 rad/s² for a Leopard 1. So a hull that slides
    /// turns freely, as one rammed does, and one at rest holds a weak push's turn a
    /// little.
    fn track_friction(&self, grip_across: f32) -> (f32, f32) {
        let [min, max] = self.mobility.bounds;
        let size = max.xz() - min.xz();
        let half_length = size.y / 2.0;
        let ratio = if self.spin.abs() < 1e-6 { f32::INFINITY } else { self.slide.abs() / (self.spin.abs() * half_length) };
        let friction = SIDE_FRICTION * grip_across;
        let spin_max = SPIN_GRIP * friction * 3.0 * size.y / size.length_squared();
        (friction * ratio.min(1.0), spin_max * (1.0 - ratio * ratio).max(0.0))
    }

    /// Take an impulse, in kg·m/s, at `arm` from its middle: its velocity and its spin
    /// change.
    fn hit(&mut self, impulse: Vec2, arm: Vec2, mass: f32, inertia: f32) {
        let velocity = self.velocity() + Vec3::new(impulse.x, 0.0, impulse.y) / mass;
        self.set_velocity(velocity);
        self.spin -= arm.perp_dot(impulse) / inertia;
    }

    /// Set its speeds along and across its heading from a velocity on the ground plane.
    fn set_velocity(&mut self, velocity: Vec3) {
        let (heading, right) = self.axes();
        self.speed = velocity.dot(heading);
        self.slide = velocity.dot(right);
    }

    /// Let it go on for `time` seconds at the speed and turning it has, its tracks holding
    /// its spin first, as in [`Self::step`].
    fn coast(&mut self, time: f32) {
        let grip_across = grip(self.direction.x, SLOPE_GRIP_ACROSS);
        let (_, spin_friction) = self.track_friction(grip_across);
        self.spin = approach(self.spin, 0.0, spin_friction * time);
        self.position += self.velocity() * time;
        self.direction.z = wrap_angle(self.direction.z + (self.yaw_rate + self.spin) * time);
    }

    /// Put it at a pose.
    fn place(&mut self, pose: Pose) {
        self.position = pose.position;
        self.direction = pose.direction;
    }

    /// Its collision box in space.
    fn obb(&self) -> Obb {
        self.obb_at(Pose::of(self))
    }

    /// Its collision box in space, were it at this pose.
    fn obb_at(&self, pose: Pose) -> Obb {
        let axes = Mat3::from_quat(rotation_of(pose.direction));
        let [min, max] = self.mobility.bounds;
        Obb {
            center: pose.position + axes * ((min + max) / 2.0),
            axes,
            half: (max - min) / 2.0,
        }
    }

    /// Its collision box on the ground plane, upright, were it at this pose: where it
    /// touches another, see [`overlap_center`].
    fn footprint_at(&self, pose: Pose) -> Footprint {
        let (sin, cos) = pose.direction.z.sin_cos();
        let heading = Vec2::new(sin, cos);
        let right = Vec2::new(cos, -sin);
        let [min, max] = self.mobility.bounds;
        let center = (min + max) / 2.0;
        let half = (max - min) / 2.0;
        Footprint {
            middle: pose.position.xz() + heading * center.z + right * center.x,
            heading,
            right,
            half: Vec2::new(half.x, half.z),
        }
    }

    /// The vehicle's `engineMode`, which the client's track scroll follows: idle (1)
    /// without flags, accelerating (3) until at the speed the flags ask for, then
    /// cruising (2), and the flags themselves.
    pub fn engine_mode(&self) -> [u8; 2] {
        
        let mode = if self.flags == 0 {
            1
        } else if (self.target_speed() - self.speed).abs() > 0.01 {
            3
        } else {
            2
        };

        [mode, self.flags]

    }

    /// The engine's gear and its RPM relative to its maximum, which the client's engine
    /// sound follows. As the captured server does: in neutral (gear 0) at idle RPM when
    /// still, else the speed's share of the top speed picks one of [`GEAR_SHIFTS`]'s
    /// gears, the RPM going from [`SHIFT_RPM`] to 1 across it; turning in place, the RPM
    /// follows the turn rate in first gear, against its fastest, on firm ground.
    pub fn engine(&self) -> (u8, f32) {

        let top_speed = if self.speed < 0.0 { self.mobility.speed_backward } else { self.mobility.speed_forward };
        let share = (self.speed.abs() / top_speed.max(0.1)).min(1.0);
        let turn_share = (self.yaw_rate.abs() / self.mobility.rotation_speed[0].max(0.01)).min(1.0);
        if share < 0.02 {
            if turn_share < 0.02 {
                return (0, IDLE_RPM);
            }
            return (1, SHIFT_RPM + (1.0 - SHIFT_RPM) * turn_share);
        }

        let gear = GEAR_SHIFTS.iter().take_while(|&&shift| share >= shift).count();
        let low = if gear == 0 { 0.0 } else { GEAR_SHIFTS[gear - 1] };
        let high = GEAR_SHIFTS.get(gear).copied().unwrap_or(1.0);
        let in_gear = ((share - low) / (high - low)).clamp(0.0, 1.0);
        (gear as u8 + 1, SHIFT_RPM + (1.0 - SHIFT_RPM) * in_gear)
        
    }

    /// The hull's orientation: yaw about Y, then pitch about X (nose down), then roll
    /// about Z (right side up), as the client's matrices compose them.
    pub fn rotation(&self) -> Quat {
        rotation_of(self.direction)
    }

    /// The hull's orientation and its tracks' speeds, as the avatar's
    /// `ownVehicleAuxPhysicsData` carries them for the client to aim from and animate the
    /// tracks with, see `WoT.packAuxVehiclePhysicsData`. From the lowest bit: yaw on 15
    /// bits over a turn, pitch on 12 bits over ±π/2, roll on 13 bits over a turn, the left
    /// then right track speeds on 8 bits each over -15..30 m/s, and the engine's relative
    /// RPM on 8 bits over 0..1.2, see [`Self::engine`].
    pub fn aux_physics_data(&self) -> u64 {
        
        use std::f32::consts::FRAC_PI_2;
        use movement_flag::*;
        
        let (left, right) = if self.flags & BLOCK_TRACKS != 0 {
            // On its handbrake, its inner track blocked and its outer one carrying it as
            // fast as it slides, as the real server's show; both blocked going straight.
            let outer = self.speed.hypot(self.slide);
            match self.flags & (ROTATE_LEFT | ROTATE_RIGHT) {
                ROTATE_LEFT => (0.0, outer),
                ROTATE_RIGHT => (outer, 0.0),
                _ => (0.0, 0.0),
            }
        } else {
            // The inner track goes slower, the outer one faster.
            let turning = self.yaw_rate * self.mobility.carrying_point.x;
            (self.speed + turning, self.speed - turning)
        };

        let roll = self.direction.x;
        let pitch = self.direction.y;
        let yaw = self.direction.z;
        pack_range(self.engine().1, 0.0, 1.2, 8) << 56
            | pack_range(right, -15.0, 30.0, 8) << 48
            | pack_range(left, -15.0, 30.0, 8) << 40
            | pack_angle(roll, 13) << 27
            | pack_range(pitch, -FRAC_PI_2, FRAC_PI_2, 12) << 15
            | pack_angle(yaw, 15)

    }

    /// Rest on the ground: at its height, rolled and pitched with its slope under the
    /// corners its tracks rest on, see `MobilityDef::carrying_point`, the ground being
    /// the terrain or a static model on it, see [`ground_height`]. Off the ground, the
    /// hull stays as it is.
    ///
    /// The tilt comes from the average slope on each axis, which is more stable than a
    /// plane through three of the points and ignores a single bump under one corner:
    ///
    /// ```text
    ///   across: (heights on the right side − heights on the left side) / (2 · 2cx)
    ///   along:  (heights at the front − heights at the back)           / (2 · 2cy)
    /// ```
    pub fn settle(&mut self, space: &Space) {

        let (heading, right) = self.axes();
        let carrying = self.mobility.carrying_point;
        let top = self.position.y + STEP_HEIGHT;

        // Its centre, then its front right, front left, back right and back left corners.
        let mut heights = [0.0; 5];
        let corners = [(0.0, 0.0), (1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)];
        for (height, (along, across)) in heights.iter_mut().zip(corners) {
            let at = self.position + heading * along * carrying.y + right * across * carrying.x;
            let Some(ground) = ground_height(space, at.xz(), top) else { return };
            *height = ground;
        }

        let [center, front_right, front_left, back_right, back_left] = heights;
        let front = front_right + front_left;
        let back = back_right + back_left;
        let right_side = front_right + back_right;
        let left_side = front_left + back_left;
        self.position.y = center;
        // Rolled positively with its right side up, pitched positively nose down, no
        // steeper than it can rest: a corner on a rock's facet doesn't tip it over.
        let roll = ((right_side - left_side) / (4.0 * carrying.x)).atan();
        let pitch = -((front - back) / (4.0 * carrying.y)).atan();
        self.direction.x = roll.clamp(-MAX_TILT, MAX_TILT);
        self.direction.y = pitch.clamp(-MAX_TILT, MAX_TILT);

    }

}

/// A destructible piece, as the client numbers it: all of a destructible, or one of a
/// structure's modules by its material kind. A destructible can be placed as several
/// models, a house's walls and roof say, each a piece of it alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Piece {
    /// Its chunk's id and its index in it, see [`PlacedDestructible`].
    pub chunk_id: u16,
    pub index: u16,
    pub module: Option<u8>,
}

/// A destructible a vehicle broke, as its clients are told of it.
#[derive(Debug, Clone, Copy)]
pub struct Breakage {
    pub piece: Piece,
    pub kind: DestructibleKind,
    /// Which way it falls, the yaw the vehicle went, in radians.
    pub yaw: f32,
    /// How fast the vehicle went, in m/s.
    pub speed: f32,
}

/// The destructibles broken in a world, and those broken since last taken, and the
/// damage those still standing took.
#[derive(Debug, Default)]
pub struct Rubble {
    broken: HashSet<Piece>,
    fresh: Vec<Breakage>,
    damage: HashMap<Piece, f32>,
}

impl Rubble {

    /// The piece a static triangle is of, if it is destructible and still stands.
    fn standing<'a>(&self, statics: &'a StaticScene, triangle: &StaticTriangle) -> Option<(Piece, &'a PlacedDestructible)> {
        let destructible = statics.destructible(triangle.instance)?;
        let module = match destructible.kind {
            DestructibleKind::Structure => Some(destructible.module(triangle.material_kind as u8)?.material_kind),
            _ => None,
        };
        let piece = Piece {
            chunk_id: destructible.chunk_id,
            index: destructible.index,
            module,
        };
        (!self.broken.contains(&piece)).then_some((piece, destructible))
    }

    /// Whether a static triangle is of a structure's module still standing: those stop
    /// what drives into them until it breaks them, while a fence or a post is driven
    /// into as it breaks, see `Hull::ram`.
    fn blocks(&self, statics: &StaticScene, triangle: &StaticTriangle) -> bool {
        self.standing(statics, triangle).is_some_and(|(piece, _)| piece.module.is_some())
    }

    /// Damage a piece: its health, and the damage it took in all.
    fn damage(&mut self, destructible: &PlacedDestructible, piece: Piece, damage: f32) -> Option<(f32, f32)> {
        let health = match piece.module {
            Some(module) => destructible.module(module)?.health,
            None => destructible.health,
        };
        let done = self.damage.entry(piece).or_default();
        *done += damage;
        Some((health, *done))
    }

    /// Break a piece, a module the others it brings down with it, each told of.
    fn break_piece(&mut self, destructible: &PlacedDestructible, piece: Piece, yaw: f32, speed: f32) {
        let depends = piece.module
            .and_then(|module| destructible.module(module))
            .map(|module| module.depends.as_slice())
            .unwrap_or_default();
        let brought = depends.iter().map(|&module| Piece {
            module: Some(module),
            ..piece
        });
        for piece in std::iter::once(piece).chain(brought) {
            if self.broken.insert(piece) {
                self.fresh.push(Breakage {
                    piece,
                    kind: destructible.kind,
                    yaw,
                    speed,
                });
            }
        }
    }

}

/// A hull's place on the ground plane, as its box sweeps it: its position and its yaw.
#[derive(Debug, Clone, Copy)]
struct Pose {
    position: Vec3,
    /// Roll, pitch and yaw.
    direction: Vec3,
}

impl Pose {

    fn of(hull: &Hull) -> Self {
        Self { 
            position: hull.position, 
            direction: hull.direction,
        }
    }

    /// The pose a share `t` of the way to another, turning the short way round.
    fn lerp(self, to: Self, t: f32) -> Self {
        Self { 
            position: self.position.lerp(to.position, t), 
            direction: self.direction + Vec3::new(
                wrap_angle(to.direction.x - self.direction.x),
                wrap_angle(to.direction.y - self.direction.y),
                wrap_angle(to.direction.z - self.direction.z),
            ) * t,
        }
    }

}

/// A hull's collision box in space, as tilted as the hull is: its middle, its axes (right,
/// up, heading) and its half sizes along them.
#[derive(Debug, Clone, Copy)]
struct Obb {
    center: Vec3,
    axes: Mat3,
    half: Vec3,
}

impl Obb {

    /// How far it reaches along an axis from its middle.
    fn reach(&self, axis: Vec3) -> f32 {
        self.half.x * self.axes.x_axis.dot(axis).abs()
            + self.half.y * self.axes.y_axis.dot(axis).abs()
            + self.half.z * self.axes.z_axis.dot(axis).abs()
    }

    /// The way it overlaps another least, by the separating axis theorem over their face
    /// normals and their edge pairs: the axis, pointing from the other box to this one,
    /// and how far they overlap along it. None if they don't overlap.
    ///
    /// For each of 15 axes (3 + 3 faces, 3 × 3 edge crossings) the boxes' shadows on it
    /// are intervals centred on the projection of their centres, with half lengths
    /// [`Self::reach`]; their overlap on it is
    /// `reach(A) + reach(B) − |(centre A − centre B) · axis|`. One axis with a gap
    /// proves the boxes apart; otherwise the axis with the smallest overlap is the
    /// shortest way out. Edge crossings of near-parallel edges are noisy (and null if
    /// parallel, skipped), so they count [`EDGE_AXIS_BIAS`] more than faces.
    fn separation(&self, other: &Self) -> Option<(Vec3, f32)> {

        let offset = self.center - other.center;

        let own = [
            self.axes.x_axis,
            self.axes.y_axis,
            self.axes.z_axis,
        ];
        let others = [
            other.axes.x_axis,
            other.axes.y_axis,
            other.axes.z_axis,
        ];

        let mut least: Option<(Vec3, f32)> = None;

        for index in 0..15 {

            let (axis, bias) = match index {
                0..3 => (own[index], 0.0),
                3..6 => (others[index - 3], 0.0),
                _ => (own[(index - 6) / 3].cross(others[(index - 6) % 3]), EDGE_AXIS_BIAS),
            };
            let length = axis.length();
            if length < 1e-4 {
                continue;
            }

            let axis = axis / length;
            let along = offset.dot(axis);
            let overlap = self.reach(axis) + other.reach(axis) - along.abs();
            if overlap <= 0.0 {
                return None;
            }

            if least.is_none_or(|(_, least)| overlap + bias < least) {
                least = Some((if along < 0.0 { -axis } else { axis }, overlap + bias));
            }

        }

        least.map(|(axis, overlap)| (axis, overlap.max(0.0)))

    }

    /// How far to push it on the ground plane to clear another, see [`Self::separation`]:
    /// none if they don't overlap, or if one is on top of the other, whose weight its
    /// ground holds.
    fn push(&self, other: &Self) -> Option<Vec2> {
        let (normal, depth) = self.separation(other)?;
        let across = normal.xz().length();
        if normal.y.abs() > STACKED_NORMAL || across < 1e-3 {
            return None;
        }
        Some(normal.xz() / across * (depth / across))
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

/// A hull and where it was at the start of its step, to sweep its box over it.
struct Sweep<'a> {
    hull: &'a Hull,
    from: Pose,
    to: Pose,
}

impl Sweep<'_> {

    /// Its box a share `t` of the way along its step.
    fn obb(&self, t: f32) -> Obb {
        self.hull.obb_at(self.from.lerp(self.to, t))
    }

}

/// The share of the step, from 0 to 1, at which two sweeping boxes first touch, found by
/// halving: 1 if they already touched where they started, which is then no use, or if
/// they touch last.
///
/// Each box is moved along a straight line from its pose at the start of the step to its
/// pose at the end (the position and the three angles in between), which is exact enough
/// for 0.1 s. Not touching at 0 and touching at 1, the interval is halved
/// [`FIRST_TOUCH_HALVINGS`] times, keeping the half where the boxes go from free to
/// touching:
///
/// ```text
///   t:  0 ─────────────── 1       free … touching
///       0 ────── ½ ────── 1       ½ touches   → keep [0, ½]
///       0 ── ¼ ── ½               ¼ is free   → keep [¼, ½]
///            ¼ ─ ⅜ ─ ½            …
/// ```
///
/// This is approximate when the boxes touch, part and then leave, again within a step,
/// which a tick is too short for.
fn first_touch(a: &Sweep, b: &Sweep) -> f32 {
    if a.obb(0.0).push(&b.obb(0.0)).is_some() {
        return 1.0;
    }
    let mut free = 0.0;
    let mut touching = 1.0;
    for _ in 0..FIRST_TOUCH_HALVINGS {
        let middle = (free + touching) / 2.0;
        if a.obb(middle).push(&b.obb(middle)).is_some() {
            touching = middle;
        } else {
            free = middle;
        }
    }
    touching
}

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
/// which push each other. A step allocates nothing. See the module documentation for what
/// a step does, in order.
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

    /// The destructibles broken since last taken, for the battle to tell its clients.
    pub fn take_breakages(&mut self) -> Drain<'_, Breakage> {
        self.rubble.fresh.drain(..)
    }

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

    /// Remember where every vehicle is, as a step starts: [`Self::collide`] finds from
    /// there where vehicles first touched, so that fast ones don't get into each other.
    pub fn remember(&mut self) {
        for (_, placed) in self.vehicles.iter_mut() {
            placed.pose = Pose::of(&placed.vehicle.hull);
        }
    }

    /// Advance every vehicle by `dt` seconds on its controls, then push apart those that
    /// ran into each other, see [`Self::collide`].
    pub fn step(&mut self, dt: f32) {
        self.remember();
        for (_, placed) in self.vehicles.iter_mut() {
            placed.vehicle.step(dt, &self.space, self.bounds, &mut self.rubble);
        }
        self.collide(dt);
        self.report_jumps(dt);
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
            let turned = wrap_angle(hull.direction.z - placed.pose.direction.z).abs();
            let rose = moved.y.abs() - moved.xz().length();
            if moved.length() > allowed || rose > 0.4 || turned > 0.25 {
                warn!(
                    from = ?placed.pose.position,
                    to = ?hull.position,
                    yaw_from = placed.pose.direction.z,
                    yaw_to = hull.direction.z,
                    speed = hull.speed,
                    "Jumped a vehicle in a step",
                );
            }
        }
    }

    /// Push apart the vehicles whose collision boxes overlap, see [`collide`]: where they
    /// first touched since they were remembered, a step of `dt` seconds ago, the vehicles
    /// pushed into others apart again, and those that touched back out of the static
    /// models and within the map's bounds, and settle them on the ground: a vehicle isn't
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

}

/// The collision of two vehicles; see the module documentation for the picture, steps
/// 4 to 7 of a tick. Push two hulls apart on the ground plane where their collision boxes
/// overlap in space, tilted as the hulls are, having moved from the given poses in `dt`
/// seconds:
/// where they first touched along the way they stop, and then each moves by the other's
/// share of their weight, and they share the speed they close in on each other at, at
/// the middle of the overlap, their momentum kept and their hits off-centre spinning
/// them, so the heavier shoves the lighter, and they move on at it for what was left of
/// the step. Driven on, the pusher's engine pushes them both, against both their grounds.
/// Returns whether they touched.
fn collide(hull: &mut Hull, from: Pose, other: &mut Hull, other_from: Pose, dt: f32) -> bool {

    let (obb, other_obb) = (hull.obb(), other.obb());
    if obb.center.distance(other_obb.center) >= obb.half.length() + other_obb.half.length() {
        return false;
    }
    if obb.push(&other_obb).is_none() {
        return false;
    }

    // Where they touch, in what they overlap on the ground plane as the step ends: at the
    // moment they first touch, boxes a hair off square meet at a corner, which would spin
    // them wildly.
    let footprint = hull.footprint_at(Pose::of(hull));
    let other_footprint = other.footprint_at(Pose::of(other));
    let contact = overlap_center(footprint.middle, footprint.heading, footprint.right, footprint.half, &other_footprint.corners());
    let arm = contact - footprint.middle;
    let other_arm = contact - other_footprint.middle;

    // Stopped where they first touched, rather than pushed out of how deep they got.
    let to = Pose::of(hull);
    let other_to = Pose::of(other);
    let t = first_touch(&Sweep { hull, from, to }, &Sweep { hull: other, from: other_from, to: other_to });
    if t < 1.0 {
        hull.place(from.lerp(to, t));
        other.place(other_from.lerp(other_to, t));
    }
    let Some(push) = hull.obb().push(&other.obb()) else { return false };

    // Pushed apart, each by the other's share of their weight.
    let mass = hull.mobility.weight;
    let other_mass = other.mobility.weight;
    let share = other_mass / (mass + other_mass);
    hull.position += Vec3::new(push.x, 0.0, push.y) * share;
    other.position -= Vec3::new(push.x, 0.0, push.y) * (1.0 - share);

    // Their speeds there, along the way they pushed, as the second box leaves the first.
    let normal = push.normalize();
    let speed = hull.point_velocity(arm);
    let other_speed = other.point_velocity(other_arm);
    let closing = (speed - other_speed).dot(normal);
    if closing < 0.0 {
        // The impulse that stops them closing in, along the normal: what each gives of
        // its weight and of its spin, a box's yaw inertia being `m (w² + l²) / 12`.
        let inertia = hull.yaw_inertia();
        let other_inertia = other.yaw_inertia();
        let lever = arm.perp_dot(normal);
        let other_lever = other_arm.perp_dot(normal);
        let effective = 1.0 / mass + 1.0 / other_mass + lever * lever / inertia + other_lever * other_lever / other_inertia;
        let impulse = -closing / effective;
        hull.hit(normal * impulse, arm, mass, inertia);
        other.hit(-normal * impulse, other_arm, other_mass, other_inertia);
    }

    // On, for what was left of the step.
    hull.coast((1.0 - t) * dt);
    other.coast((1.0 - t) * dt);
    true

}

/// The middle of what a box on the ground plane, at `middle` with its `heading` and
/// `right` axes and its `half` sizes across then along, overlaps of a convex outline of
/// four corners: the centroid of the overlap.
///
/// The outline is clipped by the box's four sides in turn (Sutherland–Hodgman: each
/// edge crossing a side is cut where it crosses), leaving a convex polygon of at most
/// 8 vertices, whose centroid is the area-weighted mean of the triangles it makes with
/// its first vertex. The plain average of the vertices would lean toward the side with
/// more of them, which clipping duplicates along the box's sides.
fn overlap_center(middle: Vec2, heading: Vec2, right: Vec2, half: Vec2, corners: &[Vec2; 4]) -> Vec2 {

    // Clip the polygon to the side of a line where `inside` is positive.
    fn clip(polygon: &[Vec2], inside: impl Fn(Vec2) -> f32) -> SmallVec<[Vec2; 8]> {
        let mut clipped = SmallVec::new();
        for (index, &from) in polygon.iter().enumerate() {
            let to = polygon[(index + 1) % polygon.len()];
            let from_inside = inside(from);
            let to_inside = inside(to);
            if from_inside >= 0.0 {
                clipped.push(from);
            }
            if (from_inside >= 0.0) != (to_inside >= 0.0) {
                clipped.push(from.lerp(to, from_inside / (from_inside - to_inside)));
            }
        }
        clipped
    }

    let mut overlap = SmallVec::<[Vec2; 8]>::from_slice(corners);
    for (axis, extent) in [(heading, half.y), (right, half.x)] {
        overlap = clip(&overlap, |point| extent - (point - middle).dot(axis));
        overlap = clip(&overlap, |point| extent + (point - middle).dot(axis));
    }
    // Its centroid, from the triangles it makes with its first corner.
    let Some(&first) = overlap.first() else { return middle };
    let mut area = 0.0;
    let mut sum = Vec2::ZERO;
    for pair in overlap.windows(2) {
        let a = pair[0] - first;
        let b = pair[1] - first;
        let cross = a.perp_dot(b);
        area += cross;
        sum += (a + b) * cross;
    }
    match area.abs() > 1e-6 {
        true => first + sum / (3.0 * area),
        false => overlap.iter().sum::<Vec2>() / overlap.len() as f32,
    }

}

/// A turret and its gun, turning toward what the client aims at at their top speeds,
/// the gun within its pitch limits at the turret's yaw.
#[derive(Debug, Clone)]
pub struct Turret {
    /// Relative to the hull, in radians.
    pub yaw: f32,
    /// Positive downward, in radians.
    pub pitch: f32,
    target: Aim,
    aiming: AimingDef,
    /// How fast the turret turned in the last step, in rad/s.
    yaw_rate: f32,
    /// The dispersion as a factor of the gun's base one, shrinking back to its ideal
    /// over the aiming time.
    dispersion_factor: f32,
}

/// What the turret turns toward.
#[derive(Debug, Clone, Copy)]
enum Aim {
    /// A point in the world. The client gives it relative to the vehicle, with
    /// `trackRelativePointWithGun`, and only sends it again when it changes, so it
    /// stays where it was in the world as the vehicle moves.
    Point(Vec3),
    /// Angles relative to the hull, as `stopTrackingWithGun` gives them.
    Angles(f32, f32),
}

impl Turret {

    pub fn new(aiming: AimingDef) -> Self {
        Self { 
            yaw: 0.0, 
            pitch: 0.0, 
            target: Aim::Angles(0.0, 0.0), 
            aiming, 
            yaw_rate: 0.0, 
            dispersion_factor: 1.0,
        }
    }

    /// Turn toward a point in the world.
    pub fn track(&mut self, point: Vec3) {
        self.target = Aim::Point(point);
    }

    /// Turn to angles relative to the hull, and stay there.
    pub fn hold(&mut self, yaw: f32, pitch: f32) {
        self.target = Aim::Angles(yaw, pitch);
    }

    /// Advance by `dt` seconds on the given hull, already advanced: turn toward the aim,
    /// then update the dispersion.
    pub fn step(&mut self, dt: f32, hull: &Hull) {
        let (yaw, pitch) = match self.target {
            // Raised for the shell to drop onto the point, then in the hull's frame,
            // which the turret and gun turn in.
            Aim::Point(point) => {
                let direction = self.ballistic_direction(point - gun_position(hull, self));
                let to = hull.rotation().inverse() * direction;
                let horizontal = (to.x * to.x + to.z * to.z).sqrt();
                (to.x.atan2(to.z), -to.y.atan2(horizontal))
            }
            Aim::Angles(yaw, pitch) => (yaw, pitch),
        };
        // Within its limits it only turns inside them, never round the back.
        let max_yaw_step = self.aiming.turret_rotation_speed * dt;
        let yaw_step = match self.aiming.yaw_limits {
            Some((min, max)) => wrap_angle(yaw).clamp(min, max) - self.yaw,
            None => wrap_angle(yaw - self.yaw),
        }.clamp(-max_yaw_step, max_yaw_step);
        self.yaw = wrap_angle(self.yaw + yaw_step);
        self.yaw_rate = yaw_step.abs() / dt;
        let pitch = pitch.clamp(self.aiming.min_pitch.at(self.yaw), self.aiming.max_pitch.at(self.yaw));
        let max_pitch_step = self.aiming.gun_rotation_speed * dt;
        self.pitch += (pitch - self.pitch).clamp(-max_pitch_step, max_pitch_step);
        self.step_dispersion(dt, hull);
    }

    /// The direction to fire the shell in for it to reach `to`, relative to the gun:
    /// the lower of the two trajectories, or 45° up when out of reach.
    ///
    /// With the shell's speed `v`, the gravity `g`, the horizontal distance `d` and the
    /// height `h` of the target above the gun, the elevation `θ` that reaches it solves
    /// `h = d·tan θ − g·d² / (2·v²·cos² θ)`, whose lower root is
    /// `tan θ = (v² − √(v⁴ − g·(g·d² + 2·h·v²))) / (g·d)`. When the root is imaginary the
    /// target is out of reach and the gun takes the 45° that goes the farthest.
    fn ballistic_direction(&self, to: Vec3) -> Vec3 {
        let speed = self.aiming.shot_speed;
        let gravity = self.aiming.shot_gravity;
        let distance = (to.x * to.x + to.z * to.z).sqrt();
        if distance < 1e-3 || gravity <= 0.0 {
            return to.normalize_or(Vec3::Z);
        }
        let v2 = speed * speed;
        let discriminant = v2 * v2 - gravity * (gravity * distance * distance + 2.0 * to.y * v2);
        let elevation = if discriminant >= 0.0 {
            ((v2 - discriminant.sqrt()) / (gravity * distance)).atan()
        } else {
            std::f32::consts::FRAC_PI_4
        };
        let yaw = to.x.atan2(to.z);
        Vec3::new(yaw.sin() * elevation.cos(), elevation.sin(), yaw.cos() * elevation.cos())
    }

    /// The dispersion's part of a step, as the client's `getOwnVehicleShotDispersionAngle`
    /// computes it: the ideal factor grows with the hull's speed, its turning and the
    /// turret's turning, and the actual one jumps up to it but only shrinks back over the
    /// aiming time. No crew, equipment or shot bonuses.
    ///
    /// ```text
    ///   ideal  = √( 1 + (speed·kₘ)² + (yaw_rate·kᵣ)² + (turret_rate·kₜ)² )
    ///   factor = ideal                                    if ideal ≥ factor    (jumps up)
    ///          = ideal + (factor − ideal)·e^(−dt / aiming_time)    otherwise   (settles)
    /// ```
    fn step_dispersion(&mut self, dt: f32, hull: &Hull) {
        let movement = hull.speed * self.aiming.dispersion_movement;
        let rotation = hull.yaw_rate * self.aiming.dispersion_rotation;
        let turret = self.yaw_rate * self.aiming.dispersion_turret_rotation;
        let ideal = (1.0 + movement * movement + rotation * rotation + turret * turret).sqrt();
        self.dispersion_factor = if ideal >= self.dispersion_factor {
            ideal
        } else {
            ideal + (self.dispersion_factor - ideal) * (-dt / self.aiming.aiming_time).exp()
        };
    }

    /// Where the gun fires from and the shell's initial velocity, as the client's
    /// `getCurShotPosition` gives them, and the current dispersion angle: what the
    /// server's gun marker shows.
    pub fn gun_marker(&self, hull: &Hull) -> (Vec3, Vec3, f32) {
        // Turret yaw, then gun pitch (positive downward), on the hull.
        let gun = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch);
        let direction = hull.rotation() * gun * Vec3::Z;
        (gun_position(hull, self), direction * self.aiming.shot_speed, self.aiming.dispersion_angle * self.dispersion_factor)
    }

    /// The angles as the `gunAnglesPacked` property carries them, see `encodeGunAngles`:
    /// the yaw on 10 bits over a turn, the pitch on 6 bits over its absolute limits.
    pub fn gun_angles_packed(&self) -> u16 {
        use std::f32::consts::{PI, TAU};
        let yaw = ((1024.0 * (self.yaw + PI) / TAU).round() as u16) & 1023;
        let (min_pitch, max_pitch) = self.aiming.absolute_pitch_limits();
        let t = if max_pitch == min_pitch { 0.0 } else { ((self.pitch - min_pitch) / (max_pitch - min_pitch)).clamp(0.0, 1.0) };
        yaw << 6 | (63.0 * t).round() as u16
    }

}

/// Where the gun fires from: its pivot on the turret, turned with it, on the hull.
fn gun_position(hull: &Hull, turret: &Turret) -> Vec3 {
    let aiming = &turret.aiming;
    hull.position + hull.rotation() * (aiming.turret_position + Quat::from_rotation_y(turret.yaw) * aiming.gun_position)
}

/// The height of the ground under a point of the ground plane, no higher than `top`:
/// the terrain's, or a static model's above it, its solid triangles flatter than
/// [`WALL_ANGLE`] from vertical or its ramps, the deck of a bridge it drives onto say,
/// not the one it drives under. None with neither.
fn ground_height(space: &Space, point: Vec2, top: f32) -> Option<f32> {
    
    let terrain = space.terrain.height_at(point.x, point.y);
    let statics = [BodyKind::Solid, BodyKind::Ramp].into_iter()
        .flat_map(|kind| space.statics.triangles_at(kind, point))
        .filter(|&(triangle, height)| height <= top && (triangle.kind == BodyKind::Ramp || triangle.normal.y.abs() >= WALL_ANGLE.sin()))
        .map(|(_, height)| height)
        .reduce(f32::max);
    
    match (terrain, statics) {
        (Some(terrain), Some(statics)) => Some(terrain.max(statics)),
        (terrain, statics) => terrain.or(statics),
    }

}

/// Where a hull's footprint, its `corners`, touches a wall it was pushed out of along
/// `normal`: its corner furthest into the wall, or the middle of those as far, within
/// 5 cm, when an edge lies flat against it.
fn wall_contact(corners: &[Vec2; 4], normal: Vec2) -> Vec2 {
    let deepest = corners.iter()
        .map(|corner| corner.dot(normal))
        .fold(f32::INFINITY, f32::min);
    let mut sum = Vec2::ZERO;
    let mut count = 0.0;
    for &corner in corners {
        if corner.dot(normal) < deepest + 0.05 {
            sum += corner;
            count += 1.0;
        }
    }
    sum / count
}

/// The outline of a convex polygon on the ground plane: a triangle clipped to two
/// heights has five vertices at most, so it never leaves the stack.
type Outline = SmallVec<[Vec2; 8]>;

/// What of a triangle lies between two heights, on the ground plane: its outline's x
/// and z, none if it lies all above or below.
fn clip_to_heights(triangle: &[Vec3; 3], low: f32, high: f32) -> Option<Outline> {

    // Clip the polygon to one side of a height, `keep` telling the side kept: a convex
    // polygon of at most `Outline`'s vertices stays so, with one more per plane.
    fn clip(polygon: &[Vec3], height: f32, keep: impl Fn(f32) -> bool) -> SmallVec<[Vec3; 8]> {
        let mut clipped = SmallVec::new();
        for (index, &from) in polygon.iter().enumerate() {
            let to = polygon[(index + 1) % polygon.len()];
            if keep(from.y) {
                clipped.push(from);
            }
            if keep(from.y) != keep(to.y) {
                clipped.push(from.lerp(to, (height - from.y) / (to.y - from.y)));
            }
        }
        clipped
    }

    let polygon = clip(triangle, low, |y| y >= low);
    let polygon = clip(&polygon, high, |y| y <= high);
    (!polygon.is_empty()).then(|| polygon.into_iter().map(|vertex| vertex.xz()).collect())
    
}

/// How far to push a box on the ground plane, at `middle` with its `heading` and `right`
/// axes and its `half` sizes across then along, out of a convex outline, least first:
/// along the axis that separates them least, of the box's two and the outline's edges'
/// normals, none if they don't overlap.
///
/// The 2D separating axis test: a rectangle and a convex polygon are apart if an axis
/// exists on which their shadows don't overlap, and it is enough to try the rectangle's
/// two axes and the normal of every edge of the polygon. On each, the box can leave the
/// polygon *backward* (to the low side) or *forward*; the shorter way is kept, and the
/// shortest of all axes is the push:
///
/// ```text
///        outline           axis a
///       ╱╲                 ├──┤ polygon's shadow   [low … high]
///      ╱  ╲                       ├──┤ box's shadow  [centre ± extent]
///     ╲    ╲
///      ╲____╲              back    = centre + extent − low      (push the box −a)
///                          forward = high − (centre − extent)   (push the box +a)
/// ```
fn box_push(middle: Vec2, heading: Vec2, right: Vec2, half: Vec2, outline: &[Vec2]) -> Option<Vec2> {

    let edges = outline.iter().zip(outline.iter().cycle().skip(1))
        .map(|(&from, &to)| (to - from).perp())
        .filter(|normal| normal.length_squared() > 1e-8)
        .map(Vec2::normalize);

    let mut least: Option<Vec2> = None;
    for axis in [heading, right].into_iter().chain(edges) {
        let center = middle.dot(axis);
        let extent = half.x * right.dot(axis).abs() + half.y * heading.dot(axis).abs();
        let (low, high) = outline.iter().map(|vertex| vertex.dot(axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), p| (low.min(p), high.max(p)));
        // Pushed back past the outline's low side, or forward past its high side.
        let back = center + extent - low;
        let forward = high - (center - extent);
        if back <= 0.0 || forward <= 0.0 {
            return None;
        }
        let push = if back < forward { -axis * back } else { axis * forward };
        if least.is_none_or(|least| push.length_squared() < least.length_squared()) {
            least = Some(push);
        }
    }
    least

}

/// A value moved toward a goal by at most `step`, without going past it.
fn approach(value: f32, goal: f32, step: f32) -> f32 {
    value + (goal - value).clamp(-step, step)
}

/// A value between bounds on `bits` bits, rounded, as the client's packing does.
fn pack_range(value: f32, min: f32, max: f32, bits: u32) -> u64 {
    let max_code = (1u64 << bits) - 1;
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    (t * max_code as f32).round() as u64 & max_code
}

/// An angle on `bits` bits over a turn from -π, rounded, as the client's packing does:
/// π wraps to -π.
fn pack_angle(angle: f32, bits: u32) -> u64 {
    let t = (angle / std::f32::consts::TAU + 0.5).clamp(0.0, 1.0);
    (t * (1u64 << bits) as f32).round() as u64 & ((1u64 << bits) - 1)
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


#[cfg(test)]
pub(super) mod tests {

    use super::*;

    /// How high the test gun is above the vehicle's origin.
    const GUN_HEIGHT: f32 = 1.7;

    /// About a Chieftain's: 750 hp for 55 t, with its physics section's rolling friction,
    /// its terrain resistance 1, 1.3 and 2.5.
    pub(crate) fn chieftain_mobility() -> MobilityDef {
        MobilityDef {
            speed_forward: 40.0 / 3.6,
            speed_backward: 15.0 / 3.6,
            rotation_speed: [28f32.to_radians(), 28f32.to_radians() / 1.3, 28f32.to_radians() / 2.5],
            engine_power: 750.0 * 735.5,
            weight: 55_000.0,
            rolling_friction: [0.0805, 0.10465, 0.20125],
            carrying_point: glam::Vec2::new(1.6, 2.4),
            bounds: [Vec3::new(-1.8, 0.0, -3.7), Vec3::new(1.8, 2.6, 3.8)],
        }
    }

    fn chieftain() -> Hull {
        Hull::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility())
    }

    #[test]
    fn drives_forward_along_heading() {
        let mut hull = chieftain();
        hull.direction.z = 2.771;
        hull.flags = movement_flag::FORWARD;
        // Quickly at first, slower as the engine's power spreads over the speed.
        hull.step(0.1, Ground::Firm);
        let first = hull.speed;
        let resistance = hull.mobility.rolling_friction[0] * GRAVITY;
        assert!((first - (MAX_ACCELERATION - resistance) * 0.1).abs() < 1e-4, "{first}");
        for _ in 0..300 {
            hull.step(0.1, Ground::Firm);
        }
        // At top speed after accelerating, heading south-east as captured.
        assert!((hull.speed - 40.0 / 3.6).abs() < 1e-3, "{}", hull.speed);
        // Short of it on soft ground, where its power only holds it at P / (m · μ · g).
        for _ in 0..300 {
            hull.step(0.1, Ground::Soft);
        }
        let mobility = &hull.mobility;
        let held = POWER_FACTOR * mobility.engine_power / (mobility.weight * mobility.rolling_friction[2] * GRAVITY);
        assert!((hull.speed - held).abs() < 0.05, "{} for {held}", hull.speed);
        assert!(hull.position.x > 0.0 && hull.position.z < 0.0);
        assert_eq!(hull.position.y, 0.0);
    }

    #[test]
    fn stops_and_turns() {
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        hull.step(1.0, Ground::Firm);
        hull.flags = movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            hull.step(0.1, Ground::Firm);
        }
        // Stopped, then turning on the spot at full speed.
        assert_eq!(hull.speed, 0.0);
        assert!((hull.yaw_rate - 28f32.to_radians()).abs() < 1e-4, "{}", hull.yaw_rate);
        // Slower while driving.
        hull.flags = movement_flag::FORWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..20 {
            hull.step(0.1, Ground::Firm);
        }
        assert!((hull.yaw_rate - 28f32.to_radians() * ROTATION_ON_MOVE).abs() < 1e-4, "{}", hull.yaw_rate);
        // Backing up while steering right turns the hull the other way, soon.
        hull.flags = movement_flag::BACKWARD | movement_flag::ROTATE_RIGHT;
        for _ in 0..5 {
            hull.step(0.1, Ground::Firm);
        }
        assert!(hull.yaw_rate < 0.0);
        // Standing still with no input reports no movement.
        hull.flags = 0;
        hull.speed = 0.0;
        hull.yaw_rate = 0.0;
        assert!(!hull.step(0.1, Ground::Firm));
    }

    #[test]
    fn drifts_on_its_handbrake() {
        let mut hull = chieftain();
        hull.speed = 10.0;
        hull.flags = movement_flag::BLOCK_TRACKS | movement_flag::ROTATE_LEFT;
        let normal = hull.mobility.rotation_speed[1] * ROTATION_ON_MOVE;
        let mut fastest: f32 = 0.0;
        for _ in 0..10 {
            hull.step(0.1, Ground::Medium);
            fastest = fastest.max(-hull.yaw_rate);
        }
        // Turning faster than it would driving, its hull swung left of where it slides.
        assert!(fastest > normal * 1.5, "{fastest} for {normal}");
        assert!(hull.slide > 0.2, "{}", hull.slide);
        // Until its blocked tracks stop it.
        for _ in 0..50 {
            hull.step(0.1, Ground::Medium);
        }
        assert_eq!((hull.speed, hull.slide), (0.0, 0.0));
    }

    #[test]
    fn holds_on_slopes_it_grips() {
        let still = |pitch: f32, flags: u8| {
            let mut hull = chieftain();
            hull.direction.y = pitch.to_radians();
            hull.flags = flags;
            for _ in 0..30 {
                hull.step(0.1, Ground::Medium);
            }
            hull.speed
        };
        // Parked nose down, it holds on 20°, and slides down 35°.
        assert_eq!(still(20.0, 0), 0.0);
        assert!(still(35.0, 0) > 1.0);
        // Driving up 20°, it crawls as fast as its power holds it against the slope; up
        // 35°, it slides back.
        let mobility = chieftain_mobility();
        let held = POWER_FACTOR * mobility.engine_power / mobility.weight
            / (mobility.rolling_friction[1] * GRAVITY + GRAVITY * 20f32.to_radians().sin());
        let up = still(-20.0, movement_flag::FORWARD);
        assert!((up - held).abs() < 0.05, "{up} for {held}");
        assert!(still(-35.0, movement_flag::FORWARD) < -1.0);
        // Across 35°, it slides down to its left, right side up.
        let mut hull = chieftain();
        hull.direction.x = 35f32.to_radians();
        for _ in 0..10 {
            hull.step(0.1, Ground::Medium);
        }
        assert!(hull.slide < -1.0, "{}", hull.slide);
    }

    /// A wall across the way at z = 10, `height` high, as two triangles.
    fn wall(height: f32) -> Space {
        let low_left = Vec3::new(-20.0, -1.0, 10.0);
        let low_right = Vec3::new(20.0, -1.0, 10.0);
        let high_left = Vec3::new(-20.0, height, 10.0);
        let high_right = Vec3::new(20.0, height, 10.0);
        let triangles = vec![
            StaticTriangle::new([low_left, low_right, high_right], 0, 0, BodyKind::Solid),
            StaticTriangle::new([low_left, high_right, high_left], 0, 0, BodyKind::Solid),
        ];
        Space { statics: StaticScene::from_triangles(triangles), ..Space::default() }
    }

    #[test]
    fn stops_against_walls_it_cannot_drive_over() {
        // Square on at full speed, it stops with its front against the wall.
        let space = wall(3.0);
        let mut hull = chieftain();
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        let front = hull.position.z + hull.mobility.bounds[1].z;
        assert!((front - 10.0).abs() < 1e-3 && hull.speed.abs() < 1e-3, "front at {front}, {} m/s", hull.speed);
        // Lower than its step, it drives over it.
        let space = wall(STEP_HEIGHT - 0.1);
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
    }

    /// A fence across the way at z = 10, 3 m high, destructible whole with this health,
    /// the 4th of its chunk; or a house's wall, its module of this health.
    fn fence(health: f32, house: bool) -> Space {
        let mut space = wall(3.0);
        let material_kind = if house { 73 } else { 0 };
        let triangles = space.statics.triangles_in(BodyKind::Solid, Vec3::splat(-30.0), Vec3::splat(30.0))
            .map(|triangle| StaticTriangle::new(triangle.vertices, 0, material_kind, BodyKind::Destructible))
            .collect();
        space.statics = StaticScene::from_triangles(triangles);
        let modules = match house {
            true => vec![wgtk::space::destructibles::Module {
                material_kind: 73,
                health,
                depends: Vec::new(),
            }],
            false => Vec::new(),
        };
        space.statics.add_destructible(0, PlacedDestructible {
            chunk_id: 0x7f7f,
            index: 3,
            kind: if house { DestructibleKind::Structure } else { DestructibleKind::Fragile },
            health,
            modules,
        });
        space
    }

    #[test]
    fn breaks_through_what_it_rams_hard_enough() {
        // A fence it rams at 20 km/h, it breaks, once, and drives on through, slowed by
        // what its health took.
        let space = fence(10.0, false);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.speed = 20.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        // Its speed before the tick it broke it, and after.
        let mut broke = None;
        for _ in 0..30 {
            let before = hull.speed;
            hull.advance(0.1, &space, None, &mut rubble);
            if broke.is_none() && !rubble.fresh.is_empty() {
                broke = Some((before, hull.speed));
            }
        }
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
        // It lost a little of its energy, its engine winning a little back within the
        // tick.
        let (before, after) = broke.unwrap();
        let weight = hull.mobility.weight / UNIT_VEHICLE_MASS;
        let kept = (before * before - 2.0 * RAM_COST * 10.0 / weight).sqrt();
        assert!(after > kept && after < kept + 0.2, "{after} for {kept}");
        let broken: Vec<Breakage> = rubble.fresh.drain(..).collect();
        assert_eq!(broken.len(), 1);
        assert_eq!((broken[0].piece.chunk_id, broken[0].piece.index, broken[0].kind), (0x7f7f, 3, DestructibleKind::Fragile));
        // Turning on the spot with its nose against one, its corner swings into it and
        // breaks it.
        let space = fence(10.0, false);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.position.z = 10.0 - hull.mobility.bounds[1].z - 0.05;
        hull.flags = movement_flag::ROTATE_LEFT;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        assert_eq!(rubble.fresh.len(), 1);
        // A house's wall too strong for it stops it as a wall would.
        let space = fence(1000.0, true);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.speed = 20.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        let front = hull.position.z + hull.mobility.bounds[1].z;
        assert!((front - 10.0).abs() < 1e-3 && rubble.fresh.is_empty(), "front at {front}");
        // One it starts against, stopped, it presses until it breaks it.
        let space = fence(50.0, true);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.position.z = 10.0 - hull.mobility.bounds[1].z - 0.01;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        assert_eq!(rubble.fresh.len(), 1);
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
    }

    /// A flat deck from z = 0 to 20, `height` high, as two triangles.
    fn deck(height: f32) -> Space {
        let near_left = Vec3::new(-5.0, height, 0.0);
        let near_right = Vec3::new(5.0, height, 0.0);
        let far_left = Vec3::new(-5.0, height, 20.0);
        let far_right = Vec3::new(5.0, height, 20.0);
        let triangles = vec![
            StaticTriangle::new([near_left, far_right, near_right], 0, 0, BodyKind::Solid),
            StaticTriangle::new([near_left, far_left, far_right], 0, 0, BodyKind::Solid),
        ];
        Space { statics: StaticScene::from_triangles(triangles), ..Space::default() }
    }

    #[test]
    fn drives_onto_decks_within_a_step() {
        // A step up, it drives onto it and rests on it.
        let space = deck(STEP_HEIGHT - 0.1);
        let mut hull = chieftain();
        hull.position.z = -6.0;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.position.z > 10.0 && hull.position.y == STEP_HEIGHT - 0.1, "{}", hull.position);
        // Higher, a bridge it drives under, it stays below.
        let space = deck(STEP_HEIGHT + 3.0);
        let mut hull = chieftain();
        hull.position.z = -6.0;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.position.z > 10.0 && hull.position.y == 0.0, "{}", hull.position);
    }

    /// A Chieftain driven forward for `steps` ticks into another parked 10 m ahead, of
    /// this weight and `height` above it: where both end.
    fn push_parked(weight: f32, height: f32, offset: f32, steps: usize) -> (Hull, Hull) {
        let mut world = World::new(Arc::new(Space::default()), None);
        let mut pusher = Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        pusher.hull.flags = movement_flag::FORWARD;
        let pusher = world.add(pusher);
        let mobility = MobilityDef { weight, ..chieftain_mobility() };
        let parked = world.add(Vehicle::new(Vec3::new(offset, height, 10.0), Vec3::ZERO, mobility, chieftain_aiming()));
        for _ in 0..steps {
            world.step(0.1);
        }
        (world.vehicle(pusher).hull.clone(), world.vehicle(parked).hull.clone())
    }

    #[test]
    fn pushes_vehicles_it_drives_into() {
        let mut alone = chieftain();
        alone.flags = movement_flag::FORWARD;
        for _ in 0..60 {
            alone.advance(0.1, &Space::default(), None, &mut Rubble::default());
        }
        // Into another as heavy, it shoves it ahead, slower than alone, never into it.
        let (pusher, parked) = push_parked(55_000.0, 0.0, 0.0, 60);
        let length = 7.5;
        assert!(parked.position.z - pusher.position.z >= length - 1e-3, "{} {}", pusher.position.z, parked.position.z);
        assert!(parked.position.z > 12.0 && pusher.position.z < alone.position.z - 2.0, "{} {} {}", pusher.position.z, parked.position.z, alone.position.z);
        assert!(pusher.position.x.abs() < 1e-3 && parked.position.x.abs() < 1e-3);
        // A lighter one, further.
        let (_, light) = push_parked(20_000.0, 0.0, 0.0, 60);
        assert!(light.position.z > parked.position.z + 1.0, "{} {}", light.position.z, parked.position.z);
        // Above it, on a bridge, it drives under.
        let (pusher, parked) = push_parked(55_000.0, 3.0, 0.0, 60);
        assert!(parked.position.z == 10.0 && pusher.position.z == alone.position.z);
    }

    /// Ram a parked vehicle's side at `speed`, `offset` from its middle along it, and let
    /// them go for 3 s: the rammer, then the rammed, which first faces +X.
    fn ram_side(speed: f32, offset: f32) -> (Hull, Hull) {
        let mut world = World::new(Arc::new(Space::default()), None);
        let [min, max] = chieftain_mobility().bounds;
        // Its nose 5 cm short of the parked one's left side.
        let start = 7.0 - (max.x - min.x) / 2.0 - max.z - 0.05;
        let mut rammer = Vehicle::new(Vec3::new(offset, 0.0, start), Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        rammer.hull.speed = speed;
        let rammer = world.add(rammer);
        let facing = Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2);
        let parked = world.add(Vehicle::new(Vec3::new(0.0, 0.0, 7.0), facing, chieftain_mobility(), chieftain_aiming()));
        for _ in 0..30 {
            world.step(0.1);
        }
        (world.vehicle(rammer).hull.clone(), world.vehicle(parked).hull.clone())
    }

    #[test]
    fn spins_vehicles_it_hits_off_centre() {
        use std::f32::consts::FRAC_PI_2;
        // Rammed in its side behind its box's middle, its nose swings towards the rammer:
        // the yaw drops; ahead of it, the other way, as much; the rammer keeps its heading.
        let [min, max] = chieftain_mobility().bounds;
        let middle = (min.z + max.z) / 2.0;
        let (rammer, behind) = ram_side(8.0, middle + 1.5);
        let (_, ahead) = ram_side(8.0, middle - 1.5);
        let behind = behind.direction.z - FRAC_PI_2;
        let ahead = ahead.direction.z - FRAC_PI_2;
        assert!(behind < -0.1, "{behind}");
        // As much give or take its turning about its position, not its box's middle.
        assert!((behind + ahead).abs() < 0.15 * behind.abs(), "{behind} {ahead}");
        assert!(rammer.direction.z.abs() < 0.01, "{}", rammer.direction.z);
        // A bump hardly turns it: its tracks hold it.
        let (_, bumped) = ram_side(3.0, 1.5);
        assert!((bumped.direction.z - FRAC_PI_2).abs() < 0.03, "{}", bumped.direction.z);
        // Nor does a slow push off its middle from behind, both moving along their tracks.
        let (pusher, pushed) = push_parked(55_000.0, 0.0, -1.5, 60);
        assert!(pushed.direction.z.abs() < 0.05 && pusher.direction.z.abs() < 0.05, "{} {}", pushed.direction.z, pusher.direction.z);
    }

    #[test]
    fn does_not_get_into_vehicles_at_speed() {
        // Driving at 80 km/h, it moves 2 m a tick: into another's side, it stops against
        // it rather than being pushed sideways through it.
        let mut world = World::new(Arc::new(Space::default()), None);
        let mut pusher = Vehicle::new(Vec3::new(0.0, 0.0, -15.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        pusher.hull.speed = 80.0 / 3.6;
        pusher.hull.flags = movement_flag::FORWARD;
        let pusher = world.add(pusher);
        let parked = world.add(Vehicle::new(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        for _ in 0..30 {
            world.step(0.1);
            let a = world.vehicle(pusher).hull.obb();
            let b = world.vehicle(parked).hull.obb();
            assert!(a.separation(&b).is_none_or(|(_, depth)| depth < 0.2), "got into it: {:?}", a.separation(&b));
        }
        let (pusher, parked) = (&world.vehicle(pusher).hull, &world.vehicle(parked).hull);
        assert!(pusher.position.z > -5.0 && parked.position.z > 0.0, "{} {}", pusher.position, parked.position);
    }

    /// A box of these half sizes at a place and tilted so, to test with.
    fn obb(center: Vec3, direction: Vec3, half: Vec3) -> Obb {
        Obb {
            center,
            axes: Mat3::from_quat(rotation_of(direction)),
            half,
        }
    }

    #[test]
    fn boxes_overlap_in_space() {
        let half = Vec3::new(1.0, 1.0, 2.0);
        let square = obb(Vec3::ZERO, Vec3::ZERO, half);
        // Side by side 0.5 m into each other, it is pushed back along the way it came.
        let (axis, depth) = obb(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, half).separation(&square).unwrap();
        assert!((axis - Vec3::X).length() < 1e-5 && (depth - 0.5).abs() < 1e-5, "{axis} {depth}");
        assert!(obb(Vec3::new(2.5, 0.0, 0.0), Vec3::ZERO, half).separation(&square).is_none());
        // One above the other, they are in contact but the ground holds it, no push.
        let above = obb(Vec3::new(0.0, 1.9, 0.0), Vec3::ZERO, half);
        assert!(above.separation(&square).is_some() && above.push(&square).is_none());
        // Nose up 40°, its front reaches higher than a flat box's: a box clear of the flat
        // one over its front end, 1.4 m up, is in the way of the tilted one.
        let over = obb(Vec3::new(0.0, 2.4, 2.5), Vec3::ZERO, half);
        assert!(over.separation(&obb(Vec3::ZERO, Vec3::ZERO, half)).is_none());
        assert!(over.separation(&obb(Vec3::ZERO, Vec3::new(0.0, -40f32.to_radians(), 0.0), half)).is_some());
    }

    #[test]
    fn stops_at_the_map_edge() {
        let space = Space::default();
        let bounds = Some((Vec2::splat(-100.0), Vec2::splat(100.0)));
        let edge = 100.0 - BOUNDS_MARGIN;
        // Square on at full speed, it stops dead, still pushing.
        let mut hull = chieftain();
        hull.position.z = 90.0;
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..20 {
            hull.advance(0.1, &space, bounds, &mut Rubble::default());
        }
        assert_eq!(hull.position.z, edge);
        assert!(hull.speed.abs() < 1e-4 && hull.position.x.abs() < 1e-4, "{} {}", hull.speed, hull.position.x);
        // At 30°, it drags along it, slower and slower: the real server's went 2.6, 1.8,
        // 0.7 then 0.2 m/s once against it.
        let mut hull = chieftain();
        hull.position.z = edge;
        hull.direction.z = -30f32.to_radians();
        hull.speed = 40.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        let mut along = Vec::new();
        for _ in 0..6 {
            let x = hull.position.x;
            hull.advance(0.1, &space, bounds, &mut Rubble::default());
            along.push((x - hull.position.x) / 0.1);
        }
        assert_eq!(hull.position.z, edge);
        assert!(along.windows(2).all(|pair| pair[1] <= pair[0]) && (2.0..3.0).contains(&along[1]) && along[4] < 1.0, "{along:?}");
    }

    pub(crate) fn chieftain_aiming() -> AimingDef {
        use crate::wot::item_defs::PitchCurve;
        let curve = |points: &[(f32, f32)]| PitchCurve(points.iter().map(|&(t, a)| (t * std::f32::consts::TAU, a.to_radians())).collect());
        AimingDef {
            turret_rotation_speed: 30f32.to_radians(),
            gun_rotation_speed: 26f32.to_radians(),
            aiming_time: 2.1,
            dispersion_turret_rotation: 0.11f32.to_degrees(),
            dispersion_after_shot: 4.0,
            dispersion_movement: 0.14 * 3.6,
            dispersion_rotation: 0.14f32.to_degrees(),
            yaw_limits: None,
            min_pitch: curve(&[(0.0, -20.0), (1.0, -20.0)]),
            // 10° of depression at the front, -1° over the engine deck at the back.
            max_pitch: curve(&[(0.0, 10.0), (0.4, 10.0), (0.45, -1.0), (0.55, -1.0), (0.6, 10.0), (1.0, 10.0)]),
            dispersion_angle: 0.0035,
            shot_speed: 716.0,
            shot_gravity: 6.28,
            reload_time: 16.0,
            // Straight above the origin, to keep the tests' geometry simple.
            turret_position: Vec3::new(0.0, 1.4, 0.0),
            gun_position: Vec3::new(0.0, GUN_HEIGHT - 1.4, 0.0),
        }
    }

    #[test]
    fn turret_turns_toward_point() {
        let mut hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        // Straight to the right of a hull facing +Z, level with the gun.
        turret.track(Vec3::new(100.0, GUN_HEIGHT, 0.0));
        turret.step(1.0, &hull);
        assert!((turret.yaw - 30f32.to_radians()).abs() < 1e-5);
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // Raised a little for the shell to drop onto it.
        assert!(turret.pitch < 0.0 && turret.pitch > -0.01);
        // The point stays put in the world as the vehicle drives past it.
        hull.position = Vec3::new(0.0, 0.0, 200.0);
        turret.step(1.0, &hull);
        assert!(turret.yaw > std::f32::consts::FRAC_PI_2);
        // Yaw 0 and the middle of the pitch range, as `encodeGunAngles` packs them.
        turret.hold(0.0, -5f32.to_radians());
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert_eq!(turret.gun_angles_packed(), 512 << 6 | 32);
    }

    #[test]
    fn holding_where_it_tracks_keeps_the_aim_on_a_tilted_hull() {
        let mut hull = chieftain();
        hull.direction = Vec3::new(0.05, 0.08, 1.2);
        let mut turret = Turret::new(chieftain_aiming());
        let point = Vec3::new(150.0, 5.0, 40.0);
        turret.track(point);
        for _ in 0..20 {
            turret.step(1.0, &hull);
        }
        // The shell's arc goes through the point.
        let (position, velocity, _) = turret.gun_marker(&hull);
        let to = point - position;
        let horizontal = Vec3::new(velocity.x, 0.0, velocity.z);
        let time = Vec3::new(to.x, 0.0, to.z).length() / horizontal.length();
        let at = position + velocity * time - Vec3::Y * 0.5 * turret.aiming.shot_gravity * time * time;
        assert!((at - point).length() < 0.05, "lands at {at}, not {point}");
        // Locking sends back the same hull-relative angles, which change nothing.
        let (yaw, pitch) = (turret.yaw, turret.pitch);
        turret.hold(yaw, pitch);
        turret.step(1.0, &hull);
        assert_eq!(turret.gun_marker(&hull).1, velocity);
    }

    #[test]
    fn engine_shifts_up_with_speed() {
        let mut hull = chieftain();
        assert_eq!(hull.engine(), (0, IDLE_RPM));
        let top = hull.mobility.speed_forward;
        let mut last = (0, 0.0);
        // Below 2% of the top speed, it counts as still.
        for step in 2..=100 {
            hull.speed = top * step as f32 / 100.0;
            let (gear, rpm) = hull.engine();
            assert!(gear >= last.0 && (SHIFT_RPM..=1.0).contains(&rpm), "{gear} {rpm} at {}", hull.speed);
            // The RPM drops only when shifting up.
            assert!(rpm >= last.1 || gear > last.0);
            last = (gear, rpm);
        }
        assert_eq!(last, (4, 1.0));
    }

    #[test]
    fn aux_physics_data_packs_as_captured() {
        // A captured value, slowing down: yaw 2.7713, pitch and roll near 0, both tracks
        // at 1.76 m/s, the RPM at 0xaf.
        let captured: u64 = 0xaf5f5f813c00f875;
        let mut hull = chieftain();
        hull.direction = Vec3::new(0.0299, 0.0012, 2.7713);
        hull.speed = 1.76;
        // Without the RPM, which depends on the engine model.
        assert_eq!(hull.aux_physics_data() & 0x00ff_ffff_ffff_ffff, captured & 0x00ff_ffff_ffff_ffff);
    }

    #[test]
    fn dispersion_widens_then_shrinks_over_aiming_time() {
        let mut hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        hull.speed = 10.0;
        turret.step(0.1, &hull);
        let moving = turret.dispersion_factor;
        assert!(moving > 1.0);
        hull.speed = 0.0;
        // After one aiming time, the excess shrinks by e.
        for _ in 0..21 {
            turret.step(0.1, &hull);
        }
        assert!(((turret.dispersion_factor - 1.0) - (moving - 1.0) / std::f32::consts::E).abs() < 1e-3);
        // Aimed straight ahead, the shell leaves at the shot speed.
        let (position, velocity, angle) = turret.gun_marker(&hull);
        assert_eq!(position, Vec3::new(0.0, GUN_HEIGHT, 0.0));
        assert!((velocity - Vec3::new(0.0, 0.0, 716.0)).length() < 1e-3);
        assert!((angle - 0.0035 * turret.dispersion_factor).abs() < 1e-6);
    }

    #[test]
    fn turret_stays_within_its_yaw_limits() {
        let hull = chieftain();
        let mut turret = Turret::new(AimingDef { yaw_limits: Some((-10f32.to_radians(), 12f32.to_radians())), ..chieftain_aiming() });
        // Behind on the right: as far right as it goes.
        turret.track(Vec3::new(10.0, GUN_HEIGHT, -100.0));
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw - 12f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
        // Behind on the left: back across the front, not round the back.
        turret.track(Vec3::new(-10.0, GUN_HEIGHT, -100.0));
        turret.step(0.5, &hull);
        assert!((turret.yaw - (12.0 - 15.0f32).to_radians()).abs() < 1e-5, "{}", turret.yaw);
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.yaw + 10f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
        // The client's angles too.
        turret.hold(1.0, 0.0);
        turret.step(1.0, &hull);
        assert!((turret.yaw - 12f32.to_radians()).abs() < 1e-5, "{}", turret.yaw);
    }

    #[test]
    fn gun_depression_depends_on_turret_yaw() {
        let hull = chieftain();
        let mut turret = Turret::new(chieftain_aiming());
        // Far below, in front then behind.
        turret.track(Vec3::new(0.0, -100.0, 10.0));
        for _ in 0..10 {
            turret.step(1.0, &hull);
        }
        assert!((turret.pitch - 10f32.to_radians()).abs() < 1e-5);
        turret.track(Vec3::new(0.0, -100.0, -10.0));
        for _ in 0..20 {
            turret.step(1.0, &hull);
        }
        assert!((turret.pitch + 1f32.to_radians()).abs() < 1e-5);
    }

}
