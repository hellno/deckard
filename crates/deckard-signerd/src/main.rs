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

    let daemon = Arc::new(Mutex::new(Daemon::new(cfg)));

    // Resolver authentication (PRD-01): if the supervising app passed us the inherited
    // capability-channel fd, serve `Resolve` ONLY there. Without it the daemon refuses every
    // `Resolve` (fail-closed) — the public socket can propose/read/execute/STOP but not approve.
    match server::adopt_control_fd() {
        Ok(Some(control)) => {
            eprintln!("signerd: listening (public socket same-uid + private resolver channel)");
            let d = Arc::clone(&daemon);
            tokio::spawn(async move {
                if let Err(e) = server::serve_control(control, d).await {
                    eprintln!("signerd: resolver control channel closed: {e}");
                }
            });
        }
        Ok(None) => {
            eprintln!("signerd: listening (same-uid only; no resolver channel — Resolve refused)");
        }
        Err(e) => {
            // A set-but-unusable fd is a launch misconfiguration: be loud, then run degraded
            // (no approvals) rather than crash-loop.
            eprintln!("signerd: resolver capability channel unavailable ({e}); Resolve refused");
        }
    }

    server::serve(listener, daemon).await
}
