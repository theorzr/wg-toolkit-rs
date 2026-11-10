//! What stops a hull on the ground plane: the walls of the map's static models
//! ([`Hull::push_out`]), and the map's edge ([`Hull::keep_within`]).
//!
//! # Walls
//!
//! The map's solid models, and the destructible walls still standing (see the `rubble`
//! module), are triangles in a grid ([`Statics`]). The hull's box is tested against
//! the triangles that are *walls* (see the `ground` module; flatter ones are ground), but
//! only against the part of each triangle in a *band* of heights, in the hull's own frame
//! (so a tilted hull's nose clears higher):
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
//! of a 2D separating-axis test ([`box_push`]). The deepest overlap is applied first, at
//! most [`WALL_PASSES`] times. The wall stops the hull's speed into it with an impulse at
//! its corner deepest in the wall (or the middle of an edge flat against it), as a
//! vehicle would that doesn't move: a corner swings the hull round. The rest slows by
//! [`WALL_FRICTION`] of it, so a tank scrapes along a wall instead of sticking.
//!
//! # The map's edge
//!
//! The same idea with 4 planes: the centre stays [`BOUNDS_MARGIN`] inside, the speed into
//! the edge is removed, and the rest slows by [`EDGE_FRICTION`].

use glam::{Vec2, Vec3, Vec3Swizzles};
use smallvec::SmallVec;

use wgtk::space::statics::{BodyKind, Statics, StaticTriangle};

use super::ground::{is_wall, STEP_HEIGHT};
use super::{Hull, Rubble};


/// How far inside the map's bounds the hull's centre stays, in metres. Fitted: the real
/// server holds it 0.3 m inside driven square on, 0.0 to 0.1 m inside at 30°.
const BOUNDS_MARGIN: f32 = 0.2;
/// How much of the speed an edge stops it also takes along it: their friction. Fitted
/// on the real server's drag along an edge driven into at 30°.
const EDGE_FRICTION: f32 = 0.25;
/// How much of the speed a wall stops it also takes along it. Our choice, an edge's
/// until fitted.
const WALL_FRICTION: f32 = EDGE_FRICTION;
/// How many times a step pushes the hull out of the static models it overlaps, the
/// deepest overlap first.
const WALL_PASSES: usize = 4;

impl Hull {

    /// Push its collision box out of the walls it overlaps, on the ground plane, see
    /// [`Self::overlaps`]: their triangles within `WALL_ANGLE` of vertical, the others
    /// being ground, between [`STEP_HEIGHT`] above its bottom and its top, the deepest
    /// overlap first, [`WALL_PASSES`] at most. What of its velocity goes into a wall
    /// stops, at its corner in the wall, which turns it, and slows the rest by
    /// [`WALL_FRICTION`].
    pub fn push_out(&mut self, statics: &Statics, rubble: &Rubble) {
        for _ in 0..WALL_PASSES {

            // 1. The deepest of the walls it overlaps, none left: it is out.
            let push = self.overlaps(statics, rubble)
                .map(|(_, push)| push)
                .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
            let Some(push) = push else { break };

            // 2. Out of it, then stopped against it.
            self.position += Vec3::new(push.x, 0.0, push.y);
            self.hit_wall(push.normalize());

        }
    }

    /// Stop what of its velocity goes into a wall it was just pushed out of along
    /// `normal`, and slow the rest by [`WALL_FRICTION`] of it.
    fn hit_wall(&mut self, normal: Vec2) {

        // Hit where it pushed into the wall: a corner swings it round, an edge flat
        // against it doesn't. The wall doesn't move, as if infinitely heavy.
        let footprint = self.footprint();
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

    /// The walls its collision box overlaps, see [`Self::push_out`]: solid ones, and the
    /// destructibles that block until broken, each with how far to push it out of it on
    /// the ground plane.
    fn overlaps<'a>(&self, statics: &'a Statics, rubble: &'a Rubble) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {
        let (min, max) = self.reach();
        let solid = statics.triangles_in(BodyKind::Solid, min, max);
        let standing = statics.triangles_in(BodyKind::Destructible, min, max)
            .filter(move |triangle| rubble.blocks(statics, triangle));
        self.overlapping(solid.chain(standing))
    }

    /// The destructible triangles still standing that its collision box overlaps, as
    /// [`Self::overlaps`] does, whether they block it or not: what it rams.
    pub fn standing_overlaps<'a>(&self, statics: &'a Statics, rubble: &'a Rubble) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {
        let (min, max) = self.reach();
        let standing = statics.triangles_in(BodyKind::Destructible, min, max)
            .filter(move |triangle| rubble.standing(statics, triangle).is_some());
        self.overlapping(standing)
    }

    /// The box around it its collision box may reach, however it is tilted: where the
    /// triangles it may overlap are.
    fn reach(&self) -> (Vec3, Vec3) {
        let [min, max] = self.mobility.bounds;
        let reach = Vec3::splat(min.abs().max(max.abs()).length());
        (self.position - reach, self.position + reach)
    }

    /// Those of these triangles that are walls its collision box overlaps, each with how
    /// far to push it out of it on the ground plane. They are tested in its own frame,
    /// tilted with it: nose up, its front clears higher.
    ///
    /// ```text
    ///   for each wall-like triangle near the hull (found in the grid by the box's reach):
    ///     1. move its 3 vertices into the hull's frame            (inverse rotation)
    ///     2. clip it to the band [bottom + STEP_HEIGHT, top]      (clip_to_heights)
    ///     3. what is left seen from above is a convex polygon     (≤ 5 vertices)
    ///     4. separate the hull's rectangle from it, least axis    (box_push)
    ///     5. bring the push back out of the hull's frame
    /// ```
    fn overlapping<'a>(&self, triangles: impl Iterator<Item = &'a StaticTriangle> + 'a) -> impl Iterator<Item = (&'a StaticTriangle, Vec2)> + 'a {

        let (position, rotation) = (self.position, self.rotation());
        let inverse = rotation.inverse();
        let [min, max] = self.mobility.bounds;
        let center = (min + max) / 2.0;
        let half = (max - min) / 2.0;
        let low = STEP_HEIGHT.max(min.y);
        let high = max.y;

        triangles
            .filter(|triangle| is_wall(triangle))
            .filter_map(move |triangle| {
                let local = triangle.vertices.map(|vertex| inverse * (vertex - position));
                let outline = clip_to_heights(&local, low, high)?;
                let push = box_push(center.xz(), Vec2::Y, Vec2::X, half.xz(), &outline)?;
                Some((triangle, (rotation * Vec3::new(push.x, 0.0, push.y)).xz()))
            })

    }

    /// Keep its centre within the map's bounds, [`BOUNDS_MARGIN`] inside them: what of
    /// its velocity goes into an edge stops, and slows the rest by [`EDGE_FRICTION`].
    /// Driven into an edge at an angle, it drags along it: its tracks keep pushing it in,
    /// and lose that share again each step.
    pub fn keep_within(&mut self, (min, max): (Vec2, Vec2)) {

        // Back inside on each axis, what of its velocity went out stopped.
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

}

/// Where a hull's footprint, its `corners`, touches a wall it was pushed out of along
/// `normal`: its corner furthest into the wall, or the middle of those as far, within
/// 5 cm, when an edge lies flat against it.
pub fn wall_contact(corners: &[Vec2; 4], normal: Vec2) -> Vec2 {
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
    let polygon = clip_to_height(triangle, low, |y| y >= low);
    let polygon = clip_to_height(&polygon, high, |y| y <= high);
    (!polygon.is_empty()).then(|| polygon.into_iter().map(|vertex| vertex.xz()).collect())
}

/// A polygon clipped to one side of a height, `keep` telling the side kept: a convex
/// polygon of at most [`Outline`]'s vertices stays so, with one more per plane.
fn clip_to_height(polygon: &[Vec3], height: f32, keep: impl Fn(f32) -> bool) -> SmallVec<[Vec3; 8]> {
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
        // Their shadows on the axis.
        let center = middle.dot(axis);
        let extent = half.x * right.dot(axis).abs() + half.y * heading.dot(axis).abs();
        let (low, high) = outline.iter().map(|vertex| vertex.dot(axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), p| (low.min(p), high.max(p)));
        // Pushed back past the outline's low side, or forward past its high side; apart
        // on any axis, they don't overlap.
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


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, wall};
    use super::super::movement_flag;
    use super::*;

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
        hull.set_direction(Vec3::new(0.0, 0.0, -30f32.to_radians()));
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

}
