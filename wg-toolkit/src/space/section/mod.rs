//! Compiled space sections structures definitions.

mod bwtb;
mod bwst;
mod bwal;
mod bwcs;
mod bwsg;
mod bwt2;
mod bsmi;
mod bsmo;

use std::io::{self, Read, Seek};

pub use bwtb::{BWTB, SectionMeta};
pub use bwst::{BWST, get_hash, get_hash_from_str};
pub use bwal::{BWAL, AssetInfo, AssetType};
pub use bwcs::BWCS;
pub use bwsg::{BWSG, ModelInfo, PositionInfo};
pub use bwt2::{BWT2, TerrainSettings1, TerrainSettings2, TerrainChunk, OutlandCascade};
pub use bsmi::{BSMI, StaticModelInstance};
pub use bsmo::{BSMO, StaticModel};


/// Alias for 4-bytes array, which is used to identify sections in a compiled space.
pub type SectionId = [u8; 4];


/// Common trait for section in compiled space binaries.
pub trait Section: Sized {

    const ID: &'static SectionId;

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self>;

}
