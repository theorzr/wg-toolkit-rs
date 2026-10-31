//! Compiled space codec, use it to open and read sections of a compiled space binaries.

pub mod section;
pub mod terrain;

use std::io::{self, Read, Seek, SeekFrom};

use section::{Section, BWTB};


/// A structure representing a full compiled space.
pub struct CompiledSpace<R> {
    pub inner: R,
    pub bwtb: BWTB,
}

impl<R: Read + Seek> CompiledSpace<R> {

    /// Create a new lazy compiled space from a seekable read implementor.
    /// This function will only read the BWTB header section  before
    /// actually returning the object.
    pub fn new(mut inner: R) -> io::Result<Self> {

        let bwtb = BWTB::decode(&mut inner)?;

        Ok(CompiledSpace {
            inner,
            bwtb,
        })

    }

    /// Decode a section from this compiled space.
    pub fn decode_section<S: Section>(&mut self) -> Option<S> {
        let meta = self.bwtb.get_section_meta(S::ID)?;
        self.inner.seek(SeekFrom::Start(meta.off as u64)).ok()?;
        Some(S::decode(&mut self.inner).unwrap())
    }

}
