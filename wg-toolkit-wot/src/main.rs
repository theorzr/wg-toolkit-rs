//! A World of Tanks server built on wg-toolkit.

mod login;
mod base;
mod proxy;
mod cluster;
mod replay;
mod link;
mod wot;

use std::net::{SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::fs::File;
use std::fs;

use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};
use tracing::level_filters::LevelFilter;
use rsa::{RsaPrivateKey, RsaPublicKey};
use clap::{Args, Parser, Subcommand};

use wgtk::res::fs::ResFilesystem;
use wgtk::script::{self, Script};


/// World of Tanks server tools.
///
/// Each app runs on its own with `app`, base apps linking to a login app which hands
/// its clients over to them. The other commands run the apps usually run together, in
/// this process.
///
/// The client should be modified to register the login app into
/// `res/scripts_config.xml`, with the login app's public key when given its private
/// key. Use the following Packed XML filter to register it into the file:
///
///   $ wgtk pxml -f D:\Games\WoT\res\scripts_config.xml.bak0000 --raw '$tmp=login/host;$n=str(WGTK);$u=str(localhost:20016);$tmp/name=$n;$tmp/short_name=$n;$tmp/url=$u;$tmp/url_token=$u;$tmp/public_key_path=str(loginapp_wgtk.pubkey);$tmp/periphery_id=int(205);login/host[^]=$tmp' > D:\Games\WoT\res\scripts_config.xml
///
#[derive(Debug, Parser)]
#[command(version, author, disable_help_subcommand = true, max_term_width = 100)]
pub struct Cli {
    /// Path to the game's directory, used to load its script model (entities,
    /// interfaces, aliases) dynamically at startup -- see `wgtk::script::load`.
    #[arg(long, global = true, env = "WOT_GAME_DIR")]
    pub game_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    App(AppArgs),
    Server(ServerArgs),
    Proxy(ProxyArgs),
    Replay(ReplayArgs),
}

/// Run a single app.
#[derive(Debug, Args)]
pub struct AppArgs {
    #[command(subcommand)]
    pub cmd: AppCommand,
}

#[derive(Debug, Subcommand)]
pub enum AppCommand {
    Login(AppLoginArgs),
    Base(AppBaseArgs),
    Proxy(AppProxyArgs),
}

/// Run a login app.
///
/// It logs clients in and hands them over to the base apps linked with it. With
/// `--real-login-app`, it forwards them to the real login app instead, and hands them
/// over to a base proxy (`app proxy`) linked with it.
#[derive(Debug, Args)]
pub struct AppLoginArgs {
    /// The address where the login app should be bound.
    #[arg(long, default_value = "127.0.0.1:20016")]
    pub bind: SocketAddrV4,
    #[command(flatten)]
    pub login: LoginArgs,
    /// The address where base apps (or base proxies) should link with the login app.
    #[arg(long, default_value = "127.0.0.1:20019")]
    pub bind_link: SocketAddr,
    /// Forward login requests to this real login app, instead of logging clients in.
    #[arg(long)]
    pub real_login_app: Option<SocketAddrV4>,
    #[command(flatten)]
    pub real: RealLoginArgs,
    /// The name of every player, until accounts are stored: a client logging in with
    /// a token doesn't give one. Not with `--real-login-app`.
    #[arg(long, default_value = "Player", conflicts_with = "real_login_app")]
    pub name: String,
}

/// Run a base app.
///
/// It takes the clients the login app it links with hands over.
#[derive(Debug, Args)]
pub struct AppBaseArgs {
    /// The address where the base app should be bound.
    #[arg(long, default_value = "127.0.0.1:20017")]
    pub bind: SocketAddrV4,
    /// The login app to link with, its `--bind-link`.
    #[arg(long, default_value = "127.0.0.1:20019")]
    pub login_link: SocketAddr,
    #[command(flatten)]
    pub base: BaseArgs,
}

/// Run a base proxy.
///
/// It takes the clients the login proxy (`app login --real-login-app`) it links with
/// hands over, and forwards them to their real base app, logging everything they
/// exchange.
#[derive(Debug, Args)]
pub struct AppProxyArgs {
    /// The address where the base proxy should be bound.
    #[arg(long, default_value = "127.0.0.1:20017")]
    pub bind: SocketAddrV4,
    /// The login proxy to link with, its `--bind-link`.
    #[arg(long, default_value = "127.0.0.1:20019")]
    pub login_link: SocketAddr,
    /// The real login app the login proxies forward to, only to check the base apps
    /// clients are switched to are on its network.
    #[arg(long)]
    pub real_login_app: SocketAddrV4,
    #[command(flatten)]
    pub capture: CaptureArgs,
}

/// Run a simple WoT server: a login app and a base app.
///
/// This is used as a proof of concept server implementation.
#[derive(Debug, Args)]
pub struct ServerArgs {
    #[command(flatten)]
    pub apps: AppsArgs,
    #[command(flatten)]
    pub login: LoginArgs,
    #[command(flatten)]
    pub base: BaseArgs,
    /// The name of every player, until accounts are stored: a client logging in with
    /// a token doesn't give one.
    #[arg(long, default_value = "Player")]
    pub name: String,
}

/// Run a proxy of the real WoT server: a login proxy and a base proxy.
///
/// The login proxy forwards login requests to the real login app by emulating a virtual
/// client from the point of view of the real login app. Once the login process has been
/// completed, it hands this virtual client and its blowfish key over to the base proxy,
/// so that it is able to keep spying on the communication without altering packets.
#[derive(Debug, Args)]
pub struct ProxyArgs {
    #[command(flatten)]
    pub apps: AppsArgs,
    #[command(flatten)]
    pub login: LoginArgs,
    /// The real login app to forward login requests to.
    #[arg(long)]
    pub real_login_app: SocketAddrV4,
    #[command(flatten)]
    pub real: RealLoginArgs,
    #[command(flatten)]
    pub capture: CaptureArgs,
}

/// Decode a capture recorded with `--capture` instead of running a server, logging
/// exactly what the base proxy logged live.
#[derive(Debug, Args)]
pub struct ReplayArgs {
    /// The capture to decode.
    pub capture: PathBuf,
    /// The real login app, give it if the capture's first packet does not come from the
    /// same network.
    #[arg(long)]
    pub real_login_app: Option<SocketAddrV4>,
    /// Also encode every decoded element again and compare it with the captured bytes,
    /// then print which element types round-trip, which differ and which have no
    /// encoder yet.
    #[arg(long)]
    pub round_trip: bool,
}

#[derive(Debug, Args)]
pub struct AppsArgs {
    /// The address where the login app should be bound.
    #[arg(long, default_value = "127.0.0.1:20016")]
    pub bind_login_app: SocketAddrV4,
    /// The address where the base app should be bound.
    #[arg(long, default_value = "127.0.0.1:20017")]
    pub bind_base_app: SocketAddrV4,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// The path to the private key, used for login app encryption.
    /// Encryption is disabled if not provided.
    #[arg(long)]
    pub priv_key_path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct RealLoginArgs {
    /// The public key of the real login app, its requests are not encrypted without.
    #[arg(long, requires = "real_login_app")]
    pub real_pub_key_path: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct CaptureArgs {
    /// Record every base app packet to this file, as a gzip-compressed capture of the
    /// decrypted packets, for later analysis with `replay`.
    #[arg(long)]
    pub capture: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BaseArgs {
    /// Give the account this vehicle, as `<nation>:<name>`
    /// (`uk:GB98_T95_FV4201_Chieftain`, names as in the game's
    /// `scripts/item_defs/vehicles/<nation>/list.xml`). Can be repeated.
    #[arg(long)]
    pub give_vehicle: Vec<String>,
    /// The map battles are on, by its name in the game's `scripts/arena_defs/_list_.xml`.
    #[arg(long, default_value = "02_malinovka")]
    pub map: String,
    /// Bots on the player's side, the player aside.
    #[arg(long, default_value_t = 0)]
    pub allies: u32,
    /// Bots on the other side.
    #[arg(long, default_value_t = 1)]
    pub enemies: u32,
    /// A vehicle bots drive, as `--give-vehicle` takes it: each bot drives the next,
    /// round, else the player's own. Can be repeated.
    #[arg(long)]
    pub bot_vehicle: Vec<String>,
    /// Whether bots fire on their enemies.
    #[arg(long)]
    pub bots_fire: bool,
    /// Whether bots drive toward their enemies, else they stay where they start.
    #[arg(long)]
    pub bots_drive: bool,
    /// Start the enemies this many metres in front of the player instead of at their
    /// spawn, facing the player.
    #[arg(long)]
    pub enemy_distance: Option<f32>,
}

/// Type alias for a result that simply returns a string on error, this will be output
/// on stderr and process returns a failed exit code.
pub type CliResult<T> = Result<T, String>;


/// Entrypoint.
fn main() -> ExitCode {

    let args = Cli::parse();

    let game_dir = args.game_dir.as_deref()
        .ok_or_else(|| format!("The game's directory is required, give --game-dir or set WOT_GAME_DIR"));

    let res = match args.cmd {
        Command::App(app) => match app.cmd {
            AppCommand::Login(args) => login::run(args),
            AppCommand::Base(args) => game_dir.and_then(|dir| base::run(args, dir)),
            AppCommand::Proxy(args) => game_dir.and_then(|dir| proxy::run(args, dir)),
        },
        Command::Server(args) => game_dir.and_then(|dir| cluster::server::run(args, dir)),
        Command::Proxy(args) => game_dir.and_then(|dir| cluster::proxy::run(args, dir)),
        Command::Replay(args) => game_dir.and_then(|dir| replay::run(args, dir)),
    };

    if let Err(message) = res {
        eprintln!("{message}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }

}

/// Log to the console, and to this trace file.
///
/// The console gets the usual human-readable, env-filtered log. The trace file always
/// gets INFO-and-up (regardless of RUST_LOG) as JSON Lines, one line per event with its
/// structured fields (element id, request id, decoded/raw content, ...), so a full
/// session (login, hangar, a battle, ...) can be replayed and its message exchanges
/// reconstructed afterwards -- this is the same `tracing` call sites used for the
/// console, just fanned out to a second, always-on, structured sink instead of a
/// bespoke logging path. TRACE is deliberately excluded here: it's internal
/// packet/channel transport bookkeeping (sequence numbers, acks, fragment reassembly,
/// ...), not application-level protocol content -- it'd drown out the actually useful
/// element-level events in the trace file.
fn init_tracing(trace_path: &str) -> CliResult<()> {

    let trace_file = File::create(trace_path)
        .map_err(|e| format!("Failed to create trace file at {trace_path}: {e}"))?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer()
            .with_filter(EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into())
                .from_env_lossy()))
        .with(tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::sync::Mutex::new(trace_file))
            .with_filter(LevelFilter::INFO))
        .init();

    Ok(())

}

/// Load the script model (entities, interfaces, aliases) for the game installed at
/// `dir`, the same way `wg-toolkit-cli`'s old `bootstrap` codegen command used to --
/// except this is loaded fresh at every startup instead of being turned into generated
/// Rust source, since `base::App` now resolves everything dynamically against it.
fn load_script(dir: &Path) -> CliResult<Script> {

    let fs = ResFilesystem::new(dir.join("res"))
        .map_err(|e| format!("Failed to open resource filesystem, reason: {e}"))?;

    script::load(&fs)
        .map_err(|e| format!("Failed to load script model, reason: {e}"))

}

impl LoginArgs {

    /// The private key the login app decrypts login requests with.
    pub fn encryption_key(&self) -> CliResult<Option<Arc<RsaPrivateKey>>> {

        let Some(priv_key_path) = self.priv_key_path.as_deref() else { return Ok(None) };

        let priv_key_content = fs::read_to_string(priv_key_path)
            .map_err(|e| format!("Failed to read private key at {}: {e}", priv_key_path.display()))?;

        Ok(Some(Arc::new(RsaPrivateKey::from_pkcs8_pem(&priv_key_content)
            .map_err(|e| format!("Failed to decode PKCS#8 private key: {e}"))?)))

    }

}

impl RealLoginArgs {

    /// The public key the login proxy encrypts login requests to the real login app
    /// with.
    pub fn encryption_key(&self) -> CliResult<Option<Arc<RsaPublicKey>>> {

        let Some(pub_key_path) = self.real_pub_key_path.as_deref() else { return Ok(None) };

        let pub_key_content = fs::read_to_string(pub_key_path)
            .map_err(|e| format!("Failed to read public key at {}: {e}", pub_key_path.display()))?;

        Ok(Some(Arc::new(RsaPublicKey::from_public_key_pem(&pub_key_content)
            .map_err(|e| format!("Failed to decode PEM public key: {e}"))?)))

    }

}
