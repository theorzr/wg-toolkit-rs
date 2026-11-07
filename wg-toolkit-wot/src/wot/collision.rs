//! Vehicle collision models, the client's: each module's
//! `hitTester/collisionModelClient`, a [`Collision`] with one body per armour group,
//! `armor_3` say, its material kind the armour's index.

use std::fmt;
use std::io;

use glam::{Affine3A, Vec3};

use wgtk::havok::collision::Collision;
use wgtk::res::fs::ResFilesystem;


/// How close two crossings of the same group are the same one, in metres: through an
/// edge or a vertex its triangles share.
const SAME_HIT: f32 = 0.001;

/// The parts of a vehicle, each with its own model, in the order of
/// `TankPartIndexes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VehiclePart {
    Chassis = 0,
    Hull = 1,
    Turret = 2,
    Gun = 3,
}

/// The models of a vehicle's parts, in [`VehiclePart`] order.
#[derive(Debug, Clone)]
pub struct VehicleCollision {
    pub parts: [CollisionModel; 4],
    /// Where the hull sits from the vehicle's origin, the chassis' `hullPosition`.
    pub hull_position: Vec3,
}

/// A part's collision model, in the part's own frame.
#[derive(Clone)]
pub struct CollisionModel {
    pub groups: Vec<ArmorGroup>,
    /// The radius of a sphere around the part's origin holding all its triangles.
    pub radius: f32,
    /// The part's bounding box, minimum then maximum, as the client knows it: hit
    /// points are encoded over it.
    pub bounds: [Vec3; 2],
}

/// A part's triangles of the same material.
#[derive(Debug, Clone)]
pub struct ArmorGroup {
    pub name: String,
    /// Its material kind: the armour index, 1 for `armor_1`, or a device's kind.
    pub material_kind: u32,
    /// What it is made of, as the vehicle's definitions give it, see
    /// [`super::item_defs::VehicleDef::load`]: none for a group they don't armour.
    pub material: Option<Material>,
    pub triangles: Vec<[Vec3; 3]>,
}

/// What an armour group is made of: a material of the game's common `materials`, as
/// the vehicle's part sets it, see `_readArmor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Its thickness, in millimetres.
    pub armor: f32,
    /// The share of a piercing shell's damage the vehicle takes behind it: 0 for
    /// screens, tracks and the gun, which only stop shells.
    pub vehicle_damage_factor: f32,
    /// Whether a shell meets it at its angle, else as if square on.
    pub use_hit_angle: bool,
    pub may_ricochet: bool,
    /// Whether a caliber of 3 times its thickness or more never ricochets off it.
    pub check_caliber_for_ricochet: bool,
    /// Whether a caliber of more than twice its thickness turns more toward its normal.
    pub check_caliber_for_hit_angle_norm: bool,
    /// Whether a shell only meets it once, whichever of its faces it crosses.
    pub collide_once_only: bool,
}

impl Material {

    /// Whether the vehicle takes damage behind it: its main armour, not a screen.
    pub fn is_main_armor(&self) -> bool {
        self.vehicle_damage_factor > 0.0
    }

}

/// Where a segment first hits a model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelHit {
    /// The fraction of the segment it goes before hitting.
    pub fraction: f32,
    /// The index of the group hit.
    pub group: usize,
    /// The triangle's normal, facing the segment's start.
    pub normal: Vec3,
}

/// Where a segment hits a vehicle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleHit {
    pub fraction: f32,
    /// How far along the segment, in metres.
    pub distance: f32,
    pub part: VehiclePart,
    /// The index of the group hit in its part's model.
    pub group: usize,
    pub material_kind: u32,
    pub material: Option<Material>,
    /// Where it hits, in the part's frame.
    pub local_point: Vec3,
    /// The way the segment goes, in the part's frame.
    pub local_direction: Vec3,
    /// The surface's normal there, in the world.
    pub normal: Vec3,
}

impl CollisionModel {

    /// Load a model from the game's resources, its groups its collision's bodies.
    pub fn load(fs: &ResFilesystem, path: &str) -> io::Result<Self> {

        let collision = Collision::load(fs, path)?;
        let groups: Vec<_> = collision.bodies.into_iter()
            .map(|body| ArmorGroup { name: body.name, material_kind: body.material_kind, material: None, triangles: body.triangles })
            .collect();

        let radius = groups.iter()
            .flat_map(|group| group.triangles.iter().flatten())
            .map(|vertex| vertex.length())
            .fold(0.0, f32::max);

        Ok(Self { groups, radius, bounds: collision.bounds })

    }

    /// Everywhere a segment, in the model's frame, crosses one of its triangles, in no
    /// order.
    pub fn hits(&self, from: Vec3, to: Vec3) -> impl Iterator<Item = ModelHit> + '_ {
        let delta = to - from;
        // Whether the segment passes the bounding sphere at all.
        let closest = from + delta * (-from.dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        let near = closest.length() <= self.radius;
        self.groups.iter().enumerate()
            .filter(move |_| near)
            .flat_map(move |(index, group)| group.triangles.iter()
                .filter_map(move |triangle| segment_triangle(from, delta, triangle))
                .map(move |(fraction, normal)| ModelHit { fraction, group: index, normal }))
    }

    /// The point of its groups that `filter` keeps closest to `point`, in the model's
    /// frame, and its distance.
    pub fn closest_point(&self, point: Vec3, filter: impl Fn(&ArmorGroup) -> bool) -> Option<(Vec3, f32)> {
        self.groups.iter()
            .filter(|group| filter(group))
            .flat_map(|group| group.triangles.iter())
            .map(|triangle| {
                let closest = closest_on_triangle(point, triangle);
                (closest, closest.distance(point))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

}

impl VehicleCollision {

    /// Where a segment first hits the vehicle, its parts placed in the world by
    /// `transforms`, in [`VehiclePart`] order.
    pub fn first_hit(&self, transforms: &[Affine3A; 4], from: Vec3, to: Vec3) -> Option<VehicleHit> {
        self.unordered_hits(transforms, from, to).min_by(|a, b| a.fraction.total_cmp(&b.fraction))
    }

    /// Everywhere a segment crosses the vehicle, nearest first, as [`Self::first_hit`]
    /// finds the first: what a shell meets on its way through. A crossing through the
    /// edge two triangles of a group share is one, see [`SAME_HIT`].
    pub fn hits(&self, transforms: &[Affine3A; 4], from: Vec3, to: Vec3) -> Vec<VehicleHit> {
        let mut hits: Vec<_> = self.unordered_hits(transforms, from, to).collect();
        hits.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
        let mut unique: Vec<VehicleHit> = Vec::with_capacity(hits.len());
        for hit in hits {
            let same = unique.iter().rev()
                .take_while(|seen| hit.distance - seen.distance < SAME_HIT)
                .any(|seen| (seen.part, seen.group) == (hit.part, hit.group));
            if !same {
                unique.push(hit);
            }
        }
        unique
    }

    /// The point of the vehicle's groups that `filter` keeps closest to `point`, both in
    /// the world, and its distance.
    pub fn closest_point(&self, transforms: &[Affine3A; 4], point: Vec3, filter: impl Fn(&ArmorGroup) -> bool) -> Option<(Vec3, f32)> {
        self.parts.iter()
            .zip(transforms)
            .filter_map(|(model, transform)| {
                let (local, _) = model.closest_point(transform.inverse().transform_point3(point), &filter)?;
                let world = transform.transform_point3(local);
                Some((world, world.distance(point)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    fn unordered_hits<'a>(&'a self, transforms: &'a [Affine3A; 4], from: Vec3, to: Vec3) -> impl Iterator<Item = VehicleHit> + 'a {
        const PARTS: [VehiclePart; 4] = [VehiclePart::Chassis, VehiclePart::Hull, VehiclePart::Turret, VehiclePart::Gun];
        let length = from.distance(to);
        PARTS.into_iter()
            .zip(&self.parts)
            .zip(transforms)
            .flat_map(move |((part, model), transform)| {
                let inverse = transform.inverse();
                let (local_from, local_to) = (inverse.transform_point3(from), inverse.transform_point3(to));
                model.hits(local_from, local_to).map(move |hit| {
                    let group = &model.groups[hit.group];
                    VehicleHit {
                        fraction: hit.fraction,
                        distance: hit.fraction * length,
                        part,
                        group: hit.group,
                        material_kind: group.material_kind,
                        material: group.material,
                        local_point: local_from.lerp(local_to, hit.fraction),
                        local_direction: (local_to - local_from).normalize(),
                        normal: transform.transform_vector3(hit.normal).normalize(),
                    }
                })
            })
    }

}

impl VehicleHit {

    /// The hit as a `VEHICLE_HIT_POINT`'s `segment`, see `encodeSegment` in the client's
    /// `helpers_common`: the line of the shell through the hit point, clipped to the
    /// part's bounding box, each end's coordinates as 8-bit fractions of the box, then
    /// the part's index and `context`, a `VEHICLE_HIT_EFFECT` for a hit point.
    pub fn encode_segment(&self, [min, max]: [Vec3; 2], context: u8) -> u64 {
        let reach = (max - min).length();
        let (from, to) = (self.local_point - self.local_direction * reach, self.local_point + self.local_direction * reach);
        let header = (self.part as u64) << 8 | u64::from(context);
        let Some((enter, exit)) = clip_to_box(from, to, min, max) else { return header };
        let quantize = |point: Vec3| ((point - min) * 255.0 / (max - min)).round().clamp(Vec3::ZERO, Vec3::splat(255.0));
        let (a, b) = (quantize(from.lerp(to, enter)), quantize(from.lerp(to, exit)));
        (b.z as u64) << 56 | (b.y as u64) << 48 | (b.x as u64) << 40
            | (a.z as u64) << 32 | (a.y as u64) << 24 | (a.x as u64) << 16
            | header
    }

}

/// The fractions of a segment where it enters and leaves a box, if it crosses it.
fn clip_to_box(from: Vec3, to: Vec3, min: Vec3, max: Vec3) -> Option<(f32, f32)> {
    let delta = to - from;
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        if delta[axis].abs() < 1e-9 {
            if from[axis] < min[axis] || from[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = ((min[axis] - from[axis]) / delta[axis], (max[axis] - from[axis]) / delta[axis]);
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    Some((enter, exit))
}

impl fmt::Debug for CollisionModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollisionModel")
            .field("groups", &self.groups.iter().map(|g| (&g.name, g.triangles.len())).collect::<Vec<_>>())
            .field("radius", &self.radius)
            .field("bounds", &self.bounds)
            .finish()
    }
}

/// The point of a triangle closest to `point` (Ericson, Real-Time Collision Detection).
fn closest_on_triangle(point: Vec3, &[a, b, c]: &[Vec3; 3]) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, point - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = point - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
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
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Where a segment from `from` along `delta` crosses a triangle, either face, as a
/// fraction of it, with the triangle's normal facing `from` (Möller–Trumbore).
fn segment_triangle(from: Vec3, delta: Vec3, [a, b, c]: &[Vec3; 3]) -> Option<(f32, Vec3)> {
    let (ab, ac) = (*b - *a, *c - *a);
    let p = delta.cross(ac);
    let det = ab.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - *a;
    let u = s.dot(p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(ab);
    let v = delta.dot(q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = ac.dot(q) / det;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let normal = ab.cross(ac).normalize();
    Some((t, if normal.dot(delta) > 0.0 { -normal } else { normal }))
}


#[cfg(test)]
mod tests {

    use super::*;
    use crate::wot::item_defs::VehicleDef;

    #[test]
    fn encodes_a_hit_as_the_client_decodes_it() {
        let hit = VehicleHit {
            fraction: 0.5,
            distance: 1.0,
            part: VehiclePart::Hull,
            group: 0,
            material_kind: 1,
            material: None,
            local_point: Vec3::new(0.0, 0.0, 1.0),
            local_direction: -Vec3::Z,
            normal: Vec3::Z,
        };
        let bounds = [Vec3::splat(-1.0), Vec3::splat(1.0)];
        let segment = hit.encode_segment(bounds, 4);
        // As `decodeSegment` does.
        let decode = |shift: u32| Vec3::new(
            (segment >> shift & 255) as f32, (segment >> (shift + 8) & 255) as f32, (segment >> (shift + 16) & 255) as f32,
        ) * 2.0 / 255.0 - Vec3::ONE;
        assert_eq!((segment >> 8 & 255, segment & 255), (1, 4));
        assert!((decode(16) - Vec3::new(0.0, 0.0, 1.0)).length() < 0.01, "{}", decode(16));
        assert!((decode(40) - Vec3::new(0.0, 0.0, -1.0)).length() < 0.01, "{}", decode(40));
    }

    #[test]
    fn segment_crosses_a_triangle() {
        let triangle = [Vec3::new(-1.0, -1.0, 5.0), Vec3::new(1.0, -1.0, 5.0), Vec3::new(0.0, 1.0, 5.0)];
        let (t, normal) = segment_triangle(Vec3::ZERO, Vec3::Z * 10.0, &triangle).unwrap();
        assert!((t - 0.5).abs() < 1e-6);
        assert_eq!(normal, -Vec3::Z);
        assert!(segment_triangle(Vec3::ZERO, Vec3::Z * 4.0, &triangle).is_none());
        assert!(segment_triangle(Vec3::X * 2.0, Vec3::Z * 10.0, &triangle).is_none());
    }

    #[test]
    fn finds_the_closest_point_of_a_triangle() {
        let triangle = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0)];
        // Over its face, past an edge, past a vertex.
        assert_eq!(closest_on_triangle(Vec3::new(0.5, 0.5, 3.0), &triangle), Vec3::new(0.5, 0.5, 0.0));
        assert_eq!(closest_on_triangle(Vec3::new(1.0, -1.0, 0.0), &triangle), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(closest_on_triangle(Vec3::new(-1.0, -1.0, 1.0), &triangle), Vec3::ZERO);
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: the Chieftain's models, hit
    /// from the front and from above.
    #[test]
    fn hits_the_chieftain() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let def = VehicleDef::load(&fs, "uk", "GB98_T95_FV4201_Chieftain").unwrap();
        // Turret and gun straight ahead.
        let aiming = &def.aiming;
        let transforms = [
            Vec3::ZERO, def.collision.hull_position, aiming.turret_position, aiming.turret_position + aiming.gun_position,
        ].map(Affine3A::from_translation);
        assert!(def.shots.iter().all(|shot| shot.pierced_sticker.is_some()), "{:?}", def.shots);
        // Its APDS shells get the APCR prefab effects (3), its HESH the HE ones (1).
        assert_eq!(def.shots.iter().map(|shot| shot.prefab_effects_index).collect::<Vec<_>>(), [Some(3), Some(3), Some(1)]);
        let collision = &def.collision;
        for model in &collision.parts {
            assert!(model.groups.iter().any(|group| !group.triangles.is_empty()), "{model:?}");
            // Every armour group has its thickness: only modules, the surveying device
            // say, may have none, which a shell only damages.
            let bare: Vec<_> = model.groups.iter()
                .filter(|group| group.material.is_none() && group.name.starts_with("armor_"))
                .map(|group| &group.name)
                .collect();
            assert!(bare.is_empty(), "{bare:?}");
        }
        // Its upper front plate is its thickest, the gun mantlet aside.
        let hull = &collision.parts[VehiclePart::Hull as usize];
        let thickest = hull.groups.iter().filter_map(|group| group.material).map(|material| material.armor).fold(0.0, f32::max);
        assert!(thickest >= 100.0, "{thickest}");
        // The upper front plate, 1.2 m up, from 50 m ahead.
        let front = collision.first_hit(&transforms, Vec3::new(0.0, 1.2, 50.0), Vec3::new(0.0, 1.2, 0.0)).unwrap();
        assert_eq!(front.part, VehiclePart::Hull, "{front:?}");
        assert!(front.normal.z > 0.0 && front.material_kind != 0, "{front:?}");
        // The turret roof, from above.
        let top = collision.first_hit(&transforms, Vec3::new(0.5, 10.0, -0.5), Vec3::new(0.5, 0.0, -0.5)).unwrap();
        assert_eq!(top.part, VehiclePart::Turret, "{top:?}");
        assert!(top.normal.y > 0.5, "{top:?}");
    }

}
