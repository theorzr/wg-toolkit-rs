//! The login app's side of its links with the base apps.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use std::net::SocketAddrV4;

use tracing::{debug, info, warn};
use rand::seq::SliceRandom;

use crate::link::{ClientHost, LinkEvent, LinkId, LinkListener, Message};


/// How long a login app waits for a base app to answer a request, before asking
/// another.
const REPLY_WAIT: Duration = Duration::from_secs(2);

/// The links of a login app with the base apps, registered once they tell their status.
#[derive(Debug)]
pub struct LoginLinkListener {
    link_listener: LinkListener,
    /// The load of every registered base app.
    loads: HashMap<LinkId, u32>,
    next_request_id: u32,
    /// The clients being hosted, by the id of the request waiting for a reply.
    hostings: HashMap<u32, Hosting>,
    events: VecDeque<ClientHosted>,
}

/// A client being hosted.
#[derive(Debug)]
struct Hosting {
    client: ClientHost,
    /// The base app asked.
    link: LinkId,
    /// The base apps left to ask if this one does not host the client.
    links: Vec<LinkId>,
    deadline: Instant,
}

/// A client hosted by a base app at this address, or by none.
#[derive(Debug)]
pub struct ClientHosted {
    pub client: ClientHost,
    pub base_app: Option<SocketAddrV4>,
}

impl LoginLinkListener {

    pub fn new(link_listener: LinkListener) -> Self {
        Self {
            link_listener,
            loads: HashMap::new(),
            next_request_id: 0,
            hostings: HashMap::new(),
            events: VecDeque::new(),
        }
    }

    /// Have a base app host this client, asking each in a random order, the result
    /// coming as an event, see [`Self::poll`].
    pub fn host_client(&mut self, client: ClientHost) {
        if self.hostings.values().any(|hosting| hosting.client.addr == client.addr) {
            debug!(addr = %client.addr, "The client is already being hosted");
            return;
        }
        let mut links = self.loads.keys().copied().collect::<Vec<_>>();
        links.shuffle(&mut rand::thread_rng());
        self.ask_next(client, links);
    }

    /// The next event, if any.
    pub fn poll(&mut self) -> Option<ClientHosted> {

        while let Some(event) = self.link_listener.try_event() {
            self.handle(event);
        }

        let now = Instant::now();
        let expired = self.hostings.iter()
            .filter(|(_, hosting)| hosting.deadline <= now)
            .map(|(&request_id, _)| request_id)
            .collect::<Vec<_>>();

        for request_id in expired {
            let hosting = self.hostings.remove(&request_id).unwrap();
            debug!(link = ?hosting.link, "A base app did not answer in time");
            self.ask_next(hosting.client, hosting.links);
        }

        self.events.pop_front()

    }

    /// Have a base app host this client, waiting for the result: the login proxy is
    /// not event-based yet.
    pub fn host_client_wait(&mut self, client: ClientHost) -> Option<SocketAddrV4> {

        let addr = client.addr;
        self.host_client(client);

        loop {

            if let Some(hosted) = self.poll() {
                if hosted.client.addr == addr {
                    return hosted.base_app;
                }
                continue;
            }

            let deadline = self.hostings.values()
                .find(|hosting| hosting.client.addr == addr)
                .map(|hosting| hosting.deadline)?;

            if let Some(event) = self.link_listener.event_timeout(deadline.saturating_duration_since(Instant::now())) {
                self.handle(event);
            }

        }

    }

    /// Ask the next of these base apps to host the client, or give the event that
    /// none does if none is left.
    fn ask_next(&mut self, client: ClientHost, mut links: Vec<LinkId>) {

        while let Some(link) = links.pop() {

            let request_id = self.next_request_id;
            self.next_request_id = self.next_request_id.wrapping_add(1);

            let message = Message::ClientHost { request_id, client: client.clone() };
            if let Err(e) = self.link_listener.send(link, &message) {
                debug!(?link, "Failed to send to a base app: {e}");
                continue;
            }

            let deadline = Instant::now() + REPLY_WAIT;
            self.hostings.insert(request_id, Hosting { client, link, links, deadline });
            return;

        }

        self.events.push_back(ClientHosted { client, base_app: None });

    }

    fn handle(&mut self, event: LinkEvent<LinkId>) {
        match event {
            LinkEvent::Linked(_) => {}
            LinkEvent::Message(link, Message::BaseStatus { load }) => {
                if self.loads.insert(link, load).is_none() {
                    info!(?link, load, "Base app registered");
                }
            }
            LinkEvent::Message(link, Message::BaseStop) | LinkEvent::Unlinked(link) => {
                self.unregister(link);
            }
            LinkEvent::Message(_, Message::ClientHosted { request_id, base_app }) => {
                if let Some(hosting) = self.hostings.remove(&request_id) {
                    match base_app {
                        Some(_) => self.events.push_back(ClientHosted { client: hosting.client, base_app }),
                        None => self.ask_next(hosting.client, hosting.links),
                    }
                }
            }
            LinkEvent::Message(link, message) => {
                warn!(?link, "Unexpected message from a base app: {message:?}");
            }
        }
    }

    /// Unregister this base app, asking the next base app for the clients it was
    /// asked to host.
    fn unregister(&mut self, link: LinkId) {

        if self.loads.remove(&link).is_some() {
            info!(?link, "Base app unregistered");
        }

        let request_ids = self.hostings.iter()
            .filter(|(_, hosting)| hosting.link == link)
            .map(|(&request_id, _)| request_id)
            .collect::<Vec<_>>();

        for request_id in request_ids {
            let hosting = self.hostings.remove(&request_id).unwrap();
            self.ask_next(hosting.client, hosting.links);
        }

    }

}


#[cfg(test)]
mod tests {

    use crate::link::LinkStream;

    use super::*;

    fn client(port: u16) -> ClientHost {
        ClientHost {
            login_key: 1,
            addr: format!("127.0.0.1:{port}").parse().unwrap(),
            blowfish_key: vec![1; 16],
            name: "Player".to_string(),
            real_base_app: None,
        }
    }

    /// Answer every `ClientHost` of this base app, hosting the client if it's the given
    /// answer.
    fn answer(stream: &LinkStream, base_app: Option<SocketAddrV4>) {
        while let Some(event) = stream.try_event() {
            if let LinkEvent::Message((), Message::ClientHost { request_id, .. }) = event {
                stream.send(&Message::ClientHosted { request_id, base_app }).unwrap();
            }
        }
    }

    #[test]
    fn host_client() {

        let listener = LinkListener::local();
        let full = LinkStream::connect_local(&listener);
        let free = LinkStream::connect_local(&listener);
        let mut login = LoginLinkListener::new(listener);

        // No base app registered yet.
        login.host_client(client(1));
        let hosted = login.poll().unwrap();
        assert_eq!((hosted.client, hosted.base_app), (client(1), None));

        full.send(&Message::BaseStatus { load: 0 }).unwrap();
        free.send(&Message::BaseStatus { load: 0 }).unwrap();
        assert!(login.poll().is_none());

        // Whichever is asked first, the full one refuses and the other hosts.
        let base_app = Some("127.0.0.1:20017".parse().unwrap());
        login.host_client(client(2));
        login.host_client(client(2));
        let mut hosted = None;
        for _ in 0..2 {
            answer(&full, None);
            answer(&free, base_app);
            hosted = login.poll();
            if hosted.is_some() {
                break;
            }
        }
        let hosted = hosted.unwrap();
        assert_eq!((hosted.client, hosted.base_app), (client(2), base_app));
        assert!(login.poll().is_none());

    }

}
