//! A special proxy application that can be used to generically forward calls it receives
//! to another application while providing debugging capabilities to inspect the network
//! without being blocking by blowfish cipher.

pub mod capture;

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::io::{self, BufReader, BufWriter};
use std::time::{Duration, Instant};
use std::num::NonZero;
use std::sync::Arc;
use std::fs::File;

use tracing::{error, trace, trace_span, warn};
use blowfish::Blowfish;

use crate::net::socket::{PacketSocket, decrypt_packet, encrypt_packet};
use crate::net::proto::{Channel, ChannelIndex, Protocol};
use crate::util::thread::{ThreadPoll, ThreadWorker};
use crate::net::packet::Packet;
use crate::net::bundle::Bundle;

use capture::{CaptureReader, CaptureWriter};


/// The unspecified address used to let the socket allocate its own address.
pub(super) const UNSPECIFIED_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));

/// The receive timeout on socket, used to ensure that we check that the thread can 
/// continue running.
pub(super) const RECV_TIMEOUT: Duration = Duration::from_secs(5);


/// The generic proxy application.
///
/// Polling it gives the events of the packets it forwards. A packet is held while its
/// events are being polled, and forwarded once they all are, at the next poll: in the
/// meantime [`Self::peer`] gives access to its peer, to change how it is forwarded.
///
/// Every packet it forwards can be recorded to a compressed capture (see
/// [`Self::set_capture`]), which [`Self::replay`] later gives the same events of.
#[derive(Debug)]
pub struct App {
    /// Inner state.
    inner: AppInner,
    /// Each peer connected and forwarded..
    peers: HashMap<SocketAddr, PeerInner>,
    /// Where every received packet is recorded, if enabled.
    capture: Option<CaptureWriter<BufWriter<File>>>,
    /// Where the packets come from.
    source: Source,
    /// Events waiting to be polled.
    events: VecDeque<Event>,
    /// The packet whose events are being polled.
    held: Option<Held>,
    /// The packet of an unknown peer, see [`Event::UnknownPeer`].
    unknown: Option<(Packet, SocketAddr)>,
}

/// Inner application structure to split from peer.
#[derive(Debug)]
struct AppInner {
    /// The main socket receiving peer packets, none when replaying a capture.
    socket: Option<PacketSocket>,
    /// Channel tracker for out packets.
    out_protocol: Protocol,
    /// Channel tracker for in packets.
    in_protocol: Protocol,
}

/// Where the packets of the app come from.
#[derive(Debug)]
enum Source {
    /// The sockets, of the app and of each peer.
    Live(ThreadPoll<SocketPollRet>),
    /// A capture being replayed.
    Capture(CaptureReader<BufReader<File>>),
    /// A capture replayed to its end.
    Ended,
}

/// A registered peer that can forward and receive packets from the real application.
#[derive(Debug)]
struct PeerInner {
    /// Handle for drop-destruction of the poll thread worker, only used for drop.
    _socket_worker: Option<ThreadWorker>,
    /// The socket represent this peer for the real application, none when replaying.
    socket: Option<PacketSocket>,
    /// The address to send packets to the peer when receiving from real application.
    addr: SocketAddr,
    /// Real address of the base server to communicate with.
    real_addr: SocketAddr,
    /// Encryption key for this peer.
    blowfish: Option<Arc<Blowfish>>,
}

/// Type of return value for our socket poll. 
#[derive(Debug)]
struct SocketPollRet {
    /// The raw I/O result containing the packet if successful.
    res: io::Result<(Packet, SocketAddr)>,
    /// The peer address if this is the result of a peer socket.
    peer: Option<SocketAddr>,
}

/// A packet held while its events are being polled, and what to forward of it.
#[derive(Debug)]
struct Held {
    /// The peer it's from or to.
    addr: SocketAddr,
    direction: PacketDirection,
    /// The packet as received, encrypted if the peer is.
    cipher: Packet,
    /// The decrypted packet, for [`Peer::patch_raw`] to read from.
    plain: Packet,
    /// Forward the packet as received, see [`Peer::suppress_forward`].
    forward_raw: bool,
    /// Forward this modified packet instead, see [`Peer::patch_raw`].
    patch: Option<Packet>,
}

impl App {

    /// Create a new proxy application with the given listening address and the address
    /// to proxy to and from.
    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        
        let socket_poll = ThreadPoll::new();

        let socket = PacketSocket::bind(addr)?;
        socket.set_recv_timeout(Some(RECV_TIMEOUT))?;

        let thread_socket = socket.clone();
        socket_poll.spawn(move || Some(SocketPollRet {
            res: thread_socket.recv_without_encryption(),
            peer: None,
        }));

        Ok(Self::with_source(Some(socket), Source::Live(socket_poll)))

    }

    /// Replay a capture recorded by [`Self::set_capture`], offline, giving the same
    /// events in the same order as during the live session, with these differences:
    /// no [`Event::UnknownPeer`], peers are created as recorded; nothing is forwarded,
    /// so [`Peer::suppress_forward`] and [`Peer::patch_raw`] have no effect and
    /// [`Peer::send_bundle`] sends nothing; and packets are already decrypted, so peers
    /// have no blowfish key unless one is set. [`Event::CaptureEnd`] comes last.
    pub fn replay(capture: CaptureReader<BufReader<File>>) -> Self {
        Self::with_source(None, Source::Capture(capture))
    }

    fn with_source(socket: Option<PacketSocket>, source: Source) -> Self {
        Self {
            inner: AppInner {
                socket,
                out_protocol: Protocol::new_forwarding(),
                in_protocol: Protocol::new_forwarding(),
            },
            peers: HashMap::new(),
            capture: None,
            source,
            events: VecDeque::new(),
            held: None,
            unknown: None,
        }
    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.inner.socket.as_ref().expect("live proxy has a socket").addr()
    }

    /// Record every packet received from now on, in both directions, to the given
    /// capture. Packets are recorded decrypted, before their events, so the capture
    /// holds exactly what [`Self::replay`] needs to reproduce this session.
    pub fn set_capture(&mut self, capture: CaptureWriter<BufWriter<File>>) {
        self.capture = Some(capture);
    }

    /// Accept the unknown peer of an [`Event::UnknownPeer`], before the next poll.
    pub fn accept_peer(&mut self, addr: SocketAddr, config: PeerConfig) -> io::Result<()> {

        let Source::Live(socket_poll) = &self.source else {
            return Ok(());
        };

        let socket = PacketSocket::bind(UNSPECIFIED_ADDR)?;
        socket.set_recv_timeout(Some(RECV_TIMEOUT))?;

        let thread_socket = socket.clone();
        let socket_worker = socket_poll.spawn_with_handle(move || Some(SocketPollRet {
            res: thread_socket.recv_without_encryption(),
            peer: Some(addr),
        }));

        self.peers.insert(addr, PeerInner {
            _socket_worker: Some(socket_worker),
            socket: Some(socket),
            addr,
            real_addr: config.real_addr,
            blowfish: config.blowfish,
        });

        Ok(())

    }

    /// The peer at this address, if any, to change how it and the packet held, if
    /// it's its, are forwarded.
    pub fn peer(&mut self, addr: SocketAddr) -> Option<Peer<'_>> {
        let peer = self.peers.get_mut(&addr)?;
        Some(Peer {
            app: &mut self.inner,
            peer,
            held: self.held.as_mut().filter(|held| held.addr == addr),
        })
    }

    /// Poll for the next event of this app, blocking until one comes.
    pub fn poll(&mut self) -> Event {
        self.poll_inner(None)
    }

    /// Same as [`Self::poll`], but returns [`Event::Timeout`] once the deadline passes
    /// with no event, so the caller gets a chance to handle its other events.
    pub fn poll_until(&mut self, deadline: Instant) -> Event {
        self.poll_inner(Some(deadline))
    }

    fn poll_inner(&mut self, deadline: Option<Instant>) -> Event {
        loop {

            if let Some(event) = self.events.pop_front() {
                return event;
            }

            // Every event of the held packet polled.
            if let Some(held) = self.held.take() {
                self.forward(held);
                continue;
            }

            // The unknown peer accepted, or not.
            if let Some((cipher_packet, addr)) = self.unknown.take() {
                if self.peers.contains_key(&addr) {
                    self.receive(cipher_packet, addr, PacketDirection::Out);
                }
                continue;
            }

            match &mut self.source {
                Source::Live(socket_poll) => {

                    let socket_poll_ret = match deadline {
                        Some(deadline) => match socket_poll.poll_until(deadline) {
                            Some(ret) => ret,
                            None => {
                                self.flush_capture();
                                return Event::Timeout;
                            }
                        },
                        None => socket_poll.poll(),
                    };

                    let (cipher_packet, addr) = match socket_poll_ret.res {
                        Ok(ret) => ret,
                        Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {
                            self.flush_capture();
                            continue;
                        }
                        Err(error) => return Event::IoError(IoErrorEvent { error, addr: socket_poll_ret.peer }),
                    };

                    match socket_poll_ret.peer {
                        // The packet has been received from the real application and
                        // should be forwarded to the peer, if still existing.
                        Some(peer_addr) => {
                            if self.peers.contains_key(&peer_addr) {
                                self.receive(cipher_packet, peer_addr, PacketDirection::In);
                            }
                        }
                        // The packet has been received from the peer and should be
                        // forwarded to the real application, once accepted if unknown.
                        None => {
                            if self.peers.contains_key(&addr) {
                                self.receive(cipher_packet, addr, PacketDirection::Out);
                            } else {
                                self.unknown = Some((cipher_packet, addr));
                                return Event::UnknownPeer(UnknownPeerEvent { addr });
                            }
                        }
                    }

                }
                Source::Capture(capture) => {

                    let record = match capture.next() {
                        Some(Ok(record)) => record,
                        Some(Err(error)) => return Event::IoError(IoErrorEvent { error, addr: None }),
                        None => {
                            let truncated = capture.truncated();
                            self.source = Source::Ended;
                            return Event::CaptureEnd(CaptureEndEvent { truncated });
                        }
                    };

                    let peer = self.peers.entry(record.peer_addr).or_insert_with(|| PeerInner {
                        _socket_worker: None,
                        socket: None,
                        addr: record.peer_addr,
                        real_addr: record.real_addr,
                        blowfish: None,
                    });

                    // It may have been changed already, but the record is what was live.
                    peer.real_addr = record.real_addr;

                    let packet = if record.undecryptable { Err(record.packet) } else { Ok(record.packet) };
                    self.handle(record.peer_addr, packet.clone().unwrap_or_else(|p| p), packet, record.direction);

                }
                Source::Ended => {
                    return Event::CaptureEnd(CaptureEndEvent { truncated: false });
                }
            }

        }
    }

    /// A packet received live from or to this peer.
    fn receive(&mut self, cipher_packet: Packet, addr: SocketAddr, direction: PacketDirection) {

        let peer = self.peers.get(&addr).expect("receiving for a known peer");

        // Now decrypt packet if the peer has symmetric encryption, otherwise use as-is.
        // A packet that fails to decrypt is still forwarded as received.
        let packet = match peer.blowfish.as_deref() {
            Some(blowfish) => decrypt_packet(cipher_packet.clone(), blowfish),
            None => Ok(cipher_packet.clone()),
        };

        // A capture that fails to be written is dropped: like decoding, capturing must
        // never get in the way of forwarding.
        if let Some(capture) = &mut self.capture {
            let (packet, undecryptable) = match &packet {
                Ok(packet) => (packet, false),
                Err(packet) => (packet, true),
            };
            if let Err(e) = capture.write(direction, peer.addr, peer.real_addr, packet, undecryptable) {
                error!("Packet capture stopped, failed to write it: {e}");
                self.capture = None;
            }
        }

        self.handle(addr, cipher_packet, packet, direction);

    }

    /// Hold a received packet, giving the events of its bundles: the decrypted packet
    /// goes through the protocol and each bundle it completes is an event, or, if it
    /// could not be decrypted, the packet as received is an event.
    fn handle(&mut self,
        addr: SocketAddr,
        cipher_packet: Packet,
        packet: Result<Packet, Packet>,
        direction: PacketDirection,
    ) {

        let plain = match &packet {
            Ok(packet) => packet.clone(),
            Err(packet) => packet.clone(),
        };

        self.held = Some(Held {
            addr,
            direction,
            cipher: cipher_packet,
            plain,
            forward_raw: true,
            patch: None,
        });

        let packet = match packet {
            Ok(packet) => packet,
            Err(packet) => {
                self.events.push_back(Event::InvalidPacketEncryption(InvalidPacketEncryptionEvent {
                    addr,
                    packet,
                    direction,
                }));
                return;
            }
        };

        let peer = self.peers.get(&addr).expect("handling for a known peer");
        let app = &mut self.inner;

        let (
            accept_protocol,
            accept_protocol_span,
            accept_out_protocol,
            accept_out_protocol_span,
        ) = match direction {
            PacketDirection::Out => (
                &mut app.out_protocol, trace_span!("out"),
                &mut app.in_protocol, trace_span!("in"),
            ),
            PacketDirection::In => (
                &mut app.in_protocol, trace_span!("in"),
                &mut app.out_protocol, trace_span!("out"),
            ),
        };

        {
            let _span = accept_protocol_span.enter();
            trace!(real_addr = %peer.real_addr, "{:width$?}", packet, width = 0);
        }

        let accept_out_ok = {
            let _span = accept_out_protocol_span.enter();
            accept_out_protocol.accept_out(&packet, peer.addr)
        };

        if !accept_out_ok {
            return;
        }

        let _span = accept_protocol_span.enter();

        let (bundles, channel) = match accept_protocol.accept(packet, peer.addr) {
            Ok(mut channel) => {
                let packet_channel = channel.is_on().then(|| PacketChannel {
                    index: channel.index(),
                });
                (channel.pop_bundles(), packet_channel)
            }
            Err(_) => return,
        };

        for bundle in bundles {
            self.events.push_back(Event::Bundle(BundleEvent {
                addr,
                bundle,
                direction,
                channel: channel.clone(),
            }));
        }

    }

    /// Forward the held packet whose events were all polled.
    fn forward(&mut self, held: Held) {

        // Nothing is forwarded when replaying.
        let Some(peer) = self.peers.get(&held.addr) else { return };
        let (Some(socket), Some(peer_socket)) = (&self.inner.socket, &peer.socket) else { return };

        let to_send = match held.patch {
            Some(patched) => match peer.blowfish.as_deref() {
                Some(blowfish) => encrypt_packet(patched, blowfish),
                None => patched,
            },
            None if held.forward_raw => held.cipher,
            None => return,
        };

        let res = match held.direction {
            PacketDirection::Out => peer_socket.send_without_encryption(&to_send, peer.real_addr),
            PacketDirection::In => socket.send_without_encryption(&to_send, peer.addr),
        };

        match res {
            Ok(_len) => {}
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                // When forwarding to a local peer, this means that the local client no
                // longer has a listening socket to send to! Ignore this peer.
                trace!("Dropped peer due to connection reset: {}", held.addr);
                self.peers.remove(&held.addr);
            }
            // A `real_addr` that the OS rejects outright (e.g. port 0, from an address
            // patched in off a desynced upstream element) does not resolve by dropping
            // the peer's socket either, and the peer is unsalvageable: drop just this
            // peer and keep serving everyone else.
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                warn!("Dropped peer due to an invalid destination address ({:?}): {}", match held.direction {
                    PacketDirection::Out => peer.real_addr,
                    PacketDirection::In => peer.addr,
                }, held.addr);
                self.peers.remove(&held.addr);
            }
            Err(error) => {
                self.events.push_back(Event::IoError(IoErrorEvent { error, addr: Some(held.addr) }));
            }
        }

    }

    /// Make the capture's tail readable, since it is otherwise only flushed as packets
    /// arrive: done when nothing is received for a while.
    fn flush_capture(&mut self) {
        if let Some(Err(e)) = self.capture.as_mut().map(CaptureWriter::flush) {
            error!("Packet capture stopped, failed to write it: {e}");
            self.capture = None;
        }
    }

}

/// An event of the proxy app.
#[derive(Debug)]
pub enum Event {
    /// The deadline given to [`App::poll_until`] passed with no event.
    Timeout,
    IoError(IoErrorEvent),
    UnknownPeer(UnknownPeerEvent),
    InvalidPacketEncryption(InvalidPacketEncryptionEvent),
    Bundle(BundleEvent),
    CaptureEnd(CaptureEndEvent),
}

/// Some IO error happened internally and optionally related to a peer.
#[derive(Debug)]
pub struct IoErrorEvent {
    pub error: io::Error,
    pub addr: Option<SocketAddr>,
}

/// A packet came from an unknown peer: it is forwarded if the peer is accepted with
/// [`App::accept_peer`] before the next poll, dropped otherwise.
#[derive(Debug)]
pub struct UnknownPeerEvent {
    pub addr: SocketAddr,
}

/// The given peer has received or sent a packet with invalid encryption, and so it
/// cannot be handled by protocol to be made into a bundle.
#[derive(Debug)]
pub struct InvalidPacketEncryptionEvent {
    pub addr: SocketAddr,
    pub packet: Packet,
    pub direction: PacketDirection,
}

/// A bundle of elements has been transferred to or from the given peer.
#[derive(Debug)]
pub struct BundleEvent {
    pub addr: SocketAddr,
    pub bundle: Bundle,
    pub direction: PacketDirection,
    pub channel: Option<PacketChannel>,
}

/// The capture replayed ended, see [`App::replay`], truncated if it ends on an
/// incomplete record, see [`CaptureReader::truncated`].
#[derive(Debug)]
pub struct CaptureEndEvent {
    pub truncated: bool,
}

/// The configuration of an accepted peer, see [`App::accept_peer`].
#[derive(Debug)]
pub struct PeerConfig {
    /// The real server address to forward the packets to after being received by proxy.
    pub real_addr: SocketAddr, 
    /// If this peer should use symmetric blowfish encryption for its packets, this will
    /// be used to intercept and read the clear packets before building the bundles.
    pub blowfish: Option<Arc<Blowfish>>,
}

/// A reference to an internal peer that allows its owner to enable or disable the
/// encryption, modify the real address and send bundles, and to change how the packet
/// held is forwarded, if it's its.
#[derive(Debug)]
pub struct Peer<'a> {
    app: &'a mut AppInner,
    peer: &'a mut PeerInner,
    /// The packet held, if it's this peer's.
    held: Option<&'a mut Held>,
}

impl<'a> Peer<'a> {

    /// Get the address of this peer.
    #[inline]
    pub fn addr(&self) -> SocketAddr {
        self.peer.addr
    }

    /// Get the real address this peer is currently being forwarded to and from.
    #[inline]
    pub fn real_addr(&self) -> SocketAddr {
        self.peer.real_addr
    }

    /// Set the real address this peer will be forwarded to and from.
    #[inline]
    pub fn set_real_addr(&mut self, real_addr: SocketAddr) {
        self.peer.real_addr = real_addr;
    }

    /// Prevent the raw packet currently being processed from being forwarded verbatim
    /// to its destination once handling of it completes. Use this when substituting a
    /// packet of your own instead (e.g. rewriting an in-flight SwitchBaseApp so the
    /// client stays pointed at the proxy instead of the real address it names).
    #[inline]
    pub fn suppress_forward(&mut self) {
        if let Some(held) = self.held.as_deref_mut() {
            held.forward_raw = false;
        }
    }

    /// Replace the first occurrence of `find` with `replace` (which must be the same
    /// length) within the packet currently being processed, and arrange for this
    /// modified copy — re-encrypted if this peer uses encryption — to be forwarded in
    /// place of the original, keeping the original framing (prefix, flags, sequence
    /// number, channel) completely untouched.
    ///
    /// Useful when a message needs to be forwarded as-is from the real application's
    /// point of view (so it gets acknowledged normally, retransmitted normally, etc.)
    /// but a specific field within it (e.g. an embedded address) needs to be rewritten
    /// before the local peer sees it.
    ///
    /// Returns `false` (leaving any previous patch untouched) if `find` could not be
    /// located in the packet's content, or if no packet of this peer is held.
    pub fn patch_raw(&mut self, find: &[u8], replace: &[u8]) -> bool {

        assert_eq!(find.len(), replace.len(), "patch_raw: find and replace must have the same length");

        let Some(held) = self.held.as_deref_mut() else {
            return false;
        };

        let mut patched = held.plain.clone();
        let slice = patched.slice_mut();

        if let Some(pos) = slice.windows(find.len().max(1)).position(|w| w == find) {
            slice[pos..pos + find.len()].copy_from_slice(replace);
            held.patch = Some(patched);
            true
        } else {
            false
        }

    }

    /// Get the currently configured blowfish encryption key for this peer, none if no
    /// encryption is currently used.
    #[inline]
    pub fn blowfish(&self) -> Option<&Blowfish> {
        self.peer.blowfish.as_deref()
    }

    /// Get the currently configured blowfish encryption key for this peer as a cloned
    /// `Arc`, none if no encryption is currently used. Useful to carry the key forward
    /// for a peer that isn't registered yet (e.g. pre-registering an expected future
    /// reconnection from the same logical session under a new local port/address).
    #[inline]
    pub fn blowfish_arc(&self) -> Option<Arc<Blowfish>> {
        self.peer.blowfish.clone()
    }

    /// Set the blowfish encryption key to be used for this peer, none if encryption
    /// should be disabled.
    #[inline]
    pub fn set_blowfish(&mut self, blowfish: Option<Arc<Blowfish>>) {
        self.peer.blowfish = blowfish;
    }

    /// Get the protocol off-channel.
    pub fn off_channel(&mut self, direction: PacketDirection) -> Channel<'_> {
        match direction {
            PacketDirection::Out => self.app.out_protocol.off_channel(self.peer.addr),
            PacketDirection::In => self.app.in_protocol.off_channel(self.peer.addr),
        }
    }

    /// Get the protocol channel.
    pub fn channel(&mut self, direction: PacketDirection, index: Option<NonZero<u32>>) -> Channel<'_> {
        match direction {
            PacketDirection::Out => self.app.out_protocol.channel(self.peer.addr, index),
            PacketDirection::In => self.app.in_protocol.channel(self.peer.addr, index),
        }
    }

    /// Immediately send a bundle to the given direction.
    ///
    /// Nothing is sent when replaying a capture, and this returns 0.
    pub fn send_bundle(&self, direction: PacketDirection, bundle: &Bundle) -> io::Result<usize> {

        let (socket, addr) = match direction {
            PacketDirection::Out => (&self.peer.socket, self.peer.real_addr),
            PacketDirection::In => (&self.app.socket, self.peer.addr),
        };

        let Some(socket) = socket else {
            return Ok(0);
        };

        if let Some(blowfish) = self.peer.blowfish.as_deref() {
            socket.send_bundle_with_encryption(bundle, addr, blowfish)
        } else {
            socket.send_bundle_without_encryption(bundle, addr)
        }

    }

}

/// Represent the forwarding direction of a packet or bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketDirection {
    /// From the peer to the real application.
    Out,
    /// From the real application to the peer.
    In,
}

/// Represent the optional channel used by a packet.
#[derive(Debug, Clone)]
pub struct PacketChannel {
    /// If the channel is indexed, this represent the index and the version of it.
    pub index: Option<ChannelIndex>,
}
