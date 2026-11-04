//! The links between the apps of a cluster.
//!
//! Links always open toward the login apps, and only the side they open toward
//! listens: login apps listen and know nobody, base apps connect to a login app, or to
//! a manager when there is one. The manager is optional: it listens for base apps and
//! connects to every login app, speaking the same protocol on both sides, so that it
//! looks like a base app to the login apps and like a login app to the base apps.
//! Once open, a link is two-way, requests go either direction.
//!
//! Both sides are event-based, a [`LinkListener`] for the apps above and a
//! [`LinkStream`] for the apps below: an app takes their [`LinkEvent`]s when it can and
//! answers with messages, the links only carry them.
//!
//! Messages are sent "to login" (upward) or "to base" (downward), whether a manager
//! stands in between or not. Requests are a noun and a verb, their replies the same
//! noun and its past participle, carrying the address of the chosen base app or a
//! refusal:
//!
//! | Request       | Direction | Reply          | Direction |
//! |---------------|-----------|----------------|-----------|
//! | `ClientPlace` | to login  | `ClientPlaced` | to base   |
//! | `ClientHost`  | to base   | `ClientHosted` | to login  |
//! | `BattlePlace` | to login  | `BattlePlaced` | to base   |
//! | `BattleHost`  | to base   | `BattleHosted` | to login  |
//!
//! A `*Place` request is answered by the top of the cluster, the manager or else the
//! login app, which picks a base app and sends it a `*Host` request. A login app does
//! nothing special with `BattlePlace`: it answers `BattlePlaced` with no base app, and
//! the base app then hosts the battle itself. A manager forwards a `*Host` request to the
//! base app it picks. Request ids are local to a link, a
//! forwarded request gets a new one. Besides, `Hello` introduces both sides of a link
//! and `BaseStatus` tells the apps above the load of a base app. A login app picks
//! among its base apps at random, a manager balances by their load.
//!
//! The apps above also control the base apps, the manager typically when scaling the
//! cluster down, and so the login app too since a manager looks like one. Draining a
//! base app needs no message: the app above just stops picking it, and its load tells
//! when it has no client left, which is when it can be stopped:
//!
//! | Message      | Direction | Meaning  |
//! |--------------|-----------|----------|
//! | `BaseStatus` | to login  | its load |
//! | `BaseStop`   | both      | exit     |
//!
//! A base app registers with a first `BaseStatus` once linked, and sends a new one
//! whenever its load changes. `BaseStop` goes both ways: a base app sends it to
//! unregister, and a link closing is an implicit one. A manager stopped by a
//! login app stops all of its base apps.
//!
//! A client logging in, coming to a base app:
//!
//! ```text
//! ┌────────┐             ┌───────────┐            ┌─────────┐            ┌──────────┐
//! │ Client │             │ Login app │            │ Manager │            │ Base app │
//! └────┬───┘             └─────┬─────┘            └────┬────┘            └─────┬────┘
//!      │ login (UDP)           │                       ┆                       │
//!      ├──────────────────────▶│                       ┆                       │
//!      │                       │ ClientHost {1, …}     ┆                       │
//!      │                       ├──────────────────────▶┆                       │
//!      │                       │                       ┆ (picks a base)        │
//!      │                       │                       ┆ ClientHost {7, …}     │
//!      │                       │                       ├──────────────────────▶│
//!      │                       │                       ┆ ClientHosted {7, addr}│
//!      │                       │                       ┆◀──────────────────────┤
//!      │                       │ ClientHosted {1, addr}┆                       │
//!      │                       │◀──────────────────────┤                       │
//!      │ login ok (addr, key)  │                       ┆                       │
//!      │◀──────────────────────┤                       ┆                       │
//!      │ base app login (key, UDP)                     ┆                       │
//!      ├──────────────────────────────────────────────────────────────────────▶│
//!      │                       │                       ┆                       │
//! ```
//!
//! A `ClientHosted` reply may also say that nothing is available: a full base app
//! answers so, and so does a manager with no base app to pick, without forwarding the
//! request. A login app getting it tries another of its links, if any, and otherwise
//! refuses the client with [`LoginError::NoBaseApp`], also used when it has no link:
//!
//! ```text
//! ┌────────┐             ┌───────────┐            ┌─────────┐            ┌──────────┐
//! │ Client │             │ Login app │            │ Manager │            │ Base app │
//! └────┬───┘             └─────┬─────┘            └────┬────┘            └─────┬────┘
//!      │ login (UDP)           │                       ┆                       │
//!      ├──────────────────────▶│                       ┆                       │
//!      │                       │ ClientHost {1, …}     ┆                       │
//!      │                       ├──────────────────────▶┆                       │
//!      │                       │                       ┆ (none available)      │
//!      │                       │ ClientHosted {1, none}┆                       │
//!      │                       │◀──────────────────────┤                       │
//!      │ NoBaseApp (UDP)       │                       ┆                       │
//!      │◀──────────────────────┤                       ┆                       │
//!      │                       │                       ┆                       │
//! ```
//!
//! [`LoginError::NoBaseApp`]: wgtk::app::login::element::LoginError::NoBaseApp
//!
//! A battle placed on a base app by the manager, its clients switched to it (possibly
//! the same base app):
//!
//! ```text
//! ┌────────┐            ┌────────────┐            ┌─────────┐           ┌────────────┐
//! │ Client │            │ Base app A │            │ Manager │           │ Base app B │
//! └────┬───┘            └──────┬─────┘            └────┬────┘           └──────┬─────┘
//!      │                       │ (battle ready)        ┆                       │
//!      │                       │ BattlePlace {3, …}    ┆                       │
//!      │                       ├──────────────────────▶┆                       │
//!      │                       │                       ┆ (picks a base)        │
//!      │                       │                       ┆ BattleHost {8, …}     │
//!      │                       │                       ├──────────────────────▶│
//!      │                       │                       ┆ BattleHosted {8, addr}│
//!      │                       │                       ┆◀──────────────────────┤
//!      │                       │ BattlePlaced {3, addr}┆                       │
//!      │                       │◀──────────────────────┤                       │
//!      │ SwitchBaseApp (UDP)   │                       ┆                       │
//!      │◀──────────────────────┤                       ┆                       │
//!      │ base app login (UDP)  │                       ┆                       │
//!      ├──────────────────────────────────────────────────────────────────────▶│
//!      │                       │                       ┆                       │
//! ```
//!
//! A client going back to a hangar after a battle is placed the same way, with
//! `ClientPlace` and `ClientHost`.

use std::net::{SocketAddr, SocketAddrV4, TcpListener, TcpStream, Shutdown};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::io::{self, Read, Write};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use tracing::{debug, info, warn};

use wgtk::util::io::{WgReadExt, WgWriteExt};


/// The version of the protocol, links of other versions are refused.
const VERSION: u8 = 1;

/// How long a link stream waits before linking again after failing to, or after
/// being unlinked.
const RELINK_DELAY: Duration = Duration::from_secs(1);

/// A client an app below is asked to host, coming with its login key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHost {
    pub login_key: u32,
    pub addr: SocketAddr,
    pub blowfish_key: Vec<u8>,
    /// The player's name, until accounts are stored, empty for the proxy.
    pub name: String,
    /// The real base app the base proxy should forward the client to.
    pub real_base_app: Option<SocketAddrV4>,
}

/// A message on a link, see the module's documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    BaseStatus { load: u32 },
    BaseStop,
    ClientHost { request_id: u32, client: ClientHost },
    /// The address of the base app hosting the client, none if nothing is available.
    ClientHosted { request_id: u32, base_app: Option<SocketAddrV4> },
}

/// What a link listener shares with its threads and its local link streams.
#[derive(Debug)]
struct ListenerShared {
    next_id: AtomicU64,
    /// Where to send to every link.
    links: Mutex<HashMap<LinkId, ListenerLink>>,
    sender: Sender<LinkEvent<LinkId>>,
}

/// Where a link listener sends to a link.
#[derive(Debug)]
enum ListenerLink {
    Tcp(TcpStream),
    /// A link stream of this process, given its events directly.
    Local(Sender<LinkEvent>),
}

/// The id of a link accepted by a [`LinkListener`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LinkId(u64);

/// An event of a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent<L = ()> {
    Linked(L),
    Message(L, Message),
    /// The link closed, an implicit `BaseStop`.
    Unlinked(L),
}

/// Listening for the links of the apps below.
#[derive(Debug)]
pub struct LinkListener {
    /// The address it listens at, none for the link streams of this process only.
    addr: Option<SocketAddr>,
    shared: Arc<ListenerShared>,
    events: Receiver<LinkEvent<LinkId>>,
}

impl LinkListener {

    /// Listen at this address for the apps below to link with, those of this process
    /// can also link with [`LinkStream::connect_local`].
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {

        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let mut link_listener = Self::new();
        link_listener.addr = Some(addr);
        info!("Linked on: {addr}");

        let shared = Arc::clone(&link_listener.shared);
        thread::Builder::new()
            .name(format!("link {addr}"))
            .spawn(move || listener_thread(listener, shared))?;

        Ok(link_listener)

    }

    /// Listen for the apps below of this process only, see
    /// [`LinkStream::connect_local`].
    pub fn local() -> Self {
        info!("Linked on: process");
        Self::new()
    }

    fn new() -> Self {
        let (sender, events) = crossbeam_channel::unbounded();
        let shared = Arc::new(ListenerShared {
            next_id: AtomicU64::new(0),
            links: Mutex::new(HashMap::new()),
            sender,
        });
        Self { addr: None, shared, events }
    }

    /// The address the apps below link to, none for those of this process only.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.addr
    }

    /// Send a message on this link.
    pub fn send(&self, link: LinkId, message: &Message) -> io::Result<()> {
        match self.shared.links.lock().unwrap().get_mut(&link) {
            Some(ListenerLink::Tcp(stream)) => 
                stream.write_all(&message.encode()),
            Some(ListenerLink::Local(sender)) => 
                sender.send(LinkEvent::Message((), message.clone()))
                    .map_err(|_| io::ErrorKind::NotConnected.into()),
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    /// The next event, if any.
    pub fn try_event(&self) -> Option<LinkEvent<LinkId>> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting for it no longer than this.
    pub fn event_timeout(&self, timeout: Duration) -> Option<LinkEvent<LinkId>> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }

}

/// The thread of a link listener, accepting links and starting a thread for each.
fn listener_thread(listener: TcpListener, shared: Arc<ListenerShared>) {
    for stream in listener.incoming() {

        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                warn!("Failed to accept a link: {e}");
                continue;
            }
        };

        let id = LinkId(shared.next_id.fetch_add(1, Ordering::Relaxed));
        let name = match stream.peer_addr() {
            Ok(addr) => format!("link from {addr}"),
            Err(_) => "link from ?".to_string(),
        };

        let shared = Arc::clone(&shared);
        let res = thread::Builder::new()
            .name(name)
            .spawn(move || listener_link_thread(stream, id, shared));

        if let Err(e) = res {
            warn!("Failed to start a link: {e}");
        }

    }
}

/// The thread of a link accepted by a link listener, until it closes.
fn listener_link_thread(mut stream: TcpStream, id: LinkId, shared: Arc<ListenerShared>) {

    let Some(addr) = hello(&mut stream) else { return };
    let Ok(write_stream) = stream.try_clone() else { return };

    shared.links.lock().unwrap().insert(id, ListenerLink::Tcp(write_stream));
    info!(%addr, ?id, "Linked with an app below");

    // Events fail to send once the link listener is dropped.
    let _ = shared.sender.send(LinkEvent::Linked(id));
    receive(&mut stream, addr, |message| {
        shared.sender.send(LinkEvent::Message(id, message)).is_ok()
    });
    let _ = shared.sender.send(LinkEvent::Unlinked(id));

    shared.links.lock().unwrap().remove(&id);
    let _ = stream.shutdown(Shutdown::Both);
    info!(%addr, ?id, "Unlinked from an app below");

}

/// Where a link stream sends to its link listener, while linked.
#[derive(Debug)]
enum StreamLink {
    Tcp(TcpStream),
    /// A link listener of this process, given its events directly.
    Local { id: LinkId, listener: Arc<ListenerShared> },
}

type SharedStream = Arc<Mutex<Option<StreamLink>>>;

/// The link to the app above, linking again whenever unlinked, until dropped.
#[derive(Debug)]
pub struct LinkStream {
    stopped: Arc<AtomicBool>,
    /// Where to send to the link listener, while linked.
    stream: SharedStream,
    events: Receiver<LinkEvent>,
}

impl LinkStream {

    /// Link with the app above listening at this address, in the background.
    pub fn connect(up_link: SocketAddr) -> io::Result<Self> {

        let stopped = Arc::new(AtomicBool::new(false));
        let stream = Arc::new(Mutex::new(None));
        let (sender, events) = crossbeam_channel::unbounded();
        info!("Linked to: {up_link}");

        let thread_stopped = Arc::clone(&stopped);
        let thread_stream = Arc::clone(&stream);
        thread::Builder::new()
            .name(format!("link to {up_link}"))
            .spawn(move || link_thread(up_link, thread_stopped, thread_stream, sender))?;

        Ok(Self { stopped, stream, events })

    }

    /// Link with this link listener of this process, through channels: linked right
    /// away, never linking again.
    pub fn connect_local(listener: &LinkListener) -> Self {

        let shared = &listener.shared;
        let id = LinkId(shared.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, events) = crossbeam_channel::unbounded();
        info!(?id, "Linked to: process");

        shared.links.lock().unwrap().insert(id, ListenerLink::Local(sender.clone()));
        let _ = sender.send(LinkEvent::Linked(()));
        let _ = shared.sender.send(LinkEvent::Linked(id));

        let stream = StreamLink::Local { id, listener: Arc::clone(shared) };
        Self {
            stopped: Arc::new(AtomicBool::new(true)),
            stream: Arc::new(Mutex::new(Some(stream))),
            events,
        }

    }

    /// Send a message on the link, failing if unlinked.
    pub fn send(&self, message: &Message) -> io::Result<()> {
        match self.stream.lock().unwrap().as_mut() {
            Some(StreamLink::Tcp(stream)) => stream.write_all(&message.encode()),
            Some(StreamLink::Local { id, listener }) => listener.sender.send(LinkEvent::Message(*id, message.clone()))
                .map_err(|_| io::ErrorKind::NotConnected.into()),
            None => Err(io::ErrorKind::NotConnected.into()),
        }
    }

    /// The next event, if any.
    pub fn try_event(&self) -> Option<LinkEvent> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting for it.
    pub fn event(&self) -> Option<LinkEvent> {
        self.events.recv().ok()
    }

}

impl Drop for LinkStream {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        match self.stream.lock().unwrap().take() {
            Some(StreamLink::Tcp(stream)) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
            Some(StreamLink::Local { id, listener }) => {
                listener.links.lock().unwrap().remove(&id);
                let _ = listener.sender.send(LinkEvent::Unlinked(id));
            }
            None => {}
        }
    }
}

/// The thread of a link stream, linking with the app above and again whenever
/// unlinked, until stopped.
fn link_thread(
    up_link: SocketAddr,
    stopped: Arc<AtomicBool>,
    shared: SharedStream,
    sender: Sender<LinkEvent>,
) {
    while !stopped.load(Ordering::Relaxed) {

        'link: {

            let mut stream = match TcpStream::connect(up_link) {
                Ok(stream) => stream,
                Err(e) => {
                    debug!(%up_link, "Failed to link with the app above: {e}");
                    break 'link;
                }
            };

            let Some(_) = hello(&mut stream) else { break 'link };
            let Ok(write_stream) = stream.try_clone() else { break 'link };

            *shared.lock().unwrap() = Some(StreamLink::Tcp(write_stream));
            info!(%up_link, "Linked with the app above");

            // Events fail to send once the link stream is dropped, then stopped.
            let _ = sender.send(LinkEvent::Linked(()));
            receive(&mut stream, up_link, |message| {
                sender.send(LinkEvent::Message((), message)).is_ok()
            });
            let _ = sender.send(LinkEvent::Unlinked(()));

            shared.lock().unwrap().take();
            info!(%up_link, "Unlinked from the app above");

        }

        thread::sleep(RELINK_DELAY);

    }
}

/// Give every message received on the link to `message` until the link closes, or
/// until `message` returns false.
fn receive(
    stream: &mut TcpStream,
    addr: SocketAddr,
    mut message: impl FnMut(Message) -> bool,
) {
    loop {

        let error = match read_frame(stream).and_then(|frame| Message::decode(&frame)) {
            Ok(received) => {
                if message(received) {
                    continue;
                }
                return;
            }
            Err(e) => e,
        };

        match error.kind() {
            io::ErrorKind::InvalidData => {
                warn!(%addr, "Received an invalid message: {error}");
            }
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset => return,
            _ => {
                debug!(%addr, "Failed to read from a link: {error}");
                return;
            }
        }

    }
}

/// The `Hello` frame, message 0, introducing both sides of a link with the protocol's
/// version.
const HELLO: [u8; 6] = [2, 0, 0, 0, 0, VERSION];

/// Exchange `Hello` on a new link, returning the other side's address if it speaks
/// this protocol's version.
fn hello(stream: &mut TcpStream) -> Option<SocketAddr> {
    let addr = stream.peer_addr().ok()?;
    let res = stream.set_nodelay(true)
        .and_then(|()| stream.write_all(&HELLO))
        .and_then(|()| read_frame(stream));
    match res {
        Ok(frame) if frame == HELLO[4..] => Some(addr),
        Ok(frame) => {
            warn!(%addr, "Refused a link with an invalid hello: {frame:?}");
            None
        }
        Err(e) => {
            warn!(%addr, "Failed to introduce a link: {e}");
            None
        }
    }
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let len = stream.read_u32()?;
    let mut frame = vec![0; len as usize];
    stream.read_exact(&mut frame)?;
    Ok(frame)
}

impl Message {

    /// The message's frame, with its length in front.
    fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0; 4];
        self.write(&mut buf).expect("writing to a vec");
        let len = (buf.len() - 4) as u32;
        buf[..4].copy_from_slice(&len.to_le_bytes());
        buf
    }

    fn write(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::BaseStatus { load } => {
                buf.write_u8(1)?;
                buf.write_u32(*load)?;
            }
            Self::BaseStop => buf.write_u8(2)?,
            Self::ClientHost { request_id, client } => {
                buf.write_u8(3)?;
                buf.write_u32(*request_id)?;
                buf.write_u32(client.login_key)?;
                buf.write_string_variable(&client.addr.to_string())?;
                buf.write_blob_variable(&client.blowfish_key)?;
                buf.write_string_variable(&client.name)?;
                write_addr(buf, client.real_base_app)?;
            }
            Self::ClientHosted { request_id, base_app } => {
                buf.write_u8(4)?;
                buf.write_u32(*request_id)?;
                write_addr(buf, *base_app)?;
            }
        }
        Ok(())
    }

    fn decode(mut buf: &[u8]) -> io::Result<Self> {
        let message = match buf.read_u8()? {
            1 => Self::BaseStatus { load: buf.read_u32()? },
            2 => Self::BaseStop,
            3 => Self::ClientHost {
                request_id: buf.read_u32()?,
                client: ClientHost {
                    login_key: buf.read_u32()?,
                    addr: parse(&buf.read_string_variable()?)?,
                    blowfish_key: buf.read_blob_variable()?,
                    name: buf.read_string_variable()?,
                    real_base_app: read_addr(&mut buf)?,
                },
            },
            4 => Self::ClientHosted { request_id: buf.read_u32()?, base_app: read_addr(&mut buf)? },
            id => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown message {id}"))),
        };
        if !buf.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "trailing bytes"));
        }
        Ok(message)
    }

}

fn write_addr(buf: &mut Vec<u8>, addr: Option<SocketAddrV4>) -> io::Result<()> {
    buf.write_string_variable(&addr.map(|addr| addr.to_string()).unwrap_or_default())
}

fn read_addr(buf: &mut &[u8]) -> io::Result<Option<SocketAddrV4>> {
    let addr = buf.read_string_variable()?;
    if addr.is_empty() { Ok(None) } else { parse(&addr).map(Some) }
}

fn parse<T: std::str::FromStr>(addr: &str) -> io::Result<T> {
    addr.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("invalid address {addr:?}")))
}


#[cfg(test)]
mod tests {

    use super::*;

    fn client(login_key: u32) -> ClientHost {
        ClientHost {
            login_key,
            addr: "127.0.0.1:51234".parse().unwrap(),
            blowfish_key: vec![1; 16],
            name: "Player".to_string(),
            real_base_app: Some("92.223.1.1:20017".parse().unwrap()),
        }
    }

    #[test]
    fn message_round_trip() {
        for message in [
            Message::BaseStatus { load: 3 },
            Message::BaseStop,
            Message::ClientHost { request_id: 7, client: client(0xDEADBEEF) },
            Message::ClientHosted { request_id: 7, base_app: None },
            Message::ClientHosted { request_id: 7, base_app: Some("127.0.0.1:20017".parse().unwrap()) },
        ] {
            let frame = message.encode();
            assert_eq!(u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize, frame.len() - 4);
            assert_eq!(Message::decode(&frame[4..]).unwrap(), message);
        }
    }

    #[test]
    fn tcp_events() {
        let listener = LinkListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let stream = LinkStream::connect(listener.addr().unwrap()).unwrap();
        events(listener, stream);
    }

    #[test]
    fn local_events() {
        let listener = LinkListener::local();
        let stream = LinkStream::connect_local(&listener);
        events(listener, stream);
    }

    /// Both sides get the events of the link and the messages of the other.
    fn events(listener: LinkListener, stream: LinkStream) {

        const WAIT: Duration = Duration::from_secs(5);

        let Some(LinkEvent::Linked(id)) = listener.event_timeout(WAIT) else { panic!("not linked") };
        assert_eq!(stream.event(), Some(LinkEvent::Linked(())));

        stream.send(&Message::BaseStatus { load: 1 }).unwrap();
        assert_eq!(listener.event_timeout(WAIT), Some(LinkEvent::Message(id, Message::BaseStatus { load: 1 })));

        let host = Message::ClientHost { request_id: 3, client: client(1) };
        listener.send(id, &host).unwrap();
        assert_eq!(stream.event(), Some(LinkEvent::Message((), host)));

        drop(stream);
        assert_eq!(listener.event_timeout(WAIT), Some(LinkEvent::Unlinked(id)));
        assert!(listener.send(id, &Message::BaseStop).is_err());

    }

}
