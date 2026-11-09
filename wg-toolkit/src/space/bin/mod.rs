//! Compiled spaces, a space's `space.bin`: a table of sections, each identified by 4
//! characters, most of them runs of vectors, a vector being its element size, its
//! element count, then its elements. Strings are referenced by their hash in the
//! [`BWST`]. Each section here is decoded as far as it is known.

mod bwst;
mod bwal;
mod bwcs;
mod bwsg;
mod bwt2;
mod bsmo;
mod bsmi;
mod wgde;

use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::res::fs::{ResFilesystem, ResReadFile};
use crate::util::io::WgReadExt;
use crate::util::fnv::fnv1a_64;

pub use bwt2::{BWT2, TerrainSettings1, TerrainSettings2, TerrainChunk, OutlandCascade};
pub use bwsg::{BWSG, ModelInfo, PositionInfo};
pub use bsmi::{BSMI, StaticModelInstance};
pub use bwal::{BWAL, AssetInfo, AssetType};
pub use wgde::{WGDE, Destructible};
pub use bsmo::{BSMO, StaticModel};
pub use bwst::BWST;
pub use bwcs::BWCS;


/// A compiled space, its sections decoded when asked for.
pub struct SpaceBin<R> {
    inner: R,
    sections: Vec<SpaceSectionHeader>,
}

/// Where a section is in its compiled space.
#[derive(Debug, Clone, Copy)]
pub struct SpaceSectionHeader {
    pub id: [u8; 4],
    pub offset: usize,
    pub len: usize,
}

/// A section of a compiled space.
pub trait SpaceSection: Sized {

    /// Its identifier, `BWST` say.
    const ID: [u8; 4];

    /// Decode it, at its start.
    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self>;

}

impl SpaceBin<io::BufReader<ResReadFile>> {

    /// Open the compiled space of a space, its `space.bin`, `spaces/08_ruinberg` say.
    pub fn load(fs: &ResFilesystem, space_dir: &str) -> io::Result<Self> {
        let path = format!("{space_dir}/space.bin");
        let file = fs.read(&path).map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
        Self::from_reader(io::BufReader::new(file))
            .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))
    }

}

impl<R: Read + Seek> SpaceBin<R> {

    /// Open a compiled space from a seekable reader: only its table of sections, `BWTB`,
    /// is read.
    pub fn from_reader(mut inner: R) -> io::Result<Self> {

        let (root, count) = read_section_header(&mut inner)?;
        if &root.id != b"BWTB" {
            return Err(invalid("not a compiled space"));
        }

        let sections = (0..count)
            .map(|_| read_section_header(&mut inner).map(|(header, _)| header))
            .collect::<io::Result<_>>()?;
        Ok(Self { inner, sections })

    }

    /// Its sections, in the order its table gives them.
    pub fn sections(&self) -> &[SpaceSectionHeader] {
        &self.sections
    }

    /// A section's header, if it has it.
    pub fn section(&self, id: [u8; 4]) -> Option<SpaceSectionHeader> {
        self.sections.iter().find(|header| header.id == id).copied()
    }

    /// A section's bytes, if it has it.
    pub fn section_bytes(&mut self, id: [u8; 4]) -> io::Result<Option<Vec<u8>>> {
        let Some(header) = self.section(id) else { return Ok(None) };
        let mut bytes = vec![0; header.len];
        self.inner.seek(SeekFrom::Start(header.offset as u64))?;
        self.inner.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }

    /// Decode a section, a not found error if it doesn't have it.
    pub fn read_section<S: SpaceSection>(&mut self) -> io::Result<S> {
        let id = String::from_utf8_lossy(&S::ID).into_owned();
        let bytes = self.section_bytes(S::ID)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no {id} section")))?;
        S::decode(&mut Cursor::new(bytes)).map_err(|e| io::Error::new(e.kind(), format!("{id} section: {e}")))
    }

}

/// A section's header, and how many sections follow it for the table's own.
fn read_section_header<R: Read>(read: &mut R) -> io::Result<(SpaceSectionHeader, usize)> {
    let mut id = [0; 4];
    read.read_exact(&mut id)?;
    read.read_u32()?;
    let offset = read.read_u32()? as usize;
    read.read_u32()?;
    let len = read.read_u32()? as usize;
    let count = read.read_u32()? as usize;
    Ok((SpaceSectionHeader { id, offset, len }, count))
}

/// Skip a vector of a section, see [`WgReadExt::read_vector`].
fn skip_vector<R: Read + Seek>(read: &mut R) -> io::Result<()> {
    let (size, count) = read.read_vector_head()?;
    read.seek(SeekFrom::Current((size * count) as i64))?;
    Ok(())
}

/// Read a single structure's size header, an error if it isn't `size`.
fn expect_single<R: Read>(read: &mut R, size: usize, what: &str) -> io::Result<()> {
    match read.read_single_head()? {
        actual if actual == size => Ok(()),
        actual => Err(invalid(format!("{what} of {actual} bytes, not {size}"))),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// A compiled space's string hash, as its sections reference strings.
pub fn get_hash(data: &[u8]) -> u32 {
    (fnv1a_64(data) & 0xFFFFFFFF) as u32
}

/// A compiled space's hash of a string, see [`get_hash`].
pub fn get_hash_from_str(string: &str) -> u32 {
    get_hash(string.as_bytes())
}
