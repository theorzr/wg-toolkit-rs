//! The login proxy and the base proxy, in this process.

use std::path::Path;
use std::thread;

use crate::{init_tracing, load_script, CliResult, ProxyArgs};
use crate::link::{LinkListener, LinkStream};
use crate::login::LoginProxy;
use crate::proxy::BaseProxy;


pub fn run(args: ProxyArgs, game_dir: &Path) -> CliResult<()> {

    init_tracing("proxy-trace.jsonl")?;

    let script = load_script(game_dir)?;
    let encryption_key = args.login.encryption_key()?;
    let real_encryption_key = args.real.encryption_key()?;

    let link_listener = LinkListener::local();
    let link = LinkStream::connect_local(&link_listener);
    let base = BaseProxy::new(args.apps.bind_base_app, args.real_login_app, script, args.capture.capture.as_deref(), link)?;
    let login = LoginProxy::new(args.apps.bind_login_app, encryption_key, args.real_login_app, real_encryption_key, link_listener)?;

    thread::scope(move |scope| {
        thread::Builder::new().name("login".to_string()).spawn_scoped(scope, move || login.run())
            .map_err(|e| format!("Failed to start the login app: {e}"))?;
        thread::Builder::new().name("base".to_string()).spawn_scoped(scope, move || base.run())
            .map_err(|e| format!("Failed to start the base app: {e}"))?;
        Ok(())
    })

}
