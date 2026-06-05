//! `deckard-signerd` — the process-isolated signer daemon entry point.
//!
//! Resolves config from the environment, prepares the `0700` runtime dir, takes the
//! single-instance lock, binds the `0600` socket, and serves the CBOR socket API until
//! killed. See the crate docs (`lib.rs`) for the security model.

use std::sync::Arc;

use tokio::sync::Mutex;

use deckard_signerd::{config::Config, daemon::Daemon, server, socket};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    eprintln!(
        "signerd: starting · socket={} · chain_id={} · rpc={}",
        cfg.socket_path.display(),
        cfg.chain_id,
        cfg.redacted_rpc(),
    );

    socket::prepare_parent(&cfg.socket_path)?;
    // Hold the single-instance lock for the whole process lifetime.
    let _lock = socket::single_instance(&cfg.socket_path)?;
    let listener = socket::bind(&cfg.socket_path)?;
    eprintln!("signerd: listening (same-uid only)");

    let daemon = Arc::new(Mutex::new(Daemon::new(cfg)));
    server::serve(listener, daemon).await
}
