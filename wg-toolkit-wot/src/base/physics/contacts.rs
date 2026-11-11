//! Where a hull touches the space, in its own frame ([`contacts_of`]): a spring under
//! each road wheel, spheres along its tracks around the wheels at their ends, and boxes,
//! its hull's and its turret's. Its body asks the space where each touches it, the
//! terrain and the static models alike, see the `body` module.
//!
//! ```text
//!   seen from the side, front to the right:
//!
//!                 ┌───────────┐                   its turret's box
//!   ┌───────────────────────────────┐             its hull's boxes: all its length over
//!   └──┬─────────────────────────┬──┘               its tracks' ends' wheels' centres,
//!      └─────────────────────────┘                  its belly between its road wheels
//!    (O)·  o   o   o   o   o   o  ·(O)            spheres: around the idler and the
//!      ·.  |   |   |   |   |   |  .·                sprocket (O), and two between each
//!        ··s···s···s···s···s···s··                  and its nearest road wheel o
//!     ==============================   ground     springs s: a ray down from each
//!                                                   road wheel's centre
//! ```
//!
//! Its tracks' spheres meet what its road wheels would run into: a rock, a step, a
//! kerb, which lift its nose as its track's front climbs onto them, giving as its road
//! wheels do. Its hull's boxes leave its tracks' ends to them: its belly, between its
//! road wheels, is as high as its tracks' ends reach down, and its nose and its tail are
//! over them, so that they touch the ground only when the hull rolls, or on what is
//! higher than its tracks.

use smallvec::SmallVec;
use glam::Vec3;

use crate::game::item_defs::{MobilityDef, Wheels, MAX_ROAD_WHEELS, MAX_TRACK_ENDS};

use super::drive::Side;


/// How many springs its suspension has along each track when its chassis' wheels
/// aren't known. Our choice: about one a metre.
const SPRINGS_PER_TRACK: usize = 6;
/// How high a road wheel's centre is over where its track touches the ground, in
/// metres, its wheels unknown. Our choice, about a real one's.
const WHEEL_HEIGHT: f32 = 0.35;
/// How high its hull's belly is, in metres, and its nose and its tail over its tracks'
/// ends, its wheels unknown. Our choice, about a real one's.
const CLEARANCE: f32 = 0.4;
const ENDS_HEIGHT: f32 = 0.8;
/// How many spheres there are around each wheel at its tracks' ends and down toward the
/// nearest road wheel: the wheel's, and those between.
const END_SPHERES: usize = 3;
/// The most contacts a hull has, a size `SmallVec` takes: a spring under each road
/// wheel, [`END_SPHERES`] at each of its tracks' ends, and its three boxes.
pub const MAX_CONTACTS: usize = 64;
const _: () = assert!(MAX_ROAD_WHEELS + END_SPHERES * MAX_TRACK_ENDS + 3 <= MAX_CONTACTS);

/// A hull's contacts, in their order, see [`contacts_of`].
pub type Contacts = SmallVec<[Contact; MAX_CONTACTS]>;

/// Where a hull touches the space, in its own frame, for its mobility: they never
/// change, and its body keeps how deep each spring was from a step to the next, by its
/// place in them. Its springs first, see [`wheel_contacts`], else, its wheels unknown,
/// [`SPRINGS_PER_TRACK`] along each track, see [`even_wheel_contacts`]. Then its boxes.
pub fn contacts_of(mobility: &MobilityDef) -> Contacts {

    let mut contacts = SmallVec::new();
    let [min, max] = mobility.bounds;
    let hull = match &mobility.wheels {
        Some(wheels) => wheel_contacts(wheels, &mut contacts),
        None => even_wheel_contacts(mobility, &mut contacts),
    };

    // Its hull: its belly between its road wheels, from its clearance up, then all its
    // length over its tracks' ends; and its turret, as it faces ahead.
    let top = max.y - 0.1;
    let boxes = [
        (
            Vec3::new(min.x, hull.clearance.clamp(min.y, top), hull.belly[0].max(min.z)),
            Vec3::new(max.x, max.y, hull.belly[1].min(max.z)),
        ),
        (min.with_y(hull.ends.clamp(min.y, top)), max),
        (mobility.turret_bounds[0], mobility.turret_bounds[1]),
    ];
    
    for (low, high) in boxes {
        contacts.push(Contact {
            shape: Shape::Box {
                center: (low + high) / 2.0,
                half: (high - low) / 2.0,
            },
            side: None,
        });
    }

    contacts

}

/// What of its hull its tracks leave to its boxes, see [`contacts_of`]: how high its
/// belly is, and its nose and its tail over its tracks' ends, and how far back and
/// forward its belly goes, between its road wheels.
struct HullShape {
    clearance: f32,
    ends: f32,
    belly: [f32; 2],
}

/// Its tracks' contacts on its wheels, and what of its hull they leave to its boxes:
/// its belly as high as its tracks' ends' wheels reach down, between its road wheels,
/// and over them from as high as their centres. Under each of its road wheels, a
/// spring, its ray down from the wheel's centre to where its track rests on the ground.
/// Around each wheel at its tracks' ends, its idlers and sprockets, which have no spring
/// of their own, a sphere as wide as the wheel and its track around it, and
/// [`END_SPHERES`] less one between it and the road wheel nearest it, along the track's
/// slope from one to the other: they give as the road wheels' springs do.
///
/// ```text
///   around the idler, front to the right:
///
///                    (O)    the idler's sphere: its radius and the track's thickness
///               ( )         those between, as wide as the wheels they are between
///          ( )
///     o     o               road wheels, each a spring
///   __s_____s_______        ground
/// ```
fn wheel_contacts(wheels: &Wheels, contacts: &mut Contacts) -> HullShape {

    for wheel in wheels.road() {
        contacts.push(Contact {
            shape: Shape::Spring {
                point: Vec3::new(wheel.center.x, 0.0, wheel.center.z),
                height: wheel.center.y,
            },
            side: Some(Side::of(wheel.center.x)),
        });
    }

    let mut hull = HullShape {
        clearance: f32::INFINITY,
        ends: 0.0,
        belly: [f32::INFINITY, f32::NEG_INFINITY],
    };

    for wheel in wheels.road() {
        hull.belly = [hull.belly[0].min(wheel.center.z), hull.belly[1].max(wheel.center.z)];
    }

    for end in wheels.ends() {

        hull.clearance = hull.clearance.min(end.center.y - end.radius - wheels.track_thickness);
        hull.ends = hull.ends.max(end.center.y);

        let out = end.center.z.signum();
        let nearest = wheels.road().iter()
            .filter(|wheel| wheel.center.x.signum() == end.center.x.signum())
            .max_by(|a, b| (a.center.z * out).total_cmp(&(b.center.z * out)));
        
        let toward = nearest.map_or((end.center, end.radius), |wheel| (wheel.center, wheel.radius));
        for index in 0..END_SPHERES {
            let share = index as f32 / END_SPHERES as f32;
            let center = end.center.lerp(toward.0, share);
            let radius = end.radius + (toward.1 - end.radius) * share + wheels.track_thickness;
            contacts.push(Contact {
                shape: Shape::Sphere {
                    center,
                    radius,
                },
                side: Some(Side::of(end.center.x)),
            });
        }

    }

    hull

}

/// Its tracks' contacts, its wheels unknown: [`SPRINGS_PER_TRACK`] springs along each
/// track, evenly over the length its tracks carry it on, see
/// `MobilityDef::carrying_point`; and what of its hull they leave to its boxes: its
/// belly over that length, [`CLEARANCE`] high, and over its tracks' ends from
/// [`ENDS_HEIGHT`].
fn even_wheel_contacts(mobility: &MobilityDef, contacts: &mut Contacts) -> HullShape {
    
    let carrying = mobility.carrying_point;
    for index in 0..2 * SPRINGS_PER_TRACK {
        let side = if index < SPRINGS_PER_TRACK { 1.0 } else { -1.0 };
        let share = (index % SPRINGS_PER_TRACK) as f32 / (SPRINGS_PER_TRACK - 1) as f32;
        contacts.push(Contact {
            shape: Shape::Spring {
                point: Vec3::new(side * carrying.x, 0.0, (share * 2.0 - 1.0) * carrying.y),
                height: WHEEL_HEIGHT,
            },
            side: Some(Side::of(side)),
        });
    }

    HullShape {
        clearance: CLEARANCE,
        ends: ENDS_HEIGHT,
        belly: [-carrying.y, carrying.y],
    }

}

/// A part of a hull that touches the space, see [`contacts_of`], in its own frame, and
/// which of its tracks it is of, if any: those grip the ground.
#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub shape: Shape,
    pub side: Option<Side>,
}

/// The shape of a [`Contact`], in its hull's frame.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// A spring under a road wheel: where its track rests on the ground, at the hull's
    /// origin's height, and how high its wheel's centre is over it, where its ray down
    /// to the ground starts.
    Spring {
        point: Vec3,
        height: f32,
    },
    /// Around a wheel at its track's end, or between it and a road wheel: it gives up its
    /// hull as the road wheels' springs do, see the `body` module.
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// Rigid: its hull's or its turret's.
    Box {
        center: Vec3,
        half: Vec3,
    },
}
