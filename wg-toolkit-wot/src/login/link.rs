//! The login app's side of its links with the base apps.

use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::net::SocketAddrV4;

use tracing::{debug, info, warn};
use rand::seq::SliceRandom;

use crate::link::{ClientHost, LinkEvent, LinkId, LinkListener, Message};


/// How long a login app waits for a base app to answer a request.
const REPLY_WAIT: Duration = Duration::from_secs(2);

/// The links of a login app with the base apps, registered once they tell their status.
#[derive(Debug)]
pub struct LoginLinkListener {
    link_listener: LinkListener,
    /// The load of every registered base app.
    loads: HashMap<LinkId, u32>,
    next_request_id: u32,
}

impl LoginLinkListener {

    pub fn new(link_listener: LinkListener) -> Self {
        Self { link_listener, loads: HashMap::new(), next_request_id: 0 }
    }

    /// Have a base app host this client, trying each in a random order, returning the
    /// address of the one hosting it, if any. The login apps are not event-based yet,
    /// so this waits for each reply.
    pub fn host_client(&mut self, client: ClientHost) -> Option<SocketAddrV4> {

        while let Some(event) = self.link_listener.try_event() {
            self.handle(event);
        }

        let mut links = self.loads.keys().copied().collect::<Vec<_>>();
        links.shuffle(&mut rand::thread_rng());

        for link in links {

            let request_id = self.next_request_id;
            self.next_request_id = self.next_request_id.wrapping_add(1);
            let message = Message::ClientHost { request_id, client: client.clone() };
            if let Err(e) = self.link_listener.send(link, &message) {
                debug!(?link, "Failed to send to a base app: {e}");
                continue;
            }

            let deadline = Instant::now() + REPLY_WAIT;
            while let Some(event) = self.link_listener.event_timeout(deadline.saturating_duration_since(Instant::now())) {
                if let Some((id, base_app)) = self.handle(event) {
                    if id == request_id {
                        if base_app.is_some() {
                            return base_app;
                        }
                        break;
                    }
                }
            }

        }

        None

    }

    /// Handle a link event, returning the reply it brings to a `ClientHost` request.
    fn handle(&mut self, event: LinkEvent<LinkId>) -> Option<(u32, Option<SocketAddrV4>)> {
        match event {
            LinkEvent::Linked(_) => {}
            LinkEvent::Message(link, Message::BaseStatus { load }) => {
                if self.loads.insert(link, load).is_none() {
                    info!(?link, load, "Base app registered");
                }
            }
            LinkEvent::Message(link, Message::BaseStop) | 
            LinkEvent::Unlinked(link) => {
                if self.loads.remove(&link).is_some() {
                    info!(?link, "Base app unregistered");
                }
            }
            LinkEvent::Message(_, Message::ClientHosted { request_id, base_app }) => {
                return Some((request_id, base_app));
            }
            LinkEvent::Message(link, message) => warn!(?link, "Unexpected message from a base app: {message:?}"),
        }
        None
    }

}
