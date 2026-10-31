//! The account data: everything the client knows about the account (inventory, stats,
//! quests, server settings...), sent as a resource in answer to `CMD_SYNC_DATA`, as a
//! zlib-compressed pickled dict. The client merges it in `Account._update`: a key
//! `(name, '_r')` replaces the cache entry `name` whole, a plain `name` is merged into
//! it. A full sync (the first, with revision 0) carries the `rev` it brings the client
//! to; later syncs give `prevRev` and `rev` in the `onCmdResponseExt` answer instead.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde_pickle::{HashableValue, Value};
use wgtk::util::io::{serde_pickle_de_options, serde_pickle_ser_options};



/// The account data, as the pickled dict it is.
#[derive(Debug, Clone)]
pub struct AccountData {
    pub data: Value,
}

impl AccountData {

    /// Decode a sync resource's data.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut raw = Vec::new();
        ZlibDecoder::new(data).read_to_end(&mut raw)?;
        let data = serde_pickle::value_from_slice(&raw, serde_pickle_de_options())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self { data })
    }

    /// Encode as a sync resource: its description, then its data.
    pub fn encode(&self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let raw = serde_pickle::value_to_vec(&self.data, serde_pickle_ser_options())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        let data = encoder.finish()?;
        Ok((resource_description(&data)?, data))
    }

    /// The revision a full sync brings the client to.
    pub fn revision(&self) -> Option<i64> {
        match self.get(&[key("rev")])? {
            Value::I64(rev) => Some(*rev),
            _ => None,
        }
    }

    /// The compact descriptors of an item type's inventory entries, by inventory id:
    /// `inventory[item_type]['compDescr']`.
    pub fn compact_descrs(&self, item_type: u8) -> impl Iterator<Item = (i64, &[u8])> + '_ {
        let descrs = self.get(&[key("inventory"), HashableValue::I64(item_type.into()), key("compDescr")]);
        let descrs = match descrs {
            Some(Value::Dict(descrs)) => Some(descrs),
            _ => None,
        };
        descrs.into_iter().flatten().filter_map(|(id, cd)| match (id, cd) {
            (HashableValue::I64(id), Value::Bytes(cd)) => Some((*id, &cd[..])),
            _ => None,
        })
    }

    /// The value at a path of dict keys.
    pub fn get(&self, path: &[HashableValue]) -> Option<&Value> {
        path.iter().try_fold(&self.data, |value, k| match value {
            Value::Dict(dict) => dict.get(k),
            _ => None,
        })
    }

    pub fn dict(&self) -> Option<&BTreeMap<HashableValue, Value>> {
        match &self.data {
            Value::Dict(dict) => Some(dict),
            _ => None,
        }
    }

}

/// A dict key as the server writes them: Python 2 `str`, so bytes.
pub fn key(name: &str) -> HashableValue {
    HashableValue::Bytes(name.as_bytes().to_vec())
}

/// The description of a resource stream: the pickled `(len, crc32)` the client checks
/// the data against (`game.onStreamComplete`), the CRC signed as Python 2's
/// `zlib.crc32` gives it.
pub fn resource_description(data: &[u8]) -> io::Result<Vec<u8>> {
    let crc32 = crc32fast::hash(data) as i32;
    let description = Value::Tuple(vec![Value::I64(data.len() as i64), Value::I64(crc32.into())]);
    serde_pickle::value_to_vec(&description, serde_pickle_ser_options())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
