//! The apps usually run together, in this process.

pub mod server;
pub mod proxy;

use crate::link::LinkListener;
use crate::CliResult;


/// The login app's link, on a free local port, for the base app of this process.
fn local_link_listener() -> CliResult<LinkListener> {
    LinkListener::bind("127.0.0.1:0".parse().unwrap()).map_err(|e| format!("Failed to bind the link: {e}"))
}
