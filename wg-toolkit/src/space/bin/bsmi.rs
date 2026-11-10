use std::io::{self, Read, Seek};

use glam::{Affine3A, Mat4};

use crate::util::io::WgReadExt;

use super::{invalid, skip_vector, SpaceSection};


/// `BSMI`: every static model placed in the space, its transform and which of
/// [`BSMO`](super::BSMO)' it is, and in which game modes. Only its first, third and fourth vectors
/// are known.
#[derive(Debug)]
pub struct BSMI {
    pub instances: Vec<StaticModelInstance>,
}

/// A static model placed in the space.
#[derive(Debug, Clone, Copy)]
pub struct StaticModelInstance {
    /// From the model's frame to the space's.
    pub transform: Affine3A,
    /// Its model's index in [`BSMO`](super::BSMO).
    pub model: u32,
    /// In which game modes it is, see the client's `SpaceVisibilityFlags.py`: its low
    /// 20 bits are the server's, a bit for each gameplay type as the space's
    /// `space.settings` maps them, its high 12 bits the client's.
    pub visibility_mask: u32,
}

impl SpaceSection for BSMI {

    const ID: [u8; 4] = *b"BSMI";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        // Each transform a row-major matrix of its scaled axes then its translation,
        // as glam reads columns.
        let transforms = read.read_vector(|buf| {
            let mut values = [0.0; 16];
            for value in &mut values {
                *value = buf.read_f32()?;
            }
            Ok(Affine3A::from_mat4(Mat4::from_cols_array(&values)))
        })?;

        skip_vector(read)?;
        let visibility_masks = read.read_vector(|buf| buf.read_u32())?;
        let models = read.read_vector(|buf| buf.read_u32())?;

        if models.len() != transforms.len() || visibility_masks.len() != transforms.len() {
            return Err(invalid("static model instances, their models and visibility differ in count"));
        }

        let instances = transforms.into_iter().zip(models).zip(visibility_masks)
            .map(|((transform, model), visibility_mask)| StaticModelInstance { transform, model, visibility_mask })
            .collect();
        Ok(Self { instances })

    }

}
