//! Two vehicles against each other ([`collide`]): their boxes in 3D, where they first
//! touched within a tick, and the push and the impulse between them. The world runs it
//! over every pair, see [`World::collide`](super::World::collide).
//!
//! # Their boxes
//!
//! Each vehicle has a box ([`MobilityDef::bounds`](crate::wot::item_defs::MobilityDef))
//! oriented as the hull is, with its roll, pitch and yaw: an [`Obb`]. A tank on a bridge
//! ramp is tilted, a tank on the deck is above one on the road. Two tilted boxes are
//! tested with the separating axis theorem in 3D. Two convex shapes don't overlap if some
//! axis exists on which their projections don't overlap, and for two boxes it is enough
//! to try 15 axes: the 3 faces' normals of each, and the 9 cross products of an edge of
//! one with an edge of the other. The axis with the *least* overlap is the cheapest way
//! to separate them:
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
//! # Where they first touched
//!
//! At 75 km/h a vehicle moves 2 m in a tick, which is about as far as its box is deep:
//! testing only at the end of the tick finds a deep overlap and the least axis can be a
//! sideways one, throwing the vehicle through the other. So the world remembers each
//! vehicle's pose at the start of the tick and sweeps both boxes over the tick, halving
//! the interval to find the first overlap:
//!
//! ```text
//!   start of tick        first touch  t*             end of tick (deep overlap)
//!   ┌─────┐   ┌─────┐    ┌─────┬─────┐                ┌─────┬▒▒▒┬─────┐
//!   │  A  │ → │  B  │    │  A  │  B  │                │  A  │▒▒▒│  B  │
//!   └─────┘   └─────┘    └─────┴─────┘                └─────┴▒▒▒┴─────┘
//!                          ▲ both are put back here (6 halvings of the tick)
//! ```
//!
//! # The push and the impulse
//!
//! From there the overlap is shallow and the axis can be trusted. The push is split by
//! weight, `A` moves by `m_B / (m_A + m_B)` of it, so the heavier one barely moves. Then
//! the speeds along the normal `n` are made equal *at the contact point*, with the usual
//! rigid-body impulse for a collision that doesn't bounce, which includes the turning
//! each vehicle takes:
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
//!     │      ┌────┼──────┐     * its centroid: the contact point
//!     │      │▒▒*▒│      │     r_A: from A's middle to *     r_B: from B's middle to *
//!     └──────┼────┘      │
//!            │     B     │
//!            └───────────┘
//! ```
//!
//! Then both go on for what was left of the tick (`(1 − t*) · dt`) at their new speeds
//! and turning, and the world puts them back out of walls and within the map.

use glam::{Mat3, Quat, Vec2, Vec3, Vec3Swizzles};
use smallvec::SmallVec;

use super::{Footprint, Hull};


/// How vertical a contact normal may be, as its height share, before one vehicle is on
/// top of the other, which its ground holds, not a push: about 45°. Our choice.
const STACKED_NORMAL: f32 = 0.7;
/// How much the overlap along an edge pair's axis counts for more than along a face's,
/// in metres: crossed edges of near-parallel boxes give noisy axes.
const EDGE_AXIS_BIAS: f32 = 0.01;
/// How many times the moment two vehicles first touched is halved: the step's length over
/// `2^FIRST_TOUCH_HALVINGS`.
const FIRST_TOUCH_HALVINGS: usize = 6;

/// Push two hulls apart on the ground plane where their collision boxes overlap in
/// space, tilted as the hulls are, having moved from the given poses in `dt` seconds:
/// where they first touched along the way they stop, and then each moves by the other's
/// share of their weight, and they share the speed they close in on each other at, at
/// the middle of the overlap, their momentum kept and their hits off-centre spinning
/// them, so the heavier shoves the lighter, and they move on at it for what was left of
/// the step. Driven on, the pusher's engine pushes them both, against both their grounds.
/// Returns whether they touched.
pub fn collide(hull: &mut Hull, from: Pose, other: &mut Hull, other_from: Pose, dt: f32) -> bool {

    // 1. Whether they touch at all: too far apart for their boxes, or apart on an axis.
    let (obb, other_obb) = (hull.obb(), other.obb());
    if obb.center.distance(other_obb.center) >= obb.half.length() + other_obb.half.length() {
        return false;
    }
    if obb.push(&other_obb).is_none() {
        return false;
    }

    // 2. Where they touch, in what they overlap on the ground plane as the step ends: at
    //    the moment they first touch, boxes a hair off square meet at a corner, which
    //    would spin them wildly.
    let footprint = hull.footprint();
    let other_footprint = other.footprint();
    let contact = overlap_center(&footprint, &other_footprint.corners());
    let arm = contact - footprint.middle;
    let other_arm = contact - other_footprint.middle;

    // 3. Stopped where they first touched, rather than pushed out of how deep they got.
    let to = Pose::of(hull);
    let other_to = Pose::of(other);
    let sweep = Sweep {
        hull,
        from,
        to,
    };
    let other_sweep = Sweep {
        hull: other,
        from: other_from,
        to: other_to,
    };
    let t = first_touch(&sweep, &other_sweep);
    if t < 1.0 {
        hull.place(from.lerp(to, t));
        other.place(other_from.lerp(other_to, t));
    }
    let Some(push) = hull.obb().push(&other.obb()) else { return false };

    // 4. Pushed apart, each by the other's share of their weight.
    let mass = hull.mobility.weight;
    let other_mass = other.mobility.weight;
    let share = other_mass / (mass + other_mass);
    hull.position += Vec3::new(push.x, 0.0, push.y) * share;
    other.position -= Vec3::new(push.x, 0.0, push.y) * (1.0 - share);

    // 5. Their speeds there, along the way they pushed, as the second box leaves the
    //    first: closing in, the impulse that stops them, along the normal, what each
    //    gives of its weight and of its spin.
    let normal = push.normalize();
    let speed = hull.point_velocity(arm);
    let other_speed = other.point_velocity(other_arm);
    let closing = (speed - other_speed).dot(normal);
    if closing < 0.0 {
        let inertia = hull.yaw_inertia();
        let other_inertia = other.yaw_inertia();
        let lever = arm.perp_dot(normal);
        let other_lever = other_arm.perp_dot(normal);
        let effective = 1.0 / mass + 1.0 / other_mass + lever * lever / inertia + other_lever * other_lever / other_inertia;
        let impulse = -closing / effective;
        hull.hit(normal * impulse, arm, mass, inertia);
        other.hit(-normal * impulse, other_arm, other_mass, other_inertia);
    }

    // 6. On, for what was left of the step.
    hull.coast((1.0 - t) * dt);
    other.coast((1.0 - t) * dt);
    true

}

impl Hull {

    /// Its collision box in space.
    pub fn obb(&self) -> Obb {
        self.obb_at(Pose::of(self))
    }

    /// Its collision box in space, were it at this pose.
    fn obb_at(&self, pose: Pose) -> Obb {
        let axes = Mat3::from_quat(pose.orientation);
        let [min, max] = self.mobility.bounds;
        Obb {
            center: pose.position + axes * ((min + max) / 2.0),
            axes,
            half: (max - min) / 2.0,
        }
    }

    /// Put it at a pose.
    fn place(&mut self, pose: Pose) {
        self.position = pose.position;
        self.orientation = pose.orientation;
    }

    /// Let it go on for `time` seconds at the speed and turning it has, its tracks holding
    /// its spin first, as in [`Self::drive`].
    fn coast(&mut self, time: f32) {
        self.hold_spin(time);
        self.position += self.velocity() * time;
        self.orientation = Quat::from_rotation_y((self.yaw_rate + self.spin) * time) * self.orientation;
    }

}

/// A hull's place, as its box sweeps it: its position and its orientation.
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

    /// The pose a share `t` of the way to another, turning the short way round.
    fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(to.position, t),
            orientation: self.orientation.slerp(to.orientation, t),
        }
    }

}

/// A hull's collision box in space, as tilted as the hull is: its middle, its axes (right,
/// up, heading) and its half sizes along them.
#[derive(Debug, Clone, Copy)]
pub struct Obb {
    pub center: Vec3,
    pub axes: Mat3,
    pub half: Vec3,
}

impl Obb {

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
    pub fn separation(&self, other: &Self) -> Option<(Vec3, f32)> {

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

    /// How far it reaches along an axis from its middle.
    fn reach(&self, axis: Vec3) -> f32 {
        self.half.x * self.axes.x_axis.dot(axis).abs()
            + self.half.y * self.axes.y_axis.dot(axis).abs()
            + self.half.z * self.axes.z_axis.dot(axis).abs()
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

/// The middle of what a hull's footprint overlaps of a convex outline of four corners:
/// the centroid of the overlap.
///
/// The outline is clipped by the box's four sides in turn (Sutherland–Hodgman: each
/// edge crossing a side is cut where it crosses), leaving a convex polygon of at most
/// 8 vertices, whose centroid is the area-weighted mean of the triangles it makes with
/// its first vertex. The plain average of the vertices would lean toward the side with
/// more of them, which clipping duplicates along the box's sides.
fn overlap_center(footprint: &Footprint, corners: &[Vec2; 4]) -> Vec2 {

    let middle = footprint.middle;
    let mut overlap = SmallVec::<[Vec2; 8]>::from_slice(corners);
    for (axis, extent) in [(footprint.heading, footprint.half.y), (footprint.right, footprint.half.x)] {
        overlap = clip_to_side(&overlap, |point| extent - (point - middle).dot(axis));
        overlap = clip_to_side(&overlap, |point| extent + (point - middle).dot(axis));
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
    if area.abs() > 1e-6 {
        first + sum / (3.0 * area)
    } else {
        overlap.iter().sum::<Vec2>() / overlap.len() as f32
    }

}

/// A polygon clipped to the side of a line where `inside` is positive.
fn clip_to_side(polygon: &[Vec2], inside: impl Fn(Vec2) -> f32) -> SmallVec<[Vec2; 8]> {
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


#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use wgtk::space::Space;

    use crate::wot::item_defs::MobilityDef;

    use super::super::fixtures::{chieftain, chieftain_aiming, chieftain_mobility};
    use super::super::{movement_flag, rotation_of, Rubble, Vehicle, World};
    use super::*;

    /// A Chieftain driven forward for `steps` ticks into another parked 10 m ahead, of
    /// this weight and `height` above it, `offset` aside: where both end.
    fn push_parked(weight: f32, height: f32, offset: f32, steps: usize) -> (Hull, Hull) {
        let mut world = World::new(Arc::new(Space::default()), None);
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
        let behind = behind.yaw() - FRAC_PI_2;
        let ahead = ahead.yaw() - FRAC_PI_2;
        assert!(behind < -0.1, "{behind}");
        // As much give or take its turning about its position, not its box's middle.
        assert!((behind + ahead).abs() < 0.15 * behind.abs(), "{behind} {ahead}");
        assert!(rammer.yaw().abs() < 0.01, "{}", rammer.yaw());
        // A bump hardly turns it: its tracks hold it.
        let (_, bumped) = ram_side(3.0, 1.5);
        assert!((bumped.yaw() - FRAC_PI_2).abs() < 0.03, "{}", bumped.yaw());
        // Nor does a slow push off its middle from behind, both moving along their tracks.
        let (pusher, pushed) = push_parked(55_000.0, 0.0, -1.5, 60);
        assert!(pushed.yaw().abs() < 0.05 && pusher.yaw().abs() < 0.05, "{} {}", pushed.yaw(), pusher.yaw());
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

}
