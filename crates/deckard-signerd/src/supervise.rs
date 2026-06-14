//! Spawn + supervise the `deckard-signerd` child process from the GUI app.
//!
//! The app owns the daemon's lifecycle: it spawns the child, restarts it (capped backoff) if
//! it crashes, and kills it on app exit (via `Drop`). The child binary is resolved from
//! `DECKARD_SIGNERD_BIN`, else next to the app binary, else `deckard-signerd` on `PATH`. The
//! socket path is passed explicitly so the app's [`SignerClient`](crate::SignerClient) and
//! the daemon agree. The child inherits stdout/stderr → the app's log.
//!
//! ## Resolver authentication (PRD-01)
//!
//! Because the app is the *parent*, it mints a private `AF_UNIX` [`socketpair`] before each
//! spawn, hands the daemon child one end by fd inheritance (its number in
//! [`crate::server::RESOLVE_FD_ENV`]), and keeps the other end as a [`ControlChannel`]. A
//! `Resolve` (approval) is honoured by the daemon ONLY on that channel — the public proposer
//! socket refuses it — so a same-uid process can no longer self-approve (THREAT-MODEL
//! residual #1). Each respawn re-mints the pair; while the daemon is restarting the channel is
//! disconnected and `Resolve` fails *closed*, never silently approving.

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deckard_contract::{RequestId, SignerRequest, SignerResponse};

use crate::frame;

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

/// How long a blocking `Resolve` round-trip waits on the control channel before giving up, so a
/// genuinely wedged daemon can never hang the caller (the app's background thread) forever.
///
/// It MUST comfortably exceed the daemon's own worst-case lock hold: the daemon serializes every
/// request behind one mutex and holds it across a broadcast bounded at `BROADCAST_TIMEOUT` (30s,
/// `daemon.rs`), so a `Resolve` can legitimately queue ~30s behind an in-flight `execute`. A
/// shorter budget would misread that normal back-pressure as a dead socket, tear the channel
/// down, and (the control channel is a single non-re-accepting connection) brick approvals until
/// the next daemon restart. A truly dead daemon still fails fast — its end closes, so the read
/// returns EOF/`ECONNRESET` long before this ceiling. The wait runs off the UI thread.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(35);

/// The app's end of the private capability channel (PRD-01). A `Resolve` travels here and ONLY
/// here; the daemon's public socket refuses approvals. Re-minted on every daemon (re)spawn —
/// while the daemon is restarting the slot is empty and [`resolve`](Self::resolve) fails closed
/// (it never silently approves). Cheap to clone; all clones share one slot.
#[derive(Clone)]
pub struct ControlChannel {
    inner: Arc<Mutex<Option<StdUnixStream>>>,
}

impl ControlChannel {
    /// An unconnected channel (no live daemon end yet). The supervisor publishes a fresh end on
    /// each spawn; tests that spawn the daemon binary themselves use [`connected`](Self::connected).
    pub fn disconnected() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Wrap an already-connected app end (the integration-test harness spawns the daemon and
    /// passes the inherited fd just like the app does).
    pub fn connected(stream: StdUnixStream) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(stream))),
        }
    }

    /// Install a freshly-minted app end (a daemon (re)spawn just happened).
    fn publish(&self, stream: StdUnixStream) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = Some(stream);
        }
    }

    /// Drop the app end (the daemon exited) so the next `resolve` fails closed until the
    /// supervisor re-handshakes with the restarted daemon.
    fn disconnect(&self) {
        if let Ok(mut slot) = self.inner.lock() {
            *slot = None;
        }
    }

    /// Send `Resolve { request_id, approved }` over the capability channel and await the
    /// daemon's `Ack`. Fails closed when the channel isn't currently connected (daemon
    /// mid-respawn).
    ///
    /// On any transport error the end is dropped. This is deliberate, not just hygiene: the
    /// `Ack` carries no request id, so reusing a stream after a timed-out/partial read could
    /// pair a later call with a stale, delayed `Ack` (a silent off-by-one). Dropping forces a
    /// fresh handshake instead. A genuinely dead daemon is then re-handshaked by the supervisor
    /// when it observes the child exit (`monitor_loop` → `disconnect`/`publish`); the generous
    /// [`CONTROL_TIMEOUT`] ensures normal back-pressure (a `Resolve` queued behind an in-flight
    /// broadcast) is never mistaken for a dead socket and torn down.
    pub fn resolve(&self, request_id: RequestId, approved: bool) -> anyhow::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("control channel poisoned"))?;
        let stream = guard.as_mut().ok_or_else(|| {
            anyhow::anyhow!("approval channel not connected (daemon restarting) — hold again")
        })?;
        let result = round_trip(stream, request_id, approved);
        if result.is_err() {
            *guard = None; // drop the broken end → re-handshake on the next attempt
        }
        result
    }
}

/// One blocking `Resolve` → `Ack` round-trip over the capability channel.
fn round_trip(
    stream: &mut StdUnixStream,
    request_id: RequestId,
    approved: bool,
) -> anyhow::Result<()> {
    let body = frame::encode(&SignerRequest::Resolve {
        request_id,
        approved,
    })?;
    frame::write_frame_blocking(stream, &body)?;
    let resp = frame::read_frame_blocking(stream)?
        .ok_or_else(|| anyhow::anyhow!("control channel closed before responding"))?;
    match frame::decode::<SignerResponse>(&resp)? {
        SignerResponse::Ack => Ok(()),
        other => Err(anyhow::anyhow!("unexpected control response: {other:?}")),
    }
}

/// Create the private capability `socketpair`. Returns `(app_end, child_fd)`:
/// - `app_end` — the app keeps this; `Resolve` frames travel here. It is set close-on-exec so
///   a later daemon respawn never inherits a stale app end, and carries a read timeout so a
///   wedged daemon can't hang the caller.
/// - `child_fd` — handed to the spawned daemon by inheritance (its number goes in
///   [`crate::server::RESOLVE_FD_ENV`]); deliberately left WITHOUT close-on-exec so it survives
///   `exec`. The caller drops it immediately after spawn — the child holds its own copy.
pub fn control_pair() -> anyhow::Result<(StdUnixStream, OwnedFd)> {
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};

    let (app_owned, child_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )?;
    // App end: close-on-exec (never inherited by a future child) + a bounded read timeout.
    set_cloexec(&app_owned)?;
    let app_end = StdUnixStream::from(app_owned);
    app_end.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    Ok((app_end, child_owned))
}

/// Set `FD_CLOEXEC` on a borrowed fd (safe nix wrapper around `fcntl(F_SETFD)`).
fn set_cloexec<F: AsRawFd>(fd: &F) -> anyhow::Result<()> {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    fcntl(fd.as_raw_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
    Ok(())
}

/// A running, self-restarting daemon child. Dropping it stops the supervisor and kills the
/// child.
pub struct DaemonSupervisor {
    shutdown: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    socket_path: PathBuf,
    control: ControlChannel,
}

impl DaemonSupervisor {
    /// Spawn the daemon bound to `socket_path` (broadcasting via `rpc_url` on `chain_id`) and a
    /// monitor thread that respawns it on crash. The child inherits the current environment
    /// plus `DECKARD_SOCKET_PATH`/`DECKARD_RPC_URL`/`DECKARD_CHAIN_ID` and the inherited
    /// capability-channel fd (`DECKARD_RESOLVE_FD`).
    pub fn spawn(socket_path: PathBuf, rpc_url: String, chain_id: u64) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let control = ControlChannel::disconnected();
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
            control: control.clone(),
        };

        std::thread::Builder::new()
            .name("deckard-signerd-sup".into())
            .spawn(move || monitor_loop(bin, env, shutdown, child, control))
            .ok();

        sup
    }

    /// The socket path the supervised daemon binds.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// A client wired to this daemon's socket (the public proposer channel: propose/read/
    /// execute/STOP).
    pub fn client(&self) -> crate::SignerClient {
        crate::SignerClient::new(self.socket_path.clone())
    }

    /// The private capability channel to the supervised daemon — the app sends `Resolve` here
    /// (the public socket refuses it). Cheap to clone.
    pub fn control(&self) -> ControlChannel {
        self.control.clone()
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

/// Spawn → poll-until-exit → backoff → respawn, until shutdown is signalled. Each spawn mints a
/// FRESH capability channel (an inherited fd serves exactly one daemon instance), publishes the
/// app end while the daemon is alive, and disconnects it when the daemon exits — so a restarted
/// daemon re-handshakes and there is never a window where `Resolve` is ungated.
fn monitor_loop(
    bin: PathBuf,
    env: ChildEnv,
    shutdown: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    control: ControlChannel,
) {
    let mut backoff = Duration::from_millis(200);
    while !shutdown.load(Ordering::SeqCst) {
        // Mint this instance's capability channel before spawning so the child can inherit it.
        let spawned = match control_pair() {
            Ok((app_end, child_fd)) => {
                let result = Command::new(&bin)
                    .env("DECKARD_SOCKET_PATH", &env.socket_path)
                    .env("DECKARD_RPC_URL", &env.rpc_url)
                    .env("DECKARD_CHAIN_ID", env.chain_id.to_string())
                    .env(
                        crate::server::RESOLVE_FD_ENV,
                        child_fd.as_raw_fd().to_string(),
                    )
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn();
                // The child inherited its own copy of `child_fd`; drop ours unconditionally so
                // the daemon end isn't held open by the parent.
                drop(child_fd);
                result.map(|child| (child, app_end))
            }
            Err(e) => {
                eprintln!("deckard: failed to create resolver capability channel: {e}");
                // Skip this spawn attempt; back off and retry.
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(5));
                continue;
            }
        };

        match spawned {
            Ok((child, app_end)) => {
                // The daemon is live: publish its control end so `Resolve` works.
                control.publish(app_end);
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
                // The daemon is gone: tear down its capability channel so a stale end is never
                // used against the next instance (the next iteration mints a fresh one).
                control.disconnect();
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
