//! The server's login app and base app, in this process.

use std::path::Path;
use std::thread;

use crate::{init_tracing, load_script, CliResult, ServerArgs};
use crate::login::LoginThread;
use crate::base::BaseThread;

use super::local_link_listener;


pub fn run(args: ServerArgs, game_dir: &Path) -> CliResult<()> {

    init_tracing("server-trace.jsonl")?;

    let script = load_script(game_dir)?;
    let encryption_key = args.login.encryption_key()?;

    let link_listener = local_link_listener()?;
    let base = BaseThread::new(args.apps.bind_base_app, script, game_dir, &args.base.give_vehicle, &args.base.map, link_listener.addr())?;
    let login = LoginThread::new(args.apps.bind_login_app, encryption_key, link_listener, &args.name)?;

    thread::scope(move |scope| {
        thread::Builder::new().name("login".to_string()).spawn_scoped(scope, move || login.run())
            .map_err(|e| format!("Failed to start the login app: {e}"))?;
        thread::Builder::new().name("base".to_string()).spawn_scoped(scope, move || base.run())
            .map_err(|e| format!("Failed to start the base app: {e}"))?;
        Ok(())
    })

}
