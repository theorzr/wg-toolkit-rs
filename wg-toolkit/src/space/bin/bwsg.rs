use std::io::{self, Read, Seek};
use std::collections::HashMap;

use crate::util::io::WgReadExt;

use super::{SpaceSection, BWST};


/// `BWSG`: static geometry, its vertices in `BSGD`.
#[derive(Debug)]
pub struct BWSG {
    pub strings: HashMap<u32, String>,
    pub models: Vec<ModelInfo>,
    pub positions: Vec<PositionInfo>,
}

/// A model of the [`BWSG`] and its resources.
#[derive(Debug)]
pub struct ModelInfo {
    pub vertices_fnv: u32,
    pub id_from: u32,
    pub id_to: u32,
    pub vertices_count: u32,
    pub vertices_type_fnv: u32,
}

/// Where [`BWSG`] data is.
#[derive(Debug)]
pub struct PositionInfo {
    pub kind: u64,
    /// The size of its vertices block, from its `.primitives`.
    pub size: u32,
    /// Its index in the data sizes.
    pub data_sizes_id: u32,
    /// Where it starts in `BSGD`.
    pub position: u32,
}

impl SpaceSection for BWSG {

    const ID: [u8; 4] = *b"BWSG";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        // It starts with its own string table.
        let strings = BWST::decode(read)?.strings;

        let models = read.read_vector(|buf| {
            Ok(ModelInfo {
                vertices_fnv: buf.read_u32()?,
                id_from: buf.read_u32()?,
                id_to: buf.read_u32()?,
                vertices_count: buf.read_u32()?,
                vertices_type_fnv: buf.read_u32()?,
            })
        })?;

        let positions = read.read_vector(|buf| {
            Ok(PositionInfo {
                kind: buf.read_u64()?,
                size: buf.read_u32()?,
                data_sizes_id: buf.read_u32()?,
                position: buf.read_u32()?,
            })
        })?;

        Ok(Self { strings, models, positions })

    }

}
