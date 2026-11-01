//! Vehicle collision models, the client's: each module's
//! `hitTester/collisionModelClient`, a Havok tagfile. Its "Collision Physics Data" has
//! one mesh body per armour group, `s_armor_3` say, and its `HKBodyFlagsData` gives each
//! group, by name without the `s_`, its material kind: the armour's index for armour.

use std::fmt;
use std::io;

use glam::{Affine3A, Vec3};

use wgtk::havok::mesh::compressed_mesh_triangles;
use wgtk::havok::tagfile::Tagfile;
use wgtk::res::fs::ResFilesystem;


/// The parts of a vehicle, each with its own model, in the order of
/// `TankPartIndexes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub triangles: Vec<[Vec3; 3]>,
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

/// Where a segment first hits a vehicle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleHit {
    pub fraction: f32,
    pub part: VehiclePart,
    pub material_kind: u32,
    /// Where it hits, in the part's frame.
    pub local_point: Vec3,
    /// The way the segment goes, in the part's frame.
    pub local_direction: Vec3,
    /// The surface's normal there, in the world.
    pub normal: Vec3,
}

impl CollisionModel {

    /// Load a model from the game's resources.
    pub fn load(fs: &ResFilesystem, path: &str) -> io::Result<Self> {
        let file = fs.read(path)
            .map(io::BufReader::new)
            .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
        let tagfile = Tagfile::from_reader(file)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: {e}")))?;
        Self::from_tagfile(&tagfile)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: no valid collision data")))
    }

    fn from_tagfile(tagfile: &Tagfile) -> Option<Self> {

        let handles: Vec<_> = tagfile.root()?
            .get("namedVariants")?.elements()?.get(0)?
            .get("variant")?.deref()?
            .get("resourceHandles")?.elements()?.into_iter()
            .filter_map(|handle| handle.deref())
            .collect();
        let variant = |name: &str| handles.iter()
            .find(|handle| handle.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|handle| handle.get("variant")?.deref());

        let body_flags = variant("HKBodyFlagsData")?;
        let bound = |name: &str| Some(Vec3::from_slice(&body_flags.get(name)?.as_f32s::<4>()?[..3]));
        let bounds = [bound("minBounds_")?, bound("maxBounds_")?];
        let material_kinds: Vec<(&str, u32)> = body_flags
            .get("flags_")?.elements()?.into_iter()
            .filter_map(|info| Some((info.get("name")?.as_str()?, info.get("normalMatKind")?.as_int()? as u32)))
            .collect();

        let mut groups = Vec::new();
        for body in variant("Collision Physics Data")?.get("bodyCinfos")?.elements()? {
            let name = body.get("name").and_then(|name| name.as_str()).unwrap_or_default();
            let name = name.strip_prefix("s_").unwrap_or(name);
            let shape = body.get("shape")?.deref()?;
            let triangles = match shape.type_name() {
                "hknpCompressedMeshShape" => compressed_mesh_triangles(shape)?,
                _ => continue,
            };
            let material_kind = material_kinds.iter().find(|(n, _)| *n == name).map_or(0, |&(_, kind)| kind);
            groups.push(ArmorGroup { name: name.to_string(), material_kind, triangles });
        }

        let radius = groups.iter()
            .flat_map(|group| group.triangles.iter().flatten())
            .map(|vertex| vertex.length())
            .fold(0.0, f32::max);
        Some(Self { groups, radius, bounds })

    }

    /// Where a segment, in the model's frame, first hits it.
    pub fn first_hit(&self, from: Vec3, to: Vec3) -> Option<ModelHit> {
        let delta = to - from;
        // Whether the segment passes the bounding sphere at all.
        let closest = from + delta * (-from.dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
        if closest.length() > self.radius {
            return None;
        }
        let mut first: Option<ModelHit> = None;
        for (index, group) in self.groups.iter().enumerate() {
            for triangle in &group.triangles {
                let Some((fraction, normal)) = segment_triangle(from, delta, triangle) else { continue };
                if first.is_none_or(|first| fraction < first.fraction) {
                    first = Some(ModelHit { fraction, group: index, normal });
                }
            }
        }
        first
    }

}

impl VehicleCollision {

    /// Where a segment first hits the vehicle, its parts placed in the world by
    /// `transforms`, in [`VehiclePart`] order.
    pub fn first_hit(&self, transforms: &[Affine3A; 4], from: Vec3, to: Vec3) -> Option<VehicleHit> {
        const PARTS: [VehiclePart; 4] = [VehiclePart::Chassis, VehiclePart::Hull, VehiclePart::Turret, VehiclePart::Gun];
        PARTS.into_iter()
            .zip(&self.parts)
            .zip(transforms)
            .filter_map(|((part, model), transform)| {
                let inverse = transform.inverse();
                let (local_from, local_to) = (inverse.transform_point3(from), inverse.transform_point3(to));
                let hit = model.first_hit(local_from, local_to)?;
                Some(VehicleHit {
                    fraction: hit.fraction,
                    part,
                    material_kind: model.groups[hit.group].material_kind,
                    local_point: local_from.lerp(local_to, hit.fraction),
                    local_direction: (local_to - local_from).normalize(),
                    normal: transform.transform_vector3(hit.normal).normalize(),
                })
            })
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
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
            part: VehiclePart::Hull,
            material_kind: 1,
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
        let collision = &def.collision;
        for model in &collision.parts {
            assert!(model.groups.iter().any(|group| !group.triangles.is_empty()), "{model:?}");
        }
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
