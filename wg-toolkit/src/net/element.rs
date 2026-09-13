//! Definitions for elements contained in bundles (and so in packets).

use std::io::{self, Read, Write};
use std::fmt;

use crate::util::AsciiFmt;
use crate::util::io::*;

use super::codec::{Codec, SimpleCodec};


/// The element id for reply.
pub const REPLY_ID: u8 = 0xFF;


/// A trait to be implemented for every element that have a preferred length and a 
/// numerical identifier, this is an alternative to [`Codec`], specifically for elements.
/// If the element's is simple, then it can instead implements [`Codec`] and be derived
/// using the simpler trait [`SimpleElement`]. 
pub trait Element<C: ?Sized>: Sized {

    /// Provide the configuration of this elements when writing it, possibly depending on
    /// the configuration.
    fn write_length(&self, config: &C) -> io::Result<ElementLength>;

    /// Write the element with the given writer and the given configuration.
    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<u8>;

    /// Provide the configuration of this elements when reading it, possibly depending on
    /// the configuration and the numeric identifier of the element.
    fn read_length(config: &C, id: u8) -> io::Result<ElementLength>;

    /// Decode the element from the given reader and the given configuration. The id the
    /// element is being decoded for is also given. This ID should be ignored for
    /// non-top-elements (in replies).
    fn read(read: &mut dyn Read, config: &C, len: usize, id: u8) -> io::Result<Self>;

}

/// A simpler alternative trait to [`Element`] for types that already implements the
/// [`Codec`] trait but with a static numerical identifier and preferred length.
pub trait SimpleElement<C: ?Sized = ()>: Codec<C> {

    /// The numeric ID for this element.
    const ID: u8;

    /// The preferred length to be prefixed before the element, if more length is actually
    /// taken then the element will be *oversized* and will induce some overhead to encode
    /// the real length!
    const LEN: ElementLength;

}

impl<E: SimpleElement<C>, C: ?Sized> Element<C> for E {

    #[inline]
    fn write_length(&self, _config: &C) -> io::Result<ElementLength> {
        Ok(Self::LEN)
    }

    #[inline]
    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<u8> {
        Codec::write(self, write, config).map(|()| Self::ID)
    }

    #[inline]
    fn read_length(_config: &C, _id: u8) -> io::Result<ElementLength> {
        Ok(Self::LEN)
    }

    #[inline]
    fn read(read: &mut dyn Read, config: &C, _len: usize, _id: u8) -> io::Result<Self> {
        Codec::read(read, config)
    }

}


/// Type of length used by a specific message codec.
/// This describes how the length of an element should be encoded in the packet.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ElementLength {
    /// A fixed length element, the length is not written in the header.
    Fixed(u32),
    /// The length is encoded on 8 bits in the element's header.
    Variable8,
    /// The length is encoded on 16 bits in the element's header.
    Variable16,
    /// The length is encoded on 24 bits in the element's header.
    Variable24,
    /// The length is encoded on 32 bits in the element's header.
    Variable32,
    /// The length is not encoded nor decode, so it's up to the element to encode and
    /// decode anything wanted, the length given to [`Element::decode`] is `u32::MAX`,
    /// and the underlying reader is not limited. If the real element being read is
    /// variable then the decoding should stop immediately after, because the remaining
    /// data of the bundle might no longer be correct because of this undefined length.
    /// **This kind of length should be used for debug, see [`DebugElementUndefined`].
    Undefined,
}

impl ElementLength {

    /// Constant for fixed zero-length message length.
    pub const ZERO: Self = Self::Fixed(0);

    /// Read the length from a given reader, this returns None if the length is full of
    /// ones (0xFF...) and therefore it's oversized and we need to handle this.
    pub fn read(self, mut reader: impl Read) -> std::io::Result<Option<u32>> {

        let (len_size, max) = match self {
            Self::Fixed(len) => return Ok(Some(len)),
            Self::Undefined => return Ok(Some(u32::MAX)),
            Self::Variable8 => (1, 0xFF), 
            Self::Variable16 => (2, 0xFFFF),
            Self::Variable24 => (3, 0xFFFFFF),
            Self::Variable32 => return reader.read_u32().map(|n| Some(n)),  // Not oversize for u32
        };

        let len = reader.read_uint(len_size)?;
        Ok((len < max).then_some(len as u32))

    }

    /// Write the length to the given writer, if the length is too big then this function
    /// returns false and the length written is full of ones (0xFF...).
    pub fn write(self, mut writer: impl Write, len: u32) -> std::io::Result<bool> {

        let (len_size, max) = match self {
            Self::Fixed(expected_len) => { 
                assert_eq!(expected_len, len, "this element has fixed length but the actual written length is not coherent"); 
                return Ok(true);
            }
            Self::Undefined => {
                return Ok(true);
            }
            Self::Variable8 =>  (1, 0xFF),
            Self::Variable16 => (2, 0xFFFF),
            Self::Variable24 => (3, 0xFFFFFF),
            Self::Variable32 => {
                // No oversize for u32 apparently, which is logic.
                // See InterfaceElement::compressLength(void *, int).
                writer.write_u32(len)?;
                return Ok(true);
            }
        };

        // Using .min to write the max (0xFF...) if oversize.
        writer.write_uint(max.min(len) as u64, len_size)?;
        Ok(len < max)

    }

    /// Return the size in bytes of this type of length.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Fixed(_) => 0,
            Self::Variable8 => 1,
            Self::Variable16 => 2,
            Self::Variable24 => 3,
            Self::Variable32 => 4,
            Self::Undefined => 0,
        }
    }

}


/// A wrapper for a reply element, with the request ID and the underlying element, use
/// the empty element `()` as element in order to just read the request id.
#[derive(Debug)]
pub struct Reply<D> {
    /// The request ID this reply is for.
    pub request_id: u32,
    /// The inner reply data.
    pub data: D
}

impl<D> Reply<D> {

    #[inline]
    pub fn new(request_id: u32, data: D) -> Self {
        Self { request_id, data }
    }
    
}

impl<D: Codec<C>, C: ?Sized> Codec<C> for Reply<D> {

    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<()> {
        write.write_u32(self.request_id)?;
        Codec::write(&self.data, write, config)
    }

    fn read(read: &mut dyn Read, config: &C) -> io::Result<Self> {
        Ok(Self {
            request_id: read.read_u32()?,
            data: Codec::read(read, config)?,
        })
    }

}

impl<D: Codec<C>, C: ?Sized> SimpleElement<C> for Reply<D> {
    const ID: u8 = REPLY_ID;
    const LEN: ElementLength = ElementLength::Variable32;
}


/// An element of fixed sized that just buffer the data.
#[derive(Clone)]
pub struct DebugElementFixed<const ID: u8, const LEN: usize> {
    pub data: [u8; LEN],
}

impl<const ID: u8, const LEN: usize> SimpleCodec for DebugElementFixed<ID, LEN> {
    
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut data = [0; LEN];
        read.read_exact(&mut data)?;
        Ok(Self { data })
    }

}

impl<const ID: u8, const LEN: usize> SimpleElement for DebugElementFixed<ID, LEN> {
    const ID: u8 = ID;
    const LEN: ElementLength = ElementLength::Fixed(LEN as u32);
}

impl<const ID: u8, const LEN: usize> fmt::Debug for DebugElementFixed<ID, LEN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugElementFixed")
            .field("id", &ID)
            .field("len", &LEN)
            .field("data", &AsciiFmt(&self.data))
            .finish()
    }
}

/// An element of variable size that just buffer the data.
#[derive(Clone)]
pub struct DebugElementVariable8<const ID: u8> {
    pub data: Vec<u8>,
}

/// An element of variable size that just buffer the data.
#[derive(Clone)]
pub struct DebugElementVariable16<const ID: u8> {
    pub data: Vec<u8>,
}

/// An element of variable size that just buffer the data.
#[derive(Clone)]
pub struct DebugElementVariable24<const ID: u8> {
    pub data: Vec<u8>,
}

/// An element of variable size that just buffer the data.
#[derive(Clone)]
pub struct DebugElementVariable32<const ID: u8> {
    pub data: Vec<u8>,
}

/// An element of undefined size that just buffer the data.
#[derive(Clone)]
pub struct DebugElementUndefined<const ID: u8> {
    pub data: Vec<u8>,
}

macro_rules! impl_debug_element_var {
    ( $ident:ident, $len:expr ) => {

        impl<const ID: u8> SimpleCodec for $ident<ID> {
            
            #[inline]
            fn write(&self, write: &mut dyn Write) -> io::Result<()> {
                write.write_all(&self.data)
            }

            #[inline]
            fn read(read: &mut dyn Read) -> io::Result<Self> {
                Ok(Self { data: read.read_blob_to_end()? })
            }

        }

        impl<const ID: u8> SimpleElement for $ident<ID> {
            const ID: u8 = ID;
            const LEN: ElementLength = $len;
        }

        impl<const ID: u8> fmt::Debug for $ident<ID> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($ident))
                    .field("id", &ID)
                    .field("len", &self.data.len())
                    .field("data", &AsciiFmt(&self.data))
                    .finish()
            }
        }

    };
}

impl_debug_element_var!(DebugElementVariable8, ElementLength::Variable8);
impl_debug_element_var!(DebugElementVariable16, ElementLength::Variable16);
impl_debug_element_var!(DebugElementVariable24, ElementLength::Variable24);
impl_debug_element_var!(DebugElementVariable32, ElementLength::Variable32);
impl_debug_element_var!(DebugElementUndefined, ElementLength::Undefined);

/// An utility structure for storing ranges of element's ids. It provides way
/// of converting between **element id** (with optional **sub-id**) and 
/// **exposed id**.
/// 
/// This structure is small and therefore can be copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementIdRange {
    pub first: u8,
    pub last: u8,
}

impl ElementIdRange {

    /// Create a new id range, with first and last ids, both included.
    pub const fn new(first: u8, last: u8) -> Self {
        Self { first, last }
    }

    #[inline]
    pub const fn contains(self, id: u8) -> bool {
        self.first <= id && id <= self.last
    }

    /// Returns the number of slots in this range.
    #[inline]
    pub const fn slots_count(self) -> u8 {
        self.last - self.first + 1
    }

    /// Returns the number of slots that requires a sub-id. These slots are 
    /// starting from the end of the range. For example, if this function
    /// returns 1, this means that the last slot (`.last`), if used, will be
    /// followed by a sub-id.
    /// 
    /// You must give the total number of exposed ids, because the presence
    /// of sub-id depends on how exposed ids can fit in the id range.
    #[inline]
    pub const fn sub_slots_count(self, exposed_count: u16) -> u8 {
        // Calculate the number of excess exposed ids, compared to slots count.
        let excess_count = exposed_count.saturating_sub(self.slots_count() as u16);
        // If the are excess slots, calculate how much additional bytes are 
        // required to represent such number.
        if excess_count > 0 {
            (excess_count / 255 + 1) as u8
        } else {
            0
        }
    }
    
    /// Returns the number of full slots that don't require a sub-id. This
    /// is the opposite of `sub_slots_count`, read its documentation.
    #[inline]
    pub const fn full_slots_count(self, exposed_count: u16) -> u8 {
        self.slots_count() - self.sub_slots_count(exposed_count)
    }

    /// Get the element's id and optional sub-id from the given exposed id
    /// and total count of exposed ids.
    pub fn from_exposed_id(self, exposed_count: u16, exposed_id: u16) -> (u8, Option<u8>) {

        let full_slots = self.full_slots_count(exposed_count);

        if exposed_id < full_slots as u16 {
            // If the exposed id fits in the full slots.
            (self.first + exposed_id as u8, None)
        } else {
            // If the given exposed id require to be put in a sub-slot.
            // First we get how much offset the given exposed id is from the first
            // sub slot (full_slots represent the first sub slot).
            let overflow = exposed_id - full_slots as u16;
            let first_sub_slot = self.first + full_slots;
            // Casts are safe.
            ((first_sub_slot as u16 + overflow / 256) as u8, Some((overflow % 256) as u8))
        }

    }

    /// Get the exposed id from an element, but only return some exposed id if
    /// it fits into 
    pub fn to_exposed_id_checked(self, exposed_count: u16, element_id: u8) -> Option<u16> {
        let raw_exposed_id = element_id - self.first;
        (raw_exposed_id < self.full_slots_count(exposed_count)).then_some(raw_exposed_id as u16)
    }

    /// Get the exposed id from an element id and optionally a sub-id, which 
    /// should be lazily provided with a closure.
    pub fn to_exposed_id(self, exposed_count: u16, element_id: u8, sub_id_getter: impl FnOnce() -> u8) -> u16 {
        
        // This is the raw exposed id, it will be used, with full_slots to determine
        // if a sub-id is needed.
        let exposed_id = element_id - self.first;
        let full_slots = self.full_slots_count(exposed_count);
        
        if exposed_id < full_slots {
            exposed_id as u16
        } else {
            // Calculate of the sub-slot offset within sub-slots.
            let offset_id = exposed_id - full_slots;
            let sub_id = sub_id_getter();
            // Calculate the final exposed id from the sub-id and offset.
            full_slots as u16 + 256 * offset_id as u16 + sub_id as u16
        }
        
    }

}
