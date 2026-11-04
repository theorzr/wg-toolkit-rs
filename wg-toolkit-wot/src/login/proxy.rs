//! The login proxy, forwarding every client to the real login app.

use std::net::SocketAddrV4;
use std::time::Instant;
use std::sync::Arc;

use rsa::{RsaPrivateKey, RsaPublicKey};
use tracing::{info, instrument, warn};

use wgtk::app::login_proxy::element::LoginError;
use wgtk::app::login_proxy;

use crate::link::{ClientHost, LinkListener};
use crate::CliResult;

use super::link::{LoginLinkListener, LINK_POLL};


/// The login app of the proxy, forwarding to the real login app and sending its
/// clients to the base proxy.
#[derive(Debug)]
pub struct LoginProxy {
    app: login_proxy::App,
    link_listener: LoginLinkListener,
}

impl LoginProxy {

    /// The login proxy at this address, sending the clients of the real login app to
    /// the base proxies among these base apps.
    pub fn new(
        addr: SocketAddrV4,
        encryption_key: Option<Arc<RsaPrivateKey>>,
        real_login_app: SocketAddrV4,
        real_encryption_key: Option<Arc<RsaPublicKey>>,
        link_listener: LinkListener,
    ) -> CliResult<Self> {

        let mut app = login_proxy::App::new(addr.into(), real_login_app.into(), real_encryption_key)
            .map_err(|e| format!("Failed to bind login app: {e}"))?;

        if let Some(encryption_key) = encryption_key {
            app.set_encryption(encryption_key);
        }

        Ok(Self { app, link_listener: LoginLinkListener::new(link_listener) })

    }

    #[instrument(name = "login", skip_all)]
    pub fn run(mut self) {

        use login_proxy::Event;

        info!("Running on: {}", self.app.addr().unwrap());

        loop {

            while let Some(hosted) = self.link_listener.poll() {
                let addr = hosted.client.addr;
                match hosted.base_app {
                    // Return the base proxy's address instead of the real one!
                    Some(base_app) => {
                        info!(%addr, %base_app, "Login success, handed to the base proxy");
                        self.app.answer_login_success(addr, base_app);
                    }
                    None => {
                        warn!(%addr, "No base proxy to hand the client to");
                        self.app.answer_login_error(addr, LoginError::NoBaseApp, String::new());
                    }
                }
            }

            match self.app.poll_until(Instant::now() + LINK_POLL) {
                Event::Timeout => {}
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
                Event::LoginSuccess(success) => {
                    info!(addr = %success.addr, base_app = %success.base_app, "Login success, handing the client to a base proxy");
                    self.link_listener.host_client(ClientHost {
                        login_key: success.login_key,
                        addr: success.addr,
                        blowfish_key: success.blowfish_key,
                        name: String::new(),
                        real_base_app: Some(success.base_app),
                    });
                }
                Event::LoginError(error) => {
                    info!(addr = %error.addr, "Login error: {:?} ({:?})", error.error, error.data);
                }
            }

        }

    }

}
