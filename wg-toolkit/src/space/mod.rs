//! Spaces, as the game ships them in `spaces/<name>`: their compiled space, see
//! [`bin`], and what moves in a space needs of it, see [`Space`].
//!
//! # Static models and destructibles, end to end
//!
//! What a vehicle can hit besides the terrain is the space's *static models*: houses,
//! rocks, fences, lamp posts, bridges. Some of them are *destructible*. Four sources
//! describe them, and [`statics::Statics::load`] joins them:
//!
//! ```text
//!   spaces/<map>/space.bin                           scripts/destructibles.xml
//!   ├─ BWST  strings, by hash ───────────┐           (every map's destructibles,
//!   ├─ BSMO  models: bounds, .havok hash ┤            by model path: kind, health,
//!   ├─ BSMI  placed models: transform,   │            a house's modules)
//!   │        model index, visibility     │                    │
//!   └─ WGDE  the client's numbering:     │                    ▼
//!            chunk → index → BSMI ids    │           destructibles::Destructibles
//!                    │                   ▼                    │
//!                    │        <model>.havok collision         │
//!                    │        (bodies of triangles, each      │
//!                    │         with a name and material kind) │
//!                    │                   │                    │
//!                    └──────────┬────────┴────────────────────┘
//!                               ▼
//!                    statics::Statics
//!                    ├─ triangles, in the world, each with its BSMI instance,
//!                    │  material kind and BodyKind (solid, ramp, destructible, remains)
//!                    ├─ a grid of 8 m cells per BodyKind, to find them near a box
//!                    └─ PlacedDestructible by BSMI instance: the client's
//!                       (chunk id, index), kind, scaled health, modules
//! ```
//!
//! The statics only describe the map as it was built. What breaks during a battle is
//! kept by its physics world, see `Rubble` in `wg-toolkit-wot`'s `base::physics`.
//!
//! Each step, in order:
//!
//! 1. [`bin`] reads the sections of `space.bin`. A string is referenced by its hash
//!    in [`bin::BWST`]. A model of [`bin::BSMO`] gives its `.havok` collision's path
//!    that way, or none for decor nothing collides with. A placed model of
//!    [`bin::BSMI`] is one of those models, with a transform and the gameplay types
//!    it exists in, see [`visibility_mask`].
//! 2. [`crate::havok::collision::Collision`] reads each model's `.havok` once: its
//!    *bodies*, each a set of triangles with a name (`s_wall_1`, `n_wood0_1`,
//!    `s_ramp_0`) and a material kind.
//! 3. [`destructibles::Destructibles`] reads `scripts/destructibles.xml`: which models
//!    are destructible, of which kind, with which health, and a house's modules.
//! 4. [`statics::BodyKind::of`] sorts each body by its name and its model's entry.
//!    Every triangle of every placed model visible in the battle goes into the statics,
//!    in the world's frame.
//! 5. [`bin::WGDE`] gives each destructible the number the client knows it by, which
//!    the server must use to say it broke: its 100 m chunk's id and its index there.
//!    One destructible can be several placed models, all numbered the same.
//!
//! # Contacts
//!
//! What moves in a space touches its surfaces, its terrain and its static models, as one.
//! It asks where it touches them, for a shape: a ray ([`Space::ray`]), a
//! sphere ([`Space::sphere`]), or a box ([`Space::cuboid`]). Each contact has a point on
//! the surface, its normal out of it toward the shape, and how deep the shape is in it,
//! with what it touched ([`Surface`]): the terrain, or a static model's triangle.
//!
//! ## Surfaces
//!
//! Both are triangles, found near the shape (`Space::surfaces_in`), which only the
//! queries see: the terrain's are read from its heights as a query walks its cells, two
//! for each, never kept; the static models' are in their grid. The static models' triangles are two-sided, their winding telling
//! nothing: a shape is pushed out the side it is nearer. The terrain is one-sided: a
//! shape under it is pushed up, whatever its depth. What a query is given to `keep`
//! decides which static triangles count, those of a destructible broken say; the
//! remains of destructibles never do, nothing being there until one breaks.
//!
//! ## Shapes
//!
//! ```text
//!   ray      the first surface it crosses within its length, Möller–Trumbore's test
//!   sphere   each surface within its radius: its point nearest the centre
//!   box      each surface it overlaps, by the separating axis test: the way out of it
//!            the least deep, along the box's 3 axes, the surface's normal, and the 9
//!            crossings of their edges, the surface's normal unless FACE_BIAS deeper;
//!            out of the terrain, only up its normal; its points the surface's corners
//!            within the box, each as deep as the box must move the way out to leave it
//! ```
//!
//! A sphere and a box are grown by a `margin`: what is that near touches already, at a
//! negative depth, so that what moves toward a surface can stop on it rather than in
//! it. A shape touching several triangles of one face, the terrain's cells or a wall's
//! two, would get their shared edges' points twice and the same normal over and over:
//! their contacts the same way out of the same surface, the terrain or one placed
//! model, are one, a sphere's the deepest, a box's `MANIFOLD` points at most, those
//! spanning the most of them. Two models in the same place, as some maps have, each
//! keep theirs.

pub mod bin;
pub mod terrain;
pub mod statics;
pub mod destructibles;

use std::io;

use glam::{Quat, Vec3, Vec3Swizzles};
use smallvec::SmallVec;

use crate::res::fs::ResFilesystem;
use crate::pxml;

use statics::{BodyKind, StaticTriangle, Statics};
use destructibles::Destructibles;
use terrain::Terrain;


/// A space as what moves in it needs it: its terrain and its static models' collision.
/// The default one is empty, with neither.
#[derive(Debug, Default)]
pub struct Space {
    pub terrain: Terrain,
    pub statics: Statics,
    /// What of the game's models is destructible, the space's or not: what the
    /// destructibles of [`Self::statics`] were read from, and the settings that apply to
    /// all of them.
    pub destructibles: Destructibles,
}

impl Space {

    /// Load a space, `spaces/08_ruinberg` say, as a gameplay type has it, `ctf` say:
    /// only its static models visible in it, see [`visibility_mask`], what of them is
    /// destructible as the game lists it.
    pub fn load(fs: &ResFilesystem, space_dir: &str, gameplay: &str) -> io::Result<Self> {
        let destructibles = Destructibles::load(fs)?;
        let mask = visibility_mask(fs, space_dir, gameplay)?;
        Ok(Self {
            terrain: Terrain::load(fs, space_dir)?,
            statics: Statics::load(fs, space_dir, &destructibles, mask)?,
            destructibles,
        })
    }

}


/// The gameplay types, `ARENA_GAMEPLAY_NAMES` in the game's `constants.py`: a type's id
/// is its index.
pub const GAMEPLAY_NAMES: &[&str] = &[
    "ctf", "domination", "assault", "nations", "ctf2", "domination2", "assault2", "fallout",
    "fallout2", "fallout3", "fallout4", "ctf30x30", "domination30x30", "sandbox", "bootcamp",
    "epic", "maps_training", "rts", "rts_1x1", "rts_bootcamp", "comp7",
];

/// How many of a visibility mask's low bits are the server's, the gameplay types', see
/// `SpaceVisibilityFlags._SERVER_FLAGS_NUMBER`.
const SERVER_VISIBILITY_BITS: u32 = 20;

/// The visibility mask of a gameplay type in a space, which the static models in it
/// have, see [`bin::StaticModelInstance::visibility_mask`]: as the client's
/// `SpaceVisibilityFlags.py` reads it, its bit as the space's
/// `space.settings/spaceVisibilityFlags` maps it, else its id.
pub fn visibility_mask(fs: &ResFilesystem, space_dir: &str, gameplay: &str) -> io::Result<u32> {

    let path = format!("{space_dir}/space.settings");
    let settings = pxml::from_reader(io::BufReader::new(fs.read(&path)?))
        .map_err(|e| invalid(format!("{path}: {e}")))?;

    let bit = match settings.get_child_element("spaceVisibilityFlags") {
        Some(flags) => flags.iter_children("type")
            .filter_map(|flag| flag.as_element())
            .find(|flag| flag.get_child_string("tag") == Some(gameplay))
            .and_then(|flag| flag.get_child_integer("bitIndex"))
            .ok_or_else(|| invalid(format!("{path}: no visibility bit for gameplay {gameplay}")))? as u32,
        None => GAMEPLAY_NAMES.iter().position(|name| *name == gameplay)
            .ok_or_else(|| invalid(format!("unknown gameplay {gameplay}")))? as u32,
    };

    if bit >= SERVER_VISIBILITY_BITS {
        return Err(invalid(format!("{path}: visibility bit {bit} of gameplay {gameplay} isn't the server's")));
    }
    Ok(1 << bit)

}


// -- Contacts --

/// How alike two contacts' normals are, as their dot product, for them to be the same
/// way out of a surface.
const SAME_WAY: f32 = 0.999;
/// The most points a box keeps of its contacts the same way out of a surface: enough
/// to rest on a face.
const MANIFOLD: usize = 4;
/// How much deeper than the least deep way out of a triangle, in metres, a box is moved
/// out along the triangle's own normal: its edges may be inside a face of many
/// triangles, a rock's say, where a box would catch on them. Our choice.
const FACE_BIAS: f32 = 0.05;
/// The static models' bodies that a shape can touch, see [`BodyKind`]: not the
/// remains of a destructible, which are nothing until it breaks.
const TOUCHED: [BodyKind; 3] = [BodyKind::Solid, BodyKind::Ramp, BodyKind::Destructible];

/// What a contact touched.
#[derive(Debug, Clone, Copy)]
pub enum Surface<'a> {
    Terrain,
    Static(&'a StaticTriangle),
}

/// Where a shape touches a surface, see the module's documentation: a point on the
/// surface, its normal out of it toward the shape, and how deep the shape is in it,
/// negative short of it within the query's margin.
#[derive(Debug, Clone, Copy)]
pub struct Contact<'a> {
    pub point: Vec3,
    pub normal: Vec3,
    pub depth: f32,
    pub surface: Surface<'a>,
}

/// Where a ray first crosses a surface: the point, the surface's normal against the
/// ray, and how far along the ray.
#[derive(Debug, Clone, Copy)]
pub struct Hit<'a> {
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
    pub surface: Surface<'a>,
}

/// A box in the world: its centre, how it is turned, and its half sizes along its own
/// axes.
#[derive(Debug, Clone, Copy)]
pub struct Cuboid {
    pub center: Vec3,
    pub rotation: Quat,
    pub half: Vec3,
}

impl Contact<'_> {

    /// Whether it is the same way out of the same surface as another: of the terrain or
    /// of the same placed model, their normals alike within [`SAME_WAY`].
    fn same_way(&self, other: &Contact) -> bool {
        let same_surface = match (self.surface, other.surface) {
            (Surface::Terrain, Surface::Terrain) => true,
            (Surface::Static(a), Surface::Static(b)) => a.instance == b.instance,
            _ => false,
        };
        same_surface && self.normal.dot(other.normal) > SAME_WAY
    }

}

/// A shape's contacts, on the stack for most.
pub type Contacts<'a> = SmallVec<[Contact<'a>; 8]>;

impl Space {

    /// The surfaces whose bounds overlap a box, minimum then maximum: the terrain's
    /// triangles, then the static models' that `keep` keeps, each with its normal,
    /// whichever way its winding has it, up for the terrain's.
    pub(crate) fn surfaces_in<'a>(
        &'a self,
        min: Vec3,
        max: Vec3,
        keep: &impl Fn(&StaticTriangle) -> bool,
        mut each: impl FnMut([Vec3; 3], Vec3, Surface<'a>),
    ) {
        self.terrain.triangles_in(min.xz(), max.xz(), |triangle| {
            let [a, b, c] = triangle;
            if a.y.min(b.y).min(c.y) <= max.y && a.y.max(b.y).max(c.y) >= min.y {
                each(triangle, (b - a).cross(c - a).normalize_or(Vec3::Y), Surface::Terrain);
            }
        });
        for kind in TOUCHED {
            for triangle in self.statics.triangles_in(kind, min, max) {
                if triangle.normal != Vec3::ZERO && keep(triangle) {
                    each(triangle.vertices, triangle.normal, Surface::Static(triangle));
                }
            }
        }
    }

    /// The first surface a ray from `origin` along `direction`, a unit vector, crosses
    /// within `length`, of those `keep` keeps: the terrain only from above.
    pub fn ray<'a>(&'a self, origin: Vec3, direction: Vec3, length: f32, keep: &impl Fn(&StaticTriangle) -> bool) -> Option<Hit<'a>> {
        let end = origin + direction * length;
        let mut first: Option<Hit<'a>> = None;
        self.surfaces_in(origin.min(end), origin.max(end), keep, |triangle, normal, surface| {
            let facing = direction.dot(normal);
            if matches!(surface, Surface::Terrain) && facing >= 0.0 {
                return;
            }
            let Some(distance) = ray_triangle(origin, direction, triangle) else { return };
            if distance <= length && first.is_none_or(|first| distance < first.distance) {
                first = Some(Hit {
                    point: origin + direction * distance,
                    normal: if facing > 0.0 { -normal } else { normal },
                    distance,
                    surface,
                });
            }
        });
        first
    }

    /// Where a sphere touches the surfaces `keep` keeps, grown by `margin`: one contact
    /// for each way out of them, the deepest, see the module's documentation.
    pub fn sphere<'a>(&'a self, center: Vec3, radius: f32, margin: f32, keep: &impl Fn(&StaticTriangle) -> bool) -> Contacts<'a> {
        let reach = Vec3::splat(radius + margin);
        let mut contacts = Contacts::new();
        self.surfaces_in(center - reach, center + reach, keep, |triangle, normal, surface| {
            let Some((point, normal, depth)) = sphere_triangle(center, radius, triangle, normal, surface) else { return };
            if depth < -margin {
                return;
            }
            let contact = Contact {
                point,
                normal,
                depth,
                surface,
            };
            match contacts.iter_mut().find(|other| other.same_way(&contact)) {
                Some(other) if other.depth < depth => *other = contact,
                Some(_) => {}
                None => contacts.push(contact),
            }
        });
        contacts
    }

    /// Where a box touches the surfaces `keep` keeps, grown by `margin`: for each way
    /// out of them, `MANIFOLD` points at most, see the module's documentation.
    pub fn cuboid<'a>(&'a self, cuboid: &Cuboid, margin: f32, keep: &impl Fn(&StaticTriangle) -> bool) -> Contacts<'a> {

        // Every point of every surface it is in.
        let reach = Vec3::splat(cuboid.half.length() + margin);
        let mut points = Contacts::new();
        self.surfaces_in(cuboid.center - reach, cuboid.center + reach, keep, |triangle, normal, surface| {
            box_triangle(cuboid, margin, triangle, normal, surface, &mut points);
        });

        // For each way out, the points spanning the most of them.
        let mut contacts = Contacts::new();
        for (index, contact) in points.iter().enumerate() {
            if points[..index].iter().any(|done| done.same_way(contact)) {
                continue;
            }
            let same: SmallVec<[Contact<'a>; 16]> = points.iter()
                .filter(|other| other.same_way(contact))
                .copied()
                .collect();
            contacts.extend(spanning(&same));
        }
        contacts

    }

}

/// How far along a ray, `direction` a unit vector, it crosses a triangle, either side:
/// Möller and Trumbore's test. None if it misses it, or crosses it behind its origin.
fn ray_triangle(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let edge_b = b - a;
    let edge_c = c - a;
    let across = direction.cross(edge_c);
    let determinant = edge_b.dot(across);
    if determinant.abs() < 1e-9 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from = origin - a;
    let u = from.dot(across) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let up = from.cross(edge_b);
    let v = direction.dot(up) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge_c.dot(up) * inverse;
    (distance >= 0.0).then_some(distance)
}

/// Where a sphere touches a triangle, its `normal` its winding's: the point on it nearest
/// the centre, the way out from there, and how deep the sphere is. Under the terrain,
/// whose triangles are one-sided, it is pushed up whatever its depth.
fn sphere_triangle(center: Vec3, radius: f32, triangle: [Vec3; 3], normal: Vec3, surface: Surface) -> Option<(Vec3, Vec3, f32)> {
    let above = (center - triangle[0]).dot(normal);
    let nearest = nearest_on_triangle(center, triangle);
    if matches!(surface, Surface::Terrain) && above < 0.0 {
        // Under it: out along its normal, from where the centre is under it if it is.
        let inside = (nearest - center).dot(normal).abs() <= -above + 1e-4;
        let point = if inside { center - normal * above } else { nearest };
        return Some((point, normal, radius - above));
    }
    let away = center - nearest;
    let distance = away.length();
    let way = if distance > 1e-6 {
        away / distance
    } else if above >= 0.0 {
        normal
    } else {
        -normal
    };
    Some((nearest, way, radius - distance))
}

/// The point of a triangle nearest a point, by which of its regions the point is in:
/// a corner's, an edge's, or its face. Ericson's `ClosestPtPointTriangle`.
fn nearest_on_triangle(point: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {

    let ab = b - a;
    let ac = c - a;
    let ap = point - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }

    let bp = point - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }

    let cp = point - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }

    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)

}

/// Where a box, grown by `margin`, is in a triangle, its `normal` its winding's: the
/// way out of it the least deep, then each of its corners within the box, as deep as
/// the box, not grown, must move that way to leave it. See the module's documentation.
///
/// ```text
///        triangle          axis a
///       ╱╲                 ├──┤ triangle's shadow   [low … high]
///      ╱  ╲                       ├──┤ box's shadow  [−r … r]
///     ╲    ╲
///      ╲____╲              back    = r − low      (out to −a)
///                          forward = high + r     (out to +a)
/// ```
fn box_triangle<'a>(cuboid: &Cuboid, margin: f32, triangle: [Vec3; 3], normal: Vec3, surface: Surface<'a>, out: &mut Contacts<'a>) {

    // 1. The triangle in the box's frame.
    let inverse = cuboid.rotation.inverse();
    let local = triangle.map(|vertex| inverse * (vertex - cuboid.center));
    let own = inverse * normal;
    let grown = cuboid.half + margin;
    let one_sided = matches!(surface, Surface::Terrain);

    // 2. The way out, the least deep: none if an axis separates them. Its own normal's,
    //    unless another is shallower by FACE_BIAS: its edges may be inside a face of
    //    many triangles, not edges at all. Out of the terrain, one surface, only up.
    let edges = [local[1] - local[0], local[2] - local[1], local[0] - local[2]];
    let crossings = edges.into_iter()
        .flat_map(|edge| [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| edge.cross(axis)))
        .filter_map(Vec3::try_normalize);
    let mut least: Option<(Vec3, f32)> = None;
    let mut face: Option<(Vec3, f32)> = None;
    for (index, axis) in [own, Vec3::X, Vec3::Y, Vec3::Z].into_iter().chain(crossings).enumerate() {
        let reach = grown.dot(axis.abs());
        let (low, high) = shadow(&local, axis);
        let back = reach - low;
        let forward = high + reach;
        if back <= 0.0 || forward <= 0.0 {
            return;
        }
        if one_sided && index > 0 {
            continue;
        }
        for (way, depth) in [(-axis, back), (axis, forward)] {
            if one_sided && way.dot(own) < 0.0 {
                continue;
            }
            if index == 0 && face.is_none_or(|(_, face)| depth < face) {
                face = Some((way, depth));
            }
            if least.is_none_or(|(_, least)| depth < least) {
                least = Some((way, depth));
            }
        }
    }
    let (way, _) = match (face, least) {
        (Some(face), Some(least)) if face.1 <= least.1 + FACE_BIAS => face,
        (_, Some(least)) => least,
        _ => return,
    };

    // 3. Its corners within the box, each as deep as the box must move to leave it.
    let mut inside: Polygon = local.into_iter().collect();
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let at = grown.dot(axis);
        inside = clip(&inside, axis, -at);
        inside = clip(&inside, -axis, -at);
    }
    let reach = cuboid.half.dot(way.abs());
    for point in inside {
        out.push(Contact {
            point: cuboid.center + cuboid.rotation * point,
            normal: cuboid.rotation * way,
            depth: point.dot(way) + reach,
            surface,
        });
    }

}

/// A convex polygon, at most a triangle cut by a box's 6 planes, so it never leaves the
/// stack.
type Polygon = SmallVec<[Vec3; 9]>;

/// What of a convex polygon lies on the side of a plane its normal points to, the plane
/// `normal · p = at`: a convex polygon, with one more vertex at most.
fn clip(polygon: &[Vec3], normal: Vec3, at: f32) -> Polygon {
    let mut clipped = Polygon::new();
    for (index, &from) in polygon.iter().enumerate() {
        let to = polygon[(index + 1) % polygon.len()];
        let from_side = from.dot(normal) - at;
        let to_side = to.dot(normal) - at;
        if from_side >= 0.0 {
            clipped.push(from);
        }
        if (from_side >= 0.0) != (to_side >= 0.0) {
            clipped.push(from.lerp(to, from_side / (from_side - to_side)));
        }
    }
    clipped
}

/// Points' shadow on an axis: their least and their most along it.
fn shadow(points: &[Vec3], axis: Vec3) -> (f32, f32) {
    points.iter()
        .map(|point| point.dot(axis))
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), along| (low.min(along), high.max(along)))
}

/// [`MANIFOLD`] of these contacts at most, those spanning the most of them: the
/// deepest, the one furthest from it, the one furthest from the line between, then the
/// one furthest from their middle.
fn spanning<'a>(contacts: &[Contact<'a>]) -> SmallVec<[Contact<'a>; MANIFOLD]> {

    let mut kept: SmallVec<[Contact<'a>; MANIFOLD]> = SmallVec::new();
    if contacts.len() <= MANIFOLD {
        kept.extend(contacts.iter().copied());
        return kept;
    }

    let furthest = |score: &dyn Fn(&Contact) -> f32| {
        contacts.iter().copied().max_by(|a, b| score(a).total_cmp(&score(b)))
    };
    let deepest = furthest(&|contact| contact.depth).unwrap();
    let far = furthest(&|contact| contact.point.distance_squared(deepest.point)).unwrap();
    let line = far.point - deepest.point;
    let off = furthest(&|contact| (contact.point - deepest.point).cross(line).length_squared()).unwrap();
    let middle = (deepest.point + far.point + off.point) / 3.0;
    let last = furthest(&|contact| contact.point.distance_squared(middle)).unwrap();

    for contact in [deepest, far, off, last] {
        if !kept.iter().any(|kept| kept.point.distance_squared(contact.point) < 1e-8) {
            kept.push(contact);
        }
    }
    kept

}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}


#[cfg(test)]
mod tests {

    use super::statics::Statics;
    use super::*;

    /// A space of these static triangles, solid, no terrain.
    fn space_of(triangles: &[[Vec3; 3]]) -> Space {
        let triangles = triangles.iter()
            .map(|&vertices| StaticTriangle::new(vertices, 0, 0, BodyKind::Solid))
            .collect();
        Space {
            statics: Statics::from_triangles(triangles),
            ..Space::default()
        }
    }

    /// A floor at 0, from -10 to 10, as two triangles wound down.
    fn floor() -> Space {
        let a = Vec3::new(-10.0, 0.0, -10.0);
        let b = Vec3::new(10.0, 0.0, -10.0);
        let c = Vec3::new(10.0, 0.0, 10.0);
        let d = Vec3::new(-10.0, 0.0, 10.0);
        space_of(&[[a, b, c], [a, c, d]])
    }

    fn all(_: &StaticTriangle) -> bool {
        true
    }

    #[test]
    fn rays_hit_the_first_surface() {
        let space = floor();
        // Down onto it, its normal up against the ray, whatever its winding.
        let hit = space.ray(Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Y, 5.0, &all).unwrap();
        assert!((hit.distance - 2.0).abs() < 1e-5 && hit.normal.distance(Vec3::Y) < 1e-5, "{hit:?}");
        // Too short, or what it keeps not that.
        assert!(space.ray(Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Y, 1.5, &all).is_none());
        assert!(space.ray(Vec3::new(1.0, 2.0, 3.0), Vec3::NEG_Y, 5.0, &|_| false).is_none());
    }

    #[test]
    fn spheres_touch_once_a_way() {
        let space = floor();
        // Sunk 0.1 m over the floor's diagonal, both its triangles touch it: one contact.
        let contacts = space.sphere(Vec3::new(0.0, 0.4, 0.0), 0.5, 0.0, &all);
        assert_eq!(contacts.len(), 1, "{contacts:?}");
        let contact = contacts[0];
        assert!((contact.depth - 0.1).abs() < 1e-5 && contact.normal.distance(Vec3::Y) < 1e-5, "{contact:?}");
        // Within the margin above it, it touches at a negative depth; beyond, not.
        assert!((space.sphere(Vec3::new(0.0, 0.52, 0.0), 0.5, 0.05, &all)[0].depth + 0.02).abs() < 1e-5);
        assert!(space.sphere(Vec3::new(0.0, 0.6, 0.0), 0.5, 0.05, &all).is_empty());
        // Against a wall's edge, it is pushed away from the edge.
        let wall = space_of(&[[Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 1.0, 0.0)]]);
        let contacts = wall.sphere(Vec3::new(0.3, 1.3, 0.0), 0.5, 0.0, &all);
        assert!(contacts.len() == 1 && contacts[0].normal.x > 0.5 && contacts[0].normal.y > 0.5, "{contacts:?}");
    }

    #[test]
    fn the_terrain_pushes_up_only() {
        let space = Space {
            terrain: Terrain::flat(2.0),
            ..Space::default()
        };
        // A sphere and a box sunk past their middles come out up, as deep as they are.
        let contacts = space.sphere(Vec3::new(50.0, 1.7, 50.0), 0.5, 0.0, &all);
        assert!(contacts.len() == 1 && contacts[0].normal.distance(Vec3::Y) < 1e-5, "{contacts:?}");
        assert!((contacts[0].depth - 0.8).abs() < 1e-4 && (contacts[0].point.y - 2.0).abs() < 1e-4, "{contacts:?}");
        let cuboid = Cuboid {
            center: Vec3::new(50.0, 1.8, 50.0),
            rotation: Quat::from_rotation_y(0.4),
            half: Vec3::new(1.0, 0.5, 2.0),
        };
        let contacts = space.cuboid(&cuboid, 0.0, &all);
        assert!(!contacts.is_empty(), "{contacts:?}");
        for contact in &contacts {
            assert!(contact.normal.distance(Vec3::Y) < 1e-4 && (contact.depth - 0.7).abs() < 1e-3, "{contact:?}");
        }
        // A ray from under it goes through.
        assert!(space.ray(Vec3::new(50.0, 1.0, 50.0), Vec3::Y, 5.0, &all).is_none());
        let hit = space.ray(Vec3::new(50.0, 3.0, 50.0), Vec3::NEG_Y, 5.0, &all).unwrap();
        assert!((hit.distance - 1.0).abs() < 1e-4 && matches!(hit.surface, Surface::Terrain), "{hit:?}");
    }

    #[test]
    fn boxes_rest_on_faces_and_meet_walls() {
        let space = floor();
        // Sunk 0.1 m into it, level: out up, at its bottom's corners, 0.1 m deep.
        let cuboid = Cuboid {
            center: Vec3::new(0.3, 0.4, 0.2),
            rotation: Quat::IDENTITY,
            half: Vec3::new(1.0, 0.5, 2.0),
        };
        let contacts = space.cuboid(&cuboid, 0.0, &all);
        assert_eq!(contacts.len(), 4, "{contacts:?}");
        for contact in &contacts {
            assert!(contact.normal.distance(Vec3::Y) < 1e-4 && (contact.depth - 0.1).abs() < 1e-4, "{contact:?}");
        }
        // Tilted onto a corner, it touches at the corner, deepest.
        let cuboid = Cuboid {
            center: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::from_rotation_z(0.3) * Quat::from_rotation_x(0.2),
            half: Vec3::new(1.0, 0.5, 2.0),
        };
        let contacts = space.cuboid(&cuboid, 0.0, &all);
        let deepest = contacts.iter().map(|contact| contact.depth).fold(0.0, f32::max);
        assert!(deepest > 0.0 && contacts.iter().all(|contact| contact.normal.y > 0.99), "{contacts:?}");
        // Its front 0.2 m into a wall, out back from it, the wall's 4 corners within it.
        let wall = space_of(&[
            [Vec3::new(-5.0, 0.0, 2.0), Vec3::new(5.0, 0.0, 2.0), Vec3::new(5.0, 5.0, 2.0)],
            [Vec3::new(-5.0, 0.0, 2.0), Vec3::new(5.0, 5.0, 2.0), Vec3::new(-5.0, 5.0, 2.0)],
        ]);
        let cuboid = Cuboid {
            center: Vec3::new(0.0, 1.0, 0.2),
            rotation: Quat::IDENTITY,
            half: Vec3::new(1.0, 0.5, 2.0),
        };
        let contacts = wall.cuboid(&cuboid, 0.0, &all);
        assert_eq!(contacts.len(), 4, "{contacts:?}");
        for contact in &contacts {
            assert!(contact.normal.distance(Vec3::NEG_Z) < 1e-4 && (contact.depth - 0.2).abs() < 1e-4, "{contact:?}");
        }
    }

}
