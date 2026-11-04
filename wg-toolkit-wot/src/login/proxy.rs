//! The login proxy, forwarding every client to the real login app.

use std::net::{SocketAddr, SocketAddrV4};
use std::time::Duration;
use std::sync::Arc;
use std::io;

use rsa::{RsaPrivateKey, RsaPublicKey};
use tracing::{error, info, info_span};

use wgtk::net::codec::WgSocketAddrV4;
use wgtk::app::login_proxy;

use crate::link::{ClientHost, LinkListener};
use crate::CliResult;

use super::link::LoginLinkListener;


/// The login app of the proxy, forwarding to the real login app and sending its
/// clients to the base proxy.
#[derive(Debug)]
pub struct LoginProxy {
    app: login_proxy::App,
    handler: LoginHandler,
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

        Ok(Self { app, handler: LoginHandler { link_listener: LoginLinkListener::new(link_listener) } })

    }

    pub fn run(mut self) {
        let _span = info_span!("login").entered();
        if let Err(e) = self.app.run(self.handler) {
            error!("Unexpected hard error: ({}) {e}", e.kind());
        }
    }

}

#[derive(Debug)]
struct LoginHandler {
    link_listener: LoginLinkListener,
}

impl login_proxy::Handler for LoginHandler {

    type Error = io::Error;

    fn receive_ping(&mut self,
        addr: SocketAddr,
        latency: Duration,
    ) -> Result<(), Self::Error> {
        info!(%addr, "Ping-Pong: {:?}", latency);
        Ok(())
    }

    fn receive_login_success(&mut self,
        addr: SocketAddr,
        blowfish_key: &[u8],
        base_app: WgSocketAddrV4,
        login_key: u32,
        _server_message: String,
    ) -> Result<WgSocketAddrV4, Self::Error> {

        info!(%addr, %base_app, "Login success");
        let client = ClientHost {
            login_key,
            addr,
            blowfish_key: blowfish_key.to_vec(),
            name: String::new(),
            real_base_app: Some(base_app.addr),
        };

        // Return the base proxy's address instead of the real one!
        self.link_listener.host_client_wait(client)
            .map(Into::into)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no base proxy to hand the client to"))

    }

    fn receive_login_error(&mut self,
        addr: SocketAddr,
        error: login_proxy::element::LoginError,
        data: String,
    ) -> Result<(), Self::Error> {
        info!(%addr, "Login error: {:?} ({data:?})", error);
        Ok(())
    }



}
