//! Packet structure definition with synchronization methods.

use std::collections::VecDeque;
use std::io::{Cursor, Read};
use std::num::NonZero;
use std::fmt;

use crate::util::io::{SliceCursor, WgReadExt, WgWriteExt};
use crate::util::{AsciiFmt, TruncateFmt};

use super::seq::Seq;


/// According to disassembly of WoT, outside of a channel, the max size if always
/// `1500 - 28 = 1472`, this includes the 4-bytes prefix. When prefix use is disabled
/// then it's only 1468 that the interface is able to receive.
pub const PACKET_CAP: usize = 1472;
/// The length of the unknown 4-byte prefix.
pub const PACKET_PREFIX_LEN: usize = 4;
/// Flags are u16.
pub const PACKET_FLAGS_LEN: usize = 2;
/// Minimum length of a raw packet, containing prefix and flags.
pub const PACKET_HEADER_LEN: usize = PACKET_PREFIX_LEN + PACKET_FLAGS_LEN;

/// The reserved footer len that should be necessarily free at the end of a packet.
/// - 8 for sequence range
/// - 2 for first request offset
/// - 4 for flag 0x1000
/// - 4 for sequence number
/// - 1 for single acks count
/// - 4 * 1 for at least one single acks
/// - 4 for cumulative ack
/// - 8 for indexed channel
/// - 4 for checksum
pub const PACKET_RESERVED_FOOTER_LEN: usize = 8 + 2 + 4 + 4 + 1 + 4 + 4 + 8 + 4;


/// Raw packet layout with only data and length. This structure provides functions for
/// growing and shrinking data, retrieving and modifying its length. Other states such
/// are footer offset or first request offset are not saved in this structure, because
/// this structure is intended to be used as backend of the [`Packet`] structure which
/// contains such state.
/// 
/// The internal data is split in multiple slices that are accessible through the API:
/// 
/// - *Raw data*, it contains the full internal data with max data length, this should
///   be used for receiving datagram from the network.
/// 
/// - *Data*, it contains all the data up to the packet's length.
/// 
#[derive(Clone)]
pub struct Packet {
    /// Inner boxed data.
    inner: Box<Inner>,
}

/// Internal packet data that is boxed.
#[derive(Clone)]
struct Inner {
    /// Full raw data of the packet.
    buf: [u8; PACKET_CAP],
    /// Length of the packet, must not be lower than minimum length which contains the 
    /// prefix and the flags. Stored as `u16` to save size, not much here but for
    /// consistency with [`SyncPacket`] fields, and we don't need more.
    len: u16,
}

impl Packet {

    #[inline]
    pub fn new() -> Self {
        Self {
            inner: Box::new(Inner {
                buf: [0; PACKET_CAP], 
                len: PACKET_HEADER_LEN as u16,
            })
        }
    }

    /// Reset this packet's length, flags and prefix.
    #[inline]
    pub fn reset(&mut self) {
        self.inner.len = PACKET_HEADER_LEN as u16;
        self.inner.buf[..PACKET_HEADER_LEN].fill(0);
    }

    /// Get a slice to the full raw data, this means that this isn't constrained by the 
    /// length of the packet.
    #[inline]
    pub fn buf(&self) -> &[u8; PACKET_CAP] {
        &self.inner.buf
    }

    /// Get a mutable slice to the full raw data, this means that this isn't constrained 
    /// by the length of the packet. The data length can be modified according to the 
    /// changes in this mutable slice.
    /// 
    /// This mutable slice can be used to receive data from an UDP datagram.
    #[inline]
    pub fn buf_mut(&mut self) -> &mut [u8; PACKET_CAP] {
        &mut self.inner.buf
    }

    /// Return the length of this packet, never below [`PACKET_HEADER_LEN`].
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len as usize
    }

    /// Set the length of this packet. The function panics if the length
    /// is not at least `PACKET_MIN_LEN` or at most `PACKET_MAX_LEN`.
    #[inline]
    pub fn set_len(&mut self, len: usize) {
        assert!(len >= PACKET_HEADER_LEN, "given length too small");
        assert!(len <= PACKET_CAP, "given length too high");
        self.inner.len = len as u16;
    }

    /// Return the available length in this packet.
    #[inline]
    pub fn free(&self) -> usize {
        PACKET_CAP - self.len()
    }

    /// Get a slice to the data, with the packet's length.
    /// 
    /// This slice can be used to send data as an UDP datagram for exemple.
    #[inline]
    pub fn slice(&self) -> &[u8] {
        &self.inner.buf[..self.inner.len as usize]
    }

    /// Get a mutable slice to the data, with the packet's length.
    #[inline]
    pub fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.inner.buf[..self.inner.len as usize]
    }

    /// Grow the packet's data by a given amount of bytes, and return a
    /// mutable slice to the newly allocated data.
    /// 
    /// This function panics if the available length is smaller than
    /// requested length.
    #[inline]
    pub fn grow(&mut self, len: usize) -> &mut [u8] {
        assert!(len <= self.free(), "not enough available data");
        let ptr = &mut self.inner.buf[self.inner.len as usize..][..len];
        self.inner.len += len as u16;  // Safe to cast because of assert
        ptr
    }

    /// Read the prefix of this packet. 
    #[inline]
    pub fn read_prefix(&self) -> u32 {
        u32::from_le_bytes(self.inner.buf[..PACKET_PREFIX_LEN].try_into().unwrap())
    }

    /// Write the prefix of this packet.
    #[inline]
    pub fn write_prefix(&mut self, prefix: u32) {
        self.inner.buf[..PACKET_PREFIX_LEN].copy_from_slice(&prefix.to_le_bytes())
    }

    /// Update the prefix of this packet according to the formula found in the assembly.
    /// 
    /// This was actually reverse engineered from the assembly of the game, to find the
    /// formula without knowing the address, you should start by searching the string
    /// `OnceOffPacket` which should be used in one place, in BigWorld source it's
    /// `OnOffSender::addOnceOffResendTimer`. This function is used in one place, in 
    /// source it's `PacketSender::sendPacket`, near the end of this function there is 
    /// a call to the `select` syscall after a call to a function which take 3 arguments,
    /// goto this function, it should start with a 'if' statement with a +300 offset in 
    /// the condition, this if contains two calls, the last one is the prefix computation
    /// (which conditionally get the offset from another structure, but we don't care and
    /// always use zero at the moment).
    pub fn update_prefix(&mut self, offset: u32) {

        let p0 = u32::from_le_bytes(self.inner.buf[PACKET_PREFIX_LEN + 0..][..4].try_into().unwrap());
        let p1 = u32::from_le_bytes(self.inner.buf[PACKET_PREFIX_LEN + 4..][..4].try_into().unwrap());

        let a = offset.wrapping_add(p0).wrapping_add(p1);
        let b = a << 13;
        let c = (b ^ a) >> 17;
        let d = c ^ b ^ a ^ ((c ^ b ^ a) << 5);

        self.write_prefix(d);

    }

    /// Read the flags of this packet.
    #[inline]
    pub fn read_flags(&self) -> u16 {
        u16::from_le_bytes(self.inner.buf[PACKET_PREFIX_LEN..][..PACKET_FLAGS_LEN].try_into().unwrap())
    }

    /// Write the flags of this packet.
    #[inline]
    pub fn write_flags(&mut self, flags: u16) {
        self.inner.buf[PACKET_PREFIX_LEN..][..PACKET_FLAGS_LEN].copy_from_slice(&flags.to_le_bytes())
    }

    /// Read the configuration of this packet into an already existing configuration, 
    /// this is practical if caller wants to stack all acks into the single dequeue.
    pub fn read_config(&self, config: &mut PacketConfig) -> Result<(), PacketConfigError> {

        let mut new = PacketConfig::new();
        // We temporarily swap the two single acks dequeue, so that when decoding the new
        // single acks will be pushed back after existing ones. If not successful we'll
        // revert any pushed ack using the saved start length.
        let start_len = config.single_acks.len();
        // Note that technically, we just want to mem::take the config dequeue, but we
        // use swap to completly avoid any drop logic (still need to check if relevant,
        // because drop might statically know that new.single_acks is empty!).
        std::mem::swap(&mut config.single_acks, &mut new.single_acks);
        // Single acks are then pushed back after existing ones.
        match new.read(self) {
            Ok(()) => {
                // Just overwrite with the new config!
                *config = new;
                Ok(())
            }
            Err(e) => {
                // Swap back and revert any change.
                std::mem::swap(&mut config.single_acks, &mut new.single_acks);
                config.single_acks.truncate(start_len);
                Err(e)
            }
        }

    }

    /// Read the configuration of this packet and returns it.
    pub fn read_config_locked_ref(&self) -> Result<PacketLockedRef<'_>, PacketConfigError> {
        let mut config = PacketConfig::new();
        config.read(self)?;
        Ok(PacketLockedRef { packet: self, config })
    }

    /// Read the configuration of this packet, and lock the packet with its configuration
    /// if successful, if not successful the packet and the error are returned.
    pub fn read_config_locked(self) -> Result<PacketLocked, (PacketConfigError, Self)> {
        let mut config = PacketConfig::new();
        match config.read(&self) {
            Ok(()) => Ok(PacketLocked { packet: self, config }),
            Err(e) => Err((e, self))
        }
    }

    /// Write the given configuration to this packet, the configuration is given with
    /// a mutable reference because the configuration will try to put the maximum number
    /// of single acks possible but it will left remaining ones inside.
    pub fn write_config(&mut self, config: &mut PacketConfig) {
        config.write(self);
    }

    /// Write the given configuration to this packet, and then return a packet locked 
    /// with this configuration, this is used to guarantee packet integrity. Note that
    /// the configuration is still given as mutable because it will try to put the 
    /// maximum number of acks into the packet, remaining ones will be left in place
    /// and the configuration in the locked packet will be a clone without single acks.
    pub fn write_config_locked(mut self, config: &mut PacketConfig) -> PacketLocked {
        self.write_config(&mut *config);
        let mut config = config.clone();
        config.single_acks = VecDeque::new();  // Hope this will optimize out the clone.
        PacketLocked { packet: self, config }
    }

}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        
        let width = f.width().unwrap_or(usize::MAX);
        let mut debug = f.debug_struct("Packet");

        debug.field("prefix", &format_args!("{:08X}", self.read_prefix()));
        debug.field("flags", &format_args!("{:04X}", self.read_flags()));
        debug.field("flags", &format_args!("{}", FlagsFmt(self.read_flags())));
        debug.field("len", &format_args!("{}", self.inner.len));
        if width != 0 {
            debug.field("body", &format_args!("{}", 
                TruncateFmt(&format_args!("{:?}", 
                    AsciiFmt(&self.slice()[PACKET_HEADER_LEN..])), width)));
        }
        debug.finish()

    }
}


/// Represent a configuration for flags their footer values to write or read on/from a
/// packet's data. This configuration allows invalid states and parameter combinations,
/// it's up to the user of this configuration (usually a channel tracker) to properly
/// set these flags.
#[derive(Clone)]
pub struct PacketConfig {
    /// Flags that are written or read from the packet, defining which of the following
    /// fields are used or not, this avoids using boolean or options.
    flags: u16,
    /// The offset of the footer within the packet, this should be below or equal to the
    /// packet's length. When reading configuration, it will be set to the deduced footer
    /// offset, where the decoding ended, and when writing configuration this will be set
    /// to the packet length before writing the footer.
    footer_offset: u16,
    /// The content offset of the first element (see bundle) that is also a request in 
    /// the packet. Zero is just after flags. 
    /// 
    /// Used when `flags::HAS_REQUESTS` is set.
    first_request_offset: u16,
    /// The sequence number of this packet, it is used if reliable mode is enabled
    /// **and/or** if the packet is a fragment of a chain of packet.
    /// 
    /// Used when `flags::IS_RELIABLE` and/or `flags::IS_FRAGMENT` is set.
    sequence_num: Seq,
    /// If this packet is a fragment (defined just after), this contains the
    /// sequence number of the first packet in the chain.
    /// 
    /// Used when `flags::IS_FRAGMENT` is set.
    sequence_first_num: Seq,
    /// If this packet is a fragment (defined in [`Self::`seq_first``]), this contains 
    /// the sequence number of the last packet in the chain (included).
    /// 
    /// Used when `flags::IS_FRAGMENT` is set.
    sequence_last_num: Seq,
    /// The cumulative ack number. This number is sent for acknowledging that all 
    /// sequence numbers up to (but excluding) this ack have been received. Exclusively
    /// used on channels.
    /// 
    /// Used when `flags::HAS_CUMULATIVE_ACK` is set.
    cumulative_ack: Seq,
    /// A queue of packets to piggyback in the footer of the packet, packets are extracted
    /// from the front of dequeue when there is enough place to put them.
    piggybacks: VecDeque<Packet>,
    /// A queue of single acks to put on the packet if space allows it, there should be
    /// at least one ack put on the packet, acks are extracted from the front of dequeue.
    single_acks: VecDeque<Seq>,
    /// Channel index when indexed, never zero if so.
    /// 
    /// Used when `flags::INDEXED_CHANNEL` is set. 
    channel_index: NonZero<u32>,
    /// Channel version when indexed, never zero if so.
    /// 
    /// Used when `flags::INDEXED_CHANNEL` is set. 
    channel_version: NonZero<u32>,
    /// This value is in use when reliable isn't set and this packet is in-channel. It
    /// indicates that this packet should only be processed if the last reliable sequence
    /// number that is contiguous 
    /// 
    /// Used when `flags::UNK1000` is set.
    last_reliable_sequence_num: Seq,
}

impl PacketConfig {

    /// Create a new packet config with every flag disabled, so no footer.
    pub fn new() -> Self {
        Self {
            flags: 0,
            footer_offset: 0,
            first_request_offset: 0,
            sequence_num: Seq::ZERO,
            sequence_first_num: Seq::ZERO,
            sequence_last_num: Seq::ZERO,
            cumulative_ack: Seq::ZERO,
            piggybacks: VecDeque::new(),
            single_acks: VecDeque::new(),
            channel_index: NonZero::new(1).unwrap(),
            channel_version: NonZero::new(1).unwrap(),
            last_reliable_sequence_num: Seq::ZERO,
        }
    }

    #[inline]
    fn has_flags(&self, flags: u16) -> bool {
        self.flags & flags == flags
    }

    #[inline]
    fn enable_flags(&mut self, flags: u16) {
        self.flags |= flags;
    }

    #[inline]
    fn disable_flags(&mut self, flags: u16) {
        self.flags &= !flags;
    }

    #[inline]
    fn switch_flags(&mut self, flags: u16, enabled: bool) {
        if enabled {
            self.enable_flags(flags);
        } else {
            self.disable_flags(flags);
        }
    }

    /// The offset of the footer within the packet, this should be below or equal to the
    /// packet's length. When reading configuration, it will be set to the deduced footer
    /// offset, where the decoding ended, and when writing configuration this will be set
    /// to the packet length before writing the footer.
    #[inline]
    pub fn footer_offset(&self) -> usize {
        self.footer_offset as usize
    }

    /// Return the offset of the next request element in this packet. The offset returned
    /// is relative to the packet's content space, so it starts after flags.
    #[inline]
    pub fn first_request_offset(&self) -> Option<usize> {
        self.has_flags(flags::HAS_REQUESTS)
            .then_some(self.first_request_offset as usize)
    }

    /// Set the first offset of the next request element in this packet. 
    /// Refer to [`Self::first_request_offset`] function for limitations.
    #[inline]
    pub fn set_first_request_offset(&mut self, offset: usize) {
        assert!(offset <= u16::MAX as usize);
        self.enable_flags(flags::HAS_REQUESTS);
        self.first_request_offset = offset as u16;
    }

    /// Clear the first request offset.
    #[inline]
    pub fn clear_first_request_offset(&mut self) {
        self.disable_flags(flags::HAS_REQUESTS);
        self.first_request_offset = 0;
    }

    /// Return the sequence number of the packet, it is used if reliable mode is enabled
    /// and/or if the packet is a fragment of a chain of packet.
    #[inline]
    pub fn sequence_num(&self) -> Seq {
        self.sequence_num
    }

    /// See [`Self::sequence_num()`].
    #[inline]
    pub fn set_sequence_num(&mut self, num: Seq) {
        self.sequence_num = num;
    }

    /// Returns the range of sequence number in case this packet is a fragment
    /// of a packet chain. Both bounds are included.
    #[inline]
    pub fn sequence_range(&self) -> Option<(Seq, Seq)> {
        self.has_flags(flags::IS_FRAGMENT)
            .then_some((self.sequence_first_num, self.sequence_last_num))
    }

    /// Set the range of sequence number if this packet is a fragment of a
    /// packet chain. Both bounds are included and `last` should be greater
    /// than `first`, this function panics if this condition is not met.
    /// 
    /// See also `clear_sequence_range` if you want to clear the range.
    /// 
    /// *Note that* the sequence number is not checked to be in bounds.
    #[inline]
    pub fn set_sequence_range(&mut self, first: Seq, last: Seq) {
        assert!(first.wrapping_cmp(last).is_lt(), "invalid range");
        self.enable_flags(flags::IS_FRAGMENT);
        self.sequence_first_num = first;
        self.sequence_last_num = last;
    }

    /// Clear the range of sequence number. After calling this, the packet 
    /// is no longer a fragment in a packet chain.
    #[inline]
    pub fn clear_sequence_range(&mut self) {
        self.disable_flags(flags::IS_FRAGMENT);
        self.sequence_first_num = Seq::ZERO; // Set zero, just for sanity.
        self.sequence_last_num = Seq::ZERO;
    }

    /// Returns true if the sender of this packet requires an acknowledgment from 
    /// the receiver upon successful reception of this packet. This will work both
    /// off-channel and on-channel, this requires that the 
    #[inline]
    pub fn reliable(&self) -> bool {
        self.has_flags(flags::IS_RELIABLE)
    }

    /// Read [`Self::reliable()`] doc for explanation of this value.
    #[inline]
    pub fn set_reliable(&mut self, reliable: bool) {
        self.switch_flags(flags::IS_RELIABLE, reliable);
    }

    /// Returns true if the create channel flag should be enabled.
    #[inline]
    pub fn create_channel(&self) -> bool {
        self.has_flags(flags::CREATE_CHANNEL)
    }

    /// Read `create_channel` doc for explanation of this value.
    #[inline]
    pub fn set_create_channel(&mut self, create_channel: bool) {
        self.switch_flags(flags::CREATE_CHANNEL, create_channel);
    }

    /// This number is sent for acknowledging that all sequence numbers up to (but 
    /// excluding) this ack have been received.
    #[inline]
    pub fn cumulative_ack(&self) -> Option<Seq> {
        self.has_flags(flags::HAS_CUMULATIVE_ACK)
            .then_some(self.cumulative_ack)
    }

    /// Set the cumulative ack if this packet. Because this value is an excluded
    /// bound, you should not set this to 0. If you want to reset the cumulative
    /// ack, use `clear_cumulative_ack` instead.
    #[inline]
    pub fn set_cumulative_ack(&mut self, sequence_num: Seq) {
        self.enable_flags(flags::HAS_CUMULATIVE_ACK);
        self.cumulative_ack = sequence_num;
    }

    /// Clear the cumulative ack from this packet.
    #[inline]
    pub fn clear_cumulative_ack(&mut self) {
        self.disable_flags(flags::HAS_CUMULATIVE_ACK);
        self.cumulative_ack = Seq::ZERO;  // Just for sanity...
    }

    /// Return a reference to the internal dequeue containing any piggyback packet.
    #[inline]
    pub fn piggybacks(&self) -> &VecDeque<Packet> {
        &self.piggybacks
    }

    /// See [`Self::piggybacks`].
    #[inline]
    pub fn piggybacks_mut(&mut self) -> &mut VecDeque<Packet> {
        &mut self.piggybacks
    }

    /// Return a reference to the internal dequeue containing single acks to add. We use
    /// a queue here because not all acks may be successfully moved into the packet if
    /// space is missing.
    #[inline]
    pub fn single_acks(&self) -> &VecDeque<Seq> {
        &self.single_acks
    }

    /// See [`Self::single_acks`].
    #[inline]
    pub fn single_acks_mut(&mut self) -> &mut VecDeque<Seq> {
        &mut self.single_acks
    }

    #[inline]
    pub fn on_channel(&self) -> bool {
        self.has_flags(flags::ON_CHANNEL)
    }

    #[inline]
    pub fn set_on_channel(&mut self, on_channel: bool) {
        self.switch_flags(flags::ON_CHANNEL, on_channel);
    }

    /// Return the indexed channel, if existing, using tuple `(id, version)`.
    #[inline]
    pub fn indexed_channel(&self) -> Option<(NonZero<u32>, NonZero<u32>)> {
        self.has_flags(flags::INDEXED_CHANNEL)
            .then_some((self.channel_index, self.channel_version))
    }

    #[inline]
    pub fn set_indexed_channel(&mut self, index: NonZero<u32>, version: NonZero<u32>) {
        self.enable_flags(flags::INDEXED_CHANNEL);
        self.channel_index = index;
        self.channel_version = version;
    }

    #[inline]
    pub fn clear_indexed_channel(&mut self) {
        self.disable_flags(flags::INDEXED_CHANNEL);
    }

    #[inline]
    pub fn has_checksum(&self) -> bool {
        self.has_flags(flags::HAS_CHECKSUM)
    }

    #[inline]
    pub fn set_has_checksum(&mut self, enabled: bool) {
        self.switch_flags(flags::HAS_CHECKSUM, enabled);
    }

    /// The usage of this value and flag 0x1000 is unknown. It will be
    /// renamed in the future if its purpose is discovered.
    #[inline]
    pub fn last_reliable_sequence_num(&self) -> Option<Seq> {
        self.has_flags(flags::UNK_1000).then_some(self.last_reliable_sequence_num)
    }

    /// The usage of this value and flag 0x1000 is unknown. It will be
    /// renamed in the future if its purpose is discovered.
    #[inline]
    pub fn set_last_reliable_sequence_num(&mut self, val: Seq) {
        self.enable_flags(flags::UNK_1000);
        self.last_reliable_sequence_num = val;
    }

    /// The usage of this value and flag 0x1000 is unknown. It will be
    /// renamed in the future if its purpose is discovered.
    #[inline]
    pub fn clear_last_reliable_sequence_num(&mut self) {
        self.disable_flags(flags::UNK_1000);
        self.last_reliable_sequence_num = Seq::ZERO;  // For sanity
    }

    /// Read the configuration from the packet. **Be careful! If not successful, the 
    /// state of this config is not guaranteed (single acks could not be deleted).**
    fn read(&mut self, packet: &Packet) -> Result<(), PacketConfigError> {

        // Create a new packet config that we'll push if read is successful.
        self.flags = packet.read_flags();

        // Create a cursor to read data from the end, we skip the header so that any
        // read of before the slice is error and so return packet corrupted error.
        let mut data = SliceCursor::new(&packet.slice()[PACKET_HEADER_LEN..]);

        // This list of flags contains all flags supported by this function.
        const KNOWN_FLAGS: u16 =
            flags::HAS_CHECKSUM |
            flags::INDEXED_CHANNEL |
            flags::HAS_CUMULATIVE_ACK |
            flags::HAS_ACKS |
            flags::HAS_SEQUENCE_NUMBER |
            flags::UNK_1000 |
            flags::HAS_REQUESTS |
            flags::IS_FRAGMENT |
            flags::ON_CHANNEL |
            flags::IS_RELIABLE |
            flags::CREATE_CHANNEL |
            flags::HAS_PIGGYBACKS;

        if self.flags & !KNOWN_FLAGS != 0 {
            return Err(PacketConfigError::UnknownFlags(self.flags & !KNOWN_FLAGS));
        }

        if self.has_flags(flags::HAS_CHECKSUM) {

            // We shrink the packet to read the checksum and then compute the checksum 
            // from the body data, which no longer contains the checksum itself!
            let expected_checksum = data.pop_back(4)
                .ok_or(PacketConfigError::MissingChecksumFooter)?
                .read_u32().unwrap();

            // Compute checksum, containing flags up to, but excluding, the checksum (-4).
            let computed_checksum = calc_checksum(Cursor::new(&packet.slice()[PACKET_PREFIX_LEN..packet.len() - 4]));

            if expected_checksum != computed_checksum {
                return Err(PacketConfigError::InvalidChecksum)
            }

        }

        if self.has_flags(flags::HAS_PIGGYBACKS) {
            loop {

                let piggyback_len = data.pop_back(2)
                    .ok_or(PacketConfigError::MissingPiggybackFooter)?
                    .read_i16().unwrap();
                
                // The last piggy back has a negative length, which is equivalent to having
                // the most significant bit set to 1, but we don't simply invert the length
                // because it would not be possible to represent zero-length, so we do 
                // '-len - 1' which is equivalent to inverting all bits. It's ones' comp.
                let piggyback_done = piggyback_len < 0;
                let piggyback_len = if piggyback_done { !piggyback_len } else { piggyback_len } as u16;

                let piggyback_slice = data.pop_back(piggyback_len as usize)
                    .ok_or(PacketConfigError::MissingPiggybackData)?;

                // Create the new packet, copy the content and just set length.
                // Note that we don't copy prefix!
                let mut piggyback_packet = Packet::new();
                piggyback_packet.buf_mut()[PACKET_PREFIX_LEN..][..piggyback_slice.len()].copy_from_slice(piggyback_slice);
                piggyback_packet.set_len(PACKET_PREFIX_LEN + piggyback_slice.len());
                self.piggybacks.push_back(piggyback_packet);

                if piggyback_done {
                    break;
                }

            }
        }

        if self.has_flags(flags::INDEXED_CHANNEL) {

            let mut cursor = data.pop_back(8).ok_or(PacketConfigError::MissingIndexedChannelFooter)?;
            let version = cursor.read_u32().unwrap();
            let index = cursor.read_u32().unwrap();

            self.channel_index = NonZero::new(index).ok_or(PacketConfigError::ZeroChannelIndex)?;
            self.channel_version = NonZero::new(version).ok_or(PacketConfigError::ZeroChannelVersion)?;

        }

        if self.has_flags(flags::HAS_CUMULATIVE_ACK) {
            self.cumulative_ack = data.pop_back(4)
                .ok_or(PacketConfigError::MissingCumulativeAckFooter)?
                .read_u32().unwrap()
                .try_into().map_err(|()| PacketConfigError::InvalidCumulativeAck)?;
        }

        if self.has_flags(flags::HAS_ACKS) {
            
            let count = data.pop_back(1).ok_or(PacketConfigError::MissingAcksCountFooter)?[0];
            if count == 0 {
                return Err(PacketConfigError::ZeroAcksCount)
            }

            for _ in 0..count {
                let ack = data.pop_back(4)
                    .ok_or(PacketConfigError::MissingAckFooter)?
                    .read_u32().unwrap()
                    .try_into().map_err(|()| PacketConfigError::InvalidAck)?;
                self.single_acks.push_back(ack);
            }

        }

        if self.has_flags(flags::HAS_SEQUENCE_NUMBER) {
            // NOTE: This will be really used if IS_RELIABLE or IS_FRAGMENT.
            self.sequence_num = data.pop_back(4)
                .ok_or(PacketConfigError::MissingSequenceNumFooter)?
                .read_u32().unwrap()
                .try_into().map_err(|()| PacketConfigError::InvalidSequenceNum)?;
        }

        if self.has_flags(flags::UNK_1000) {
            self.last_reliable_sequence_num = data.pop_back(4)
                .ok_or(PacketConfigError::MissingLastReliableSequenceNumFooter)?
                .read_u32().unwrap()
                .try_into().map_err(|()| PacketConfigError::InvalidLastReliableSequenceNum)?;
        }

        if self.has_flags(flags::HAS_REQUESTS) {
            
            self.first_request_offset = data.pop_back(2)
                .ok_or(PacketConfigError::MissingFirstRequestOffsetFooter)?
                .read_u16().unwrap();

            if self.first_request_offset < PACKET_FLAGS_LEN as u16 {
                return Err(PacketConfigError::InvalidFirstRequestOffset);
            } else {
                self.first_request_offset -= PACKET_FLAGS_LEN as u16;
            }

        }

        if self.has_flags(flags::IS_FRAGMENT) {
            
            let mut cursor = data.pop_back(8)
                .ok_or(PacketConfigError::MissingSequenceRangeFooter)?;
            
            self.sequence_first_num = cursor.read_u32().unwrap()
                .try_into().map_err(|()| PacketConfigError::InvalidSequenceRangeFirst)?;
            self.sequence_last_num = cursor.read_u32().unwrap()
                .try_into().map_err(|()| PacketConfigError::InvalidSequenceRangeLast)?;
            
            if Seq::wrapping_cmp(self.sequence_first_num, self.sequence_last_num).is_ge() {
                return Err(PacketConfigError::InvalidSequenceRange)
            }

        }
        
        // Because the data don't include the header we add it to the remaining length.
        self.footer_offset = (PACKET_HEADER_LEN + data.len()) as u16;

        Ok(())

    }

    /// Write the current configuration to the given packet, the footers will be written
    /// after the packet length, growing it. The free data length should also be greater
    /// or equal to `PACKET_RESERVED_FOOTER_LEN`.
    fn write(&mut self, packet: &mut Packet) {

        debug_assert!(packet.len() >= PACKET_HEADER_LEN);
        debug_assert!(packet.free() >= PACKET_RESERVED_FOOTER_LEN);

        self.footer_offset = packet.len() as u16;

        // Min footer += 8
        if let Some((first_num, last_num)) = self.sequence_range() {
            let mut cursor = packet.grow(8);
            cursor.write_u32(first_num.get()).unwrap();
            cursor.write_u32(last_num.get()).unwrap();
        }

        // Min footer += 2
        if let Some(first_request_offset) = self.first_request_offset() {
            packet.grow(2).write_u16((first_request_offset + PACKET_FLAGS_LEN) as u16).unwrap();
        }

        // Min footer += 4
        if let Some(val) = self.last_reliable_sequence_num() {
            packet.grow(4).write_u32(val.get()).unwrap();
        }

        // Min footer += 4
        if self.has_flags(flags::IS_RELIABLE) || self.has_flags(flags::IS_FRAGMENT) {
            // This flags is not used by getters/setters so it's safe to change it here.
            self.enable_flags(flags::HAS_SEQUENCE_NUMBER);
            packet.grow(4).write_u32(self.sequence_num.get()).unwrap();
        } else {
            self.disable_flags(flags::HAS_SEQUENCE_NUMBER);
        }

        // Min footer += 1 + 4
        if !self.single_acks.is_empty() {

            // Unused by getters/setters so it's safe here.
            self.enable_flags(flags::HAS_ACKS);

            // Compute the remaining footer length for acks (not counting the count).
            let available_len = packet.free()
                - if self.cumulative_ack().is_some() { 4 } else { 0 }
                - if self.indexed_channel().is_some() { 8 } else { 0 }
                - if self.has_checksum() { 4 } else { 0 }
                - 1; // Acks count

            // Debug assert, and cap to the max number of acks.
            debug_assert!(available_len >= 4, "PACKET_MIN_FOOTER_LEN should ensure at least one single ack");
            let available_len = available_len.min(u8::MAX as usize * 4);

            let mut count = 0u8;
            while let Some(ack) = self.single_acks.pop_front() {
                count += 1;
                packet.grow(4).write_u32(ack.get()).unwrap();
                if available_len < 4 {
                    break;
                }
            }

            debug_assert!(count != 0);
            packet.grow(1)[0] = count;

        } else {
            self.disable_flags(flags::HAS_ACKS);
        }

        // Min footer += 4
        if let Some(cumulative_ack) = self.cumulative_ack() {
            packet.grow(4).write_u32(cumulative_ack.get()).unwrap();
        }

        // Min footer += 8
        if let Some((id, version)) = self.indexed_channel() {
            let mut cursor = packet.grow(8);
            cursor.write_u32(version.get()).unwrap();
            cursor.write_u32(id.get()).unwrap();
        }

        // Now add as many piggyback packets as possible!
        let mut has_piggyback = false;
        while let Some(piggyback_packet) = self.piggybacks.front() {
            // Check if the packet can be safely added, ending with its own length (i16),
            // we must also consider checksum!
            let piggyback_slice = piggyback_packet.slice();
            if packet.free() >= piggyback_slice.len() + 2 + if self.has_checksum() { 4 } else { 0 } {
                packet.grow(piggyback_slice.len()).copy_from_slice(piggyback_slice);
                packet.grow(2).write_i16(piggyback_slice.len() as i16).unwrap();
                self.piggybacks.pop_front().unwrap();
                has_piggyback = true;
            } else {
                if has_piggyback {
                    // When breaking, we must mark any previously inserted piggyback to be
                    // the last one, so we negate of its bits.
                    for byte in packet.slice_mut().split_last_chunk_mut::<2>().unwrap().1 {
                        *byte = !*byte;
                    }
                }
                break;
            }
        }
        self.switch_flags(flags::HAS_PIGGYBACKS, has_piggyback);

        // Write flags just before computing any checksum.
        packet.write_flags(self.flags);

        // If checksum enabled, compute the checksum of the whole body of the packet,
        // which range from flags to the end of the footer. The checksum will be
        // appended to the footer after computing the checksum.
        // Min footer += 4
        if self.has_checksum() {
            let checksum = calc_checksum(Cursor::new(&packet.slice()[PACKET_PREFIX_LEN..]));
            packet.grow(4).write_u32(checksum).unwrap();
        }
        
    }

}

impl fmt::Debug for PacketConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        
        let alt = f.alternate();
        let mut debug = f.debug_struct("PacketConfig");

        if alt {

            debug.field("footer_offset", &self.footer_offset());

            if let Some(val) = self.first_request_offset() {
                debug.field("first_request_offset", &val);
            }

            if self.reliable() || self.sequence_range().is_some() {
                debug.field("sequence_num", &self.sequence_num());
            }

            if let Some(val) = self.sequence_range() {
                debug.field("sequence_range", &val);
            }

            if self.reliable() { debug.field("reliable", &true); }
            if self.create_channel() { debug.field("create_channel", &true); }

            if let Some(val) = self.cumulative_ack() {
                debug.field("cumulative_ack", &val);
            }

            if self.on_channel() { debug.field("on_channel", &true); }

            if let Some((index, version)) = self.indexed_channel() {
                debug.field("channel_index", &index);
                debug.field("channel_version", &version);
            }

            if self.has_checksum() { debug.field("has_checksum", &true); }

            if let Some(val) = self.last_reliable_sequence_num() {
                debug.field("unk_1000", &val);
            }

        } else {
            debug.field("footer_offset", &self.footer_offset());
            debug.field("first_request_offset", &self.first_request_offset());
            debug.field("sequence_num", &self.sequence_num());
            debug.field("sequence_range", &self.sequence_range());
            debug.field("reliable", &self.reliable());
            debug.field("create_channel", &self.create_channel());
            debug.field("cumulative_ack", &self.cumulative_ack());
            debug.field("single_acks", &self.single_acks());
            debug.field("on_channel", &self.on_channel());
            debug.field("indexed_channel", &self.indexed_channel());
            debug.field("has_checksum", &self.has_checksum());
            debug.field("unk_1000", &self.last_reliable_sequence_num());
        }

        debug.finish()
            
    }
}


/// Represent a packet that has been read or written a configuration, both are kept in
/// this structure in order to provide guarantee that their content is not modified, and
/// therefore that the packet's data and the configuration are fully synchronized.
#[derive(Debug)]
pub struct PacketLocked {
    /// The packet with real data.
    packet: Packet,
    /// The configuration, synchronized with the packet.
    config: PacketConfig,
}

impl PacketLocked {

    #[inline]
    pub fn packet(&self) -> &Packet {
        &self.packet
    }

    #[inline]
    pub fn config(&self) -> &PacketConfig {
        &self.config
    }

    #[inline]
    pub fn destruct(self) -> (Packet, PacketConfig) {
        (self.packet, self.config)
    }

    /// Piggybacks can overpass the immutable locking restriction because modifying or
    /// taking any packet won't change the coherency of the packet's flags or data. 
    /// Note however that any user of this locked packet will see any modifications.
    #[inline]
    pub fn piggybacks_mut(&mut self) -> &mut VecDeque<Packet> {
        self.config.piggybacks_mut()
    }

    /// Singles acks can overpass the immutable locking restriction because modifying or
    /// taking any sequence won't change the coherency of the packet's flags or data. 
    /// Note however that any user of this locked packet will see any modifications.
    #[inline]
    pub fn single_acks_mut(&mut self) -> &mut VecDeque<Seq> {
        self.config.single_acks_mut()
    }

}

/// Same as [`PacketLocked`] but is borrowing the packet.
#[derive(Debug)]
pub struct PacketLockedRef<'a> {
    /// The packet with real data.
    packet: &'a Packet,
    /// The configuration, synchronized with the packet.
    config: PacketConfig,
}

impl<'a> PacketLockedRef<'a> {

    #[inline]
    pub fn packet(&self) -> &'a Packet {
        self.packet
    }

    #[inline]
    pub fn config(&self) -> &PacketConfig {
        &self.config
    }

    #[inline]
    pub fn destruct(self) -> (&'a Packet, PacketConfig) {
        (self.packet, self.config)
    }

}


/// Generic function to calculate the checksum from a reader and
/// a given number of bytes available.
fn calc_checksum(mut reader: impl Read) -> u32 {
    let mut checksum = 0;
    while let Ok(num) = reader.read_u32() {
        checksum ^= num;
    }
    checksum
}


/// Internal module defining flags for packets.
#[allow(unused)]
pub mod flags {
    pub const HAS_REQUESTS: u16         = 0x0001;
    pub const HAS_PIGGYBACKS: u16       = 0x0002;
    pub const HAS_ACKS: u16             = 0x0004;
    pub const ON_CHANNEL: u16           = 0x0008;
    pub const IS_RELIABLE: u16          = 0x0010;
    pub const IS_FRAGMENT: u16          = 0x0020;
    pub const HAS_SEQUENCE_NUMBER: u16  = 0x0040;
    pub const INDEXED_CHANNEL: u16      = 0x0080;
    pub const HAS_CHECKSUM: u16         = 0x0100;
    pub const CREATE_CHANNEL: u16       = 0x0200;
    pub const HAS_CUMULATIVE_ACK: u16   = 0x0400;
    // Looks like this flag is only used with CHANNEL_CREATE, so not on external interface!
    pub const UNK_0800: u16             = 0x0800;
    /// - It has been found that this flag is only set when RELIABLE is not set and we 
    ///   are in a channel, there is also an unknown parameter on the bundle that should
    ///   be false and a parameter somewhere related to the channel structure.
    /// 
    /// - It is also tightly coupled to the sequence number and isn't set without it,
    ///   its value (see preparePackets).
    /// 
    /// - NOT ALLOWED OFF CHANNEL
    /// 
    /// - First value seems to be 0xFFFFFFF because it's -1 the initial channel's value
    ///   (which is zero), and 0xFFFFFFF is seqMask(0 - 1).
    pub const UNK_1000: u16             = 0x1000;
}


/// Wrapper structure for displaying flags.
struct FlagsFmt(pub u16);

impl fmt::Display for FlagsFmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        static NAMES: [&'static str; 13] = [
            "REQS",
            "PIGB",
            "ACKS",
            "CHAN",
            "RELI",
            "FRAG",
            "SEQN",
            "INDX",
            "CSUM",
            "CREA",
            "CUMU",
            "0800",
            "LAST",
        ];

        let mut flag = self.0;
        let mut prev = false;
        for flag_name in NAMES {
            if flag & 1 != 0 {
                if prev {
                    f.write_str("|")?;
                }
                f.write_str(flag_name)?;
                prev = true;
            }
            flag >>= 1;
        }

        if flag != 0 {
            if prev {
                f.write_str("|")?;
            }
            f.write_fmt(format_args!("{:04X}?", flag << NAMES.len()))?;
        }

        Ok(())

    }
}

impl fmt::Debug for FlagsFmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Flags(")?;
        fmt::Display::fmt(self, f)?;
        f.write_str(")")?;
        Ok(())
    }
}


/// Packet error when reading invalid config from a packet.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PacketConfigError {
    #[error("unknown flags: {0:04X}")]
    UnknownFlags(u16),
    #[error("missing checksum footer")]
    MissingChecksumFooter,
    #[error("missing piggyback footer")]
    MissingPiggybackFooter,
    #[error("missing piggyback data")]
    MissingPiggybackData,
    #[error("missing indexed channel footer")]
    MissingIndexedChannelFooter,
    #[error("zero channel index")]
    ZeroChannelIndex,
    #[error("zero channel version")]
    ZeroChannelVersion,
    #[error("missing cumulative ack footer")]
    MissingCumulativeAckFooter,
    #[error("invalid cumulative ack")]
    InvalidCumulativeAck,
    #[error("missing acks count footer")]
    MissingAcksCountFooter,
    #[error("zero acks count")]
    ZeroAcksCount,
    #[error("missing ack footer")]
    MissingAckFooter,
    #[error("invalid ack")]
    InvalidAck,
    #[error("missing sequence number footer")]
    MissingSequenceNumFooter,
    #[error("invalid sequence number")]
    InvalidSequenceNum,
    #[error("missing last reliable sequence number footer")]
    MissingLastReliableSequenceNumFooter,
    #[error("invalid last reliable sequence number")]
    InvalidLastReliableSequenceNum,
    #[error("missing first request offset footer")]
    MissingFirstRequestOffsetFooter,
    #[error("invalid first request offset")]
    InvalidFirstRequestOffset,
    #[error("missing sequence range footer")]
    MissingSequenceRangeFooter,
    #[error("invalid sequence range first")]
    InvalidSequenceRangeFirst,
    #[error("invalid sequence range first")]
    InvalidSequenceRangeLast,
    #[error("invalid sequence range")]
    InvalidSequenceRange,
    #[error("invalid checksum")]
    InvalidChecksum
}
