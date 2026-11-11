//! The server's login app, logging every client in.

use std::net::{SocketAddr, SocketAddrV4};
use std::collections::HashMap;
use std::time::Instant;
use std::sync::Arc;

use tracing::{info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use rsa::RsaPrivateKey;

use wgtk::app::login::element::LoginError;
use wgtk::app::login;

use crate::link::{ClientHost, LinkListener, LINK_POLL};
use crate::CliResult;

use super::link::LoginLinkListener;

#[derive(Debug)]
pub struct LoginThread {
    app: login::App,
    link_listener: LoginLinkListener,
    login_challenges: HashMap<SocketAddr, bool>,
    /// The name of the players logging in without a username, by token.
    name: String,
}

impl LoginThread {

    /// The login app, at this address, sending its clients to these base apps.
    pub fn new(
        addr: SocketAddrV4,
        encryption_key: Option<Arc<RsaPrivateKey>>,
        link_listener: LinkListener,
        name: &str,
    ) -> CliResult<Self> {

        let mut app = login::App::new(addr.into())
            .map_err(|e| format!("Failed to bind login app: {e}"))?;

        if let Some(encryption_key) = encryption_key {
            app.set_encryption(encryption_key);
        }

        Ok(Self {
            app,
            link_listener: LoginLinkListener::new(link_listener),
            login_challenges: HashMap::new(),
            name: name.to_string(),
        })

    }

    #[instrument(name = "login", skip_all)]
    pub fn run(mut self) {

        use login::Event;

        info!("Running on: {}", self.app.addr().unwrap());

        if self.app.has_encryption() {
            info!("Enabled encryption");
        }

        loop {

            while let Some(hosted) = self.link_listener.poll() {
                let addr = hosted.client.addr;
                match hosted.base_app {
                    Some(base_app) => {
                        info!(%addr, %base_app, name = %hosted.client.name, "Logged in, handed to a base app");
                        self.app.answer_login_success(addr, base_app, hosted.client.login_key, String::new());
                    }
                    None => {
                        warn!(%addr, "Refused a login: no base app to hand the client to");
                        self.app.answer_login_error(addr, LoginError::NoBaseApp, String::new());
                    }
                }
            }

            match self.app.poll_until(Instant::now() + LINK_POLL) {
                Event::Timeout => {}
                Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Failed to talk with a client: {}", error.error);
                    } else {
                        warn!("Failed to use the socket: {}", error.error);
                    }
                }
                Event::Ping(ping) => {
                    info!(addr = %ping.addr, latency = ?ping.latency, "Answered a ping");
                }
                Event::Login(login) => {

                    if !*self.login_challenges.entry(login.addr).or_default() {
                        info!(addr = %login.addr, "Sent a login challenge");
                        self.app.answer_login_challenge(login.addr);
                    } else {

                        // Answered once a base app hosts the client: it knows of the
                        // client before the client knows of it.
                        info!(addr = %login.addr, "Accepted a login, handing the client to a base app");
                        self.link_listener.host_client(ClientHost {
                            login_key: OsRng.next_u32(),
                            addr: login.addr,
                            blowfish_key: login.request.blowfish_key.clone(),
                            name: player_name(&login.request.username, &self.name),
                            real_base_app: None,
                        });

                        // app.answer_login_error(login.addr, LoginError::Banned, "{\"bans\":\"{\\\"expiryTime\\\":1726435530,\\\"reason\\\":\\\"It's the reason\\\"}\"}".to_string());
                        
                    }
                
                }
                Event::Challenge(challenge) => {
                    info!(addr = %challenge.addr, "Completed a login challenge");
                    if let Some(completed) = self.login_challenges.get_mut(&challenge.addr) {
                        *completed = true;
                    }
                }
            }

        }

    }

}

/// The name a client logging in with this username plays as: its username up to any `@`,
/// as the game's are e-mails, else the given one, logging in by token.
fn player_name(username: &str, default: &str) -> String {
    let name = username.split('@').next().unwrap_or_default().trim();
    if name.is_empty() {
        default.to_string()
    } else {
        name.to_string()
    }
}
