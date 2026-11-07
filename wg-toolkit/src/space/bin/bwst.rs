use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use crate::util::io::WgReadExt;

use super::{get_hash, invalid, SpaceSection};


/// `BWST`: the strings the other sections reference, by their [`get_hash`].
#[derive(Debug)]
pub struct BWST {
    pub strings: HashMap<u32, String>,
}

impl BWST {

    /// Try to get a string from its hash.
    pub fn get_string(&self, hash: u32) -> Option<&str> {
        self.strings.get(&hash).map(String::as_str)
    }

}

impl SpaceSection for BWST {

    const ID: [u8; 4] = *b"BWST";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        let entries = read.read_vector(|buf| {
            Ok((buf.read_u32()?, buf.read_u32()? as u64, buf.read_u32()? as usize))
        })?;

        let strings_len = read.read_u32()? as u64;
        let strings_offset = read.stream_position()?;

        let mut strings = HashMap::with_capacity(entries.len());
        for (_hash, offset, len) in entries {
            read.seek(SeekFrom::Start(strings_offset + offset))?;
            let mut bytes = vec![0; len];
            read.read_exact(&mut bytes)?;
            let string = String::from_utf8(bytes).map_err(|_| invalid("a string isn't UTF-8"))?;
            strings.insert(get_hash(string.as_bytes()), string);
        }

        read.seek(SeekFrom::Start(strings_offset + strings_len))?;
        Ok(Self { strings })

    }

}
