//! Packed XML codec. This codec is widely use in Wargaming games' files.
//! 
//! This is basically a binary compression of an XML file, excepting that
//! some pattern can't be reproduced into. This is why a custom set of
//! structures are introduced in this module to handle this case, such as
//! [`Value`] and [`Element`].

mod de;
mod ser;
mod value;

pub use de::{from_reader, from_bytes, DeError};
pub use value::{Value, Vector, Element};
pub use ser::to_writer;


/// Magic of a packed XML file.
pub const MAGIC: &[u8; 4] = b"\x45\x4E\xA1\x62";


/// Internally used data types for values.
#[derive(Debug, Clone, Copy)]
enum DataType {
    Element = 0,
    String = 1,
    Integer = 2,
    /// A 32-bit float vector of any size.
    Vector = 3,
    Boolean = 4,
    /// This special kind act like a compressed string.
    /// This type is only used when the string to compress has a length
    /// that is a multiple of 4 and composed of the base64 charset. In 
    /// such case the string is base64-decoded, the resulting bytes
    /// are used instead of the string. To get the original string
    /// we need to encode the input.
    CompressedString = 5,
}

impl DataType {

    /// Return the data type from its raw 
    fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            0 => Self::Element,
            1 => Self::String,
            2 => Self::Integer,
            3 => Self::Vector,
            4 => Self::Boolean,
            5 => Self::CompressedString,
            _ => return None
        })
    }

    fn to_raw(self) -> u32 {
        match self {
            DataType::Element => 0,
            DataType::String => 1,
            DataType::Integer => 2,
            DataType::Vector => 3,
            DataType::Boolean => 4,
            DataType::CompressedString => 5
        }
    }

}
