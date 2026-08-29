//! Login application where clients ping and initiate connection in order to be routed
//! to the base application afterward.

pub mod element;

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::io;

use crypto_common::KeyInit;
use rsa::RsaPrivateKey;
use blowfish::Blowfish;

use rand::rngs::OsRng;
use rand::RngCore;

use tracing::trace;

use crate::net::bundle::{Bundle, NextElementReader, ElementReader};
use crate::util::cuckoo::CuckooContext;
use crate::net::socket::PacketSocket;
use crate::net::proto::Protocol;
use super::io_invalid_data;

use element::{
    Ping,
    LoginRequest,
    LoginResponse, LoginChallenge,
    LoginSuccess, LoginError,
    ChallengeResponse, CuckooCycleResponse,
};


/// The login application.
#[derive(Debug)]
pub struct App {
    /// Internal socket for this application.
    socket: PacketSocket,
    /// The packet tracker used to build bundles.
    protocol: Protocol,
    /// Queue of events that are waiting to be returned.
    events: VecDeque<Event>,
    /// A temporary bundle for sending.
    bundle: Bundle,
    /// Optional private key to set if encryption is enabled on the login app. This 
    /// implies that the client should use the matching public key when logging in in
    /// order to validate.
    encryption_key: Option<Arc<RsaPrivateKey>>,
    /// Login requests of each client in process with the login app.
    pending_requests: HashMap<SocketAddr, PendingRequest>,
    /// Responses to be sent in response to login or challenge requests.
    pending_responses: VecDeque<PendingResponse>,
    /// Issued and pending challenges.
    pending_challenges: HashMap<SocketAddr, PendingChallenge>,
    /// Used for benchmarking performance.
    received_instant: Option<Instant>,
}
impl App {

    pub fn new(addr: SocketAddr) -> io::Result<Self> {
        Ok(Self {
            socket: PacketSocket::bind(addr)?,
            protocol: Protocol::new(),
            events: VecDeque::new(),
            bundle: Bundle::new(),
            encryption_key: None,
            pending_requests: HashMap::new(),
            pending_responses: VecDeque::new(),
            pending_challenges: HashMap::new(),
            received_instant: None,
        })
    }

    /// Get the address this app is bound to.
    pub fn addr(&self) -> io::Result<SocketAddr> {
        self.socket.addr()
    }

    /// Enable encryption on login app, given a RSA private key, the client should use 
    /// the matching public key in order to validate this server.
    pub fn set_encryption(&mut self, key: Arc<RsaPrivateKey>) {
        self.encryption_key = Some(key);
    }

    /// As opposed to [`Self::set_private_key`], disable encryption on login app.
    pub fn remove_encryption(&mut self) {
        self.encryption_key = None;
    }

    /// Return true if encryption is enabled on this login app.
    pub fn has_encryption(&self) -> bool {
        self.encryption_key.is_some()
    }

    /// Poll for the next event of this login app, blocking.
    pub fn poll(&mut self) -> Event {
        loop {

            // Empty the events before.
            while let Some(event) = self.events.pop_front() {
                return event;
            }

            // Then send pending login responses.
            while let Some(res) = self.pending_responses.pop_front() {
                let addr = res.addr;
                if let Err(error) = self.send_response(res) {
                    return Event::IoError(IoErrorEvent { error, addr: Some(addr) });
                }
            }

            let (packet, addr) = match self.socket.recv() {
                Ok(ret) => ret,
                Err(error) => return Event::IoError(IoErrorEvent { error, addr: None }),
            };
            
            let Ok(mut channel) = self.protocol.accept(packet, addr) else {
                continue;
            };

            let Some(bundle) = channel.next_bundle() else {
                continue;
            };

            self.received_instant = Some(Instant::now());

            // Fully read the bundle to determine how to handle that client.
            let mut reader = bundle.element_reader();
            while let Some(reader) = reader.next() {
                match reader {
                    NextElementReader::Element(elt) => {
                        if let Err(error) = self.handle_element(elt, addr) {
                            return Event::IoError(IoErrorEvent { error, addr: Some(addr) });
                        }
                    }
                    NextElementReader::Reply(reply) => {
                        return Event::IoError(IoErrorEvent {
                            error: io_invalid_data(format_args!("unexpected reply #{}", reply.request_id())),
                            addr: Some(addr),
                        });
                    }
                }
            }

        }
    }

    /// Handle an element read from the given address.
    fn handle_element(&mut self, elt: ElementReader, addr: SocketAddr) -> io::Result<()> {
        match elt.id() {
            element::id::PING => self.handle_ping(elt, addr),
            element::id::LOGIN_REQUEST => self.handle_login_request(elt, addr),
            element::id::CHALLENGE_RESPONSE => self.handle_challenge_response(elt, addr),
            id => Err(io_invalid_data(format_args!("unexpected element #{id}"))),
        }
    }

    /// Handle a ping request to the login node, we answer as fast as possible.
    fn handle_ping(&mut self, elt: ElementReader, addr: SocketAddr) -> io::Result<()> {

        let ping = elt.read_simple::<Ping>()?;
        let request_id = ping.request_id
            .ok_or_else(|| io_invalid_data(format_args!("ping should be a request")))?;

        self.bundle.clear();
        self.bundle.element_writer().write_simple_reply(ping.element, request_id);
        self.protocol.off_channel(addr).prepare(&mut self.bundle, false);
        self.socket.send_bundle_without_encryption(&self.bundle, addr)?;

        let latency = self.received_instant.unwrap().elapsed();
        self.events.push_back(Event::Ping(PingEvent { 
            addr,
            latency,
        }));

        Ok(())

    }

    /// Handle a login request to the login node.
    fn handle_login_request(&mut self, elt: ElementReader, addr: SocketAddr) -> io::Result<()> {

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

        // Update or insert the login tracker... 
        self.pending_requests.insert(addr, PendingRequest {
            blowfish,
            request_id,
        });

        self.events.push_back(Event::Login(LoginEvent {
            addr,
            request: login.element,
        }));

        Ok(())

    }

    fn handle_challenge_response(&mut self, elt: ElementReader, addr: SocketAddr) -> io::Result<()> {

        let Some(pending_challenge) = self.pending_challenges.remove(&addr) else {
            return Err(io_invalid_data(format_args!("unexpected challenge")));
        };

        let challenge = elt.read_simple::<ChallengeResponse<CuckooCycleResponse>>()?;

        trace!("Received key: {:?}, sent prefix: {:?}", 
            crate::util::AsciiFmt(&challenge.element.data.key),
            crate::util::AsciiFmt(&pending_challenge.key_prefix));

        // Start by checking coherency.
        if !challenge.element.data.key.starts_with(&pending_challenge.key_prefix) {
            return Err(io_invalid_data(format_args!("challenge has invalid key prefix")));
        }

        trace!("Received solution: {:?}, sent max nonce: {}", 
            challenge.element.data.solution, 
            pending_challenge.max_nonce);

        let cuckoo = CuckooContext::new(pending_challenge.max_nonce, &challenge.element.data.key);
        if !cuckoo.verify_bw(&challenge.element.data.solution) {
            return Err(io_invalid_data(format_args!("challenge has invalid solution")));
        }
        
        self.events.push_back(Event::Challenge(ChallengeEvent {
            addr,
        }));

        Ok(())

    }

    /// In response to a [`LoginRequestEvent`], authorize a client to log into the base
    /// application, giving them its address and a login key that will be used to 
    /// register itself.
    /// 
    /// This returns the blowfish encryption instance if a client was effectively 
    /// waiting for a response.
    pub fn answer_login_success(&mut self, 
        addr: SocketAddr, 
        app_addr: SocketAddrV4, 
        login_key: u32,
        server_message: String
    ) -> Option<Arc<Blowfish>> {
        self.answer_login_response(addr, LoginResponse::Success(LoginSuccess {
            addr: app_addr.into(),
            login_key,
            server_message,
        }))
    }

    /// In response to a [`LoginRequestEvent`], authorize a client to log into the base
    /// application, giving them its address and a login key that will be used to 
    /// register itself.
    /// 
    /// This returns true if a client was effectively waiting for a response.
    pub fn answer_login_error(&mut self, 
        addr: SocketAddr, 
        error: LoginError,
        message: String,
    ) -> bool {
        self.answer_login_response(addr, LoginResponse::Error(error, message)).is_some()
    }

    /// In response to a [`LoginRequestEvent`], send a client the challenge it should
    /// complete. This implementation issue a Cuckoo Cycle challenge, but that's a detail.
    /// 
    /// This returns true if a client was effectively waiting for a response.
    pub fn answer_login_challenge(&mut self,
        addr: SocketAddr,
    ) -> bool {

        let easiness = 0.9;

        let key_prefix_value = OsRng.next_u64();
        let key_prefix = format!("{key_prefix_value:>02X}").into_bytes();
        let max_nonce = ((1 << 20) as f32 * easiness) as u32;

        let success = self.answer_login_response(addr, LoginResponse::Challenge(LoginChallenge::CuckooCycle { 
            key_prefix: key_prefix.clone(), 
            max_nonce,
        })).is_some();

        if !success {
            return false;
        }

        self.pending_challenges.insert(addr, PendingChallenge {
            key_prefix: key_prefix.clone(),
            max_nonce,
        });

        true

    }

    /// Internal wrapper for answering a login response.
    #[inline]
    fn answer_login_response(&mut self, addr: SocketAddr, response: LoginResponse) -> Option<Arc<Blowfish>> {

        let Some(request) = self.pending_requests.remove(&addr) else {
            return None;
        };

        let bf = Arc::clone(&request.blowfish);

        self.pending_responses.push_back(PendingResponse {
            request,
            addr,
            inner: response,
        });

        Some(bf)

    }

    /// Send a challenge to the given client, this only works if a tracker exists.
    fn send_response(&mut self, response: PendingResponse) -> io::Result<()> {

        self.bundle.clear();
        self.bundle.element_writer().write_reply(response.inner, response.request.request_id, &*response.request.blowfish);

        self.protocol.off_channel(response.addr).prepare(&mut self.bundle, false);
        self.socket.send_bundle_without_encryption(&self.bundle, response.addr)?;

        Ok(())

    }

}

/// Describe a client trying to log into the server.
#[derive(Debug)]
struct PendingRequest {
    /// This is the blowfish key as sent by the client when requesting login.
    blowfish: Arc<Blowfish>,
    /// Id of the last request the client sent and where replies should be sent.
    request_id: u32,
}

/// Describe a response pending to be sent to an address.
#[derive(Debug)]
struct PendingResponse {
    /// Initial request leading to this response.
    request: PendingRequest,
    /// The address of the client.
    addr: SocketAddr,
    /// Inner login response.
    inner: LoginResponse,
}

/// Describe a challenge that have been issued, this is currently about a Cuckoo Cycle.
#[derive(Debug)]
struct PendingChallenge {
    /// The key prefix expected for the answered key.
    key_prefix: Vec<u8>,
    /// The configured max nonce.
    max_nonce: u32,
}

// ============ //
//    EVENTS    //
// ============ //

/// An event that happened in the login app regarding the login process.
#[derive(Debug)]
pub enum Event {
    IoError(IoErrorEvent),
    Ping(PingEvent),
    Login(LoginEvent),
    Challenge(ChallengeEvent),
}

/// Some IO error happened internally and optionally related to a client.
#[derive(Debug)]
pub struct IoErrorEvent {
    /// The IO error.
    pub error: io::Error,
    /// An optional client address related to the error.
    pub addr: Option<SocketAddr>,
}

/// A client has pinged the login app.
#[derive(Debug)]
pub struct PingEvent {
    /// The address of the client that pinged the login app.
    pub addr: SocketAddr,
    /// Duration between reception of the ping packet to it being fully sent, this is
    /// the internal latency, not representing the real latency to the client.
    pub latency: Duration,
}

/// A client has made a request to login, this request can be answered with the app using:
/// - [`App::answer_login_success`]
/// - [`App::answer_login_error`]
/// - [`App::answer_login_challenge`]
#[derive(Debug)]
pub struct LoginEvent {
    /// The address of the client that request a login.
    pub addr: SocketAddr,
    /// The request received.
    pub request: LoginRequest,
}

/// A challenge has been answered by the client, this will usually followed by another
/// [`LoginEvent`] to request again.
#[derive(Debug)]
pub struct ChallengeEvent {
    /// The address of the client that request a login.
    pub addr: SocketAddr,
}