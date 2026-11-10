use std::io::{self, Read, Seek};

use crate::util::io::WgReadExt;

use super::SpaceSection;


/// `BWAL`: assets the space uses.
#[derive(Debug)]
pub struct BWAL {
    pub assets: Vec<AssetInfo>,
}

#[derive(Debug)]
pub struct AssetInfo {
    pub asset_type: AssetType,
    /// Its path, as a [`BWST`](super::BWST) hash.
    pub string_fnv: u32,
}

#[derive(Debug)]
pub enum AssetType {
    ParticlesResource,
    WaterReflectionTexture,
    ControlPointRadiusPath,
    ModelResource,
    /// A kind not known.
    Unknown(u32),
}

impl SpaceSection for BWAL {

    const ID: [u8; 4] = *b"BWAL";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {
        let assets = read.read_vector(|buf| {
            let kind = match buf.read_u32()? {
                1 => AssetType::ParticlesResource,
                2 => AssetType::WaterReflectionTexture,
                5 => AssetType::ControlPointRadiusPath,
                6 => AssetType::ModelResource,
                kind => AssetType::Unknown(kind),
            };
            Ok(AssetInfo { asset_type: kind, string_fnv: buf.read_u32()? })
        })?;
        Ok(Self { assets })
    }

}
