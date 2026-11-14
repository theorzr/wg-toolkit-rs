use std::io::{self, Read, Seek};

use glam::{Affine3A, Mat4};

use crate::util::io::WgReadExt;

use super::SpaceSection;


/// `SpTr`: the speed trees placed in the space, trees and bushes alike, which aren't
/// static models: the client draws them with SpeedTree. [`super::WGDE`] numbers the
/// destructible ones by their index here.
///
/// It holds two vectors of as many elements, one per tree: first each tree's
/// placement, see [`SpeedTree`], 80 bytes in its version 3 (BigWorld 14.4.1's
/// `StaticSceneSpeedTreeTypes::SpeedTree`, version 1, has the same fields but the
/// last); then 52 bytes of each one's destruction parameters, a hash then numbers as
/// `destructibles.xml` gives its kind, which this doesn't read.
#[derive(Debug)]
pub struct SPTR {
    pub trees: Vec<SpeedTree>,
}

/// A speed tree placed in the space, see [`SPTR`].
#[derive(Debug, Clone)]
pub struct SpeedTree {
    /// Its placement in the world: its scaled axes, then the point it stands on.
    pub transform: Affine3A,
    /// The hash of its `.srt` file's path, see [`super::BWST`]: its kind, as
    /// `destructibles.xml` lists trees.
    pub resource: u32,
    /// The seed its look is varied by.
    pub seed: u32,
    /// `StaticSceneSpeedTreeTypes::Flags`: whether it casts a shadow (1), and shows in
    /// reflections (2).
    pub flags: u32,
}

impl SpaceSection for SPTR {

    const ID: [u8; 4] = *b"SpTr";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        // A row-major matrix of its scaled axes then its translation, as glam reads
        // columns, as `BSMI`'s, then its fields, and one of unknown use, always -1 on
        // Malinovka.
        let trees = read.read_vector(|buf| {
            let mut values = [0.0; 16];
            for value in &mut values {
                *value = buf.read_f32()?;
            }
            Ok(SpeedTree {
                transform: Affine3A::from_mat4(Mat4::from_cols_array(&values)),
                resource: buf.read_u32()?,
                seed: buf.read_u32()?,
                flags: buf.read_u32()?,
            })
        })?;

        Ok(Self { trees })

    }

}
