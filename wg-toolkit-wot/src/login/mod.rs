//! The login app, logging clients in or forwarding them to the real login app, then
//! handing them over to a base app.

mod link;
mod server;
mod proxy;

use crate::{init_tracing, AppLoginArgs, CliResult};
use crate::link::LinkListener;

pub use server::LoginThread;
pub use proxy::LoginProxy;


pub fn run(args: AppLoginArgs) -> CliResult<()> {

    init_tracing("login-trace.jsonl")?;

    let encryption_key = args.login.encryption_key()?;
    let link_listener = LinkListener::bind(args.bind_link)
        .map_err(|e| format!("Failed to bind the link at {}: {e}", args.bind_link))?;

    if let Some(real_login_app) = args.real_login_app {
        let real_encryption_key = args.real.encryption_key()?;
        LoginProxy::new(args.bind, encryption_key, real_login_app, real_encryption_key, link_listener)?.run();
    } else {
        LoginThread::new(args.bind, encryption_key, link_listener, &args.name)?.run();
    }

    Ok(())

}
