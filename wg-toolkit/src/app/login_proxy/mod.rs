//! The login proxy application for intercepting and forwarding login requests.

use std::collections::{hash_map, HashMap, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::io;

use rsa::{RsaPrivateKey, RsaPublicKey};
use tracing::{trace, trace_span};
use crypto_common::KeyInit;
use blowfish::Blowfish;

use crate::net::bundle::{Bundle, NextElementReader, ReplyReader, ElementReader};
use crate::util::thread::{ThreadPoll, ThreadWorker};
use crate::net::socket::PacketSocket;
use crate::net::proto::Protocol;
use crate::net::packet::Packet;

use super::login::element::{LoginError, LoginRequest, LoginResponse, LoginSuccess, Ping, ChallengeResponse, CuckooCycleResponse};
use super::proxy::{UNSPECIFIED_ADDR, RECV_TIMEOUT};
use super::io_invalid_data;

pub use super::login::element;  // Re-export the login elements.


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
    /// Events waiting to be polled.
    events: VecDeque<Event>,
    /// The successful logins waiting to be answered, by the peer's address.
    pending_successes: HashMap<SocketAddr, PendingSuccess>,
}

/// A successful login of the real login app, waiting to be forwarded with the base app
/// address to give, see [`App::answer_login_success`].
#[derive(Debug)]
struct PendingSuccess {
    request_id: u32,
    blowfish: Arc<Blowfish>,
    success: LoginSuccess,
    /// The prefix of the real login app's packet, see [`Inner::handle_in`].
    prefix: u32,
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
    Login { blowfish: Arc<Blowfish>, blowfish_key: Vec<u8> },
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
                events: VecDeque::new(),
                pending_successes: HashMap::new(),
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

    /// Poll for the next event of this login app, blocking until one comes.
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

            if let Some(event) = self.inner.events.pop_front() {
                return event;
            }

            self.drop_dead_peers();

            let socket_poll_ret = match deadline {
                Some(deadline) => match self.inner.socket_poll.poll_until(deadline) {
                    Some(ret) => ret,
                    None => return Event::Timeout,
                },
                None => self.inner.socket_poll.poll(),
            };

            let peer = socket_poll_ret.peer;
            if let Err(error) = self.handle(socket_poll_ret) {
                return Event::IoError(IoErrorEvent { error, addr: peer });
            }

        }
    }

    /// Drop the peers inactive for too long, which also terminates their poll threads.
    fn drop_dead_peers(&mut self) {
        if !self.peers.is_empty() {
            let now = Instant::now();
            let pending_successes = &mut self.inner.pending_successes;
            self.peers.retain(|addr, peer| {
                if now - peer.last_time >= DEAD_PEER_TIMEOUT {
                    trace!("Dropped peer due to inactivity: {addr}");
                    pending_successes.remove(addr);
                    false
                } else {
                    true
                }
            });
        }
    }

    /// Handle a packet received, or the error receiving it.
    fn handle(&mut self, socket_poll_ret: SocketPollRet) -> io::Result<()> {

        let (packet, addr) = match socket_poll_ret.res {
            Ok(ret) => ret,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => return Ok(()),
            Err(e) => return Err(e),
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
                self.inner.handle_in(peer, bundle)?;
            } else {
                self.inner.handle_out(peer, bundle)?;
            }
        }

        Ok(())

    }

    /// Answer a successful login, see [`Event::LoginSuccess`], giving the peer this
    /// base app address.
    pub fn answer_login_success(&mut self, addr: SocketAddr, base_app: SocketAddrV4) {
        if let Some(mut pending) = self.inner.pending_successes.remove(&addr) {
            pending.success.addr = base_app.into();
            let prefix = pending.prefix;
            self.inner.answer_login(addr, pending, LoginResponse::Success, Some(prefix));
        }
    }

    /// Answer a successful login, see [`Event::LoginSuccess`], with an error instead.
    pub fn answer_login_error(&mut self, addr: SocketAddr, error: LoginError, data: String) {
        if let Some(pending) = self.inner.pending_successes.remove(&addr) {
            self.inner.answer_login(addr, pending, |_| LoginResponse::Error(error, data), None);
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
        
        self.bundle.element_writer().write_simple_request(ping.element, request_id)?;

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
            kind: PeerLastRequestKind::Login { blowfish, blowfish_key: login.element.blowfish_key.clone() },
        });

        if let Some(encryption_key) = self.real_encryption_key.as_deref() {
            self.bundle.element_writer().write_request(login.element.clone(), request_id, encryption_key)?;
        } else {
            self.bundle.element_writer().write_simple_request(login.element.clone(), request_id)?;
        }

        Ok(())

    }

    fn handle_challenge_response(&mut self, _peer: &mut Peer, elt: ElementReader) -> io::Result<()> {
        let challenge = elt.read_simple::<ChallengeResponse<CuckooCycleResponse>>()?;
        self.bundle.element_writer().write_simple(challenge.element)?;
        Ok(())
    }

    /// Handle a bundle of the real login app.
    ///
    /// We currently know how to compute the prefix of packets, but the official login
    /// application is returning a slightly wrong (from our POV) prefix ONLY on
    /// successful login responses, and we don't know yet how it produces it. This
    /// prefix is also always the same from what have been observed (64C20486). From the
    /// client decompilation attempt it looks like the client is aware of that and in
    /// case of successful login it use the latest received prefix (the one previously
    /// mentioned) as the prefix offset for the rest of the communications with the base
    /// app.
    ///
    /// As a temporary fix, a successful login, answered later, keeps the prefix of its
    /// packet to brainlessly inherit it.
    fn handle_in(&mut self, peer: &mut Peer, bundle: Bundle) -> io::Result<()> {

        self.bundle.clear();

        let mut reader = bundle.element_reader();
        while let Some(reader) = reader.next() {
            match reader {
                NextElementReader::Element(elt) =>
                    return Err(io_invalid_data(format_args!("unexpected element #{}", elt.id()))),
                NextElementReader::Reply(reply) =>
                    self.handle_in_reply(peer, reply)?,
            }
        }

        if !self.bundle.is_empty() {
            self.out_protocol.off_channel(peer.addr).prepare(&mut self.bundle, false);
            self.socket.send_bundle_without_encryption(&self.bundle, peer.addr)?;
        }

        Ok(())

    }

    fn handle_in_reply(&mut self, peer: &mut Peer, elt: ReplyReader) -> io::Result<()> {

        let request_id = elt.request_id();
        if peer.last_request.as_ref().map(|l| l.request_id) != Some(request_id) {
            return Err(io_invalid_data(format_args!("unexpected reply #{}", request_id)));
        }

        let last_request = peer.last_request.take().unwrap();
        let latency = last_request.time.elapsed();

        match last_request.kind {
            PeerLastRequestKind::Ping {  } => {

                self.events.push_back(Event::Ping(PingEvent { addr: peer.addr, latency }));

                let ping = elt.read_simple::<Ping>()?;
                self.bundle.element_writer().write_simple_reply(ping, request_id)?;
                
            }
            PeerLastRequestKind::Login { blowfish, blowfish_key } => {

                let login = elt.read::<LoginResponse, _>(&*blowfish)?;

                if let LoginResponse::Success(success) = login {
                    self.events.push_back(Event::LoginSuccess(LoginSuccessEvent {
                        addr: peer.addr,
                        blowfish_key,
                        base_app: success.addr.addr,
                        login_key: success.login_key,
                        server_message: success.server_message.clone(),
                    }));
                    self.pending_successes.insert(peer.addr, PendingSuccess {
                        request_id,
                        blowfish,
                        success,
                        prefix: self.in_protocol.last_accepted_prefix(),
                    });
                    return Ok(());
                }

                if let LoginResponse::Error(error, data) = &login {
                    self.events.push_back(Event::LoginError(LoginErrorEvent {
                        addr: peer.addr,
                        error: *error,
                        data: data.clone(),
                    }));
                }

                self.bundle.element_writer().write_reply(login, request_id, &*blowfish)?;

            }
        }

        Ok(())

    }

}

impl Inner {

    /// Answer a successful login with this response, built from it, with the given
    /// packet prefix.
    fn answer_login(&mut self,
        addr: SocketAddr,
        pending: PendingSuccess,
        response: impl FnOnce(LoginSuccess) -> LoginResponse,
        prefix: Option<u32>,
    ) {

        self.bundle.clear();
        let res = self.bundle.element_writer().write_reply(response(pending.success), pending.request_id, &*pending.blowfish);
        if let Err(error) = res {
            self.events.push_back(Event::IoError(IoErrorEvent { error, addr: Some(addr) }));
            return;
        }

        self.out_protocol.off_channel(addr).prepare(&mut self.bundle, false);
        if let Some(prefix) = prefix {
            self.bundle.write_prefix(prefix);
        }

        if let Err(error) = self.socket.send_bundle_without_encryption(&self.bundle, addr) {
            self.events.push_back(Event::IoError(IoErrorEvent { error, addr: Some(addr) }));
        }

    }

}

/// An event of the login proxy.
#[derive(Debug)]
pub enum Event {
    /// The deadline given to [`App::poll_until`] passed with no event.
    Timeout,
    IoError(IoErrorEvent),
    Ping(PingEvent),
    LoginSuccess(LoginSuccessEvent),
    LoginError(LoginErrorEvent),
}

/// Some IO error happened internally and optionally related to a peer.
#[derive(Debug)]
pub struct IoErrorEvent {
    pub error: io::Error,
    pub addr: Option<SocketAddr>,
}

/// A peer was answered its ping, after this latency from the real login app.
#[derive(Debug)]
pub struct PingEvent {
    pub addr: SocketAddr,
    pub latency: Duration,
}

/// A peer successfully logged into the real login app, with everything intercepted.
///
/// The blowfish key is negotiated and must be used on the base app that this peer is
/// expected to connect to, whose address is also given. The first element sent to the
/// base app is not encrypted with the blowfish key and contains the login key. The
/// server message is usually a JSON string.
///
/// The peer is answered once the app is given the base app address to return to it,
/// with [`App::answer_login_success`]: this can be a proxy base app that will forward
/// to the real one.
#[derive(Debug)]
pub struct LoginSuccessEvent {
    pub addr: SocketAddr,
    pub blowfish_key: Vec<u8>,
    pub base_app: SocketAddrV4,
    pub login_key: u32,
    pub server_message: String,
}

/// A peer failed to log into the real login app.
#[derive(Debug)]
pub struct LoginErrorEvent {
    pub addr: SocketAddr,
    pub error: LoginError,
    pub data: String,
}
