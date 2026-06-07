//! Spawn + supervise the `deckard-signerd` child process from the GUI app.
//!
//! The app owns the daemon's lifecycle: it spawns the child, restarts it (capped backoff) if
//! it crashes, and kills it on app exit (via `Drop`). The child binary is resolved from
//! `DECKARD_SIGNERD_BIN`, else next to the app binary, else `deckard-signerd` on `PATH`. The
//! socket path is passed explicitly so the app's [`SignerClient`](crate::SignerClient) and
//! the daemon agree. The child inherits stdout/stderr → the app's log.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Resolve the `deckard-signerd` binary: explicit override, then a sibling of the running
/// app binary, then the bare name (PATH lookup).
pub fn resolve_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("DECKARD_SIGNERD_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("deckard-signerd");
            if sibling.exists() {
                return sibling;
            }
        }
    }
    PathBuf::from("deckard-signerd")
}

/// A running, self-restarting daemon child. Dropping it stops the supervisor and kills the
/// child.
pub struct DaemonSupervisor {
    shutdown: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    socket_path: PathBuf,
}

impl DaemonSupervisor {
    /// Spawn the daemon bound to `socket_path` (broadcasting via `rpc_url` on `chain_id`) and a
    /// monitor thread that respawns it on crash. The child inherits the current environment
    /// plus `DECKARD_SOCKET_PATH`/`DECKARD_RPC_URL`/`DECKARD_CHAIN_ID`.
    pub fn spawn(socket_path: PathBuf, rpc_url: String, chain_id: u64) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let bin = resolve_binary();
        let env = ChildEnv {
            socket_path: socket_path.clone(),
            rpc_url,
            chain_id,
        };

        let sup = Self {
            shutdown: Arc::clone(&shutdown),
            child: Arc::clone(&child),
            socket_path,
        };

        std::thread::Builder::new()
            .name("deckard-signerd-sup".into())
            .spawn(move || monitor_loop(bin, env, shutdown, child))
            .ok();

        sup
    }

    /// The socket path the supervised daemon binds.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// A client wired to this daemon's socket.
    pub fn client(&self) -> crate::SignerClient {
        crate::SignerClient::new(self.socket_path.clone())
    }
}

impl Drop for DaemonSupervisor {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// The environment the daemon child needs to agree with the app on socket + chain.
struct ChildEnv {
    socket_path: PathBuf,
    rpc_url: String,
    chain_id: u64,
}

/// Spawn → poll-until-exit → backoff → respawn, until shutdown is signalled.
fn monitor_loop(
    bin: PathBuf,
    env: ChildEnv,
    shutdown: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
) {
    let mut backoff = Duration::from_millis(200);
    while !shutdown.load(Ordering::SeqCst) {
        match Command::new(&bin)
            .env("DECKARD_SOCKET_PATH", &env.socket_path)
            .env("DECKARD_RPC_URL", &env.rpc_url)
            .env("DECKARD_CHAIN_ID", env.chain_id.to_string())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => {
                if let Ok(mut slot) = child_slot.lock() {
                    *slot = Some(child);
                }
                // Poll for exit, releasing the lock between polls so Drop can kill the child.
                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        return;
                    }
                    let exited = match child_slot.lock() {
                        Ok(mut slot) => match slot.as_mut() {
                            Some(c) => match c.try_wait() {
                                Ok(Some(_status)) => {
                                    *slot = None;
                                    true
                                }
                                Ok(None) => false,
                                Err(_) => {
                                    *slot = None;
                                    true
                                }
                            },
                            None => true, // Drop took it
                        },
                        Err(_) => true,
                    };
                    if exited {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    backoff = Duration::from_millis(200); // healthy run resets the backoff
                }
            }
            Err(e) => {
                eprintln!("deckard: failed to spawn signerd ({}): {e}", bin.display());
            }
        }

        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}
