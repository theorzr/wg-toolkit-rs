//! Triangles of the Havok shapes collision models use.

use glam::Vec3;

use super::tagfile::Node;


/// The triangles of a `hknpCompressedMeshShape`, `None` if malformed.
///
/// Its mesh tree is split in sections, each with its vertices packed in a `u32` as
/// 11, 11 and 10 bits scaled by the section's `codecParms` (3 offsets, 3 scales), or
/// shared with other sections, packed in a `u64` as 21, 21 and 22 bits over the whole
/// tree's domain. A primitive's 4 indices go to the section's packed vertices first,
/// then to its shared ones; it is a triangle if its last two are the same, else a
/// quad.
pub fn compressed_mesh_triangles(shape: Node) -> Option<Vec<[Vec3; 3]>> {

    let tree = shape.get("data")?.deref()?.get("meshTree")?;
    let domain = tree.get("domain")?;
    let domain_min = Vec3::from_slice(&domain.get("min")?.as_f32s::<4>()?[..3]);
    let domain_max = Vec3::from_slice(&domain.get("max")?.as_f32s::<4>()?[..3]);
    let shared_scale = (domain_max - domain_min) / Vec3::new(0x1FFFFF as f32, 0x1FFFFF as f32, 0x3FFFFF as f32);

    let packed = tree.get("packedVertices")?.elements()?;
    let shared = tree.get("sharedVertices")?.elements()?;
    let shared_index = tree.get("sharedVerticesIndex")?.elements()?;
    let primitives = tree.get("primitives")?.elements()?;
    let int = |node: Option<Node>| node?.as_int();

    let mut triangles = Vec::new();
    for section in tree.get("sections")?.elements()? {

        let parms = section.get("codecParms")?.as_f32s::<6>()?;
        let (offset, scale) = (Vec3::from_slice(&parms[..3]), Vec3::from_slice(&parms[3..]));
        let first_packed = int(section.get("firstPackedVertexIndex"))? as usize;
        let num_packed = int(section.get("numPackedVertices"))? as usize;
        let first_shared = int(section.get("firstSharedVertexIndex"))? as usize;
        let first_primitive = int(section.get("firstPrimitiveIndex"))? as usize;
        let num_primitives = int(section.get("numPrimitives"))? as usize;

        let vertex = |index: usize| -> Option<Vec3> {
            if index < num_packed {
                let v = int(packed.get(first_packed + index))? as u32;
                let q = Vec3::new((v & 0x7FF) as f32, (v >> 11 & 0x7FF) as f32, (v >> 22) as f32);
                Some(offset + q * scale)
            } else {
                let shared_at = int(shared_index.get(first_shared + index - num_packed))? as usize;
                let v = int(shared.get(shared_at))? as u64;
                let q = Vec3::new((v & 0x1FFFFF) as f32, (v >> 21 & 0x1FFFFF) as f32, (v >> 42) as f32);
                Some(domain_min + q * shared_scale)
            }
        };

        for primitive in (first_primitive..first_primitive + num_primitives).map(|i| primitives.get(i)) {
            let indices = primitive?.get("indices")?.elements()?;
            let mut index = [0; 4];
            for (i, element) in index.iter_mut().zip(indices.into_iter()) {
                *i = element.as_int()? as usize;
            }
            let [a, b, c, d] = index.map(vertex);
            let (a, b, c) = (a?, b?, c?);
            triangles.push([a, b, c]);
            if index[2] != index[3] {
                triangles.push([a, c, d?]);
            }
        }

    }

    Some(triangles)

}
