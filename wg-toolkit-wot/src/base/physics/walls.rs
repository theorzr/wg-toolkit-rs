//! What stops a hull besides what its body holds it off, see the `body` module: the
//! map's edge ([`Hull::keep_within`]). And what it is about to ram ([`Hull::rammed`]),
//! see the `rubble` module.
//!
//! # Walls
//!
//! A wall is no different from the ground: what the hull's body touches of the space,
//! its terrain and its static models alike, holds it off along its normal, see the
//! `body` module, and its tracks grip only what is flat enough. Another vehicle shoving
//! it into one is held by both, in the same passes, see the `collision` module.
//!
//! # The map's edge
//!
//! The map's edge is 4 planes, on the ground plane: the centre stays [`BOUNDS_MARGIN`]
//! inside, the speed into the edge is removed, and the rest slows by [`EDGE_FRICTION`].

use glam::{Quat, Vec2, Vec3, Vec3Swizzles};
use smallvec::SmallVec;

use wgtk::space::{Cuboid, Surface};
use wgtk::space::statics::{BodyKind, StaticTriangle};
use wgtk::space::Space;

use super::contacts::Shape;
use super::{Hull, Rubble};


/// How far inside the map's bounds the hull's centre stays, in metres. Fitted: the real
/// server holds it 0.3 m inside driven square on, 0.0 to 0.1 m inside at 30°.
const BOUNDS_MARGIN: f32 = 0.2;
/// How much of the speed an edge stops it also takes along it: their friction. Fitted
/// on the real server's drag along an edge driven into at 30°.
const EDGE_FRICTION: f32 = 0.25;

impl Hull {

    /// The destructible triangles still standing it will be in by the end of a tick of
    /// `dt` seconds, as its speeds on the ground plane take it, its box grown by `reach`:
    /// what it rams, see `Hull::ram`, with which way it goes into each on the ground
    /// plane.
    pub fn rammed<'a>(&self, dt: f32, reach: f32, space: &'a Space, rubble: &Rubble) -> SmallVec<[(&'a StaticTriangle, Vec2); 8]> {
        let position = self.position + self.velocity() * dt;
        let orientation = (Quat::from_rotation_y(self.yaw_rate * dt) * self.orientation).normalize();
        let cuboid = self.hull_box(position, orientation);
        let keep = |triangle: &StaticTriangle| {
            !triangle.vehicles_pass
                && triangle.kind == BodyKind::Destructible
                && rubble.standing(&space.statics, triangle).is_some()
        };
        space.cuboid(&cuboid, reach, &keep)
            .into_iter()
            .filter_map(|contact| match contact.surface {
                Surface::Static(triangle) => Some((triangle, -contact.normal.xz())),
                Surface::Terrain => None,
            })
            .collect()
    }

    /// Its collision box, were it at `position` turned by `orientation`: its boxes
    /// together, its belly's, its upper hull's and its turret's, see the `contacts`
    /// module.
    fn hull_box(&self, position: Vec3, orientation: Quat) -> Cuboid {
        let (low, high) = self.contacts.iter()
            .filter_map(|contact| match contact.shape {
                Shape::Box { center, half } => Some((center - half, center + half)),
                _ => None,
            })
            .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(low, high), (min, max)| (low.min(min), high.max(max)));
        Cuboid {
            center: position + orientation * ((low + high) / 2.0),
            rotation: orientation,
            half: (high - low) / 2.0,
        }
    }

    // -- The map's edge --

    /// Keep its centre within the map's bounds, [`BOUNDS_MARGIN`] inside them: what of
    /// its velocity goes into an edge stops, and slows the rest by [`EDGE_FRICTION`].
    /// Driven into an edge at an angle, it drags along it: its tracks keep pushing it in,
    /// and lose that share again each step.
    pub fn keep_within(&mut self, (min, max): (Vec2, Vec2)) {

        // Back inside on each axis, what of its velocity went out stopped.
        let mut velocity = self.velocity();
        let mut stopped = Vec3::ZERO;
        for (axis, (least, most)) in [(0, (min.x, max.x)), (2, (min.y, max.y))] {
            let least = least + BOUNDS_MARGIN;
            let most = most - BOUNDS_MARGIN;
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

/// Where a hull's footprint, its `corners`, touches a wall it goes into against
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


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use super::super::fixtures::{chieftain, nose, slope, wall};
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
        // Its tracks still pushing at the ground, the wall holding it higher, it leans back
        // a little, its nose's bottom forward.
        let front = nose(&hull);
        assert!((front - 10.0).abs() < 2e-3 && hull.speed.abs() < 1e-2, "front at {front}, {} m/s", hull.speed);
        // A kerb lower than its tracks' front, it drives over.
        let space = wall(0.3);
        let mut hull = chieftain();
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut Rubble::default());
        }
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
    }

    #[test]
    fn stops_at_the_map_edge() {
        let space = slope(0.0);
        let bounds = Some((Vec2::splat(-100.0), Vec2::splat(100.0)));
        let edge = 100.0 - BOUNDS_MARGIN;
        // Square on at full speed, it stops dead, still pushing.
        let mut hull = chieftain();
        hull.position.z = 90.0;
        hull.speed = 40.0 / 3.6;
        hull.match_tracks();
        hull.flags = movement_flag::FORWARD;
        for _ in 0..20 {
            hull.advance(0.1, &space, bounds, &mut Rubble::default());
        }
        assert_eq!(hull.position.z, edge);
        assert!(hull.speed.abs() < 1e-4 && hull.position.x.abs() < 1e-4, "{} {}", hull.speed, hull.position.x);
        // At 30°, it drags along it, slower and slower, then crawls as its tracks push it
        // on: the real server's went 2.6, 1.8, 0.7 then 0.2 m/s once against it, ours
        // 2.8, 1.9, 1.2 then 0.5.
        let mut hull = chieftain();
        hull.position.z = edge;
        hull.set_direction(Vec3::new(0.0, 0.0, -30f32.to_radians()));
        hull.speed = 40.0 / 3.6;
        hull.match_tracks();
        hull.flags = movement_flag::FORWARD;
        let mut along = Vec::new();
        for _ in 0..6 {
            let x = hull.position.x;
            hull.advance(0.1, &space, bounds, &mut Rubble::default());
            along.push((x - hull.position.x) / 0.1);
        }
        assert_eq!(hull.position.z, edge);
        let slowing = along[..5].windows(2).all(|pair| pair[1] <= pair[0]);
        assert!(slowing && (2.0..3.0).contains(&along[1]) && along[4] < 1.0 && along[5] < 0.6, "{along:?}");
    }

}
