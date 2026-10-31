//! A special proxy application that can be used to generically forward calls it receives
//! to another application while providing debugging capabilities to inspect the network
//! without being blocking by blowfish cipher.

pub mod capture;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::collections::{hash_map, HashMap};
use std::io::{self, BufWriter};
use std::time::Duration;
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
/// Every packet it forwards can be recorded to a compressed capture (see
/// [`Self::set_capture`]), which [`replay`] later feeds to the same [`Handler`] offline.
#[derive(Debug)]
pub struct App {
    /// Inner state.
    inner: AppInner,
    /// Each peer connected and forwarded..
    peers: HashMap<SocketAddr, PeerInner>,
    /// Where every received packet is recorded, if enabled.
    capture: Option<CaptureWriter<BufWriter<File>>>,
}

/// Inner application structure to split from peer.
#[derive(Debug)]
struct AppInner {
    /// Thread poll for socket result.
    socket_poll: ThreadPoll<SocketPollRet>,
    /// The main socket receiving peer packets, none when replaying a capture.
    socket: Option<PacketSocket>,
    /// Channel tracker for out packets.
    out_protocol: Protocol,
    /// Channel tracker for in packets.
    in_protocol: Protocol,
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

/// What the handler decided to do with the packet it was given.
#[derive(Debug)]
struct Outcome {
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

        Ok(Self {
            inner: AppInner {
                socket_poll,
                socket: Some(socket),
                out_protocol: Protocol::new_forwarding(),
                in_protocol: Protocol::new_forwarding(),
            },
            peers: HashMap::new(),
            capture: None,
        })

    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.inner.socket.as_ref().expect("live proxy has a socket").addr()
    }

    /// Record every packet received from now on, in both directions, to the given
    /// capture. Packets are recorded decrypted, before the handler sees them, so the
    /// capture holds exactly what [`replay`] needs to reproduce this session.
    pub fn set_capture(&mut self, capture: CaptureWriter<BufWriter<File>>) {
        self.capture = Some(capture);
    }

    /// Blocking poll of this application with the given handler.
    pub fn poll<H: Handler>(&mut self, handler: &mut H) -> Result<(), H::Error> {

        let socket_poll_ret = self.inner.socket_poll.poll();
        let (cipher_packet, addr) = match socket_poll_ret.res {
            Ok(ret) => ret,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {
                // Nothing received for a while: a good time to make the capture's tail
                // readable, since it is otherwise only flushed as packets arrive.
                if let Some(Err(e)) = self.capture.as_mut().map(CaptureWriter::flush) {
                    error!("Packet capture stopped, failed to write it: {e}");
                    self.capture = None;
                }
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };

        let peer;
        let direction;

        if let Some(peer_addr) = &socket_poll_ret.peer {

            // The packet has been received from the real application and should be
            // forwarded to the peer.
            peer = match self.peers.get_mut(peer_addr) {
                Some(peer) => peer,
                None => return Ok(()),  // Receiving event from a no longer existing peer.
            };
            direction = PacketDirection::In;

        } else {

            // The packet has been received from the peer and should be forwarded to the
            // real login application.
            peer = match self.peers.entry(addr) {
                hash_map::Entry::Occupied(o) => o.into_mut(),
                hash_map::Entry::Vacant(v) => {

                    // If we have no peer for this, we ask the handler the initial
                    // configuration for this peer. If the handle don't accept the peer
                    // then we just return and ignore the packet.
                    let Some(peer_config) = handler.accept_peer(addr)? else {
                        return Ok(());
                    };

                    let socket = PacketSocket::bind(UNSPECIFIED_ADDR)?;
                    socket.set_recv_timeout(Some(RECV_TIMEOUT))?;

                    let thread_socket = socket.clone();
                    let socket_worker = self.inner.socket_poll.spawn_with_handle(move || Some(SocketPollRet {
                        res: thread_socket.recv_without_encryption(),
                        peer: Some(addr),
                    }));

                    v.insert(PeerInner {
                        _socket_worker: Some(socket_worker),
                        socket: Some(socket),
                        addr,
                        real_addr: peer_config.real_addr,
                        blowfish: peer_config.blowfish,
                    })

                }
            };
            direction = PacketDirection::Out;

        }

        // Now decrypt packet if the peer has symmetric encryption, otherwise use as-is.
        // A packet that fails to decrypt is still forwarded as received below.
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

        let outcome = handle_packet(&mut self.inner, peer, handler, packet, direction)?;

        let to_send = match outcome.patch {
            Some(patched) => match peer.blowfish.as_deref() {
                Some(blowfish) => encrypt_packet(patched, blowfish),
                None => patched,
            },
            None if outcome.forward_raw => cipher_packet,
            None => return Ok(()),
        };

        let res = match direction {
            PacketDirection::Out => peer.socket.as_ref().expect("live peer has a socket")
                .send_without_encryption(&to_send, peer.real_addr),
            PacketDirection::In => self.inner.socket.as_ref().expect("live proxy has a socket")
                .send_without_encryption(&to_send, peer.addr),
        };

        match res {
            Ok(_len) => {}
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                // When forwarding to a local peer, this means that the local client no
                // longer has a listening socket to send to! Ignore this peer.
                trace!("Dropped peer due to connection reset: {addr}");
                // Unwrap because we have this peer and it exists.
                self.peers.remove(&addr).unwrap();
            }
            // A handler-supplied `real_addr` that the OS rejects outright (e.g. port
            // 0, from an address patched in off a desynced upstream element) must not
            // propagate. Unlike `ConnectionReset` it does not resolve by dropping the
            // peer's socket, and the peer is unsalvageable either way, but letting it
            // bubble out of `poll` ends `run`'s loop for every other connected peer
            // too. Drop just this peer and keep serving everyone else.
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                warn!("Dropped peer due to an invalid destination address ({:?}): {addr}", match direction {
                    PacketDirection::Out => peer.real_addr,
                    PacketDirection::In => peer.addr,
                });
                self.peers.remove(&addr).unwrap();
            }
            Err(e) => {
                return Err(e.into());
            }
        }

        Ok(())

    }

    /// Same as [`Self::loop`] but indefinitely looping until an error is returned.
    pub fn run<H: Handler>(&mut self, mut handler: H) -> Result<(), H::Error> {
        loop {
            self.poll(&mut handler)?;
        }
    }

}

/// Feed a capture recorded by [`App::set_capture`] to the given handler, offline, as
/// if the proxy were receiving it again.
///
/// The handler sees the same calls in the same order as during the live session, with
/// these differences: [`Handler::accept_peer`] is not called, peers are created as
/// recorded; nothing is forwarded, so [`Peer::suppress_forward`] and
/// [`Peer::patch_raw`] have no effect and [`Peer::send_bundle`] sends nothing; and
/// packets are already decrypted, so peers have no blowfish key unless the handler
/// sets one.
///
/// Returns whether the capture was truncated, see [`CaptureReader::truncated`].
pub fn replay<H, R>(mut capture: CaptureReader<R>, handler: &mut H) -> Result<bool, H::Error>
where
    H: Handler,
    R: io::Read,
{

    let mut app = AppInner {
        socket_poll: ThreadPoll::new(),
        socket: None,
        out_protocol: Protocol::new_forwarding(),
        in_protocol: Protocol::new_forwarding(),
    };

    let mut peers = HashMap::new();

    for record in &mut capture {

        let record = record?;

        let peer = peers.entry(record.peer_addr).or_insert_with(|| PeerInner {
            _socket_worker: None,
            socket: None,
            addr: record.peer_addr,
            real_addr: record.real_addr,
            blowfish: None,
        });

        // The handler may have changed it already, but the record is what was live.
        peer.real_addr = record.real_addr;

        let packet = if record.undecryptable { Err(record.packet) } else { Ok(record.packet) };
        handle_packet(&mut app, peer, handler, packet, record.direction)?;

    }

    Ok(capture.truncated())

}

/// Give one received packet to the handler: the decrypted packet goes through the
/// protocol and each bundle it completes is handed over, or, if it could not be
/// decrypted, the packet as received is reported as such.
fn handle_packet<H: Handler>(
    app: &mut AppInner,
    peer: &mut PeerInner,
    handler: &mut H,
    packet: Result<Packet, Packet>,
    direction: PacketDirection,
) -> Result<Outcome, H::Error> {

    let mut outcome = Outcome { forward_raw: true, patch: None };

    let packet = match packet {
        Ok(packet) => packet,
        Err(cipher_packet) => {
            let peer_ref = Peer {
                app,
                peer,
                forward_raw: &mut outcome.forward_raw,
                plain: &cipher_packet,
                patch: &mut outcome.patch,
            };
            handler.receive_invalid_packet_encryption(peer_ref, cipher_packet.clone(), direction)?;
            return Ok(outcome);
        }
    };

    // Keep a copy of the decrypted content around for `Peer::patch_raw` to read
    // from, since `packet` itself is about to be consumed by `accept` below.
    let packet_plain = packet.clone();

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
        return Ok(outcome);
    }

    let _span = accept_protocol_span.enter();

    let (bundles, packet_channel) = match accept_protocol.accept(packet, peer.addr) {
        Ok(mut channel) => {
            let packet_channel = channel.is_on().then(|| PacketChannel {
                index: channel.index(),
            });
            (channel.pop_bundles(), packet_channel)
        }
        Err(_) => return Ok(outcome),
    };

    for bundle in bundles {
        let peer_ref = Peer {
            app: &mut *app,
            peer: &mut *peer,
            forward_raw: &mut outcome.forward_raw,
            plain: &packet_plain,
            patch: &mut outcome.patch,
        };
        handler.receive_bundle(peer_ref, bundle, direction, packet_channel.clone())?;
    }

    Ok(outcome)

}

/// A handler for events when polling the application.
pub trait Handler {

    /// The error type that should be able to be constructed from I/O error.
    type Error: From<io::Error>;

    /// The given peer is currently unknown and should be configured. This handler should
    /// return none to ignore that new peer.
    /// 
    /// The default implementation reject all peers.
    fn accept_peer(&mut self, 
        addr: SocketAddr,
    ) -> Result<Option<PeerConfig>, Self::Error> {
        let _ = (addr,);
        Ok(None)
    }

    /// The given peer has received or sent a packet with invalid encryption, and so it
    /// cannot be handled by protocol to be made into a bundle.
    /// 
    /// The default implementation does nothing.
    fn receive_invalid_packet_encryption(&mut self,
        peer: Peer,
        packet: Packet,
        direction: PacketDirection, 
    ) -> Result<(), Self::Error> {
        let _ = (peer, packet, direction);
        Ok(())
    }

    /// A bundle of elements has been transferred to or from the given peer.
    /// 
    /// The default implementation does nothing.
    fn receive_bundle(&mut self, 
        peer: Peer, 
        bundle: Bundle, 
        direction: PacketDirection, 
        channel: Option<PacketChannel>,
    ) -> Result<(), Self::Error> {
        let _ = (peer, bundle, direction, channel);
        Ok(())
    }

}

/// Blanket impl.
impl Handler for () {
    type Error = io::Error;
}

/// The type returned for new peers when accepted by the handler.
#[derive(Debug)]
pub struct PeerConfig {
    /// The real server address to forward the packets to after being received by proxy.
    pub real_addr: SocketAddr, 
    /// If this peer should use symmetric blowfish encryption for its packets, this will
    /// be used to intercept and read the clear packets before building the bundles.
    pub blowfish: Option<Arc<Blowfish>>,
}

/// A reference to an internal peer that allows its owner to enable or disable the 
/// encryption, modify the real address and send bundles.
#[derive(Debug)]
pub struct Peer<'a> {
    app: &'a mut AppInner,
    peer: &'a mut PeerInner,
    /// Cleared by `suppress_forward` to prevent the packet currently being processed
    /// from being forwarded verbatim to its destination once handling completes.
    forward_raw: &'a mut bool,
    /// The decrypted content of the packet currently being processed (or the raw
    /// packet as-is if this peer has no encryption), for [`Self::patch_raw`] to read
    /// from when building a modified copy to forward instead.
    plain: &'a Packet,
    /// Set by [`Self::patch_raw`] to a modified copy of `plain` that should be
    /// forwarded (re-encrypted if applicable) in place of the original raw packet,
    /// keeping the same framing (prefix, flags, sequence number, channel).
    patch: &'a mut Option<Packet>,
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
        *self.forward_raw = false;
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
    /// located in the packet's content.
    pub fn patch_raw(&mut self, find: &[u8], replace: &[u8]) -> bool {

        assert_eq!(find.len(), replace.len(), "patch_raw: find and replace must have the same length");

        let mut patched = self.plain.clone();
        let slice = patched.slice_mut();

        if let Some(pos) = slice.windows(find.len().max(1)).position(|w| w == find) {
            slice[pos..pos + find.len()].copy_from_slice(replace);
            *self.patch = Some(patched);
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
