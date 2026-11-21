//! A tree falling, as the client animates it, so that the server knows where it lies as
//! the players see it: its foliage hides what is behind it there, see `wgtk::space::foliage`.
//!
//! The server tells the clients three things of a fall, see `fallenTrees`: the way it
//! falls, the vehicle's; how fast it starts, of 4 steps of the vehicle's speed; and how
//! far it may tip, [`TreeFall::limit`], where its top meets the terrain or its trunk a
//! static model, a house or a rock say, see [`fall_limit`]. The client does
//! the rest, `_DestructiblesAnimator` in its `AreaDestructibles.py`, which this is a port
//! of: the tree is a rod hinged at its foot, of its kind's mass and height
//! ([`FallParams`]) scaled by its scale, falling under gravity and the air's drag, its
//! branches a damped spring once it tips within `spring_angle` of its limit.
//!
//! ```text
//!   each step, of at most MAX_STEP:
//!     pitch held within 0 and its limit, stopped there
//!     torque = ½·h·G·m·sin(pitch) − pitch_speed·air_resist·h²/4
//!            − (pitch + spring_angle − limit)·spring_stiffness·h²/4   (within the
//!            − pitch_speed·spring_resist·h²/4                          spring's angle)
//!     pitch_speed += dt·torque / (m·h²/3)
//!     still, |pitch_speed| + |torque| < STOP_EPSILON, or MAX_DURATION in: it lies
//!     pitch += dt·pitch_speed
//! ```
//!
//! It lies where gravity and its branches balance, `wg_solveDestructibleFallPitch` for a
//! client that didn't see it fall: a maple of the most common parameters, falling on
//! flat ground, lies at about 80°, held up by its crown. Its placement then is its own
//! turned by its pitch about the level axis across its fall, its foot sunk
//! `bury_depth · sin(pitch)`, see [`TreeFall::transform`]. The client steps by its
//! frames, a step each at 60 fps; the server by its ticks, five of 0.02 s: where it
//! lies is the same.

use std::f32::consts::PI;

use glam::{Affine3A, Mat3A, Quat, Vec3, Vec3A};

use wgtk::space::statics::{PlacedTree, StaticTriangle};
use wgtk::space::destructibles::FallParams;
use wgtk::space::bin::TREE_INSTANCE;
use wgtk::space::Space;

use super::body::blocking;
use super::Rubble;


/// Gravity, in m/s²: `physics_shared.G`.
const G: f32 = 9.81;
/// How fast a fall starts, in rad/s, for each step of its speed, one more:
/// `__INIT_SPEED_NORMALISER`.
const INIT_SPEED_NORMALISER: f32 = 0.45;
/// How long a step of a fall is at most, in seconds: `__MAX_PHYS_UPDATE_DELAY`.
const MAX_STEP: f32 = 0.02;
/// How still a fall is once it lies, its pitch speed and torque together:
/// `__STOP_SIMULATION_EPSILON`.
const STOP_EPSILON: f32 = PI / 500.0;
/// How long a fall lasts at most, in seconds: `__MAX_SIMULATION_DURATION`.
const MAX_DURATION: f32 = 8.0;
/// How fast a vehicle goes for each step of a fall's speed, in m/s, of 3 at most.
/// Fitted on falling atoms: the captured Pershing knocked down lamp posts at 5.5 and
/// 12 m/s, which fell at 2 and 3; the captured trees fell mostly at 3.
const FALL_SPEED_STEP: f32 = 2.5;
/// How finely a tree's limit is looked for, in radians, see [`fall_limit`], before it
/// is refined, and how far it may tip at most. Our choice: a degree, and 135°, past
/// any tree the captures show, 103° at most.
const LIMIT_STEP: f32 = 1.0f32.to_radians();
const MAX_LIMIT: f32 = 135.0f32.to_radians();
/// How many halvings refine a tree's limit, see [`fall_limit`]: to a 250th of a degree.
const LIMIT_REFINES: usize = 8;
/// How far up its trunk a tree looks for what it falls against, in metres, see
/// [`blocked`]: past the ground and the decks around its foot. Our choice.
const TRUNK_CLEARANCE: f32 = 1.0;


/// A tree falling, or lying, see the module's documentation.
#[derive(Debug, Clone)]
pub struct TreeFall {
    /// The tree's index in its space's speed trees, its plant's, see
    /// `wgtk::space::foliage::Plant::tree`.
    pub tree: u32,
    /// The way it falls, the yaw of its fall, in radians: the vehicle's.
    pub yaw: f32,
    /// How fast it starts, of 0 to 3, see [`FALL_SPEED_STEP`].
    pub speed: u8,
    /// How far it may tip, in radians from upright: until its top meets the terrain,
    /// see [`fall_limit`].
    pub limit: f32,
    /// How far it tips now, and how fast, in radians and rad/s.
    pitch: f32,
    pitch_speed: f32,
    /// How long it has fallen, in seconds, and whether it still does.
    time: f32,
    falling: bool,
    /// Its kind's way of falling, scaled by its scale as the client scales it.
    mass: f32,
    height: f32,
    spring_angle: f32,
    spring_stiffness: f32,
    spring_resist: f32,
    air_resist: f32,
    bury_depth: f32,
    /// Its placement as it stood.
    placement: Affine3A,
}

impl TreeFall {

    /// A tree knocked down by a vehicle going `speed` in m/s toward `yaw`, falling in
    /// `space` against what of it stands, see [`fall_limit`], as it falls:
    /// `showFallTree`. None for a tree whose kind doesn't say how it falls.
    pub fn new(tree: &PlacedTree, space: &Space, rubble: &Rubble, yaw: f32, speed: f32) -> Option<Self> {
        let fall: FallParams = tree.fall?;
        let scale = tree.scale;
        let height = fall.height * scale;
        let speed = ((speed / FALL_SPEED_STEP) as u8).min(3);
        Some(Self {
            tree: tree.tree,
            yaw,
            speed,
            limit: fall_limit(space, rubble, tree.transform.translation.into(), height, yaw),
            pitch: 0.0,
            pitch_speed: (f32::from(speed) + 1.0) * INIT_SPEED_NORMALISER,
            time: 0.0,
            falling: true,
            mass: fall.mass * scale * scale * scale,
            height,
            spring_angle: fall.spring_angle,
            spring_stiffness: fall.spring_stiffness * scale * scale,
            spring_resist: fall.spring_resist * scale * scale,
            air_resist: fall.air_resist * scale * scale,
            bury_depth: fall.bury_depth * scale,
            placement: tree.transform,
        })
    }

    /// Let it fall for `dt` seconds, in steps of at most [`MAX_STEP`], as the client's
    /// `__update` does: whether it lies from then on, having fallen till now.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.falling {
            return false;
        }
        let steps = (dt / MAX_STEP).ceil().max(1.0);
        for _ in 0..steps as usize {
            if !self.move_body(dt / steps) {
                self.falling = false;
                return true;
            }
        }
        false
    }

    /// One step of its fall, `__moveBody`: whether it still falls.
    fn move_body(&mut self, dt: f32) -> bool {

        if self.pitch < 0.0 {
            self.pitch_speed = 0.0;
            self.pitch = 0.0;
        }
        if self.pitch > self.limit {
            self.pitch_speed = 0.0;
            self.pitch = self.limit;
        }

        // Gravity, the air's drag, and its branches once within their angle.
        let mp = self.height * self.height * 0.25;
        let mut torque = 0.5 * self.height * G * self.mass * self.pitch.sin();
        torque -= self.pitch_speed * self.air_resist * mp;
        if self.pitch + self.spring_angle > self.limit {
            let pen = self.pitch + self.spring_angle - self.limit;
            torque -= pen * self.spring_stiffness * mp;
            torque -= self.pitch_speed * self.spring_resist * mp;
        }

        let inertia = self.mass * self.height * self.height / 3.0;
        if inertia != 0.0 {
            self.pitch_speed += dt * torque / inertia;
        }
        self.time += dt;
        if self.pitch_speed.abs() + torque.abs() < STOP_EPSILON || self.time > MAX_DURATION {
            return false;
        }
        self.pitch += dt * self.pitch_speed;
        true

    }

    /// How far it tips now, in radians from upright.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// How tall it is, in metres, as its kind falls, scaled.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Its placement now, `__positionBodyModel`: as it stood, then turned by its pitch
    /// about the level axis across its fall, its top going its way, its foot sunk by
    /// its bury depth as it tips.
    pub fn transform(&self) -> Affine3A {
        let turn = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch) * Quat::from_rotation_y(-self.yaw);
        let sunk = Vec3A::new(0.0, self.bury_depth * self.pitch.sin(), 0.0);
        Affine3A {
            matrix3: Mat3A::from_quat(turn) * self.placement.matrix3,
            translation: self.placement.translation - sunk,
        }
    }

}

/// How far a tree of this `height` standing at `foot` may tip falling toward `yaw`, in
/// radians from upright: until it meets the terrain or a static model, see
/// [`blocked`], looked for by [`LIMIT_STEP`], then refined, at most [`MAX_LIMIT`].
fn fall_limit(space: &Space, rubble: &Rubble, foot: Vec3, height: f32, yaw: f32) -> f32 {
    let mut above = 0.0;
    let mut pitch = LIMIT_STEP;
    while pitch < MAX_LIMIT {
        if blocked(space, rubble, foot, height, yaw, pitch) {
            let mut below = pitch;
            for _ in 0..LIMIT_REFINES {
                let middle = (above + below) / 2.0;
                if blocked(space, rubble, foot, height, yaw, middle) {
                    below = middle;
                } else {
                    above = middle;
                }
            }
            return below;
        }
        above = pitch;
        pitch += LIMIT_STEP;
    }
    MAX_LIMIT
}

/// Whether a tree of this `height` at `foot`, tipped by `pitch` toward `yaw`, meets
/// what it falls on: its top at or under the terrain, off it under its foot's height,
/// or its trunk, from [`TRUNK_CLEARANCE`] up, through a static model that stops
/// vehicles, see [`blocking`], but other trees' trunks, ours.
fn blocked(space: &Space, rubble: &Rubble, foot: Vec3, height: f32, yaw: f32, pitch: f32) -> bool {

    let along = Vec3::new(yaw.sin() * pitch.sin(), pitch.cos(), yaw.cos() * pitch.sin());
    let top = foot + along * height;
    if top.y <= space.terrain.height_at(top.x, top.z).unwrap_or(foot.y) {
        return true;
    }

    let start = foot + along * TRUNK_CLEARANCE;
    space.static_ray(start, along, height - TRUNK_CLEARANCE, &|triangle: &StaticTriangle| {
        triangle.instance & TREE_INSTANCE == 0 && blocking(space, rubble)(triangle)
    }).is_some()

}

#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;

    use super::super::fixtures::wall;
    use super::*;

    /// A tree of the most common kind's way of falling, standing at `foot`.
    fn maple(foot: Vec3) -> PlacedTree {
        PlacedTree {
            chunk_id: 0x7f7f,
            index: 3,
            tree: 12,
            transform: Affine3A::from_translation(foot),
            scale: 1.0,
            health: 120.0,
            fall: Some(FallParams {
                mass: 800.0,
                height: 10.0,
                spring_angle: 0.26,
                spring_stiffness: 20000.0,
                spring_resist: 4000.0,
                air_resist: 20.0,
                bury_depth: 0.15,
            }),
        }
    }

    #[test]
    fn lies_where_gravity_and_its_branches_balance() {

        // On flat ground, it may tip a quarter turn, and lies short of it, held up by
        // its crown, where the client's solver puts it.
        let space = Space::default();
        let mut fall = TreeFall::new(&maple(Vec3::ZERO), &space, &Rubble::default(), 0.0, 10.0).unwrap();
        assert!((fall.limit - FRAC_PI_2).abs() < 1e-3, "{}", fall.limit);
        assert_eq!(fall.speed, 3);
        let mut ticks = 0;
        while !fall.step(0.1) {
            ticks += 1;
            assert!(ticks < 100, "still falling");
        }
        // Balanced: G·m·sin(p) = ½·h·k·(p − (limit − spring angle)).
        let pitch = fall.pitch();
        let gravity = G * 800.0 * pitch.sin();
        let branches = 0.5 * 10.0 * 20000.0 * (pitch - (fall.limit - 0.26));
        assert!((gravity - branches).abs() < 0.02 * gravity, "{gravity} against {branches} at {pitch}");
        assert!(pitch > 1.36 && pitch < 1.42, "lies at {pitch}");

        // Its top went its way, +Z, its foot sunk a little.
        let transform = fall.transform();
        let top = transform.transform_point3(Vec3::Y * 10.0);
        assert!(top.z > 9.5 && top.x.abs() < 1e-3 && top.y < 2.5, "top at {top}");
        assert!(transform.translation.y < 0.0 && transform.translation.y > -0.15);

    }


    #[test]
    fn falls_against_a_wall_short_of_the_ground() {
        // A wall 3 m high at z = 10, a tree 10 m high 6 m in front of it falling its
        // way: its trunk meets the wall's top at atan(6 / 3) from upright.
        let space = wall(3.0);
        let fall = TreeFall::new(&maple(Vec3::new(0.0, 0.0, 4.0)), &space, &Rubble::default(), 0.0, 10.0).unwrap();
        assert!((fall.limit - 2.0f32.atan()).abs() < 0.01, "{}", fall.limit);
        // Falling away from it, it lies on the ground.
        let fall = TreeFall::new(&maple(Vec3::new(0.0, 0.0, 4.0)), &space, &Rubble::default(), PI, 10.0).unwrap();
        assert!((fall.limit - FRAC_PI_2).abs() < 0.01, "{}", fall.limit);
    }

}
