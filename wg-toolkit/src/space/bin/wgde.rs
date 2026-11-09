use std::io::{self, Read, Seek};

use crate::util::io::WgReadExt;

use super::{invalid, SpaceSection};


/// `WGDE`: the space's destructibles as the client numbers them, by the 100 m chunk
/// they are in then their index in it: what the server's `AreaDestructibles` entity of
/// a chunk lists as destroyed, see the client's `AreaDestructibles.py`. Each is placed
/// static models of [`super::BSMI`], or none, as the trees aren't: they are speed trees.
///
/// Its first vector holds each chunk's id, `(x + 127) << 8 | (z + 127)` of its grid
/// indexes, then the first of its destructibles in the second vector and how many it
/// has; the second each destructible's first and last instance in the third, inclusive,
/// none when the last comes before the first; the third the instances' indexes in
/// `BSMI`, some with their top bit set, its meaning unknown.
#[derive(Debug)]
pub struct WGDE {
    pub destructibles: Vec<Destructible>,
}

/// A destructible of the space, see [`WGDE`].
#[derive(Debug, Clone)]
pub struct Destructible {
    /// The id of the chunk it is in.
    pub chunk_id: u16,
    /// Its index in its chunk.
    pub index: u16,
    /// Its static model instances, their indexes in `BSMI`.
    pub instances: Vec<u32>,
}

impl SpaceSection for WGDE {

    const ID: [u8; 4] = *b"WGDE";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        let chunks = read.read_vector(|buf| Ok((buf.read_u32()?, buf.read_u32()?, buf.read_u32()?)))?;
        let ranges = read.read_vector(|buf| Ok((buf.read_u32()?, buf.read_u32()?)))?;
        let instances = read.read_vector(|buf| buf.read_u32())?;

        let mut destructibles = Vec::with_capacity(ranges.len());
        for (chunk_id, first, count) in chunks {
            let chunk_id = u16::try_from(chunk_id).map_err(|_| invalid("destructible chunk id out of range"))?;
            for index in 0..count {
                let Some(&(first_instance, last_instance)) = ranges.get((first + index) as usize) else {
                    return Err(invalid("destructible out of its chunk's range"));
                };
                let mut own = Vec::new();
                if last_instance != u32::MAX {
                    for instance in first_instance..=last_instance {
                        let Some(&instance) = instances.get(instance as usize) else {
                            return Err(invalid("destructible instance out of range"));
                        };
                        own.push(instance & 0x7fff_ffff);
                    }
                }
                destructibles.push(Destructible {
                    chunk_id,
                    index: index as u16,
                    instances: own,
                });
            }
        }

        Ok(Self { destructibles })

    }

}
