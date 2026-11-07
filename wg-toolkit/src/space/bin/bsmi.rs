use std::io::{self, Read, Seek};

use glam::{Affine3A, Mat4};

use crate::util::io::WgReadExt;

use super::{invalid, skip_vector, SpaceSection};


/// `BSMI`: every static model placed in the space, its transform and which of
/// [`BSMO`]' it is. Only its first and fourth vectors are known.
#[derive(Debug)]
pub struct BSMI {
    pub instances: Vec<StaticModelInstance>,
}

/// A static model placed in the space.
#[derive(Debug, Clone, Copy)]
pub struct StaticModelInstance {
    /// From the model's frame to the space's.
    pub transform: Affine3A,
    /// Its model's index in [`BSMO`].
    pub model: u32,
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
        skip_vector(read)?;
        let models = read.read_vector(|buf| buf.read_u32())?;

        if models.len() != transforms.len() {
            return Err(invalid("static model instances and their models differ in count"));
        }

        let instances = transforms.into_iter().zip(models)
            .map(|(transform, model)| StaticModelInstance { transform, model })
            .collect();
        Ok(Self { instances })

    }

}
