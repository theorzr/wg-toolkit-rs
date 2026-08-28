//! Structures for managing bundles of packets.

use std::io::{self, Write, Read};
use std::fmt;

use tracing::warn;

use super::packet::{self, PacketConfig, PacketLocked, Packet};
use super::element::{Element, Reply, REPLY_ID};
use super::codec::Codec;

use crate::util::io::{WgReadExt, WgWriteExt, IoCounter};
use crate::net::element::ElementLength;
use crate::util::AsciiFmt;


/// The maximum length for writing bundle elements, it's basically the packet capacity 
/// with prefix, flags and reserved footer length subtracted.
pub const BUNDLE_PACKET_CAP: usize = 
    packet::PACKET_CAP - 
    packet::PACKET_HEADER_LEN - 
    packet::PACKET_RESERVED_FOOTER_LEN;
    
const REQUEST_ID_LEN: usize = 4;
const REQUEST_NEXT_LEN: usize = 2;
const REQUEST_HEADER_LEN: usize = REQUEST_ID_LEN + REQUEST_NEXT_LEN;

/// It makes no sense to have more packets, this will allow us to optimize some sizes
/// of control structures, using only `u16` to index packets.
const BUNDLE_MAX_PACKET_COUNT: usize = u16::MAX as _;


/// A bundle is a sequence of packets that are used to store elements. 
/// Elements of various types, like regular elements, requests or replies can be simply 
/// added and the number of packets contained in this bundle is automatically adjusted 
/// if no more space is available.
#[derive(Debug)]
pub struct Bundle {
    /// Chain of packets.
    packets: Vec<BundlePacket>,
    /// Remaining free size in the current packet. When zero, it will always trigger the
    /// creation of a new packet before any subsequent write.
    free: u16,
    /// Offset in the current packet of the last "next request offset" link (u16) that 
    /// should be filled when a new request element is added, to point to its body offset 
    /// (starting with flags, so value 0 or 1 equals "no next request"). That offset is
    /// in content space.
    last_request_link_offset: Option<u16>,
}

impl Bundle {

    pub fn new() -> Self {
        Self {
            packets: Vec::new(),
            free: 0,
            last_request_link_offset: None,
        }
    }

    pub fn new_with_single(packet: PacketLocked) -> Self {
        std::iter::once(packet).collect()
    }
    
    pub fn new_with_multiple(packets: impl Iterator<Item = PacketLocked>) -> Self {
        packets.collect()
    }

    /// Return the number of packets in this bundle.
    #[inline]
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Clear the bundle by removing all packets.
    pub fn clear(&mut self) {
        self.packets.clear();
        self.free = 0;
        self.last_request_link_offset = None;
    }

    /// Push a new packet in this bundle, the packet must be locked to ensure that the
    /// associated read/write config is synchronized with the packet's content. Writing
    /// any element after pushing will necessarily push a new packet, because we don't
    /// know where is the last request element in that new packet.
    pub fn push(&mut self, locked: PacketLocked) {

        assert!(self.packets.len() < BUNDLE_MAX_PACKET_COUNT, "too much packets");

        let (packet, config) = locked.destruct();

        self.packets.push(BundlePacket {
            packet,
            len: (config.footer_offset() - packet::PACKET_HEADER_LEN) as u16,
            first_request_offset: config.first_request_offset().map(|s| s as u16),
        });

        // Set the free size to zero to force create a new packet on any write.
        self.free = 0;
        self.last_request_link_offset = None;

    }

    /// Push a new empty packet in that bundle, the bundle will be able to write into it
    /// when needed.
    pub fn push_empty(&mut self) {
        
        assert!(self.packets.len() < BUNDLE_MAX_PACKET_COUNT, "too much packets");

        self.packets.push(BundlePacket {
            packet: Packet::new(),
            len: 0,
            first_request_offset: None,
        });

        self.free = BUNDLE_PACKET_CAP as u16;
        self.last_request_link_offset = None;

    }

    /// Write the given configuration to all packets in this bundle. However, there will
    /// be some modifications to the packet configuration to apply the bundle's 
    /// parameters: 
    /// - the sequence number will be incremented after each packet;
    /// - if the bundle contains more than one packet, the sequence range will be set 
    ///   accordingly, so the whole sequence range must've been allocated;
    /// - if the bundle contains zero or one packet, sequence range is disabled;
    /// - for each packet, the first request offset will be forced;
    /// - any cumulative ack is cleared after the first packet.
    /// 
    /// Any subsequent 
    pub fn write_config(&mut self, config: &mut PacketConfig) {
        
        if self.packets.len() > 1 {
            config.set_sequence_range(config.sequence_num(), config.sequence_num() + self.packets.len() as u32 - 1);
        } else {
            config.clear_sequence_range();
        }

        for packet in &mut self.packets {
            
            if let Some(offset) = packet.first_request_offset {
                config.set_first_request_offset(offset as usize);
            } else {
                config.clear_first_request_offset();
            }

            packet.packet.write_config(&mut *config);

            config.set_sequence_num(config.sequence_num() + 1);
            config.clear_cumulative_ack();
            
        }

        config.clear_first_request_offset();  // Some cleanup...

    }

    /// Write the given prefix to all packet.
    pub fn write_prefix(&mut self, prefix: u32) {
        for packet in &mut self.packets {
            packet.packet.write_prefix(prefix);
        }
    }

    /// Update the prefix of all packets using the given offset. 
    /// See [`RawPacket::update_prefix`].
    pub fn update_prefix(&mut self, offset: u32) {
        for packet in &mut self.packets {
            packet.packet.update_prefix(offset);
        }
    }

    /// An iterator to all packets in this bundle, not modifiable.
    pub fn iter(&self) -> impl Iterator<Item = &'_ Packet> + '_ {
        self.packets.iter().map(|p| &p.packet)
    }

    /// Destruct this bundle into its component packets.
    pub fn into_iter(self) -> impl Iterator<Item = Packet> {
        self.packets.into_iter().map(|p| p.packet)
    }

    /// See [`BundleElementReader`].
    pub fn element_reader(&self) -> BundleElementReader<'_> {
        BundleElementReader::new(self)
    }

    /// See [`BundleElementWriter`].
    pub fn element_writer(&mut self) -> BundleElementWriter<'_> {
        BundleElementWriter::new(self)
    }

    /// Reserve a single byte.
    fn reserve_single(&mut self) -> &mut u8 {
        if self.free == 0 {
            self.push_empty();
        }
        let packet = self.packets.last_mut().unwrap();
        self.free -= 1;
        &mut packet.grow(1)[0]
    }

    /// Reserve exactly the given length in the current packet or a new one if
    /// such space is not available in the current packet. **Given length must 
    /// not exceed bundle packet capacity.**
    /// 
    /// This function is currently only used for writing the element's header.
    fn reserve_exact(&mut self, len: usize) -> &mut [u8] {
        debug_assert!(len != 0, "cannot reserve zero byte");
        debug_assert!(len <= BUNDLE_PACKET_CAP, "cannot reserve exact more that bundle packet capacity");
        let len = len as u16;  // Safe cast because of assert.
        if self.free < len {
            self.push_empty();
        }
        let packet = self.packets.last_mut().unwrap();
        self.free -= len;
        packet.grow(len as usize)
    }

    /// Reserve up to the given length in the current packet, if no byte is
    /// available in the current packet, a new packet is created. The final
    /// reserved length is the size of the returned slice.
    fn reserve(&mut self, len: usize) -> &mut [u8] {
        debug_assert!(len != 0, "cannot reserve zero byte");
        if self.free == 0 {
            self.push_empty();
        }
        let packet = self.packets.last_mut().unwrap();
        let len = len.min(self.free as usize);
        self.free -= len as u16;  // Safe cast because of '.min' on 'self.free'.
        packet.grow(len)
    }

}

/// A bundle can be created from an iterator of locked packets.
impl FromIterator<PacketLocked> for Bundle {
    fn from_iter<T: IntoIterator<Item = PacketLocked>>(iter: T) -> Self {
        let mut bundle = Self::new();
        for packet in iter {
            bundle.push(packet);
        }
        bundle
    }
}


/// Internal storage structure for a bundle's packet.
#[derive(Debug)]
struct BundlePacket {
    /// The packet data.
    packet: Packet,
    /// The total length of element's content written to this packet, this doesn't count
    /// the underlying packet's header (prefix and flags) and footer.
    len: u16,
    /// Offset of the first request within element's content space.
    first_request_offset: Option<u16>,
}

impl BundlePacket {

    /// Return the length taken by elements in that packet.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Return the remaining free size for storing bundle elements in that packet.
    #[inline]
    pub fn free(&self) -> usize {
        BUNDLE_PACKET_CAP - self.len()
    }

    /// Get a slice to the data, with the packet's length.
    #[inline]
    pub fn slice(&self) -> &[u8] {
        &self.packet.slice()[packet::PACKET_HEADER_LEN..][..self.len as usize]
    }

    /// Get a mutable slice to the data, with the packet's length.
    #[inline]
    pub fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.packet.slice_mut()[packet::PACKET_HEADER_LEN..][..self.len as usize]
    }

    #[inline]
    pub fn grow(&mut self, len: usize) -> &mut [u8] {
        assert!(len <= self.free(), "not enough available data");
        self.len += len as u16;
        self.packet.grow(len)
    }

}


/// An internal writer implementation used to append data to a bundle,
/// adding packets if needed.
struct BundleWriter<'a> {
    bundle: &'a mut Bundle,
}

impl<'a> BundleWriter<'a> {

    /// Construct a new bundle writer, must be constructed only if at least one packet
    /// is already existing in the bundle.
    fn new(bundle: &'a mut Bundle) -> Self {
        Self { bundle }
    }

}

impl<'a> Write for BundleWriter<'a> {

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let slice = self.bundle.reserve(buf.len());
        slice.copy_from_slice(&buf[..slice.len()]);
        Ok(slice.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

}


/// A simple reader for bundle that join all packet's bodies into a single stream. This 
/// is internally used by [`BundleElementReader`] for reading elements and replies.
/// 
/// *Note that it implements clone in order to save advancement of the reader and 
/// allowing rollbacks.*
#[derive(Clone)]
struct BundleReader<'a> {
    /// Back reference to the bundle, this only take one pointer.
    bundle: &'a Bundle,
    /// See [`BUNDLE_MAX_PACKET_COUNT`] for why using `u16` to store index.
    packet_index: u16,
    /// Keep track of current offset the content is at, relative to packet's content.
    content_offset: u16,
    /// The content remaining to read, we really want to optimize reading this content
    /// so we keep a slice of this it here to avoid the double indirection of going into
    /// a bundle packet reference and then into the packet's boxed data.
    content: &'a [u8],
}

impl<'a> BundleReader<'a> {

    fn new(bundle: &'a Bundle) -> Self {
        Self {
            bundle,
            packet_index: 0,
            content_offset: 0,
            content: bundle.packets.first().map(|p| p.slice()).unwrap_or(&[]),
        }
    }

    fn ensure_inner(&mut self) -> bool {
        while self.content.is_empty() {
            // We don't want to go above length.
            if self.packet_index < self.bundle.packets.len() as u16 {
                self.packet_index += 1;
            }
            // Still use checked '.get' because value may overwrite.
            if let Some(packet) = self.bundle.packets.get(self.packet_index as usize) {
                self.content_offset = 0;
                self.content = packet.slice();
            } else {
                return false;
            }
        }
        true
    }

    /// Ensure that a non-empty contiguous slice of content is available to read.
    pub fn ensure(&mut self) -> Option<&'a [u8]> {
        self.ensure_inner().then_some(self.content)
    }

    #[inline]
    pub fn content_offset(&self) -> u16 {
        self.content_offset
    }

    /// Get the packet index currently being read, note that the current content may be
    /// empty for this packet, and [`Self::ensure`] should be called before to get the right
    /// packet to read.
    #[inline]
    pub fn packet_index(&self) -> u16 {
        self.packet_index
    }

    /// Same as [`Self::packet_index`] but returns the reference to the packet.
    pub fn packet(&self) -> Option<&'a BundlePacket> {
        self.bundle.packets.get(self.packet_index as usize)
    }

    /// Advance the current reader by a given amount. Return true if successful, if not
    /// the reader has been emptied by remaining delta could not be advanced.
    pub fn advance(&mut self, mut delta: usize) -> io::Result<()> {
        while delta != 0 {
            if !self.ensure_inner() { return Err(io::ErrorKind::UnexpectedEof.into()) }
            let len = delta.min(self.content.len());
            self.content = &self.content[len..];
            self.content_offset += len as u16;
            delta -= len;
        }
        Ok(())
    }

}

impl<'a> Read for BundleReader<'a> {

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ensure_inner() {
            let len = buf.len().min(self.content.len());
            buf[..len].copy_from_slice(&self.content[..len]);
            self.content = &self.content[len..];
            self.content_offset += len as u16;
            Ok(len)
        } else {
            Ok(0)
        }
    }

}

impl fmt::Debug for BundleReader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BundleReader")
            .finish()
    }
}


/// The full description of an element being read or to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleElement<E> {
    /// The actual element.
    pub element: E,
    /// The request ID if the element is a request. Not to be confused 
    /// with the reply ID if the element is a `Reply`.
    pub request_id: Option<u32>
}

impl<E> BundleElement<E> {

    /// Map this read element's type into another one with the given 
    /// closure.
    pub fn map<U, F: FnOnce(E) -> U>(self, f: F) -> BundleElement<U> {
        BundleElement {
            element: f(self.element), 
            request_id: self.request_id
        }
    }

}


/// The structure used to write elements to a bundle.
pub struct BundleElementWriter<'a> {
    bundle: &'a mut Bundle,
}

impl<'a> BundleElementWriter<'a> {

    fn new(bundle: &'a mut Bundle) -> Self {
        Self {
            bundle,
        }
    }

    /// Add an element to this bundle.
    #[inline]
    pub fn write<E: Element<C>, C>(&mut self, element: E, config: &C) {
        self.write_raw(BundleElement { element, request_id: None }, config)
    }

    /// Add a simple element to this bundle. Such elements have no config.
    #[inline]
    pub fn write_simple<E: Element<()>>(&mut self, element: E) {
        self.write(element, &())
    }

    /// Add a request element to this bundle, with a given request ID.
    #[inline]
    pub fn write_request<E: Element<C>, C>(&mut self, element: E, request_id: u32, config: &C) {
        self.write_raw(BundleElement { element, request_id: Some(request_id) }, config)
    }

    /// Add a request element to this bundle, with a given request ID. 
    /// Such elements have no config.
    #[inline]
    pub fn write_simple_request<E: Element<()>>(&mut self, element: E, request_id: u32) {
        self.write_request(element, request_id, &())
    }

    /// Add a reply element to this bundle, for a given request ID.
    /// 
    /// Such elements are special and don't require an ID, because they 
    /// are always of  a 32-bit variable length and prefixed with the 
    /// request ID.
    #[inline]
    pub fn write_reply<D: Codec<C>, C>(&mut self, data: D, request_id: u32, config: &C) {
        self.write(Reply::new(request_id, data), config)
    }

    /// Add a reply element to this bundle, for a given request ID.
    /// Such elements have no config.
    #[inline]
    pub fn write_simple_reply<D: Codec<()>>(&mut self, data: D, request_id: u32) {
        self.write_reply(data, request_id, &())
    }

    /// Raw method to add an element to this bundle, given an ID, the 
    /// element and its config. With an optional request ID.
    pub fn write_raw<E: Element<C>, C>(&mut self, element: BundleElement<E>, config: &C) {

        let elt_len_kind = element.element.write_length(config).unwrap();  // FIXME: NO UNWRAP!!

        // Allocate element's header, +1 for element's ID, +6 reply_id and link offset.
        // Using reserve exact so all the header is contiguous.
        let header_len = 1 + elt_len_kind.len() + if element.request_id.is_some() { REQUEST_HEADER_LEN } else { 0 };
        let header_slice = self.bundle.reserve_exact(header_len);

        // If it's a request, write the request ID followed 
        if let Some(request_id) = element.request_id {
            let mut request_header_slice = &mut header_slice[header_len - REQUEST_HEADER_LEN..][..REQUEST_HEADER_LEN];
            request_header_slice.write_u32(request_id).unwrap();
            request_header_slice.write_u16(0).unwrap(); // Next request offset set to null.
        }

        // Keep the packet index to rewrite the packet's length after writing it.
        let init_packet_index = self.bundle.packets.len() - 1;

        // IMPORTANT: All offsets are in the content, not absolute.
        let init_packet = &mut self.bundle.packets[init_packet_index];
        let init_packet_len = init_packet.len();
        let init_packet_elt_offset = init_packet_len - header_len;

        // NOTE: We add flags length to element offset because offset contains flags.
        if element.request_id.is_some() {
        
            if let Some(last_request_link_offset) = self.bundle.last_request_link_offset {
                let mut request_next_slice = &mut init_packet.slice_mut()[last_request_link_offset as usize..][..REQUEST_NEXT_LEN];
                request_next_slice.write_u16((packet::PACKET_FLAGS_LEN + init_packet_elt_offset) as u16).unwrap();
            } else {
                init_packet.first_request_offset = Some(init_packet_elt_offset as u16);
            }
            
            self.bundle.last_request_link_offset = Some((init_packet_len - REQUEST_NEXT_LEN) as u16);
            
        }

        // Write the actual element's content. For now we just unwrap the encode result,
        // because no IO error should be produced by a BundleWriter.
        let mut writer = IoCounter::new(BundleWriter::new(&mut *self.bundle));
        let elt_id = element.element.write(&mut writer, config).unwrap();
        let elt_len = u32::try_from(writer.count()).expect("too many bytes written at once, more that u32::MAX");

        // Finally write id and length, we can unwrap because we know that enough length is available.
        let header_len_slice = &mut self.bundle.packets[init_packet_index].slice_mut()[init_packet_elt_offset..];
        header_len_slice[0] = elt_id;
        // Early return if no oversize!
        if elt_len_kind.write(&mut header_len_slice[1..], elt_len).unwrap() {
            return;
        }

        // If we land here then we need to handle oversize length compression...
        // In this case we'll write the full u32 length replacing the first 4 bytes of 
        // the message and we move these first 4 bytes at the end of the message!! WTF?
        let mut packet_index = init_packet_index;
        let mut content_offset = init_packet_len;
        let mut written_len = elt_len;
        for _ in 0..4 {

            // Extract the moved byte and replace it with lower byte of length, note that 
            // we are written little endian, so least significant first.
            let packet = &mut self.bundle.packets[packet_index];
            let moved_byte = std::mem::replace(&mut packet.slice_mut()[content_offset], written_len as u8);
            written_len >>= 8;

            // Increment content offset and packet index, because it may span two packets.
            content_offset += 1;
            if content_offset == packet.len() {
                packet_index += 1;
                content_offset = 0;
            }

            // Reserve one by one because it may span two packets.
            *self.bundle.reserve_single() = moved_byte;

        }

    }

}


/// The structure used to iterate over a bundle's elements, providing
/// a developer-friendly API that automatically handle reply elements.
/// 
/// This structure can be obtained from [`Bundle::element_reader`].
pub struct BundleElementReader<'a> {
    bundle_reader: BundleReader<'a>,
    last_packet_index: u16,
    next_request_offset: Option<u16>,
}

impl<'a> BundleElementReader<'a> {

    /// Internal constructor used by [`Bundle`] to create the reader.
    fn new(bundle: &'a Bundle) -> Self {

        let bundle_reader = BundleReader::new(bundle);

        Self {
            next_request_offset: bundle_reader.packet().and_then(|p| p.first_request_offset),
            last_packet_index: 0,
            bundle_reader,
        }
        
    }

    /// Read the current element's identifier. This call return the same result until
    /// you explicitly choose to go to the next element while reading the element. This
    /// method takes self by mutable reference because it may need to go to the next
    /// packet when needed.
    pub fn next_id(&mut self) -> Option<u8> {
        self.bundle_reader.ensure().map(|content| content[0])
    }

    /// Read the current element, return a guard that you should use with a 
    /// the element depending on its type with. *This is a simpler version to use over
    /// standard `read_element` method because it handle reply elements for you.*
    pub fn next(&mut self) -> Option<NextElementReader<'_, 'a>> {
        match self.next_id() {
            Some(REPLY_ID) => {
                match self.read::<Reply<()>, ()>(&(), false) {
                    Ok(elt) => {
                        Some(NextElementReader::Reply(ReplyReader(self, elt.element.request_id)))
                    }
                    Err(_) => None
                }
            }
            Some(id) => {
                Some(NextElementReader::Element(ElementReader(self, id)))
            }
            None => None
        }
    }

    /// Try to decode the current element using a given codec. You can choose to go
    /// to the next element using the `next` argument.
    pub fn read<E: Element<C>, C>(&mut self, config: &C, next: bool) -> io::Result<BundleElement<E>> {

        // Here we ensure that we have some bytes to read the next element from.
        let Some(slice) = self.bundle_reader.ensure() else {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more element to read from in the packets"));
        };
        
        // We also update the next request offset if we are on a new packet!
        let start_packet_index = self.bundle_reader.packet_index();
        if self.last_packet_index != start_packet_index {
            self.next_request_offset = self.bundle_reader.packet().and_then(|p| p.first_request_offset);
            self.last_packet_index = start_packet_index;
        }

        // Once we have a non-empty header slice, check if it correspond to the next 
        // request that we are expecting.
        let offset = self.bundle_reader.content_offset();
        let request = self.next_request_offset == Some(offset);

        // Get the element id ahead of time because we need to get the element length.
        let elt_id = slice[0];  // Slice should not be empty.
        let elt_len_kind = E::read_length(config, elt_id)?;

        // Compute the required contiguous length of the header, add request header 
        // length if that element is a request.
        let header_len = 1 + elt_len_kind.len() + if request { REQUEST_HEADER_LEN } else { 0 };
        
        // We requires that the element's header is written contiguous in a single packet.
        if slice.len() < header_len {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the header of the next element is not contiguous"));
        }

        // Keep a clone in order to rollback if not 'next' or any error happens.
        let reader_save = self.bundle_reader.clone();

        // After length has been checked, we can read all this for sure, so we unwrap.
        let elt_id = self.bundle_reader.read_u8().unwrap();
        let elt_len = elt_len_kind.read(&mut self.bundle_reader).unwrap();

        // If the element is a request, we read the next request offset, if that offset
        // is 0 (or 1 but that value is never used) then there is no next request.
        let reply_id = if request {
            let reply_id = self.bundle_reader.read_u32()?;
            let next_request_offset = self.bundle_reader.read_u16()?;
            self.next_request_offset = next_request_offset.checked_sub(packet::PACKET_FLAGS_LEN as u16);
            Some(reply_id)
        } else {
            None
        };

        // If the length is oversized, we need to interpret the first 4 bytes of this 
        // element as the full u32 length...
        let elt_len_oversize = elt_len.is_none();
        let elt_len = match elt_len {
            Some(elt_len) => elt_len,
            None => self.bundle_reader.read_u32()?,
        };

        // Read the last 4 bytes after the element
        let mut moved_bytes = &mut [0; 4][..];
        if elt_len_oversize {
            let mut moved_bytes_reader = self.bundle_reader.clone();
            // -4 for oversize length we just read.
            moved_bytes_reader.advance(elt_len as usize - 4)?;
            moved_bytes_reader.read_exact(&mut *moved_bytes)?;
        } else {
            // Make it empty, no moved bytes to start with!
            moved_bytes = &mut [];
        }

        // We avoid branching to two kind of readers so we chain with moved bytes when
        // oversized, or empty slice if not necessary.
        let elt_reader = moved_bytes.chain(&mut self.bundle_reader);
        let mut elt_reader = elt_reader.take(elt_len as u64);
        let element = match E::read(&mut elt_reader, config, elt_len as usize, elt_id) {
            Ok(ret) => ret,
            Err(e) => {
                self.bundle_reader = reader_save;  // Rollback before going further.
                return Err(e);
            }
        };

        if next {
            // Don't do anything for undefined length, we let the element advance were
            // it wants, and there is no oversize by definition.
            if elt_len_kind != ElementLength::Undefined {

                // Just a warning because the decoding process didn't read all the data. This
                // warning is just enabled when going next, it avoids getting the error when
                // reading the reply header for example.
                let unread_len = elt_reader.limit() as usize;
                if unread_len != 0 {
                    // `unread_len` comes from the element's own declared length, which we
                    // cannot fully trust for elements we don't recognize (e.g. an entity
                    // method with an exposed id outside our generated tables) -- propagate
                    // instead of unwrapping so a bogus/oversized length degrades to a
                    // logged decode error instead of panicking the whole proxy thread.
                    let unread_data = match self.bundle_reader.read_blob(unread_len) {
                        Ok(data) => data,
                        Err(e) => {
                            self.bundle_reader = reader_save;  // Rollback, same as an E::read failure above.
                            return Err(e);
                        }
                    };
                    warn!("remaining data while reading element of type '{}': {:?}", std::any::type_name::<E>(), AsciiFmt(&unread_data));
                }

                // We advance the reader by the amount that has not been read. Unwrapping
                // because it should succeed because the element reader has read this much.
                self.bundle_reader.advance(moved_bytes.len()).unwrap();

            }
        } else {
            // Not going next, only rollback the internal reader.
            self.bundle_reader = reader_save;
        }

        Ok(BundleElement {
            element,
            request_id: reply_id
        })

    }

}

impl fmt::Debug for BundleElementReader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BundleElementReader")
            .field("bundle_reader", &self.bundle_reader)
            .field("next_request_offset", &self.next_request_offset)
            .finish()
    }
}

/// A handle for reading an element or reply on the [`BundleElementReader`].
#[derive(Debug)]
pub enum NextElementReader<'reader, 'bundle> {
    /// A top element with a proper ID and a reader.
    Element(ElementReader<'reader, 'bundle>),
    /// A reply element with request ID and a reader.
    Reply(ReplyReader<'reader, 'bundle>)
}

impl NextElementReader<'_, '_> {

    /// Return `true` if this element is a simple one.
    pub fn is_element(&self) -> bool {
        matches!(self, NextElementReader::Element(_))
    }

    /// Return `true` if this element is a reply.
    pub fn is_reply(&self) -> bool {
        matches!(self, NextElementReader::Reply(_))
    }

}

/// The simple variant of element, provides direct decoding using a codec.
pub struct ElementReader<'reader, 'bundle>(&'reader mut BundleElementReader<'bundle>, u8);

impl ElementReader<'_, '_> {

    /// Get the numeric identifier of the element being read.
    #[inline]
    pub fn id(&self) -> u8 {
        self.1
    }

    /// Same as `read` but never go to the next element *(this is why this method doesn't take
    /// self by value)*.
    pub fn read_stable<E: Element<C>, C>(&mut self, config: &C) -> io::Result<BundleElement<E>> {
        self.0.read(config, false)
    }

    #[inline]
    pub fn read_simple_stable<E: Element<()>>(&mut self) -> io::Result<BundleElement<E>> {
        self.read_stable::<E, ()>(&())
    }

    /// Read the element using the given codec. This method take self by value and automatically
    /// go the next element if read is successful, if not successful you will need to call
    /// `Bundle::next_element` again.
    pub fn read<E: Element<C>, C>(self, config: &C) -> io::Result<BundleElement<E>> {
        self.0.read(config, true)
    }

    #[inline]
    pub fn read_simple<E: Element<()>>(self) -> io::Result<BundleElement<E>> {
        self.read::<E, ()>(&())
    }

}

impl fmt::Debug for ElementReader<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ElementReader").field("id", &self.1).finish()
    }
}

/// The reply variant of element, provides a way to read replies and get `Reply` elements
/// containing the final element.
pub struct ReplyReader<'reader, 'bundle>(&'reader mut BundleElementReader<'bundle>, u32);

impl ReplyReader<'_, '_> {

    /// Get the request id this reply is for.
    #[inline]
    pub fn request_id(&self) -> u32 {
        self.1
    }

    /// Same as `read` but never go to the next element *(this is why this method doesn't take
    /// self by value)*.
    ///
    /// This method doesn't returns the reply element but the final element.
    pub fn read_stable<D: Codec<C>, C>(&mut self, config: &C) -> io::Result<D> {
        let reply = self.0.read::<Reply<D>, C>(config, false)?;
        if reply.request_id.is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "got request id on a reply"));
        }
        Ok(reply.element.data)
    }

    #[inline]
    pub fn read_simple_stable<D: Codec<()>>(&mut self) -> io::Result<D> {
        self.read_stable::<D, ()>(&())
    }

    /// Read the reply element using the given codec. This method take self by value and
    /// automatically go the next element if read is successful, if not successful you
    /// will need to call `Bundle::next_element` again.
    ///
    /// This method doesn't returns the reply element but the final element.
    pub fn read<D: Codec<C>, C>(self, config: &C) -> io::Result<D> {
        let reply = self.0.read::<Reply<D>, C>(config, true)?;
        if reply.request_id.is_some() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "got request id on a reply"));
        }
        Ok(reply.element.data)
    }

    #[inline]
    pub fn read_simple<D: Codec<()>>(self) -> io::Result<D> {
        self.read::<D, ()>(&())
    }

}

impl fmt::Debug for ReplyReader<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplyReader").field("request_id", &self.1).finish()
    }
}
