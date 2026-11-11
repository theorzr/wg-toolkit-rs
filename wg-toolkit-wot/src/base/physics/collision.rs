//! Vehicles against each other, in 3D: each is a [`Solid`], its hull's boxes and its
//! tracks' ends' spheres where its step of its body starts, and where two touch, the
//! world holds them apart as it holds each off the space, in the same passes, see
//! [`World::step`](super::World::step).
//!
//! # Where they touch
//!
//! Each vehicle's body touches the others' as it touches the space, see the `contacts`
//! module: its boxes and its tracks' ends' spheres against the others' boxes, and its
//! springs' rays down onto the others' boxes, which carry it then, pushed back as hard
//! as its springs push, see the `body` module. A box against a box, by the separating
//! axis theorem: two convex shapes don't overlap if some axis exists on which their
//! shadows don't, and for two boxes it is enough to try 15 axes, the 3 faces' normals
//! of each and the 9 crossings of an edge of one with an edge of the other. The axis
//! with the *least* overlap is the cheapest way apart:
//!
//! ```text
//!   on one axis n:      A ├────────────────┤
//!                                    B ├────────────────┤
//!                                      ├────┤ depth d      ◀ least over the 15 axes
//! ```
//!
//! Along a face's normal, its box is the reference: the other's face most against it,
//! the incident face, is clipped by the reference face's sides, and each of its corners
//! left is a point of contact, as deep as it is past the reference face. A box resting
//! on another's face touches it at four points, and is held level on it. Along an edge
//! crossing, the two edges touch at one point, where they come nearest:
//!
//! ```text
//!       incident face  ┌───────────┐                ┌─╲
//!                ╶─────┼─*───────*─┼─────╴          │  ╲  edges crossing:
//!     reference  │     └───────────┘     │          │ * ╲ one point
//!         face   │          B            │          └────
//!                └───────────────────────┘
//! ```
//!
//! Faces are preferred to edges by [`EDGE_AXIS_BIAS`]: near-parallel edges cross on a
//! noisy axis. A sphere touches a box at the box's point nearest its centre, or, its
//! centre inside it, out of its nearest face.
//!
//! # Holding them apart
//!
//! Each point is held as the space holds a box, see `Hull::solve_pass`, but with both
//! vehicles moving: the impulse stops the speed they close at there, against both their
//! masses and inertias, and they share it, so that the lighter is shoved the more, and
//! a hit off one's middle spins it. They close no more than their gap in a step, so
//! that fast ones meet rather than go into each other, and their points are looked for
//! as far apart as they could close in a step. They rub on each other, held back by
//! [`VEHICLE_FRICTION`] of how hard they are pressed. What one got into the other is
//! moved back out, split, as from the space:
//!
//! ```text
//!   v_close = (v_A + ω_A × r_A − v_B − ω_B × r_B) · n          (< 0 when closing in)
//!
//!                     −(v_close + max(−d, 0) / h)
//!   j = ─────────────────────────────────────────────────────    its sum ≥ 0
//!        1/m_A + 1/m_B + (r_A × n) · I_A⁻¹ (r_A × n) + (r_B × n) · I_B⁻¹ (r_B × n)
//!
//!   A: v += j·n/m_A     ω += I_A⁻¹ (r_A × j·n)                 B: the opposite
//! ```
//!
//! where `n` is the way out of B, toward A, `d` how deep they are, negative apart, `h`
//! a step, and `r_A`, `r_B` the arms from each one's centre of mass to the point.

use glam::{Mat3, Quat, Vec3};
use smallvec::SmallVec;

use wgtk::app::slots::SlotKey;

use super::body::{Mass, Step, CONTACT_MARGIN, STOP_RECOVERY, STOP_SPEED};
use super::contacts::Shape;
use super::Hull;


/// How much the overlap along an edge pair's axis counts for more than along a face's,
/// in metres: crossed edges of near-parallel boxes give noisy axes. Our choice.
const EDGE_AXIS_BIAS: f32 = 0.05;
/// How much two vehicles rubbing hold each other back, as a share of how hard they are
/// pressed together. From the game: `CONTACT_FRICTION_VEHICLES` in `physics_shared.py`.
const VEHICLE_FRICTION: f32 = 0.3;

/// A vehicle's body where its step starts, for the others to touch: its boxes and its
/// tracks' ends' spheres, a sphere around them all, and the most any of its points
/// moves, in m/s. Its springs' rays touch others, and aren't touched.
#[derive(Debug, Clone)]
pub struct Solid {
    pub key: SlotKey,
    center: Vec3,
    radius: f32,
    speed: f32,
    boxes: SmallVec<[Obb; 3]>,
    spheres: SmallVec<[(Vec3, f32); 16]>,
}

impl Solid {

    /// A hull's body, the vehicle `key` of its world, where it is.
    pub fn of(key: SlotKey, hull: &Hull) -> Self {

        let axes = Mat3::from_quat(hull.orientation);
        let mut boxes = SmallVec::new();
        let mut spheres = SmallVec::new();
        for contact in &hull.contacts {
            match contact.shape {
                Shape::Spring { .. } => {}
                Shape::Sphere { center, radius } => spheres.push((hull.position + axes * center, radius)),
                Shape::Box { center, half } => boxes.push(Obb {
                    center: hull.position + axes * center,
                    axes,
                    half,
                }),
            }
        }

        let (center, radius) = hull.reach();
        Self {
            key,
            center,
            radius,
            speed: hull.linear_velocity.length() + hull.angular_velocity.length() * radius,
            boxes,
            spheres,
        }

    }

    /// Where a ray from `from` along `direction`, a unit vector, first enters its boxes
    /// within `length`: how far, and their normal there.
    pub fn ray(&self, from: Vec3, direction: Vec3, length: f32) -> Option<(f32, Vec3)> {
        if (from - self.center).length() > self.radius + length {
            return None;
        }
        self.boxes.iter()
            .filter_map(|obb| obb.ray(from, direction, length))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Where it touches another, both grown by `margin`, see the module's documentation:
    /// each point, the way out of the other toward it, and how deep they are there,
    /// negative short of it.
    fn meet(&self, other: &Self, margin: f32, mut each: impl FnMut(Vec3, Vec3, f32)) {
        if self.center.distance(other.center) > self.radius + other.radius + margin {
            return;
        }
        for obb in &self.boxes {
            for other_obb in &other.boxes {
                obb.meet(other_obb, margin, &mut each);
            }
            for &(center, radius) in &other.spheres {
                if let Some((point, normal, depth)) = obb_sphere(obb, center, radius, margin) {
                    each(point, -normal, depth);
                }
            }
        }
        for &(center, radius) in &self.spheres {
            for other_obb in &other.boxes {
                if let Some((point, normal, depth)) = obb_sphere(other_obb, center, radius, margin) {
                    each(point, normal, depth);
                }
            }
        }
    }

    /// How deep it is in another, at its deepest: none if they don't touch.
    pub fn depth_in(&self, other: &Self) -> Option<f32> {
        let mut deepest: Option<f32> = None;
        self.meet(other, 0.0, |_, _, depth| {
            deepest = Some(deepest.map_or(depth, |deepest| deepest.max(depth)));
        });
        deepest
    }

}

/// Where two vehicles' bodies touch in a step `step` seconds long, see
/// [`Solid::meet`], to hold apart: looked for as far apart as they could close in it.
pub fn touching(first: &Solid, second: &Solid, step: f32, out: &mut Vec<Touching>) {
    let margin = CONTACT_MARGIN + (first.speed + second.speed) * step;
    first.meet(second, margin, |point, normal, depth| out.push(Touching {
        first: first.key,
        second: second.key,
        point,
        normal,
        depth,
        stop: 0.0,
        friction: Vec3::ZERO,
    }));
}

/// A point where two vehicles' bodies touch in a step, see [`touching`]: the way out of
/// the second toward the first, how deep they are, negative apart, and the impulses
/// holding them apart and rubbing so far.
#[derive(Debug, Clone, Copy)]
pub struct Touching {
    pub first: SlotKey,
    pub second: SlotKey,
    point: Vec3,
    normal: Vec3,
    depth: f32,
    stop: f32,
    friction: Vec3,
}

/// One pass of holding two vehicles apart where they touch, each its hull and its step
/// under way, see the module's documentation: what they close at there stopped, shared
/// by their masses, their rubbing held back by [`VEHICLE_FRICTION`], and what they are
/// in each other moved back out of, split.
pub fn hold_apart(
    touching: &mut Touching,
    (first, first_step): (&mut Hull, &mut Step),
    (second, second_step): (&mut Hull, &mut Step),
    step: f32,
) {

    let normal = touching.normal;
    let arm = touching.point - first_step.mass_center;
    let other_arm = touching.point - second_step.mass_center;
    let (body, other_body) = (first_step.body, second_step.body);
    let effective = body.per_impulse(arm, normal) + other_body.per_impulse(other_arm, normal);

    // 1. Held apart: closing no more than their gap.
    let closing = (first.velocity_at(arm) - second.velocity_at(other_arm)).dot(normal);
    let gap = (-touching.depth).max(0.0) / step;
    let held = (touching.stop - (gap + closing) / effective).max(0.0);
    let impulse = normal * (held - touching.stop);
    touching.stop = held;
    share(impulse, (first, arm, &body), (second, other_arm, &other_body));

    // 2. Rubbing, as hard as they are pressed together.
    let relative = first.velocity_at(arm) - second.velocity_at(other_arm);
    let slip = relative - normal * relative.dot(normal);
    if let Some(way) = slip.try_normalize() {
        let along = body.per_impulse(arm, way) + other_body.per_impulse(other_arm, way);
        let wanted = touching.friction - way * (slip.length() / along);
        let friction = wanted.clamp_length_max(VEHICLE_FRICTION * touching.stop);
        let change = friction - touching.friction;
        touching.friction = friction;
        share(change, (first, arm, &body), (second, other_arm, &other_body));
    }

    // 3. In each other: moved back out, without keeping the speed it took.
    if touching.depth > 0.0 {
        let moved = (first_step.push_velocity + first_step.push_spin.cross(arm)
            - second_step.push_velocity - second_step.push_spin.cross(other_arm)).dot(normal);
        let short = (STOP_RECOVERY * touching.depth / step).min(STOP_SPEED) - moved;
        if short > 0.0 {
            let impulse = normal * (short / effective);
            first_step.push_velocity += impulse / body.mass;
            first_step.push_spin += body.inverse_inertia * arm.cross(impulse);
            second_step.push_velocity -= impulse / other_body.mass;
            second_step.push_spin -= other_body.inverse_inertia * other_arm.cross(impulse);
        }
    }

}

/// An impulse given to the first of two hulls, at its arm, and taken from the second,
/// at its own.
fn share(
    impulse: Vec3,
    (first, arm, body): (&mut Hull, Vec3, &Mass),
    (second, other_arm, other_body): (&mut Hull, Vec3, &Mass),
) {
    first.take(impulse, arm, body);
    second.take(-impulse, other_arm, other_body);
}

impl Hull {

    /// A sphere around its boxes and its tracks' ends, where it is: its middle and its
    /// radius.
    pub fn reach(&self) -> (Vec3, f32) {
        let (low, high) = self.contacts.iter()
            .filter_map(|contact| match contact.shape {
                Shape::Box { center, half } => Some((center - half, center + half)),
                Shape::Sphere { center, radius } => Some((center - radius, center + radius)),
                Shape::Spring { .. } => None,
            })
            .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(low, high), (min, max)| (low.min(min), high.max(max)));
        (self.position + self.orientation * ((low + high) / 2.0), (high - low).length() / 2.0)
    }

}

/// A hull's place: its position and its orientation.
#[derive(Debug, Clone, Copy)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Pose {

    pub fn of(hull: &Hull) -> Self {
        Self {
            position: hull.position,
            orientation: hull.orientation,
        }
    }

}

/// A box in space, turned: its middle, its axes (right, up, heading) and its half sizes
/// along them.
#[derive(Debug, Clone, Copy)]
pub struct Obb {
    pub center: Vec3,
    pub axes: Mat3,
    pub half: Vec3,
}

/// An axis two boxes are apart along, or overlap least along: a face's normal of the
/// first or of the second, by its axis, or the crossing of the first's edge along one
/// of its axes with the second's along one of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    First(usize),
    Second(usize),
    Edges(usize, usize),
}

impl Obb {

    /// Its axis, by its index: right, up, heading.
    fn axis(&self, index: usize) -> Vec3 {
        self.axes.col(index)
    }

    /// How far it reaches along an axis from its middle.
    fn reach(&self, axis: Vec3) -> f32 {
        self.half.x * self.axes.x_axis.dot(axis).abs()
            + self.half.y * self.axes.y_axis.dot(axis).abs()
            + self.half.z * self.axes.z_axis.dot(axis).abs()
    }

    /// The way it overlaps another least, both grown by `margin`, by the separating axis
    /// theorem over their face normals and their edge pairs: the axis, pointing from the
    /// other box to this one, how far they overlap along it, not grown, and which it is.
    /// None if they are apart.
    ///
    /// For each of 15 axes (3 + 3 faces, 3 × 3 edge crossings) the boxes' shadows on it
    /// are intervals centred on the projection of their centres, with half lengths
    /// [`Self::reach`]; their overlap on it is
    /// `reach(A) + reach(B) − |(centre A − centre B) · axis|`. One axis with a gap
    /// proves the boxes apart; otherwise the axis with the smallest overlap is the
    /// shortest way out. Edge crossings of near-parallel edges are noisy (and null if
    /// parallel, skipped), so they count [`EDGE_AXIS_BIAS`] more than faces.
    fn least_overlap(&self, other: &Self, margin: f32) -> Option<(Vec3, f32, Axis)> {

        let offset = self.center - other.center;
        let mut least: Option<(Vec3, f32, f32, Axis)> = None;

        for index in 0..15 {

            let (axis, bias, which) = match index {
                0..3 => (self.axis(index), 0.0, Axis::First(index)),
                3..6 => (other.axis(index - 3), 0.0, Axis::Second(index - 3)),
                _ => {
                    let (own, others) = ((index - 6) / 3, (index - 6) % 3);
                    (self.axis(own).cross(other.axis(others)), EDGE_AXIS_BIAS, Axis::Edges(own, others))
                }
            };
            let Some(axis) = axis.try_normalize().filter(|_| axis.length() > 1e-3) else { continue };

            let along = offset.dot(axis);
            let overlap = self.reach(axis) + other.reach(axis) - along.abs();
            if overlap <= -margin {
                return None;
            }

            if least.is_none_or(|(_, _, least, _)| overlap + bias < least) {
                let way = if along < 0.0 { -axis } else { axis };
                least = Some((way, overlap, overlap + bias, which));
            }

        }

        least.map(|(way, overlap, _, which)| (way, overlap, which))

    }

    /// Where it touches another, both grown by `margin`, see the module's documentation:
    /// each point, the way out of the other toward it, and how deep it is there.
    fn meet(&self, other: &Self, margin: f32, each: &mut impl FnMut(Vec3, Vec3, f32)) {

        let Some((normal, overlap, axis)) = self.least_overlap(other, margin) else { return };

        // Edges crossing: where they come nearest.
        let (reference, incident, out, index) = match axis {
            Axis::First(index) => (self, other, -normal, index),
            Axis::Second(index) => (other, self, normal, index),
            Axis::Edges(own, others) => {
                let (point, other_point) = nearest_on_edges(self.edge(own, -normal), other.edge(others, normal));
                each((point + other_point) / 2.0, normal, overlap);
                return;
            }
        };

        // 1. The incident face: its box's face most against the reference face, `out` of
        //    which the incident box is.
        let facing = (0..3)
            .max_by(|&a, &b| incident.axis(a).dot(out).abs().total_cmp(&incident.axis(b).dot(out).abs()))
            .unwrap_or(0);
        let face_normal = incident.axis(facing) * -incident.axis(facing).dot(out).signum();
        let middle = incident.center + face_normal * incident.half[facing];
        let (u, v) = ((facing + 1) % 3, (facing + 2) % 3);
        let (u_reach, v_reach) = (incident.axis(u) * incident.half[u], incident.axis(v) * incident.half[v]);
        let mut face: SmallVec<[Vec3; 8]> = SmallVec::from_slice(&[
            middle + u_reach + v_reach,
            middle + u_reach - v_reach,
            middle - u_reach - v_reach,
            middle - u_reach + v_reach,
        ]);

        // 2. Clipped to the reference face's sides.
        for side in [(index + 1) % 3, (index + 2) % 3] {
            let axis = reference.axis(side);
            let at = reference.center.dot(axis);
            face = clip(&face, axis, at + reference.half[side]);
            face = clip(&face, -axis, -at + reference.half[side]);
        }

        // 3. Each corner left, as deep as it is past the reference face, at the middle
        //    between them.
        let plane = reference.center.dot(out) + reference.half[index];
        for corner in face {
            let depth = plane - corner.dot(out);
            if depth >= -margin {
                each(corner + out * (depth / 2.0), normal, depth);
            }
        }

    }

    /// Its edge along its axis `index` furthest the way `toward`: its middle and its
    /// half, its axis times its half size.
    fn edge(&self, index: usize, toward: Vec3) -> (Vec3, Vec3) {
        let mut middle = self.center;
        for other in [(index + 1) % 3, (index + 2) % 3] {
            middle += self.axis(other) * self.half[other] * self.axis(other).dot(toward).signum();
        }
        (middle, self.axis(index) * self.half[index])
    }

    /// Where a ray from `from` along `direction`, a unit vector, enters it within
    /// `length`, by the slabs between its faces: how far, and its face's normal there.
    /// None from inside it.
    fn ray(&self, from: Vec3, direction: Vec3, length: f32) -> Option<(f32, Vec3)> {
        let mut enter = f32::NEG_INFINITY;
        let mut exit = f32::INFINITY;
        let mut normal = Vec3::ZERO;
        for index in 0..3 {
            let axis = self.axis(index);
            let offset = (from - self.center).dot(axis);
            let speed = direction.dot(axis);
            if speed.abs() < 1e-9 {
                if offset.abs() > self.half[index] {
                    return None;
                }
                continue;
            }
            let near = (-speed.signum() * self.half[index] - offset) / speed;
            let far = (speed.signum() * self.half[index] - offset) / speed;
            if near > enter {
                enter = near;
                normal = axis * -speed.signum();
            }
            exit = exit.min(far);
        }
        (enter >= 0.0 && enter <= exit && enter <= length).then_some((enter, normal))
    }

}

/// Where a sphere touches a box, grown by `margin`: the box's point nearest its centre,
/// the way out of the box toward it, and how deep it is; its centre inside the box, out
/// of its nearest face.
fn obb_sphere(obb: &Obb, center: Vec3, radius: f32, margin: f32) -> Option<(Vec3, Vec3, f32)> {

    let local = obb.axes.transpose() * (center - obb.center);
    let nearest = local.clamp(-obb.half, obb.half);

    if nearest != local {
        let point = obb.center + obb.axes * nearest;
        let away = center - point;
        let distance = away.length();
        if distance > radius + margin {
            return None;
        }
        return Some((point, away / distance.max(1e-6), radius - distance));
    }

    // Inside: out of the face it is nearest.
    let index = (0..3)
        .min_by(|&a, &b| (obb.half[a] - local[a].abs()).total_cmp(&(obb.half[b] - local[b].abs())))
        .unwrap_or(0);
    let mut on_face = local;
    on_face[index] = obb.half[index] * local[index].signum();
    let normal = obb.axis(index) * local[index].signum();
    Some((obb.center + obb.axes * on_face, normal, radius + obb.half[index] - local[index].abs()))

}

/// The points of two edges, each its middle and its half, where they come nearest.
fn nearest_on_edges((middle, half): (Vec3, Vec3), (other_middle, other_half): (Vec3, Vec3)) -> (Vec3, Vec3) {
    let (length, other_length) = (half.length().max(1e-6), other_half.length().max(1e-6));
    let (way, other_way) = (half / length, other_half / other_length);
    let offset = middle - other_middle;
    let along = way.dot(other_way);
    let (e, f) = (way.dot(offset), other_way.dot(offset));
    let denominator = 1.0 - along * along;
    let s = if denominator > 1e-6 { ((along * f - e) / denominator).clamp(-length, length) } else { 0.0 };
    let t = (f + along * s).clamp(-other_length, other_length);
    let s = (along * t - e).clamp(-length, length);
    (middle + way * s, other_middle + other_way * t)
}

/// What of a convex polygon lies on the side of a plane `axis · p ≤ at`: a convex
/// polygon, with one more vertex at most.
fn clip(polygon: &[Vec3], axis: Vec3, at: f32) -> SmallVec<[Vec3; 8]> {
    let mut clipped = SmallVec::new();
    for (index, &from) in polygon.iter().enumerate() {
        let to = polygon[(index + 1) % polygon.len()];
        let from_inside = at - from.dot(axis);
        let to_inside = at - to.dot(axis);
        if from_inside >= 0.0 {
            clipped.push(from);
        }
        if (from_inside >= 0.0) != (to_inside >= 0.0) {
            clipped.push(from.lerp(to, from_inside / (from_inside - to_inside)));
        }
    }
    clipped
}


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use wgtk::app::slots::Slots;
    use wgtk::space::Space;

    use crate::wot::item_defs::MobilityDef;

    use super::super::fixtures::{chieftain, chieftain_aiming, chieftain_mobility, slope};
    use super::super::{movement_flag, rotation_of, Rubble, Vehicle, World};
    use super::*;

    /// How deep two hulls are in each other at their deepest, none if they don't touch.
    fn depth(hull: &Hull, other: &Hull) -> Option<f32> {
        let key = Slots::default().insert(());
        Solid::of(key, hull).depth_in(&Solid::of(key, other))
    }

    /// A Chieftain driven forward for `steps` ticks into another parked 10 m ahead, of
    /// this weight and `height` above it, `offset` aside, on flat ground, or on none above
    /// it: where both end.
    fn push_parked(weight: f32, height: f32, offset: f32, steps: usize) -> (Hull, Hull) {
        let space = if height == 0.0 { slope(0.0) } else { Space::default() };
        let mut world = World::new(Arc::new(space), None);
        let mut pusher = Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        pusher.hull.flags = movement_flag::FORWARD;
        let pusher = world.add(pusher);
        let mobility = MobilityDef {
            weight,
            ..chieftain_mobility()
        };
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
        let flat = slope(0.0);
        for _ in 0..60 {
            alone.advance(0.1, &flat, None, &mut Rubble::default());
        }
        // Into another as heavy, standing, it pushes it ahead, at a crawl, never into it:
        // the other's brakes hold it back nearly as hard as its engine pushes.
        let (pusher, parked) = push_parked(55_000.0, 0.0, 0.0, 60);
        let overlap = depth(&pusher, &parked);
        assert!(overlap.is_none_or(|depth| depth < 0.02), "{overlap:?}");
        assert!(parked.position.z > 12.0 && pusher.position.z < alone.position.z - 2.0, "{} {} {}", pusher.position.z, parked.position.z, alone.position.z);
        // Straight on, give or take its solver's jitter between its tracks: a few
        // centimetres over 8 m pushed.
        assert!(pusher.position.x.abs() < 0.1 && parked.position.x.abs() < 0.1, "{} {}", pusher.position.x, parked.position.x);
        // A lighter one, further.
        let (_, light) = push_parked(20_000.0, 0.0, 0.0, 60);
        assert!(light.position.z > parked.position.z + 1.0, "{} {}", light.position.z, parked.position.z);
        // Above it, on a bridge, it drives under, as it would alone.
        let mut alone = chieftain();
        alone.flags = movement_flag::FORWARD;
        for _ in 0..60 {
            alone.advance(0.1, &Space::default(), None, &mut Rubble::default());
        }
        let (pusher, parked) = push_parked(55_000.0, 3.0, 0.0, 60);
        assert!(parked.position.z == 10.0 && pusher.position.z == alone.position.z, "{} {}", parked.position, pusher.position);
    }

    /// Ram a parked vehicle's side at `speed`, `offset` from its middle along it, and let
    /// them go for 3 s: the rammer, then the rammed, which first faces +X.
    fn ram_side(speed: f32, offset: f32) -> (Hull, Hull) {
        let mut world = World::new(Arc::new(slope(0.0)), None);
        let [min, max] = chieftain_mobility().bounds;
        // Its nose 5 cm short of the parked one's left side.
        let start = 7.0 - (max.x - min.x) / 2.0 - max.z - 0.05;
        let mut rammer = Vehicle::new(Vec3::new(offset, 0.0, start), Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        rammer.hull.speed = speed;
        rammer.hull.match_tracks();
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
        // Rammed in its side behind its box's middle, its nose swings towards the rammer,
        // a few degrees, its tracks holding it: the yaw drops; ahead of it, the other way,
        // as much; the rammer keeps its heading.
        let [min, max] = chieftain_mobility().bounds;
        let middle = (min.z + max.z) / 2.0;
        let (rammer, behind) = ram_side(8.0, middle + 1.5);
        let (_, ahead) = ram_side(8.0, middle - 1.5);
        let behind = behind.yaw() - FRAC_PI_2;
        let ahead = ahead.yaw() - FRAC_PI_2;
        assert!(behind < -0.03, "{behind}");
        // As much give or take its turning about its position, not its box's middle. The
        // rammed one's side swinging across its nose, the rammer's tracks hold it to a
        // degree or so.
        assert!((behind + ahead).abs() < 0.3 * behind.abs(), "{behind} {ahead}");
        assert!(rammer.yaw().abs() < 0.05, "{}", rammer.yaw());
        // A bump turns it much less.
        let (_, bumped) = ram_side(3.0, middle + 1.5);
        let bumped = bumped.yaw() - FRAC_PI_2;
        assert!(bumped.abs() < 0.6 * behind.abs(), "{bumped} {behind}");
        // Pushed slowly off its middle from behind, the pair turns together, hardly, away
        // from the push: its brakes hold it back along its tracks, and its tracks its
        // turning. Not checked against the real server.
        let (pusher, pushed) = push_parked(55_000.0, 0.0, -1.5, 60);
        let together = (pushed.yaw() - pusher.yaw()).abs() < 0.05;
        assert!(together && (-0.3..0.0).contains(&pushed.yaw()), "{} {}", pushed.yaw(), pusher.yaw());
    }

    #[test]
    fn does_not_get_into_vehicles_at_speed() {
        // Driving at 80 km/h, it moves 2 m a tick: into another's side, it stops against
        // it rather than being pushed sideways through it.
        let mut world = World::new(Arc::new(slope(0.0)), None);
        let mut pusher = Vehicle::new(Vec3::new(0.0, 0.0, -15.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        pusher.hull.speed = 80.0 / 3.6;
        pusher.hull.match_tracks();
        pusher.hull.flags = movement_flag::FORWARD;
        let pusher = world.add(pusher);
        let parked = world.add(Vehicle::new(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        for _ in 0..30 {
            world.step(0.1);
            let overlap = depth(&world.vehicle(pusher).hull, &world.vehicle(parked).hull);
            assert!(overlap.is_none_or(|depth| depth < 0.1), "got into it: {overlap:?}");
        }
        let (pusher, parked) = (&world.vehicle(pusher).hull, &world.vehicle(parked).hull);
        assert!(pusher.position.z > -5.0 && parked.position.z > 0.0, "{} {}", pusher.position, parked.position);
    }

    #[test]
    fn rests_on_vehicles_it_lands_on() {
        // Dropped onto another from 1 m above its top, it lands on it, on its springs, and
        // rests there, never in it; the one under bears its weight, sinking further into
        // its own springs than alone.
        let mut world = World::new(Arc::new(slope(0.0)), None);
        let under = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let [_, max] = chieftain_mobility().bounds;
        let top = Vehicle::new(Vec3::new(0.0, max.y + 1.0, 0.5), Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        let top = world.add(top);
        for _ in 0..50 {
            world.step(0.1);
            let overlap = depth(&world.vehicle(top).hull, &world.vehicle(under).hull);
            assert!(overlap.is_none_or(|depth| depth < 0.1), "got into it: {overlap:?}");
        }
        let (top, under) = (&world.vehicle(top).hull, &world.vehicle(under).hull);
        assert!(top.position.y > max.y - 0.3 && top.linear_velocity.length() < 0.05, "{} {}", top.position, top.linear_velocity);
        assert!(under.position.y < -0.05, "{}", under.position);
        assert!(!top.airborne, "its tracks on the other's top");
    }

    #[test]
    fn stalls_against_vehicles_driving_into_it() {
        // Two as heavy driving into each other, a little off each other's middle, stop
        // nose to nose, neither rearing up onto the other, nor getting into it.
        let mut world = World::new(Arc::new(slope(0.0)), None);
        let mut first = Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming());
        first.hull.flags = movement_flag::FORWARD;
        let first = world.add(first);
        let facing = Vec3::new(0.0, 0.0, std::f32::consts::PI);
        let mut second = Vehicle::new(Vec3::new(-0.5, 0.0, 12.0), facing, chieftain_mobility(), chieftain_aiming());
        second.hull.flags = movement_flag::FORWARD;
        let second = world.add(second);
        for _ in 0..60 {
            world.step(0.1);
            let (first, second) = (&world.vehicle(first).hull, &world.vehicle(second).hull);
            let overlap = depth(first, second);
            assert!(overlap.is_none_or(|depth| depth < 0.05), "got into it: {overlap:?}");
            for hull in [first, second] {
                assert!(hull.direction().y.abs() < 5f32.to_radians() && hull.position.y.abs() < 0.1, "{} {}", hull.position, hull.direction());
            }
        }
        let (first, second) = (&world.vehicle(first).hull, &world.vehicle(second).hull);
        assert!(first.speed.abs() < 0.1 && second.speed.abs() < 0.1, "{} {}", first.speed, second.speed);
    }

    /// A box of these half sizes at a place and tilted so, to test with.
    fn obb(center: Vec3, direction: Vec3, half: Vec3) -> Obb {
        Obb {
            center,
            axes: Mat3::from_quat(rotation_of(direction)),
            half,
        }
    }

    /// Where two boxes touch: each point, its normal and its depth.
    fn meet(obb: &Obb, other: &Obb) -> Vec<(Vec3, Vec3, f32)> {
        let mut points = Vec::new();
        obb.meet(other, 0.0, &mut |point, normal, depth| points.push((point, normal, depth)));
        points
    }

    #[test]
    fn boxes_touch_in_space() {
        let half = Vec3::new(1.0, 1.0, 2.0);
        let square = obb(Vec3::ZERO, Vec3::ZERO, half);
        // Side by side 0.5 m into each other, it is pushed back along the way it came, at
        // the four corners of where their faces meet.
        let points = meet(&obb(Vec3::new(1.5, 0.0, 0.0), Vec3::ZERO, half), &square);
        assert_eq!(points.len(), 4, "{points:?}");
        for (_, normal, depth) in points {
            assert!((normal - Vec3::X).length() < 1e-5 && (depth - 0.5).abs() < 1e-5, "{normal} {depth}");
        }
        assert!(meet(&obb(Vec3::new(2.5, 0.0, 0.0), Vec3::ZERO, half), &square).is_empty());
        // One a little into the other's top, up out of it.
        let points = meet(&obb(Vec3::new(0.3, 1.9, 0.5), Vec3::ZERO, half), &square);
        assert!(points.len() == 4 && points.iter().all(|(_, normal, _)| (*normal - Vec3::Y).length() < 1e-5), "{points:?}");
        // Nose up 40°, its front reaches higher than a flat box's: a box clear of the flat
        // one over its front end, 1.4 m up, is in the way of the tilted one.
        let over = obb(Vec3::new(0.0, 2.4, 2.5), Vec3::ZERO, half);
        assert!(meet(&over, &obb(Vec3::ZERO, Vec3::ZERO, half)).is_empty());
        assert!(!meet(&over, &obb(Vec3::ZERO, Vec3::new(0.0, -40f32.to_radians(), 0.0), half)).is_empty());
        // Turned 45° about two axes, edge on edge: one point, between the edges.
        let turned = obb(Vec3::new(0.0, 2.3, 0.0), Vec3::new(45f32.to_radians(), 0.0, 45f32.to_radians()), Vec3::ONE);
        let points = meet(&turned, &obb(Vec3::ZERO, Vec3::ZERO, Vec3::ONE));
        assert!(!points.is_empty() && points.iter().all(|(_, normal, _)| normal.y > 0.5), "{points:?}");
        // A ray down onto it enters its top.
        let (distance, normal) = square.ray(Vec3::new(0.5, 3.0, 0.0), Vec3::NEG_Y, 5.0).unwrap();
        assert!((distance - 2.0).abs() < 1e-5 && (normal - Vec3::Y).length() < 1e-5, "{distance} {normal}");
        assert!(square.ray(Vec3::new(0.5, 0.0, 0.0), Vec3::NEG_Y, 5.0).is_none());
    }

}
