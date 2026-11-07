use std::io::{self, Read, Seek};

use glam::Vec3;

use crate::util::io::WgReadExt;

use super::{invalid, skip_vector, SpaceSection};


/// The vectors of `BSMO` between its models' bounds and their collision, of unknown
/// use.
const UNKNOWN_VECTORS: usize = 12;

/// `BSMO`: the static models [`BSMI`] places, their bounds, their
/// render primitives and their collision. Only its third and sixteenth vectors are
/// known, the others are skipped.
#[derive(Debug)]
pub struct BSMO {
    pub models: Vec<StaticModel>,
}

/// A static model, its strings as [`BWST`] hashes.
#[derive(Debug, Clone, Copy)]
pub struct StaticModel {
    /// Its bounding box in its own frame, minimum then maximum.
    pub bounds: [Vec3; 2],
    /// Its render primitives, a `.primitives` resource.
    pub primitives_fnv: u32,
    /// Its collision, a `.havok` resource, none for decor nothing collides with.
    pub havok_fnv: Option<u32>,
    /// Its collision's material, `n_wood0` or `default` say, when it has one.
    pub material_fnv: u32,
}

impl SpaceSection for BSMO {

    const ID: [u8; 4] = *b"BSMO";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        skip_vector(read)?;
        skip_vector(read)?;
        let bounds = read.read_vector(|buf| {
            let min = buf.read_vec3()?;
            let max = buf.read_vec3()?;
            let primitives_fnv = buf.read_u32()?;
            Ok(([min, max], primitives_fnv))
        })?;
        for _ in 0..UNKNOWN_VECTORS {
            skip_vector(read)?;
        }
        let collisions = read.read_vector(|buf| Ok((buf.read_u32()?, buf.read_u32()?)))?;

        if collisions.len() != bounds.len() {
            return Err(invalid("static models and their collisions differ in count"));
        }

        let models = bounds.into_iter().zip(collisions)
            .map(|((bounds, primitives_fnv), (havok_fnv, material_fnv))| StaticModel {
                bounds,
                primitives_fnv,
                havok_fnv: (havok_fnv != 0).then_some(havok_fnv),
                material_fnv,
            })
            .collect();
        Ok(Self { models })

    }

}
