//! Implementation of a simple demonstration WoT server.

pub mod r#gen;
pub mod proxy;
pub mod emulator;

use std::sync::Arc;
use std::fs;

use std::fs::File;

use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, Layer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::{CliResult, WotArgs};


/// Entrypoint.
pub fn cmd_wot(args: WotArgs) -> CliResult<()> {

    // The console gets the usual human-readable, env-filtered log. The trace file
    // always gets INFO-and-up (regardless of RUST_LOG) as JSON Lines, one line per
    // event with its structured fields (element id, request id, decoded/raw content,
    // ...), so a full session (login, hangar, a battle, ...) can be replayed and its
    // message exchanges reconstructed afterwards -- this is the same `tracing` call
    // sites used for the console, just fanned out to a second, always-on, structured
    // sink instead of a bespoke logging path. TRACE is deliberately excluded here: it's
    // internal packet/channel transport bookkeeping (sequence numbers, acks, fragment
    // reassembly, ...), not application-level protocol content -- it'd drown out the
    // actually useful element-level events in the trace file.
    let trace_path = "proxy-trace.jsonl";
    let trace_file = File::create(trace_path)
        .map_err(|e| format!("Failed to create trace file at {trace_path}: {e}"))?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer()
            .with_filter(EnvFilter::builder()
                .with_default_directive(LevelFilter::TRACE.into())
                .from_env_lossy()))
        .with(tracing_subscriber::fmt::layer()
            .json()
            .with_writer(std::sync::Mutex::new(trace_file))
            .with_filter(LevelFilter::INFO))
        .init();

    // Start by decoding the private key...
    let encryption_key;
    if let Some(priv_key_path) = args.priv_key_path.as_deref() {

        let priv_key_content = fs::read_to_string(priv_key_path)
            .map_err(|e| format!("Failed to read private key at {}: {e}", priv_key_path.display()))?;

        encryption_key = Some(Arc::new(RsaPrivateKey::from_pkcs8_pem(&priv_key_content)
            .map_err(|e| format!("Failed to decode PKCS#8 private key: {e}"))?));

    } else {
        encryption_key = None;
    }

    if let Some(real_login_app) = args.real_login_app {

        let real_encryption_key;
        if let Some(pub_key_path) = args.real_pub_key_path.as_deref() {
            
            let pub_key_content = fs::read_to_string(pub_key_path)
                .map_err(|e| format!("Failed to read public key at {}: {e}", pub_key_path.display()))?;

            let pub_key = Arc::new(RsaPublicKey::from_public_key_pem(&pub_key_content)
                .map_err(|e| format!("Failed to decode PEM public key: {e}"))?);

            real_encryption_key = Some(pub_key);

        } else {
            real_encryption_key = None;
        }
        
        proxy::run(args.login_app, real_login_app, args.base_app, encryption_key, real_encryption_key)
        
    } else {
        emulator::run(args.login_app, args.base_app, encryption_key)
    }

}
