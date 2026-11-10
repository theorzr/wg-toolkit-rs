//! Where a hull's body touches the ground, in its own frame ([`contacts_of`]): a
//! spring under each road wheel, rigid points on its track around the wheels at its
//! ends, and rigid points on its hull's and its turret's boxes. Its body holds it on them,
//! see the `body` module.
//!
//! ```text
//!   seen from the side, front to the right:
//!
//!                *-----*-----*                  turret's box: its top's corners, middle
//!   *-------------*-------------*               hull's box: its top's corners, edges'
//!   |                           |                 middles and middle
//!   *                           *               half its height: its corners
//!   |                           |
//!   |#  *                   *  #|               its sides over its tracks' ends
//!    (O)  o   o   o   o   o  (O)                road wheels o, idler and sprocket (O)
//!      #__s___s___s___s___s__#                  springs s, under the road wheels;
//!                                               # rigid, on the track around the ends
//! ```

use smallvec::SmallVec;
use glam::Vec3;

use crate::wot::item_defs::{MobilityDef, Wheels, MAX_ROAD_WHEELS, MAX_TRACK_ENDS};

use super::body::SUSPENSION_TRAVEL;


/// How many contacts its suspension has along each track when its chassis' wheels
/// aren't known. Our choice: about one a metre.
const CONTACTS_PER_TRACK: usize = 6;
/// How many rigid contacts a hull's body has, its hull's box and its turret's, see
/// [`body_contacts`].
const BODY_CONTACTS: usize = 22;
/// The most contacts a hull has, a size `SmallVec` takes: a spring under each road
/// wheel, 4 rigid ones at each of its tracks' ends, and its body's.
pub const MAX_CONTACTS: usize = 96;
const _: () = assert!(MAX_ROAD_WHEELS + 4 * MAX_TRACK_ENDS + BODY_CONTACTS <= MAX_CONTACTS);

/// A hull's contacts, in their order, see [`contacts_of`].
pub type Contacts = SmallVec<[Contact; MAX_CONTACTS]>;

/// The points a hull's suspension and its body hold it at, in its own frame, for its
/// mobility: they never change, and its body keeps how deep each was in the ground from
/// a step to the next, by its place in them. Its tracks' first, see [`wheel_contacts`],
/// else, its wheels unknown, [`CONTACTS_PER_TRACK`] springs along each track, evenly
/// over the length its tracks carry it on, see [`even_contacts`]. Then its body's, see
/// [`body_contacts`].
pub fn contacts_of(mobility: &MobilityDef) -> Contacts {
    let mut contacts = SmallVec::new();
    match &mobility.wheels {
        Some(wheels) => wheel_contacts(wheels, &mut contacts),
        None => even_contacts(mobility, &mut contacts),
    }
    body_contacts(mobility, &mut contacts);
    contacts
}

/// Its tracks' contacts on its wheels. Under each of its road wheels, a spring, at its
/// origin's height, where its track rests on the ground. Around each wheel at its
/// tracks' ends, its idlers and sprockets, which have no spring, 4 rigid ones on its
/// track: under the wheel, at 45° out and down, at its outer end, and halfway down
/// the track from there to the road wheel nearest it, where a rock or a step meets the
/// track first.
///
/// ```text
///   around the idler, front to the right:
///
///                         (O)#   tip       each "radius + track thickness" out
///                           #    corner    of the idler's centre, but halfway
///                         #      below
///     o     o     o   #          halfway
///   __s_____s_____s__________    ground
/// ```
fn wheel_contacts(wheels: &Wheels, contacts: &mut Contacts) {

    for wheel in wheels.road() {
        contacts.push(Contact {
            point: Vec3::new(wheel.center.x, 0.0, wheel.center.z),
            kind: ContactKind::Spring,
        });
    }

    for end in wheels.ends() {
        // Out toward its end of the track, along it, as far as the track is around it.
        let out = end.center.z.signum();
        let reach = end.radius + wheels.track_thickness;
        let diagonal = std::f32::consts::FRAC_1_SQRT_2;
        let below = end.center - Vec3::Y * reach;
        let corner = end.center + Vec3::new(0.0, -diagonal, out * diagonal) * reach;
        let tip = end.center + Vec3::Z * out * reach;
        // Halfway from its corner to the road wheel nearest it, on its side.
        let nearest = wheels.road().iter()
            .filter(|wheel| wheel.center.x.signum() == end.center.x.signum())
            .max_by(|a, b| (a.center.z * out).total_cmp(&(b.center.z * out)));
        let halfway = nearest.map(|wheel| Vec3::new(wheel.center.x, 0.0, wheel.center.z).lerp(corner, 0.5));
        for point in [halfway, Some(below), Some(corner), Some(tip)].into_iter().flatten() {
            // Never lower than the springs rest: it would always be in the ground.
            contacts.push(Contact {
                point: point.with_y(point.y.max(0.0)),
                kind: ContactKind::TrackEnd,
            });
        }
    }

}

/// Its tracks' contacts, its wheels unknown: [`CONTACTS_PER_TRACK`] springs along each
/// track, evenly over the length its tracks carry it on, see
/// `MobilityDef::carrying_point`.
fn even_contacts(mobility: &MobilityDef, contacts: &mut Contacts) {
    let carrying = mobility.carrying_point;
    for index in 0..2 * CONTACTS_PER_TRACK {
        let side = if index < CONTACTS_PER_TRACK { 1.0 } else { -1.0 };
        let share = (index % CONTACTS_PER_TRACK) as f32 / (CONTACTS_PER_TRACK - 1) as f32;
        contacts.push(Contact {
            point: Vec3::new(side * carrying.x, 0.0, (share * 2.0 - 1.0) * carrying.y),
            kind: ContactKind::Spring,
        });
    }
}

/// Its body's rigid contacts, [`BODY_CONTACTS`] of them: what hits the ground when it
/// rolls over. Its hull's box's 4 corners at its top and 4 at half its height, the
/// middles of its top's edges and its top's middle, then its turret's box's top
/// corners and middle, as it faces ahead. At its box's sides' bottom, over its tracks'
/// ends on the ground, 4 more, their outer edges: on its side it lies on them, its
/// centre of mass being lower than half its height. They are as high as its springs
/// stop, [`SUSPENSION_TRAVEL`]: upright, they touch only once its springs did, or
/// rolled 40° onto them. Not its box's bottom corners, which reach past its tracks
/// and would scrape on every bump.
fn body_contacts(mobility: &MobilityDef, contacts: &mut Contacts) {

    let [min, max] = mobility.bounds;
    let middle = (min + max) / 2.0;
    let mut points: SmallVec<[Vec3; BODY_CONTACTS]> = SmallVec::new();

    // Its hull's box: at each corner, its top, half its height, and its side's bottom
    // over its track's end.
    let low = min.y.max(SUSPENSION_TRAVEL);
    let carried = mobility.carrying_point.y;
    for (x, z) in [(min.x, min.z), (min.x, max.z), (max.x, min.z), (max.x, max.z)] {
        points.push(Vec3::new(x, max.y, z));
        points.push(Vec3::new(x, middle.y, z));
        points.push(Vec3::new(x, low, z.signum() * carried));
    }
    // Its top's edges' middles and its middle.
    for (x, z) in [(middle.x, min.z), (middle.x, max.z), (min.x, middle.z), (max.x, middle.z), (middle.x, middle.z)] {
        points.push(Vec3::new(x, max.y, z));
    }

    // Its turret's box's top.
    let [turret_min, turret_max] = mobility.turret_bounds;
    let turret_middle = (turret_min + turret_max) / 2.0;
    for (x, z) in [
        (turret_min.x, turret_min.z),
        (turret_min.x, turret_max.z),
        (turret_max.x, turret_min.z),
        (turret_max.x, turret_max.z),
        (turret_middle.x, turret_middle.z),
    ] {
        points.push(Vec3::new(x, turret_max.y, z));
    }

    for point in points {
        contacts.push(Contact {
            point,
            kind: ContactKind::Body,
        });
    }

}

/// A point a hull touches the ground at, see [`contacts_of`], in its own frame.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub point: Vec3,
    pub kind: ContactKind,
}

/// What of a hull a [`Contact`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactKind {
    /// Under a road wheel, on a spring.
    Spring,
    /// On its track around a wheel at its end, rigid: its tracks grip there too.
    TrackEnd,
    /// On its hull or its turret, rigid: it scrapes there, nothing grips.
    Body,
}
