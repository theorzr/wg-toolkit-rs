//! The emulator implementation of the login + base applications, we are trying to
//! reproduce the official server implementation.

use std::collections::{hash_map, HashMap};
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::thread;

use tracing::{debug, info, instrument, warn};
use rsa::rand_core::{OsRng, RngCore};
use wgtk::app::{login, base};
use wgtk::script::Script;
use rsa::RsaPrivateKey;
use blowfish::Blowfish;

mod capture;

use std::path::Path;
use std::io;

use wgtk::app::dispatch::MethodCall;
use wgtk::script::Value;

use capture::{Answer, CapturedSession};
use crate::CliResult;


pub fn run(
    login_app_addr: SocketAddrV4,
    base_app_addr: SocketAddrV4,
    encryption_key: Option<Arc<RsaPrivateKey>>,
    script: Script,
    from_capture: Option<&Path>,
) -> CliResult<()> {

    let mut login_app = login::App::new(login_app_addr.into())
        .map_err(|e| format!("Failed to bind login app: {e}"))?;

    if let Some(encryption_key) = encryption_key {
        login_app.set_encryption(encryption_key);
    }

    let base_app = base::App::new(base_app_addr.into(), script)
        .map_err(|e| format!("Failed to bind base app: {e}"))?;

    let session = match from_capture {
        Some(path) => {
            let session = CapturedSession::load(path, base_app.dispatch())
                .map_err(|e| format!("Failed to extract the session from {}: {e}", path.display()))?;
            info!("Serving from capture {}: account {}, {} requests",
                path.display(), if session.account.is_some() { "found" } else { "missing" }, session.requests.len());
            for request in &session.requests {
                let answers: Vec<String> = request.answers.iter().map(|answer| match answer {
                    Answer::Call { method, .. } => method.to_string(),
                    Answer::Resource { data, .. } => format!("resource ({} bytes)", data.len()),
                }).collect();
                debug!("  {} {:?} -> {}", request.method, request.args.get(..2), answers.join(", "));
            }
            Some(session)
        }
        None => None,
    };

    let shared = Arc::new(Shared {
        login_clients: Mutex::new(HashMap::new()),
    });

    let login_thread = LoginThread {
        app: login_app,
        shared: Arc::clone(&shared),
        base_app_addr,
        login_challenges: HashMap::new(),
    };

    let base_thread = BaseThread {
        app: base_app,
        shared,
        session,
        players: HashMap::new(),
    };

    thread::scope(move |scope| {
        scope.spawn(move || login_thread.run());
        scope.spawn(move || base_thread.run());
    });

    Ok(())

}

#[derive(Debug)]
struct LoginThread {
    app: login::App,
    shared: Arc<Shared>,
    base_app_addr: SocketAddrV4,
    login_challenges: HashMap<SocketAddr, bool>,
}

#[derive(Debug)]
struct BaseThread {
    app: base::App,
    shared: Arc<Shared>,
    /// What to serve, when running from a capture.
    session: Option<CapturedSession>,
    /// Each client's player entity.
    players: HashMap<SocketAddr, base::Handle>,
}

#[derive(Debug)]
struct Shared {
    login_clients: Mutex<HashMap<u32, LoginClient>>,
}

#[derive(Debug)]
struct LoginClient {
    addr: SocketAddr,
    blowfish: Arc<Blowfish>,
}

impl LoginThread {

    #[instrument(name = "login", skip_all)]
    fn run(mut self) {

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

                        info!(addr = %login.addr, "Login success");

                        let mut clients = self.shared.login_clients.lock().unwrap();
                        let (login_key, slot) = loop {
                            let login_key = OsRng.next_u32();
                            match clients.entry(login_key) {
                                hash_map::Entry::Occupied(_) => continue,
                                hash_map::Entry::Vacant(v) => break (login_key, v),
                            }
                        };

                        let blowfish = self.app.answer_login_success(login.addr, self.base_app_addr, login_key, String::new()).unwrap();

                        slot.insert(LoginClient {
                            addr: login.addr,
                            blowfish,
                        });

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

impl BaseThread {

    /// Bring a client that enabled entities to its account, as the captured server did:
    /// the `Account` entity, then `showGUI`.
    fn enter_account(&mut self, addr: SocketAddr) -> io::Result<()> {

        let Some(session) = &self.session else { return Ok(()) };

        let Some((data, components_count)) = &session.account else { return Ok(()) };
        let handle = self.app.create_base_player(addr, "Account", data.clone(), *components_count)?;
        self.players.insert(addr, handle);
        info!(%addr, "Created account {}", handle.entity_id());

        if let Some(args) = &session.show_gui {
            self.app.select_player_entity(handle)?;
            self.app.call_method(handle, "showGUI", args.clone())?;
        }

        Ok(())

    }

    /// Answer a client request with what the captured server answered to the closest
    /// captured request, with the new request's id in place of the captured one.
    fn answer(&mut self, addr: SocketAddr, method: &str, args: &[Value]) -> io::Result<()> {

        let (Some(session), Some(&handle)) = (&self.session, self.players.get(&addr)) else { return Ok(()) };
        let Some(request_id) = args.first().and_then(Value::as_i64) else { return Ok(()) };

        let Some(captured) = session.find_request(method, args) else {
            warn!(%addr, "No captured answer to {method} {args:?}");
            return Ok(());
        };

        for answer in &captured.answers {
            match answer {
                Answer::Call { method, args } => {
                    let mut args = args.clone();
                    if let Some(first) = args.first_mut() {
                        *first = with_request_id(first, request_id);
                    }
                    info!(%addr, "Answering with {method} {}", format!("{args:?}").chars().take(200).collect::<String>());
                    self.app.select_player_entity(handle)?;
                    self.app.call_method(handle, method, args)?;
                }
                Answer::Resource { description, data } => {
                    info!(%addr, "Answering with resource {request_id}, {} bytes", data.len());
                    self.app.push_resource(addr, request_id as u16, description.clone(), data)?;
                }
            }
        }

        Ok(())

    }

    #[instrument(name = "base", skip_all)]
    fn run(mut self) {

        info!("Running on: {}", self.app.addr().unwrap());

        loop {

            match self.app.poll() {
                base::Event::IoError(error) => {
                    if let Some(addr) = error.addr {
                        warn!(%addr, "Error: {}", error.error);
                    } else {
                        warn!("Error: {}", error.error);
                    }
                }
                base::Event::Login(login) => {
                    
                    let mut clients = self.shared.login_clients.lock().unwrap();
                    let client = match clients.remove(&login.login_key) {
                        Some(client) => client,
                        None => {
                            info!(addr = %login.addr, "Login #{}... Invalid key", login.attempt_num);
                            continue;
                        }
                    };

                    if client.addr != login.addr {
                        info!(addr = %login.addr, "Login #{}... Invalid address", login.attempt_num);
                        continue;
                    }
                    
                    info!(addr = %login.addr, "Login #{}... Success", login.attempt_num);
                    self.app.answer_login_success(login.addr, client.blowfish);

                    // Right after the login reply, as the captured server did.
                    if let Some((frequency, unknown)) = self.session.as_ref().and_then(|s| s.update_frequency) {
                        if let Err(e) = self.app.update_frequency(login.addr, frequency, unknown, 0) {
                            warn!(addr = %login.addr, "Failed to send the update frequency: {e}");
                        }
                    }

                }
                base::Event::EnableEntities(event) => {
                    info!(addr = %event.addr, "Entities enabled");
                    if let Err(e) = self.enter_account(event.addr) {
                        warn!(addr = %event.addr, "Failed to create the account: {e}");
                    }
                }
                base::Event::BaseMethod(event) => {
                    match &event.call {
                        MethodCall::Known { name, args } => {
                            info!(addr = %event.addr, "Base method call on entity {}: {name} {args:?}", event.entity_id);
                            if let Err(e) = self.answer(event.addr, name, args) {
                                warn!(addr = %event.addr, "Failed to answer {name}: {e}");
                            }
                        }
                        MethodCall::Unknown { .. } =>
                            warn!(addr = %event.addr, "Base method call on entity {}: {:?}", event.entity_id, event.call),
                    }
                }
                base::Event::CellMethod(event) => {
                    info!(addr = %event.addr, "Cell method call on entity {}", event.entity_id);
                }

            }

            // let entity: Handle<entity::Login> = self.app.create_base_player(addr, entity::Login {
            //     accountDBID_s: "09518858105".to_string(),
            //     loginPriority: 0,
            // });

            // self.app.call_method(entity, entity::Login_Client::setPeripheryRoutingGroup());
            // self.app.reset_entities(addr);

        }

    }

}

/// The same integer value type as `value`, holding `id`: a request id must keep the
/// type the method declares.
fn with_request_id(value: &Value, id: i64) -> Value {
    match value {
        Value::Int8(_) => Value::Int8(id as i8),
        Value::Int16(_) => Value::Int16(id as i16),
        Value::Int32(_) => Value::Int32(id as i32),
        Value::Int64(_) => Value::Int64(id),
        Value::UInt8(_) => Value::UInt8(id as u8),
        Value::UInt16(_) => Value::UInt16(id as u16),
        Value::UInt32(_) => Value::UInt32(id as u32),
        Value::UInt64(_) => Value::UInt64(id as u64),
        other => other.clone(),
    }
}
