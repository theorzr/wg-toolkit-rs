//! Collision models, as the game ships them for its vehicles' parts and its maps' static
//! models: a Havok tagfile whose named variants each hold a "Collision Physics Data"
//! system of mesh bodies, and an `HKBodyFlagsData` giving their bounds and each body's
//! material kind and collision flags, by its name without its `s_` (solid) or `n_` prefix. A vehicle's part
//! has one variant, a destructible building one per fragment, `n_wood0_1_5` say.

use std::io;

use glam::Vec3;

use crate::res::fs::ResFilesystem;

use super::mesh::compressed_mesh_triangles;
use super::tagfile::{Node, Tagfile};


/// A collision model's bodies, every variant's, in the model's frame.
#[derive(Debug, Clone)]
pub struct Collision {
    pub bodies: Vec<CollisionBody>,
    /// Its bounding box, minimum then maximum, as its body flags give it.
    pub bounds: [Vec3; 2],
}

/// A body of a collision model: triangles of one material.
#[derive(Debug, Clone)]
pub struct CollisionBody {
    /// Its name without its `s_` or `n_` prefix, `armor_3` or `n_wood0_1_5` say.
    pub name: String,
    /// Its material kind, 0 if its body flags don't give one.
    pub material_kind: u32,
    /// Its collision flags, as its body flags give them, 0 if they don't, see
    /// [`Self::shells_pass`].
    pub flags: u32,
    pub triangles: Vec<[Vec3; 3]>,
}

impl CollisionBody {

    /// The collision flag of a body shells fly through untouched, see
    /// [`Self::shells_pass`].
    pub const SHELLS_PASS: u32 = 1 << 1;

    /// Whether shells fly through it untouched, by its flags. No game source says so;
    /// it is inferred from the game's fences. A light fence's breakable body (`n_wood0`,
    /// 1 health) has this flag, while a stone fence's has 0. The invisible `s_wall` and
    /// `s_ramp` helper bodies that guide vehicles over a fence have it too, with 4 and 8
    /// (14 in all).
    pub fn shells_pass(&self) -> bool {
        self.flags & Self::SHELLS_PASS != 0
    }

}

impl Collision {

    /// Load a collision model from the game's resources, a `.havok` file.
    pub fn load(fs: &ResFilesystem, path: &str) -> io::Result<Self> {
        let file = fs.read(path)
            .map(io::BufReader::new)
            .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
        let tagfile = Tagfile::from_reader(file)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: {e}")))?;
        Self::from_tagfile(&tagfile)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: no valid collision data")))
    }

    /// Read a collision model from its tagfile, `None` if it isn't one. Only its mesh
    /// bodies are read.
    pub fn from_tagfile(tagfile: &Tagfile) -> Option<Self> {

        let mut bodies = Vec::new();
        let mut bounds: Option<[Vec3; 2]> = None;

        for named in tagfile.root()?.get("namedVariants")?.elements()? {

            let handles: Vec<_> = named.get("variant")?.deref()?
                .get("resourceHandles")?.elements()?.into_iter()
                .filter_map(|handle| handle.deref())
                .collect();

            let body_flags = variant(&handles, "HKBodyFlagsData")?;
            let [min, max] = [vector(body_flags, "minBounds_")?, vector(body_flags, "maxBounds_")?];

            bounds = Some(bounds.map_or([min, max], |[least, most]| [least.min(min), most.max(max)]));

            // Each body's name, material kind and collision flags.
            let infos: Vec<(&str, u32, u32)> = body_flags
                .get("flags_")?.elements()?.into_iter()
                .filter_map(|info| {
                    let name = info.get("name")?.as_str()?;
                    let material_kind = info.get("normalMatKind")?.as_int()? as u32;
                    let flags = info.get("collisionFlags").and_then(|flags| flags.as_int()).unwrap_or(0) as u32;
                    Some((name, material_kind, flags))
                })
                .collect();

            for body in variant(&handles, "Collision Physics Data")?.get("bodyCinfos")?.elements()? {

                let name = body.get("name").and_then(|name| name.as_str()).unwrap_or_default();
                let name = name.strip_prefix("s_").or_else(|| name.strip_prefix("n_")).unwrap_or(name);
                let shape = body.get("shape")?.deref()?;

                let triangles = match shape.type_name() {
                    "hknpCompressedMeshShape" => compressed_mesh_triangles(shape)?,
                    _ => continue,
                };

                let info = infos.iter().find(|(n, _, _)| *n == name);
                bodies.push(CollisionBody {
                    name: name.to_string(),
                    material_kind: info.map_or(0, |&(_, kind, _)| kind),
                    flags: info.map_or(0, |&(_, _, flags)| flags),
                    triangles,
                });

            }

        }

        Some(Self { bodies, bounds: bounds? })

    }

}

/// The object of a named resource handle.
fn variant<'a>(handles: &[Node<'a>], name: &str) -> Option<Node<'a>> {
    handles.iter()
        .find(|handle| handle.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|handle| handle.get("variant")?.deref())
}

/// A record's `hkVector4` member, its first 3 coordinates.
fn vector(record: Node, member: &str) -> Option<Vec3> {
    Some(Vec3::from_slice(&record.get(member)?.as_f32s::<4>()?[..3]))
}
