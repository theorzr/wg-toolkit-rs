use std::io::{self, Read, Seek, SeekFrom};

use glam::{Affine3A, Mat4};

use crate::util::io::WgReadExt;

use super::{Section, SectionId};


/// StaticModelInstances section: every static model placed in the space, its
/// transform and which of [`BSMO`](super::BSMO)'s models it is. Only its first and
/// fourth vectors are known.
#[derive(Debug)]
pub struct BSMI {
    pub instances: Vec<StaticModelInstance>,
}

impl Section for BSMI {

    const ID: &'static SectionId = b"BSMI";

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
            return Err(io::Error::new(io::ErrorKind::InvalidData, "static model instances and their models differ in count"));
        }

        let instances = transforms.into_iter().zip(models)
            .map(|(transform, model)| StaticModelInstance { transform, model })
            .collect();
        Ok(Self { instances })

    }

}


/// A static model placed in the space.
#[derive(Debug, Clone, Copy)]
pub struct StaticModelInstance {
    /// From the model's frame to the space's.
    pub transform: Affine3A,
    /// Its model's index in [`BSMO`](super::BSMO).
    pub model: u32,
}


/// Skip a vector of a section, see [`WgReadExt::read_vector`].
pub(super) fn skip_vector<R: Read + Seek>(read: &mut R) -> io::Result<()> {
    let (size, count) = read.read_vector_head()?;
    read.seek(SeekFrom::Current((size * count) as i64))?;
    Ok(())
}
