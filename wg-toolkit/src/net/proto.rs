//! Protocol with reliability, defragmenting of bundles and channel support.

use std::collections::{hash_map, HashMap, VecDeque};
use std::time::{Duration, Instant};
use std::net::SocketAddr;
use std::cmp::Ordering;
use std::num::NonZero;

use tracing::{instrument, trace, trace_span, warn};

use super::packet::{Packet, PacketConfig, PacketLocked, PacketConfigError};
use super::seq::{Seq, SeqAlloc};
use super::bundle::Bundle;


/// The (currently hardcoded) timeout on bundle fragments.
const FRAGMENT_TIMEOUT: Duration = Duration::from_secs(10);


/// A protocol tracker for an interface, providing support for accepting and preparing
/// bundles, with reliability, defragmenting and (off)channel support.
#[derive(Debug)]
pub struct Protocol {
    /// State shared with channel handles.
    shared: ProtocolShared,
    /// For each address we are exchanging with, we store a special off-channel channel.
    off_channels: HashMap<SocketAddr, OffChannel>,
    /// Known channels for each address, with optional channel indexing.
    channels: HashMap<(SocketAddr, Option<NonZero<u32>>), OnChannel>,
}

/// A structure referenced by any channel handle, containing shared states.
#[derive(Debug)]
struct ProtocolShared {
    /// Sequence number allocator used for off-channel communications.
    off_seq_alloc: SeqAlloc,
    /// Represent the last prefix being write to a packet.
    last_accepted_prefix: u32,
    /// The current prefix offset being used for updating all packets' prefixes.
    prefix_offset: u32,
    /// Set during the most recent [`Protocol::accept`] call if it turned out that the
    /// accepted packet — or, importantly, any of its piggybacked packets, which are
    /// how a peer commonly resends an unacknowledged reliable packet without a whole
    /// new standalone datagram — carried a reliable sequence number already seen
    /// before. Piggybacks are otherwise processed transparently inside `accept`, with
    /// no other visible effect on its return value, so a caller that ever chooses to
    /// withhold a specific reliable packet from a third party (instead of forwarding
    /// it verbatim) needs this to notice when that same packet's content resurfaces
    /// bundled inside a later, otherwise unrelated packet, and avoid leaking it that
    /// way too. Read via [`Channel::take_duplicate_reliable_found`] on the channel
    /// handle returned by the `accept` call in question (since by the time this is
    /// interesting to know, that handle is the only thing still borrowed from here).
    duplicate_reliable_found: bool,
}

impl Protocol {

    pub fn new() -> Self {
        Self {
            shared: ProtocolShared {
                off_seq_alloc: SeqAlloc::new(Seq::ZERO + 1),
                last_accepted_prefix: 0,
                prefix_offset: 0,
                duplicate_reliable_found: false,
            },
            off_channels: HashMap::new(),
            channels: HashMap::new(),
        }
    }

    /// Return the off-channel handle, the off-channel isn't a real channel and bundles
    /// prepared with it will be sent out of channel, but this is used as a shared 
    /// interface with in-channel.
    pub fn off_channel(&mut self, addr: SocketAddr) -> Channel<'_> {

        let channel = self.off_channels.entry(addr)
            .or_insert_with(|| OffChannel {
                off: OffChannelData::new(),
            });

        Channel {
            inner: GenericChannel {
                shared: &mut self.shared,
                off: &mut channel.off,
                on: None,
            }
        }

    }

    /// Return a handle to a channel associated with the given address, optionally 
    /// indexed if desired, the channel is created if not already existing with initial
    /// version of 1.
    pub fn channel(&mut self, addr: SocketAddr, index: Option<NonZero<u32>>) -> Channel<'_> {

        let channel = self.channels.entry((addr, index))
            .or_insert_with(|| OnChannel {
                off: OffChannelData::new(),
                on: match index {
                    None => OnChannelData::new_without_index(),
                    Some(index) => OnChannelData::new_with_index(index),
                },
            });

        Channel {
            inner: GenericChannel {
                shared: &mut self.shared,
                off: &mut channel.off,
                on: Some(&mut channel.on),
            }
        }

    }

    /// Reset the prefix offset to zero.
    #[inline]
    pub fn reset_prefix_offset(&mut self) {
        self.shared.prefix_offset = 0;
    }

    /// Return the last accepted prefix from any packet.
    #[inline]
    pub fn last_accepted_prefix(&self) -> u32 {
        self.shared.last_accepted_prefix
    }

    /// Set the current prefix offset used for computing the prefix of prepared packets
    /// from the value of the last accepted packet.
    #[inline]
    pub fn transfer_prefix_offset_from_last_received(&mut self) {
        self.shared.prefix_offset = self.shared.last_accepted_prefix;
        // self.shared.prefix_offset = 0x7A11751F;
    }

    /// Accept a new incoming packet and return the associated channel where the packet
    /// has been accepted. If any error in packet's decoding happens, then it's returned
    /// as an error and this packet, this packet cannot be accepted as-is. Any piggyback
    /// packet is still registered but errors are not forwarded.
    #[instrument(name = "accept", level = "trace", skip(self, packet))]
    #[inline(always)]
    pub fn accept(&mut self, packet: Packet, addr: SocketAddr) -> Result<Channel<'_>, Packet> {
        // Reset here (rather than in `accept_inner`, which recurses for piggybacks) so
        // that the flag reflects the *whole* call, top-level packet and piggybacks alike.
        self.shared.duplicate_reliable_found = false;
        self.accept_inner(packet, addr)
    }

    /// Internal wrapper used to improve tracing of recursive span with piggybacks.
    fn accept_inner(&mut self, packet: Packet, addr: SocketAddr) -> Result<Channel<'_>, Packet> {

        let time = Instant::now();
        let mut packet = match packet.read_config_locked() {
            Ok(packet) => packet,
            Err((error, packet)) => {
                warn!("Failed to read config: {error}");
                return Err(packet);
            }
        };

        // Immediately process any piggyback packet, because they must have been 
        // initially sent way before the current packet we are decoding.
        for piggyback in std::mem::take(packet.piggybacks_mut()) {
            trace!("Processing piggyback packet: {piggyback:?}");
            let _span = trace_span!("pigb").entered();
            // Ignore any error on channel decoding.
            let _ = self.accept_inner(piggyback, addr);
        }

        self.shared.last_accepted_prefix = packet.packet().read_prefix();

        // Start by finding the appropriate channel for this packet regarding the local
        // socket address and channel-related flags on this packet.
        let mut channel;
        if packet.config().on_channel() {

            let on_channel;
            if let Some((index, version)) = packet.config().indexed_channel() {
                
                trace!("Is on-channel: {index} v{version}");
                on_channel = self.channels.entry((addr, Some(index)))
                    .or_insert_with(|| OnChannel {
                        off: OffChannelData::new(),
                        on: OnChannelData::new_with_index_version(index, version),
                    });

                // Unwrap because the channel should have index.
                let current_version = on_channel.on.index.unwrap().version;
                if version < current_version {
                    trace!("Outdated, expected v{current_version}");
                    // TODO: outdated packet
                    return Err(packet.destruct().0);
                }

            } else {
                trace!("Is on-channel: not indexed");
                on_channel = self.channels.entry((addr, None))
                    .or_insert_with(|| OnChannel {
                        off: OffChannelData::new(),
                        on: OnChannelData::new_without_index(),
                    });
            }

            channel = GenericChannel {
                shared: &mut self.shared,
                off: &mut on_channel.off,
                on: Some(&mut on_channel.on),
            };

        } else {

            trace!("Is off-channel");
            let off_channel = self.off_channels.entry(addr)
                .or_insert_with(|| OffChannel { off: OffChannelData::new() });

            channel = GenericChannel {
                shared: &mut self.shared,
                off: &mut off_channel.off,
                on: None,
            };

        }

        // Cumulative ack is not supposed to be used off-channel.
        if let Some(cumulative_ack) = packet.config().cumulative_ack() {

            // Cumulative ack is not supported off-channel.
            if channel.on.is_none() {
                warn!("Cumulative ack is not supported off-channel");
                return Err(packet.destruct().0);
            }

            channel.off.ack_out_reliable_packet_cumulative(cumulative_ack);

        }

        // Immediately handle single acks because this packet may be buffered to be 
        // processed in-order.
        for &ack in packet.config().single_acks() {
            channel.off.ack_out_reliable_packet(ack);
        }

        // Reliable packet must be acknowledged later.
        if packet.config().reliable() {
            
            if packet.config().last_reliable_sequence_num().is_some() {
                warn!("Last reliable sequence is not support with reliable");
                return Err(packet.destruct().0);
            }

            let sequence_num = packet.config().sequence_num();
            channel.off.add_in_reliable_packet(sequence_num);

            // When on-channel with reliable packets, we must track the cumulative ack
            // and buffer any packet that is received out-of-order!
            if let Some(on) = channel.on.as_deref_mut() {
                // Keep this channel's own sequence allocator in sync with numbers we
                // merely observe passing through (as opposed to allocating ourselves),
                // so that if we ever need to inject a packet of our own on this channel
                // (e.g. a proxy substituting a rewritten packet) it continues seamlessly
                // from here instead of colliding with or lagging behind this stream.
                on.seq_alloc.observe(sequence_num);
                if on.add_in_reliable_packet(packet) {
                    channel.shared.duplicate_reliable_found = true;
                }
                while let Some(bundle) = on.pop_in_reliable_bundle() {
                    channel.off.in_bundles.push_back(bundle);
                }
                // Shortcut to
                return Ok(Channel { inner: channel });
            }

        } else if let Some(last_reliable_sequence_num) = packet.config().last_reliable_sequence_num() {

            // In this case we must ensure that current expected sequence is equal to
            // this given sequence number + 1.
            if let Some(on) = channel.on.as_deref_mut() {
                if last_reliable_sequence_num != on.in_reliable_expected_seq - 1 {
                    warn!("Invalid last reliable sequence number, expected: {}, got: {}",
                        on.in_reliable_expected_seq - 1, last_reliable_sequence_num);
                    return Err(packet.destruct().0);
                }
            } else {
                warn!("Last reliable sequence is not supported off-channel");
                return Err(packet.destruct().0);
            }

        }

        // If we land here, it's either because the packet isn't reliable, or if the 
        // packet is reliable but we are not in-channel (the latter seems forbidden by
        // WG source code, but it must be verified). TLDR, the packet don't need to
        // be reordered, so the logic is much simpler: we use off-channel fragments map.
        channel.off.add_in_packet(packet, time);

        Ok(Channel { inner: channel })

    }

    /// Accept an outgoing packet, this should never be used in practice because the
    /// [`Channel::prepare`] method used to prepare complete bundles is already handling
    /// the reliable tracking. However, this function is used for proxies where we never
    /// manually prepare bundles but instead just forward packets, in such case we should
    /// be able to simulate preparation of outgoing packets.
    #[instrument(level = "trace", skip(self, packet))]
    pub fn accept_out(&mut self, packet: &Packet, addr: SocketAddr) -> bool {

        let time = Instant::now();
        let locked = match packet.read_config_locked_ref() {
            Ok(locked) => locked,
            Err(error) => {
                warn!("Failed to read config: {error}");
                return false;
            }
        };

        let channel;
        if locked.config().on_channel() {

            let on_channel;
            if let Some((index, version)) = locked.config().indexed_channel() {
                trace!("Is on-channel: {index} v{version}");
                on_channel = self.channels.entry((addr, Some(index)))
                    .or_insert_with(|| OnChannel {
                        off: OffChannelData::new(),
                        on: OnChannelData::new_with_index_version(index, version),
                    });
            } else {
                trace!("Is on-channel: not indexed");
                on_channel = self.channels.entry((addr, None))
                    .or_insert_with(|| OnChannel {
                        off: OffChannelData::new(),
                        on: OnChannelData::new_without_index(),
                    });
            }

            channel = GenericChannel {
                shared: &mut self.shared,
                off: &mut on_channel.off,
                on: Some(&mut on_channel.on),
            };

        } else {

            trace!("Is off-channel");
            let off_channel = self.off_channels.entry(addr)
                .or_insert_with(|| OffChannel { off: OffChannelData::new() });

            channel = GenericChannel {
                shared: &mut self.shared,
                off: &mut off_channel.off,
                on: None,
            };

        }

        if let Some(cumulative_ack) = locked.config().cumulative_ack() {
            if channel.on.is_some() {
                channel.off.ack_in_reliable_packet_cumulative(cumulative_ack);
            } else {
                warn!("Cumulative ack is not supported off-channel");
                return false;
            }
        }

        for &single_ack in locked.config().single_acks() {
            channel.off.ack_in_reliable_packet(single_ack);
        }

        if locked.config().reliable() {
            channel.off.add_out_reliable_packet_unordered(locked.config().sequence_num(), time);
        }

        true

    }

}

/// A handle to a channel or to the special off-channel fake channel.
/// 
/// Represent a channel between the app and a client with specific socket address. A 
/// channel is a way to create multiple communication channels between the same pair
/// of addresses and also provides support for reliable communication with sequence
/// number acks and resend logic (not yet implemented).
#[derive(Debug)]
pub struct Channel<'a> {
    inner: GenericChannel<'a>,
}

impl Channel<'_> {

    /// Return true if this generic channel represents an actual on-channel one.
    pub fn is_on(&self) -> bool {
        self.inner.on.is_some()
    }

    /// Return true if this generic channel represents an actual off-channel one.
    pub fn is_off(&self) -> bool {
        !self.is_on()
    }

    /// If this generic channel is an actual on-channel one (see [`Self::is_on`]), and
    /// it has and index (and version) then it's returned.
    pub fn index(&self) -> Option<ChannelIndex> {
        self.inner.on.as_deref().and_then(|on| on.index)
    }

    /// Read and reset the flag set by the [`Protocol::accept`] call that produced this
    /// channel handle, indicating that the accepted packet — or one of its piggybacked
    /// packets — carried a reliable sequence number already seen before. See the field
    /// doc on `ProtocolShared::duplicate_reliable_found` for why this matters: a peer
    /// commonly resends a still-unacknowledged reliable packet by piggybacking it onto
    /// a later, otherwise unrelated packet, rather than as a new standalone datagram,
    /// so a caller that withholds a specific reliable packet from a third party needs
    /// this to notice its content resurfacing that way and avoid leaking it regardless.
    pub fn take_duplicate_reliable_found(&mut self) -> bool {
        std::mem::replace(&mut self.inner.shared.duplicate_reliable_found, false)
    }

    /// Pop the next bundle able to be received, if any, this ensures that bundles are
    /// received in the correct order!
    pub fn next_bundle(&mut self) -> Option<Bundle> {
        self.inner.off.in_bundles.pop_front()
    }

    /// Pop all the next bundles, in order, and without borrowing the channel, returning
    /// an owned iterator. In general, it's better to use the [`Self::next_bundle`] 
    /// function when borrowing is not an issue.
    /// 
    /// This function does not allocate if there is only one (or zero) bundle!
    pub fn pop_bundles(&mut self) -> impl Iterator<Item = Bundle> + use<> {

        // Our goal is to avoid allocation when we only have one bundle, so we use a vec
        // only when necessary and return the chained option + vec.
        let bundle = self.inner.off.in_bundles.pop_front();
        // Collecting no bundle should not allocate any vector's memory.
        let rest = self.inner.off.in_bundles.drain(..).collect::<Vec<_>>();

        bundle.into_iter().chain(rest)

    }

    /// Prepare a bundle to be sent, adding acks and other configuration required by this
    /// tracker into all packets. After this function, all packets are ready to be sent
    /// and the bundle should not be touched for this to remain true.
    /// 
    /// NOTE: FIXME: It's said that external interfaces don't allow off-channel reliable 
    /// communication (see packet_receiver.cpp, line 977).
    #[instrument(level = "trace", skip(self, bundle))]
    pub fn prepare(&mut self, bundle: &mut Bundle, reliable: bool) {

        let time = Instant::now();
        let bundle_len = bundle.len() as u32;
        trace!("Count: {bundle_len}");
        
        // Create a common packet config for all the bundle.
        let mut packet_config = PacketConfig::new();

        packet_config.set_reliable(reliable);

        if bundle_len > 1 || reliable {
            let sequence_num = self.inner.alloc_sequence_num(bundle_len, reliable);
            trace!("Allocated sequence numbers: {}..{}", sequence_num, sequence_num + bundle_len);
            packet_config.set_sequence_num(sequence_num);
            if reliable {
                for i in 0..bundle_len {
                    self.inner.off.add_out_reliable_packet(sequence_num + i, time);
                }
            }
        }
        
        if let Some(on) = self.inner.on.as_deref_mut() {
            packet_config.set_on_channel(true);
            packet_config.set_cumulative_ack(on.in_reliable_expected_seq);
            if let Some(index) = on.index {
                packet_config.set_indexed_channel(index.index, index.version);
                trace!("Is on-channel: {} v{}", index.index, index.version);
            } else {
                trace!("Is on-channel: not indexed");
            }
        } else {
            trace!("Is off-channel");
        }

        if !self.inner.off.in_reliable_packets.is_empty() {
            trace!("Pending single acks: {:?}", self.inner.off.in_reliable_packets);
        }
        
        trace!("Using prefix offset: 0x{:08X}", self.inner.shared.prefix_offset);
        
        // This swap is simple: it places the dequeue of all received reliable packets 
        // and their sequence numbers into the packet config's acks queue. We must 
        // remember after this to transfer back the remaining sequence numbers that
        // have not been sent from the packet config.
        std::mem::swap(&mut self.inner.off.in_reliable_packets, packet_config.single_acks_mut());
        debug_assert!(self.inner.off.in_reliable_packets.is_empty(), "packet config acks were not empty");

        bundle.write_config(&mut packet_config);
        bundle.update_prefix(self.inner.shared.prefix_offset);

        // Now we need to restore acks that have not been sent: swap back (read above).
        std::mem::swap(&mut self.inner.off.in_reliable_packets, packet_config.single_acks_mut());
        debug_assert!(packet_config.single_acks().is_empty(), "packet config acks should be empty");

        if !self.inner.off.in_reliable_packets.is_empty() {
            trace!("Remaining single acks: {:?}", self.inner.off.in_reliable_packets)
        }

    }

}

/// Documented from BigWorld source code (programming\bigworld\lib\network\udp_channel.hpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelIndex {
    /// An indexed channel is basically a way of multiplexing multiple
	/// channels between a pair of addresses.  Regular channels distinguish
	/// traffic solely on the basis of address, so in situations where you need
	/// multiple channels between a pair of addresses (i.e. channels between
	/// base and cell entities) you use indexed channels to keep the streams
	/// separate.
    pub index: NonZero<u32>,
	/// Indexed channels have a 'version' number which basically tracks how
	/// many times they have been offloaded.  This allows us to correctly
	/// determine which incoming packets are out-of-date and also helps
	/// identify the most up-to-date information about lost entities in a
	/// restore situation.
    pub version: NonZero<u32>,
}

///  Kind of error that caused a packet to be rejected from this socket and not received.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PacketRejectionError {
    /// The packet is part of a sequence but no other packets of the sequence have been
    /// found and therefore no bundle can be reconstructed.
    #[error("timed out")]
    TimedOut,
    /// The packet could not be synchronized from its data.
    #[error("config error: {0}")]
    Config(#[from] PacketConfigError),
}

/// Common data for off-channel and on-channel communication.
#[derive(Debug)]
struct OffChannelData {
    /// All sequences marked as reliable, so that we can ensure that they are received.
    /// It should be naturally sorted (debug_asserted).
    out_reliable_packets: Vec<OutReliablePacket>,
    /// A dequeue containing all received reliable packets for which we should send ack.
    /// It doesn't need to be sorted.
    in_reliable_packets: VecDeque<Seq>,
    /// Bundle fragments tracking, mapped to the first sequence.
    in_fragments: HashMap<Seq, Fragments>,
    /// Buffered bundles that can be retrieved by the client!
    in_bundles: VecDeque<Bundle>,
}

/// A reliable packet that we sent at given time and waiting for an acknowledgment.
#[derive(Debug)]
struct OutReliablePacket {
    /// The sequence number.
    sequence_num: Seq,
    /// The time this sequence has been sent.
    time: Instant,
}

impl OffChannelData {

    fn new() -> Self {
        Self {
            out_reliable_packets: Vec::new(),
            in_reliable_packets: VecDeque::new(),
            in_fragments: HashMap::new(),
            in_bundles: VecDeque::new(),
        }
    }

    /// TODO: We'll also need to automatically resend the packet's content after some 
    /// time.
    fn add_out_reliable_packet(&mut self, sequence_num: Seq, time: Instant) {
    
        // We are keeping reliable packets ordered by their sequence number and also by
        // their time (Instant::now() can only grow).
        debug_assert!(
            self.out_reliable_packets.is_empty() || 
            self.out_reliable_packets.last().unwrap().sequence_num.wrapping_cmp(sequence_num).is_le(),
            "reliable packet sequence number should be greater than previous ones");
        
        trace!("Add reliable packet: {sequence_num}");
        self.out_reliable_packets.push(OutReliablePacket {
            sequence_num,
            time,
        });

    }

    /// Same as [`Self::add_reliable_packet()`] but accepting unordered sequence number,
    /// this is used by [`PacketTracker::accept_out`] and proxy. *The insertion is still
    /// more performant when inserting a sequence number that is almost the largest in
    /// the set.*
    fn add_out_reliable_packet_unordered(&mut self, sequence_num: Seq, time: Instant) {

        trace!("Add reliable packet (unordered): {sequence_num}");
        
        let mut insert_index = 0;
        for (i, packet) in self.out_reliable_packets.iter().enumerate().rev() {
            match sequence_num.wrapping_cmp(packet.sequence_num) {
                Ordering::Equal => return,  // Ignore duplicate.
                Ordering::Less => continue,
                Ordering::Greater => {
                    insert_index = i + 1;
                    break;
                }
            }
        }
        
        self.out_reliable_packets.insert(insert_index, OutReliablePacket {
            sequence_num,
            time,
        });

    }

    /// When a single ack is received on a packet, this can be called to 
    fn ack_out_reliable_packet(&mut self, sequence_num: Seq) {

        // Naive search for now, because Seq is complicated!
        let index = self.out_reliable_packets.iter()
            .position(|packet| sequence_num == packet.sequence_num);

        if let Some(index) = index {
            let reliable_packet = self.out_reliable_packets.remove(index);
            trace!("Single ack for reliable packet: {sequence_num} after {:?}", reliable_packet.time.elapsed());
        }

    }

    /// When a cumulative ack is received, this can be used to acknowledge all sequences
    /// up to, but excluding, the given sequence number. Not supported off-channel but
    /// still present here.
    fn ack_out_reliable_packet_cumulative(&mut self, sequence_num: Seq) {

        // Naive search for now, because Seq is complicated!
        // Using '.le' because it's cumulative ack is exclusive!
        let drain_len = self.out_reliable_packets.iter()
            .position(|packet| sequence_num.wrapping_cmp(packet.sequence_num).is_le())
            .unwrap_or(self.out_reliable_packets.len());

        trace!("Cumulative ack for reliable packets: ..{sequence_num}");
        for reliable_packet in self.out_reliable_packets.drain(..drain_len) {
            trace!("Cumulative ack for a previous packet: {}, after: {:?}", 
                reliable_packet.sequence_num, reliable_packet.time.elapsed());
        }

    }

    /// Register a simple reliable packet to be acknowledged in the future.
    fn add_in_reliable_packet(&mut self, sequence_num: Seq) {
        // It's unsorted so we don't care of duplicate entries!
        self.in_reliable_packets.push_back(sequence_num);
        trace!("Received reliable packet: {sequence_num}");
    }

    /// Force acknowledgement of a received reliable packet, used with `accept_out` only.
    fn ack_in_reliable_packet(&mut self, sequence_num: Seq) {
        // Swap remove because order don't matter.
        if let Some(pos) = self.in_reliable_packets.iter().position(|&num| num == sequence_num) {
            self.in_reliable_packets.swap_remove_back(pos);
        }
    }
    
    /// For proxy and accept_out...
    fn ack_in_reliable_packet_cumulative(&mut self, sequence_num: Seq) {
        // Same logic as above in 'pop_received_reliable_packet_cumulative'.
        self.in_reliable_packets.retain(|&num| num.wrapping_cmp(sequence_num).is_ge());
    }

    /// Push a packet that may be a bundle's fragment, if a bundle is completed, it is 
    /// added to the internal bundles queue, there is no ordering guaranteed with such
    /// packet, see [`OnChannelData::add_in_reliable_packet`] for reordering.
    fn add_in_packet(&mut self, packet: PacketLocked, time: Instant) {

        let bundle = match packet.config().sequence_range() {
            Some((first_seq, last_seq)) => {

                let relative_num = packet.config().sequence_num() - first_seq;
                trace!("Fragment: {} ({}..={})", 
                    packet.config().sequence_num(), first_seq.get(), last_seq.get());

                match self.in_fragments.entry(first_seq) {
                    hash_map::Entry::Occupied(mut o) => {

                        // If this fragments is too old, timeout every packet in it
                        // and start again with the packet.
                        // FIXME: Maybe dumb?
                        if o.get().is_old(time, FRAGMENT_TIMEOUT) {
                            // let mut fragments = o.remove();
                            // self.rejected_packets.extend(fragments.drain()
                            //     .map(|packet| (addr, packet, PacketRejectionError::TimedOut)));
                            return;
                        }

                        o.get_mut().set(relative_num, packet);

                        // When all fragments are collected, remove entry and return.
                        if !o.get().is_full() {
                            return;
                        }

                        o.remove().into_bundle()

                    },
                    hash_map::Entry::Vacant(v) => {
                        let mut fragments = Fragments::new(last_seq - first_seq + 1);
                        fragments.set(relative_num, packet);
                        v.insert(fragments);
                        return;
                    }
                }

            }
            _ => Bundle::new_with_single(packet)
        };

        self.in_bundles.push_back(bundle);

    }

}

/// Data specific to on-channel communication.
#[derive(Debug)]
struct OnChannelData {
    /// Optional index for this channel.
    index: Option<ChannelIndex>,
    /// The next sequence number to return for this channel, **only for reliable 
    /// packets**, non-reliable packets sent on-channel are using the off-channel alloc.
    seq_alloc: SeqAlloc,
    /// Most of the time, reliable sequence numbers are received in order, and so we can
    /// just increment this counter in order to know that all packets up to (but 
    /// excluding) this number has been received.
    /// 
    /// NOTE: This is the same as `inSeqAt_` in BW source.
    in_reliable_expected_seq: Seq,
    in_reliable_contiguous_packets: VecDeque<PacketLocked>,
    in_reliable_packets: VecDeque<PacketLocked>,
}

impl OnChannelData {

    fn new(index: Option<ChannelIndex>) -> Self {
        Self {
            index,
            seq_alloc: SeqAlloc::new(Seq::ZERO),
            in_reliable_expected_seq: Seq::ZERO,
            in_reliable_contiguous_packets: VecDeque::new(),
            in_reliable_packets: VecDeque::new(),
        }
    }

    fn new_without_index() -> Self {
        Self::new(None)
    }

    fn new_with_index_version(index: NonZero<u32>, version: NonZero<u32>) -> Self {
        Self::new(Some(ChannelIndex { index, version }))
    }

    fn new_with_index(index: NonZero<u32>) -> Self {
        Self::new_with_index_version(index, NonZero::new(1).unwrap())
    }

    /// Add a received (in) reliable packet to the internal re-ordering logic of this
    /// channel, this will automatically construct an ordered bundle when completed.
    /// This also updates the cumulative ack that can be sent back.
    ///
    /// After this function has filled contiguous and buffered packets, you may want to
    /// user [`Self::pop_in_reliable_bundle`] to pop any completed contiguous bundle.
    ///
    /// Returns `true` if this sequence number had already been fully processed or was
    /// already buffered — i.e. this call carried no new information. This commonly
    /// happens when a peer piggybacks a still-unacknowledged reliable packet onto a
    /// later, unrelated one: from here it looks identical to a plain retransmission.
    fn add_in_reliable_packet(&mut self, packet: PacketLocked) -> bool {

        debug_assert!(packet.config().reliable(), "given packet should be reliable");

        let sequence_num = packet.config().sequence_num();

        match sequence_num.wrapping_cmp(self.in_reliable_expected_seq) {
            Ordering::Equal => {

                // This is the best scenario, packet is received in-order, so we push the
                // packet after the currently contiguous sequence.
                self.in_reliable_expected_seq += 1;
                self.in_reliable_contiguous_packets.push_back(packet);

                // By inserting this packet, we may a filled a gap in the sequences, so
                // we increment it while it's possible.
                while let Some(packet) = self.in_reliable_packets.front() {
                    if packet.config().sequence_num() == self.in_reliable_expected_seq {
                        trace!("Unbuffered reliable packet: {}", packet.config().sequence_num());
                        self.in_reliable_expected_seq += 1;
                        self.in_reliable_contiguous_packets.push_back(self.in_reliable_packets.pop_front().unwrap());
                    } else {
                        break;  // Not contiguous.
                    }
                }

            }
            Ordering::Less => {
                // Do nothing, the sequence number may have been already received...
                return true;
            }
            Ordering::Greater => {

                // Warning if we get many buffered packets which indicate that we probably
                // lost track of one of the
                if self.in_reliable_packets.len() > 50 {
                    warn!("Buffered too many in reliable packets: {}", self.in_reliable_packets.len());
                }

                // We search where we can insert the packet, starting from the end because
                // it's still likely to receive packets in order.
                let mut insert_index = 0;
                for (i, buffered_packet) in self.in_reliable_packets.iter().enumerate().rev() {
                    match sequence_num.wrapping_cmp(buffered_packet.config().sequence_num()) {
                        Ordering::Equal => return true,  // Duplicate packet, just abort.
                        Ordering::Less => continue,
                        Ordering::Greater => {
                            insert_index = i + 1;
                            break;
                        }
                    }
                }

                self.in_reliable_packets.insert(insert_index, packet);
                trace!("Buffered reliable packet at: {insert_index}");

                // let debug_seqs = self.in_reliable_packets.iter()
                //     .map(|packet| packet.config().sequence_num().get())
                //     .collect::<Vec<_>>();
                // trace!("Buffered packets: {debug_seqs:?}");

            }
        }

        trace!("Received reliable packet cumulative: {}, contiguous: {}, buffered: {} (first: {:?})",
            self.in_reliable_expected_seq,
            self.in_reliable_contiguous_packets.len(),
            self.in_reliable_packets.len(),
            self.in_reliable_packets.front().map(|packet| packet.config().sequence_num().get()));

        false

    }

    /// Try to construct any reliable bundle if possible.
    fn pop_in_reliable_bundle(&mut self) -> Option<Bundle> {
        loop {
            let first_packet = self.in_reliable_contiguous_packets.front()?;
            if let Some((first_seq, last_seq)) = first_packet.config().sequence_range() {

                trace!("Reliable fragment: {}..={}", first_seq.get(), last_seq.get());

                // Checking coherency of the sequence range, the first contiguous packet 
                // should also start the range, if not the case we just pop that invalid
                // packet.
                if first_packet.config().sequence_num() != first_seq {
                    warn!("Missing the first fragment packet, got: {}, range: {}..={}",
                        first_packet.config().sequence_num(), first_seq.get(), last_seq.get());
                    // Forget this invalid packet and continue to the next packet.
                    let _ = self.in_reliable_contiguous_packets.pop_front();
                    continue;
                }

                // Because we ensure that packets are contiguous with strictly increasing 
                // seq, we just check that we have enough packets to complete this bundle,
                // this guarantee that we have all first_seq..=last_seq existing in it.
                // TODO: Add a anti-DOS check here to ensure that bundle length isn't absurd.
                let bundle_len = (last_seq - first_seq + 1) as usize;
                if self.in_reliable_contiguous_packets.len() < bundle_len {
                    trace!("Reliable fragment: not enough contiguous, expected: {}, got: {}",
                        bundle_len, self.in_reliable_contiguous_packets.len());
                    return None;
                }

                // Enough packets, check that all packets have the same sequence range.
                let mut coherent = true;
                for second_packet in self.in_reliable_contiguous_packets.iter().take(bundle_len).skip(1) {
                    if second_packet.config().sequence_range() != Some((first_seq, last_seq)) {
                        let (first_seq_, last_seq_) = second_packet.config().sequence_range().unwrap();
                        warn!("Incoherent fragment packet, got {}..={}, expected {}..={}",
                            first_seq_.get(), last_seq_.get(), first_seq.get(), last_seq.get());
                        coherent = false;
                        break;
                    }
                }

                // We drain all elements anyway, but only returns if all packets are valid.
                let drain = self.in_reliable_contiguous_packets.drain(..bundle_len);
                if coherent {
                    return Some(Bundle::new_with_multiple(drain))
                } else {
                    continue;
                }

            } else {
                // Unwrap because we know that this packet exists.
                let single_packet = self.in_reliable_contiguous_packets.pop_front().unwrap();
                return Some(Bundle::new_with_single(single_packet));
            }
        }
    }

}

#[derive(Debug)]
#[repr(C)]
struct OffChannel {
    off: OffChannelData,
}

#[derive(Debug)]
#[repr(C)]
struct OnChannel {
    off: OffChannelData,
    on: OnChannelData,
}

/// Internal structure used to reference a channel like a handle to it, providing an
/// internal common interface between both.
#[derive(Debug)]
struct GenericChannel<'a> {
    shared: &'a mut ProtocolShared,
    off: &'a mut OffChannelData,
    on: Option<&'a mut OnChannelData>,
}

impl GenericChannel<'_> {

    /// Generic sequence number allocation depending on the context of the channel.
    fn alloc_sequence_num(&mut self, count: u32, reliable: bool) -> Seq {
        if let Some(on) = self.on.as_deref_mut() {
            if reliable {
                return on.seq_alloc.alloc(count);
            }
        }
        self.shared.off_seq_alloc.alloc(count)
    }

}

/// Internal structure to keep fragments from a given sequence.
#[derive(Debug)]
struct Fragments {
    fragments: Vec<Option<PacketLocked>>,
    seq_count: u32,
    last_update: Instant,
}

impl Fragments {

    /// Create from sequence length.
    fn new(seq_len: u32) -> Self {
        Self {
            fragments: (0..seq_len).map(|_| None).collect(),
            seq_count: 0,
            last_update: Instant::now()
        }
    }

    /// Set a fragment.
    fn set(&mut self, num: u32, packet: PacketLocked) {
        let frag = &mut self.fragments[num as usize];
        if frag.is_none() {
            self.seq_count += 1;
        }
        self.last_update = Instant::now();
        *frag = Some(packet);
    }

    #[inline]
    fn is_old(&self, time: Instant, timeout: Duration) -> bool {
        time - self.last_update > timeout
    }

    #[inline]
    fn is_full(&self) -> bool {
        self.seq_count as usize == self.fragments.len()
    }

    /// Convert this structure to a bundle, **safe to call only if `is_full() == true`**.
    #[inline]
    fn into_bundle(self) -> Bundle {
        assert!(self.is_full());
        self.fragments.into_iter()
            .map(|o| o.unwrap())
            .collect()
    }

}
