//! The login proxy application for intercepting and forwarding login requests.

use std::collections::{hash_map, HashMap};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::io;

use rsa::{RsaPrivateKey, RsaPublicKey};
use crypto_common::KeyInit;
use blowfish::Blowfish;

use tracing::{trace, trace_span};

use crate::net::bundle::{Bundle, NextElementReader, ReplyReader, ElementReader};
use crate::net::socket::PacketSocket;
use crate::net::proto::Protocol;
use crate::net::packet::Packet;
use crate::net::codec::WgSocketAddrV4;

use crate::util::thread::{ThreadPoll, ThreadWorker};

pub use super::login::element;  // Re-export the login elements.
use super::login::element::{LoginError, LoginRequest, LoginResponse, Ping, ChallengeResponse, CuckooCycleResponse};
use super::proxy::{UNSPECIFIED_ADDR, RECV_TIMEOUT};
use super::io_invalid_data;


const DEAD_PEER_TIMEOUT: Duration = Duration::from_secs(10);


/// A special login application that acts as a proxy that forwards login request to a
/// real login server by re-encrypting the requests in order to intercept in clear the
/// requests.
#[derive(Debug)]
pub struct App {
    /// Internal state.
    inner: Inner,
    /// Map of all peers in the ping or login process.
    peers: HashMap<SocketAddr, Peer>,
}

#[derive(Debug)]
struct Inner {
    /// Thread poll for socket result.
    socket_poll: ThreadPoll<SocketPollRet>,
    /// Internal socket for this application.
    socket: PacketSocket,
    /// Optional private key to set if encryption is enabled on the login app. This 
    /// implies that the client should use the matching public key when logging in in
    /// order to validate.
    encryption_key: Option<Arc<RsaPrivateKey>>,
    /// The address of the real application where we proxy all packets.
    real_addr: SocketAddr,
    /// Encryption key for sending to the real login application.
    real_encryption_key: Option<Arc<RsaPublicKey>>,
    /// Protocol for accepting out packets and preparing in packets.
    out_protocol: Protocol,
    /// Protocol for accepting in packets and preparing out packets.
    in_protocol: Protocol,
    /// A temporary bundle for sending.
    bundle: Bundle,
}

#[derive(Debug)]
struct Peer {
    /// Handle for drop-destruction of the poll thread worker, only used for drop.
    _socket_worker: ThreadWorker,
    /// The socket represent this peer for the real application.
    socket: PacketSocket,
    /// The address to send packets to the peer when receiving from real application.
    addr: SocketAddr,
    /// Last time a paquet was received from this peer.
    last_time: Instant,
    /// Information about the last request made by the client, if any.
    last_request: Option<PeerLastRequest>,
}

#[derive(Debug)]
struct PeerLastRequest {
    request_id: u32,
    time: Instant,
    kind: PeerLastRequestKind,
}

#[derive(Debug)]
enum PeerLastRequestKind {
    Ping {},
    Login { blowfish: Arc<Blowfish>, },
}

/// Type of return value for our socket poll. 
#[derive(Debug)]
struct SocketPollRet {
    /// The raw I/O result containing the packet if successful.
    res: io::Result<(Packet, SocketAddr)>,
    /// The peer address if this is the result of a peer socket.
    peer: Option<SocketAddr>,
}

impl App {

    pub fn new(addr: SocketAddr, real_addr: SocketAddr, real_encryption_key: Option<Arc<RsaPublicKey>>) -> io::Result<Self> {
        
        let socket_poll = ThreadPoll::new();

        let socket = PacketSocket::bind(addr)?;
        socket.set_recv_timeout(Some(RECV_TIMEOUT))?;

        let thread_socket = socket.clone();
        socket_poll.spawn(move || Some(SocketPollRet {
            res: thread_socket.recv_without_encryption(),
            peer: None,
        }));

        Ok(Self {
            inner: Inner {
                socket_poll,
                socket,
                encryption_key: None,
                real_addr,
                real_encryption_key,
                out_protocol: Protocol::new(),
                in_protocol: Protocol::new(),
                bundle: Bundle::new(),
            },
            peers: HashMap::new(),
        })

    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.inner.socket.addr()
    }

    /// Enable encryption on login app, given a RSA private key, the client should use 
    /// the matching public key in order to validate this server.
    pub fn set_encryption(&mut self, key: Arc<RsaPrivateKey>) {
        self.inner.encryption_key = Some(key);
    }

    /// As opposed to [`Self::set_private_key`], disable encryption on login app.
    pub fn remove_encryption(&mut self) {
        self.inner.encryption_key = None;
    }

    /// Return true if encryption is enabled on this login app.
    pub fn has_encryption(&self) -> bool {
        self.inner.encryption_key.is_some()
    }

    /// Poll for the next event of this login app, blocking.
    pub fn poll<H: Handler>(&mut self, handler: &mut H) -> Result<(), H::Error> {

        // Dropping dead peers, this will also terminate poll threads.
        if !self.peers.is_empty() {
            let now = Instant::now();
            self.peers.retain(|addr, peer| {
                if now - peer.last_time >= DEAD_PEER_TIMEOUT {
                    trace!("Dropped peer due to inactivity: {addr}");
                    false
                } else {
                    true
                }
            });
        }

        let socket_poll_ret = self.inner.socket_poll.poll();

        let (packet, addr) = match socket_poll_ret.res {
            Ok(ret) => ret,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        
        // debug!("<{}: [{:08X}] {:?}", addr, packet.raw().read_prefix(), packet.raw());

        let now = Instant::now();
        let _span;
        let protocol;
        let peer;

        if let Some(peer_addr) = &socket_poll_ret.peer {

            // The packet has been received from the real login application and should be
            // forwarded to the peer.
            _span = trace_span!("in").entered();
            protocol = &mut self.inner.in_protocol;
            peer = match self.peers.get_mut(peer_addr) {
                Some(peer) => peer,
                None => return Ok(()), // Ignore if we received an event from a dead peer.
            };

        } else {
            
            // The packet has been received from the peer and should be forwarded to the 
            // real login application.
            _span = trace_span!("out").entered();
            protocol = &mut self.inner.out_protocol;
            peer = match self.peers.entry(addr) {
                hash_map::Entry::Occupied(o) => o.into_mut(),
                hash_map::Entry::Vacant(v) => {
                    
                    let socket = PacketSocket::bind(UNSPECIFIED_ADDR)?;
                    socket.set_recv_timeout(Some(RECV_TIMEOUT))?;

                    let thread_socket = socket.clone();
                    let _socket_worker = self.inner.socket_poll.spawn_with_handle(move || Some(SocketPollRet {
                        res: thread_socket.recv_without_encryption(),
                        peer: Some(addr),
                    }));

                    v.insert(Peer {
                        _socket_worker,
                        socket,
                        addr,
                        last_time: now,
                        last_request: None,
                    })

                }
            };

        }

        peer.last_time = now;

        let mut channel = match protocol.accept(packet, peer.addr) {
            Ok(channel) => channel,
            Err(_packet) => return Ok(()),
        };

        for bundle in channel.pop_bundles() {
            if socket_poll_ret.peer.is_some() {
                self.inner.handle_in(&mut *handler, peer, bundle)?;
            } else {
                self.inner.handle_out(peer, bundle)?;
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

impl Inner {

    fn handle_out(&mut self, peer: &mut Peer, bundle: Bundle) -> io::Result<()> {
        
        self.bundle.clear();

        let mut reader = bundle.element_reader();
        while let Some(reader) = reader.next() {
            match reader {
                NextElementReader::Element(elt) => 
                    self.handle_out_element(peer, elt)?,
                NextElementReader::Reply(reply) => 
                    return Err(io_invalid_data(format_args!("unexpected reply #{}", reply.request_id()))),
            }
        }

        if !self.bundle.is_empty() {
            self.in_protocol.off_channel(peer.addr).prepare(&mut self.bundle, false);
            // for packet in self.bundle.packets() {
            //     debug!(">{}: [{:08X}] {:?}", self.real_addr, packet.raw().read_prefix(), packet.raw());
            // }
            peer.socket.send_bundle_without_encryption(&self.bundle, self.real_addr)?;
        }

        Ok(())

    }

    fn handle_out_element(&mut self, peer: &mut Peer, elt: ElementReader) -> io::Result<()> {
        match elt.id() {
            element::id::PING => self.handle_out_ping(peer, elt),
            element::id::LOGIN_REQUEST => self.handle_login_request(peer, elt),
            element::id::CHALLENGE_RESPONSE => self.handle_challenge_response(peer, elt),
            id => Err(io_invalid_data(format_args!("unexpected element #{id}"))),
        }
    }

    /// Handle a ping request to the login node, we answer as fast as possible.
    fn handle_out_ping(&mut self, peer: &mut Peer, elt: ElementReader) -> io::Result<()> {

        let ping = elt.read_simple::<Ping>()?;
        let request_id = ping.request_id
            .ok_or_else(|| io_invalid_data(format_args!("ping should be a request")))?;

        peer.last_request = Some(PeerLastRequest {
            request_id,
            time: Instant::now(),
            kind: PeerLastRequestKind::Ping {  },
        });
        
        self.bundle.element_writer().write_simple_request(ping.element, request_id);

        Ok(())

    }

    /// Handle a login request to the login node.
    fn handle_login_request(&mut self, peer: &mut Peer, elt: ElementReader) -> io::Result<()> {
        
        let login;
        if let Some(encryption_key) = self.encryption_key.as_deref() {
            login = elt.read::<LoginRequest, _>(encryption_key)?;
        } else {
            login = elt.read_simple::<LoginRequest>()?;
        }

        let request_id = login.request_id
            .ok_or_else(|| io_invalid_data(format_args!("login should be a request")))?;

        let blowfish = Arc::new(Blowfish::new_from_slice(&login.element.blowfish_key)
            .map_err(|_| io_invalid_data(format_args!("login has invalid blowfish key: {:?}", login.element.blowfish_key)))?);

        peer.last_request = Some(PeerLastRequest {
            request_id,
            time: Instant::now(),
            kind: PeerLastRequestKind::Login { blowfish },
        });

        if let Some(encryption_key) = self.real_encryption_key.as_deref() {
            self.bundle.element_writer().write_request(login.element.clone(), request_id, encryption_key);
        } else {
            self.bundle.element_writer().write_simple_request(login.element.clone(), request_id);
        }

        Ok(())

    }

    fn handle_challenge_response(&mut self, _peer: &mut Peer, elt: ElementReader) -> io::Result<()> {
        let challenge = elt.read_simple::<ChallengeResponse<CuckooCycleResponse>>()?;
        self.bundle.element_writer().write_simple(challenge.element);
        Ok(())
    }

    fn handle_in<H: Handler>(&mut self, handler: &mut H, peer: &mut Peer, bundle: Bundle) -> Result<(), H::Error> {

        self.bundle.clear();
        
        // We currently know how to compute the prefix of packets, but the official login
        // application is returning a slightly wrong (from our POV) prefix ONLY on 
        // successful login responses, and we don't know yet how it produces it.
        // This prefix is also always the same from what have been observed (64C20486).
        // From the client decompilation attempt it looks like the client is aware of 
        // that and in case of successful login it use the latest received prefix (the
        // one previously mentioned) as the prefix offset for the rest of the 
        // communications with the base app.
        //
        // As a temporary fix, we set this flag to true only on successful login, and
        // in this case we brainlessly inherit the prefix.
        let mut inherit_prefix = false;

        let mut reader = bundle.element_reader();
        while let Some(reader) = reader.next() {
            match reader {
                NextElementReader::Element(elt) => 
                    return Err(io_invalid_data(format_args!("unexpected element #{}", elt.id())).into()),
                NextElementReader::Reply(reply) => 
                    self.handle_in_reply(&mut *handler, peer, reply, &mut inherit_prefix)?,
            }
        }

        if !self.bundle.is_empty() {
            self.out_protocol.off_channel(peer.addr).prepare(&mut self.bundle, false);
            if inherit_prefix {
                self.bundle.write_prefix(self.in_protocol.last_accepted_prefix());
            }
            // for packet in self.bundle.packets_mut() {
            //     debug!(">{}: [{:08X}] {:?}", peer.addr, packet.raw().read_prefix(), packet.raw());
            // }
            self.socket.send_bundle_without_encryption(&self.bundle, peer.addr)?;
        }

        Ok(())

    }

    fn handle_in_reply<H: Handler>(&mut self, handler: &mut H, peer: &mut Peer, elt: ReplyReader, inherit_prefix: &mut bool) -> Result<(), H::Error> {
        
        let request_id = elt.request_id();
        if peer.last_request.as_ref().map(|l| l.request_id) != Some(request_id) {
            return Err(io_invalid_data(format_args!("unexpected reply #{}", request_id)).into());
        }

        let last_request = peer.last_request.take().unwrap();
        let latency = last_request.time.elapsed();

        match last_request.kind {
            PeerLastRequestKind::Ping {  } => {

                handler.receive_ping(peer.addr, latency)?;

                let ping = elt.read_simple::<Ping>()?;
                self.bundle.element_writer().write_simple_reply(ping, request_id);
                
            }
            PeerLastRequestKind::Login { blowfish } => {

                let mut login = elt.read::<LoginResponse, _>(&*blowfish)?;
                
                if let LoginResponse::Success(success) = &mut login {

                    *inherit_prefix = true;
                    success.addr = handler.receive_login_success(
                        peer.addr, 
                        Arc::clone(&blowfish), 
                        success.addr, 
                        success.login_key, 
                        success.server_message.clone(),
                    )?;
                    
                } else if let LoginResponse::Error(error, data) = &login {

                    handler.receive_login_error(peer.addr, *error, data.clone())?;

                }

                self.bundle.element_writer().write_reply(login, request_id, &*blowfish);
                
            }
        }

        Ok(())

    }

}

/// A handler for events when polling the application.
pub trait Handler {

    /// The error type that should be able to be constructed from I/O error.
    type Error: From<io::Error>;

    /// The given peer has received a ping, the latency from sending to reception of the
    /// ping by the peer is also given.
    /// 
    /// The default implementation does nothing.
    fn receive_ping(&mut self,
        addr: SocketAddr,
        latency: Duration,
    ) -> Result<(), Self::Error> {
        let _ = (addr, latency);
        Ok(())
    }

    /// The given peer has successfully logged into the real login application, this
    /// function receives all the relevant information that have been intercepted.
    /// 
    /// The blowfish key is negotiated and must be used on the base application that
    /// this peer is expected to connect to, the base app address is also given. The
    /// first element sent to the base app is not encrypted with the blowfish key and
    /// contains the negotiated login key.
    /// 
    /// A server message is also given by the server, which is usually a JSON string.
    /// 
    /// This function should return the base app address that will actually be returned
    /// to the peer, this can be used to return a proxy base app that will forward to
    /// the real base application.
    /// 
    /// The default implementation returns the base app address given in parameter.
    fn receive_login_success(&mut self,
        addr: SocketAddr,
        blowfish: Arc<Blowfish>,
        base_app_addr: WgSocketAddrV4,
        login_key: u32,
        server_message: String,
    ) -> Result<WgSocketAddrV4, Self::Error> {
        let _ = (addr, blowfish, login_key, server_message);
        Ok(base_app_addr)
    }

    /// The given peer has failed its login process, this function receives all the
    /// relevant information that have been intercepted.
    /// 
    /// The default implementation does nothing.
    fn receive_login_error(&mut self,
        addr: SocketAddr,
        error: LoginError,
        data: String,
    ) -> Result<(), Self::Error> {
        let _ = (addr, error, data);
        Ok(())
    }

}
