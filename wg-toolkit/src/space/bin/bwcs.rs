use std::io::{self, Read, Seek};

use crate::util::io::WgReadExt;

use super::{expect_single, SpaceSection};


/// `BWCS`: the compiled space's settings, of unknown meaning.
#[derive(Debug)]
pub struct BWCS {
    pub values: [f32; 6],
}

impl SpaceSection for BWCS {

    const ID: [u8; 4] = *b"BWCS";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {
        expect_single(read, 24, "settings")?;
        let mut values = [0.0; 6];
        for value in &mut values {
            *value = read.read_f32()?;
        }
        Ok(Self { values })
    }

}
