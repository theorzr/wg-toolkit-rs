//! The server's login app, logging every client in.

use std::net::{SocketAddr, SocketAddrV4};
use std::collections::HashMap;
use std::sync::Arc;

use tracing::{info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use rsa::RsaPrivateKey;

use wgtk::app::login::element::LoginError;
use wgtk::app::login;

use crate::link::{ClientHost, LinkListener};
use crate::CliResult;

use super::link::LoginLinkListener;


#[derive(Debug)]
pub struct LoginThread {
    app: login::App,
    link_listener: LoginLinkListener,
    login_challenges: HashMap<SocketAddr, bool>,
    /// The name of every player, until accounts are stored.
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
            info!("Encryption enabled");
        }

        loop {
            match self.app.poll() {
                Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Error: {}", error.error);
                    } else {
                        warn!("Error: {}", error.error);
                    }
                }
                Event::Ping(ping) => {
                    info!(addr = %ping.addr, "Ping-Pong: {:?}", ping.latency);
                }
                Event::Login(login) => {

                    if !*self.login_challenges.entry(login.addr).or_default() {
                        info!(addr = %login.addr, "Login pending, sending challenge");
                        self.app.answer_login_challenge(login.addr);
                    } else {

                        info!(addr = %login.addr, "Login success, as {}", self.name);

                        // The base app knows of the client before the client knows of it.
                        let login_key = OsRng.next_u32();
                        let client = ClientHost {
                            login_key,
                            addr: login.addr,
                            blowfish_key: login.request.blowfish_key.clone(),
                            name: self.name.clone(),
                            real_base_app: None,
                        };
                        let Some(base_app) = self.link_listener.host_client(client) else {
                            warn!(addr = %login.addr, "No base app to hand the client to");
                            self.app.answer_login_error(login.addr, LoginError::NoBaseApp, String::new());
                            continue;
                        };

                        self.app.answer_login_success(login.addr, base_app, login_key, String::new());

                        // app.answer_login_error(login.addr, LoginError::Banned, "{\"bans\":\"{\\\"expiryTime\\\":1726435530,\\\"reason\\\":\\\"It's the reason\\\"}\"}".to_string());
                        
                    }
                
                }
                Event::Challenge(challenge) => {
                    info!(addr = %challenge.addr, "Challenge successful...");
                    if let Some(completed) = self.login_challenges.get_mut(&challenge.addr) {
                        *completed = true;
                    }
                }
            }
        }

    }

}
