//! Collision models, as the game ships them for its vehicles' parts and its maps' static
//! models: a Havok tagfile whose named variants each hold a "Collision Physics Data"
//! system of mesh bodies, and an `HKBodyFlagsData` giving their bounds and each body's
//! material kind, by its name without its `s_` (solid) or `n_` prefix. A vehicle's part
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
    pub triangles: Vec<[Vec3; 3]>,
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

            let material_kinds: Vec<(&str, u32)> = body_flags
                .get("flags_")?.elements()?.into_iter()
                .filter_map(|info| Some((info.get("name")?.as_str()?, info.get("normalMatKind")?.as_int()? as u32)))
                .collect();

            for body in variant(&handles, "Collision Physics Data")?.get("bodyCinfos")?.elements()? {

                let name = body.get("name").and_then(|name| name.as_str()).unwrap_or_default();
                let name = name.strip_prefix("s_").or_else(|| name.strip_prefix("n_")).unwrap_or(name);
                let shape = body.get("shape")?.deref()?;

                let triangles = match shape.type_name() {
                    "hknpCompressedMeshShape" => compressed_mesh_triangles(shape)?,
                    _ => continue,
                };

                bodies.push(CollisionBody {
                    name: name.to_string(),
                    material_kind: material_kinds.iter().find(|(n, _)| *n == name).map_or(0, |&(_, kind)| kind),
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
