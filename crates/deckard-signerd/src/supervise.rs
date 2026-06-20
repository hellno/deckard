//! Spawn + supervise the `deckard-signerd` child process from the GUI app.
//!
//! The app owns the daemon's lifecycle: it spawns the child, restarts it (capped backoff) if
//! it crashes, and kills it on app exit (via `Drop`).
//!
//! ## Launch provenance (finding C1)
//!
//! The child is the only process that ever holds the decrypted key, so HOW it is launched is a
//! trust boundary. Two hardening rules are enforced here:
//! - **Cleared, minimal environment.** `apply_child_env` calls `env_clear()` on the child, then
//!   sets back ONLY the control vars the supervisor computes plus a fixed allowlist
//!   (`FORWARDED_ENV_ALLOWLIST`). A poisoned *app* environment — `LD_PRELOAD`, `LD_AUDIT`,
//!   `DYLD_INSERT_LIBRARIES`, an injected `$PATH` — therefore cannot loader-inject the key-holder.
//! - **Verified bundled binary.** In a release build `resolve_binary` resolves to exactly ONE
//!   canonical path (the bundled sibling of the app binary) and verifies ownership / permissions /
//!   symlink before exec. The attacker-influenceable `DECKARD_SIGNERD_BIN` / `$PATH` resolver is
//!   compiled in ONLY under the dev/test `dev-signerd-bin` feature, never in `default`.
//!
//! The socket path is passed explicitly so the app's [`SignerClient`](crate::SignerClient) and
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

use std::ffi::OsString;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deckard_contract::{RequestId, SignerRequest, SignerResponse};

use crate::frame;

/// The bundled daemon binary's file name — a sibling of the running app binary (in a macOS
/// `.app`: `Contents/MacOS/deckard-signerd` next to `Contents/MacOS/deckard`).
const DAEMON_BIN_NAME: &str = "deckard-signerd";

// finding C1 / #106: the dev/test binary resolver must be physically ABSENT from a release-profile
// artifact — not merely off-by-default. A cargo feature is independent of the build profile, so a
// determined `cargo build --release --features dev-signerd-bin` (or `--all-features`) could
// otherwise produce an optimized binary with the attacker-influenceable resolver. This guard makes
// that combination fail to COMPILE: `dev-signerd-bin` is only legal with debug assertions on (the
// dev/test profiles `just run`/`qa`/`demo`/`cargo test` use). The shipped artifacts (`just bundle`,
// `release.yml`) build with default features in release, so they are unaffected.
#[cfg(all(feature = "dev-signerd-bin", not(debug_assertions)))]
compile_error!(
    "`dev-signerd-bin` is a dev/test-only daemon-binary resolver and must never be compiled into a \
     release build (finding C1 / #106). Build without it (the default = the verified bundled path), \
     or use a debug/test profile."
);

/// Resolve the `deckard-signerd` binary to launch — the **release** path (finding C1).
///
/// A release artifact resolves to exactly ONE canonical location: the bundled sibling of the
/// running app binary, verified (ownership / permissions / symlink) *before* we hand it the unlock
/// passphrase. There is deliberately **no** `DECKARD_SIGNERD_BIN` env override and **no** bare
/// `$PATH` search — both are attacker-influenceable, and substituting the daemon binary captures
/// the forwarded passphrase, bypassing both the keystore envelope and the process split. The loose
/// env/sibling/`$PATH` resolver exists ONLY under the dev/test `dev-signerd-bin` feature, so it
/// physically cannot be compiled into a release build.
#[cfg(not(feature = "dev-signerd-bin"))]
fn resolve_binary() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("cannot resolve the running app binary: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("running app binary has no parent directory"))?;
    let candidate = dir.join(DAEMON_BIN_NAME);
    // TODO(#106): once the signed-release pipeline lands (SECURITY.md — reproducible + Apple-
    // notarized builds), additionally verify a code signature or a pinned content hash of
    // `candidate` here, before exec. Until then the ownership/permission/symlink gate below IS the
    // launch-provenance check; it is intentionally NOT a substitute for signature verification.
    verify_bundled_binary(&candidate)
}

/// Provenance-check a candidate daemon binary before exec (release path, finding C1). Refuses any
/// binary an *other* (non-us, non-root) user — or a group/world writer — could have substituted to
/// capture the forwarded unlock passphrase. All checks fail closed:
/// - the path is a **regular file**, not a **symlink** (a symlink is an attacker redirect);
/// - it is **owned by root or by us** (`euid`), never another user;
/// - it is **not group/world-writable**;
/// - its **parent directory** is owned by root or us AND is not group/world-writable, so neither a
///   different user (the dir owner can always replace entries) nor a group/world writer can swap the
///   file out from under us.
///
/// **Honest residual** (the documented same-uid boundary, `THREAT-MODEL.md`): this gate stops an
/// *other-user* substitution and catches a misconfigured (world-writable / mislabeled) install, but
/// it does NOT stop a **same-uid** attacker on a **user-owned** install, and a check-to-exec
/// (TOCTOU) window remains. Both are closed by the deferred **signature / hash** verification (the
/// OS loader validates a signed binary at exec time) — see `TODO(#106)` in [`resolve_binary`]. Only
/// the immediate parent is checked here; full ancestor-chain hardening rides with that signing work.
#[cfg(not(feature = "dev-signerd-bin"))]
fn verify_bundled_binary(candidate: &std::path::Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let us = crate::auth::our_uid();

    // lstat — does NOT follow a final symlink: a symlinked daemon binary is an attacker redirect.
    let meta = std::fs::symlink_metadata(candidate).map_err(|e| {
        anyhow::anyhow!(
            "bundled signerd binary {} is unavailable: {e}",
            candidate.display()
        )
    })?;
    let ft = meta.file_type();
    anyhow::ensure!(
        !ft.is_symlink(),
        "refusing to launch signerd: {} is a symlink (possible attacker redirect)",
        candidate.display()
    );
    anyhow::ensure!(
        ft.is_file(),
        "refusing to launch signerd: {} is not a regular file",
        candidate.display()
    );
    let owner = meta.uid();
    anyhow::ensure!(
        owner == 0 || owner == us,
        "refusing to launch signerd: {} is owned by uid {owner}, not root or us ({us}) — a \
         substituted binary would capture the unlock passphrase",
        candidate.display()
    );
    anyhow::ensure!(
        (meta.mode() & 0o022) == 0,
        "refusing to launch signerd: {} is group/world-writable (mode {:#o})",
        candidate.display(),
        meta.mode() & 0o7777
    );

    if let Some(parent) = candidate.parent().filter(|p| !p.as_os_str().is_empty()) {
        let pmeta = std::fs::metadata(parent).map_err(|e| {
            anyhow::anyhow!("cannot stat signerd directory {}: {e}", parent.display())
        })?;
        let powner = pmeta.uid();
        // A directory's OWNER can replace any entry in it regardless of file perms, so a parent
        // owned by another (non-root) user is itself a substitution vector even at mode 0755.
        anyhow::ensure!(
            powner == 0 || powner == us,
            "refusing to launch signerd: directory {} is owned by uid {powner}, not root or us \
             ({us}) — its owner could swap the binary",
            parent.display()
        );
        // Reject any group/world-writable parent (no sticky-bit carve-out: the sticky bit only
        // fences OTHER uids, and a release binary never legitimately lives in a world-writable dir).
        anyhow::ensure!(
            (pmeta.mode() & 0o022) == 0,
            "refusing to launch signerd: directory {} is group/world-writable (mode {:#o}) — the \
             binary could be swapped",
            parent.display(),
            pmeta.mode() & 0o7777
        );
    }

    // The verified `candidate` is returned (NOT canonicalized): a TOCTOU window remains — see the
    // "honest residual" note above; signature verification (deferred) is the durable close.
    Ok(candidate.to_path_buf())
}

/// Resolve the `deckard-signerd` binary — the **dev/test** path (`dev-signerd-bin` feature).
///
/// The loose, convenient resolver: an explicit `DECKARD_SIGNERD_BIN` override, then a sibling of
/// the running app binary, then the bare name (`$PATH` lookup). This is what `just run` / `just qa`
/// / `just demo` and the test harnesses use. It is gated behind a feature that is **never** in
/// `default`, so a release artifact (`just bundle`, `release.yml` — both default-feature builds)
/// physically cannot compile this attacker-influenceable resolution (finding C1).
#[cfg(feature = "dev-signerd-bin")]
fn resolve_binary() -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os("DECKARD_SIGNERD_BIN").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(DAEMON_BIN_NAME);
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }
    // Bare-name fallback: resolve it against the PARENT's `$PATH` to an absolute path NOW, because
    // the child's environment is cleared (`apply_child_env` strips `$PATH`) — the daemon spawn can't
    // run its own `$PATH` lookup. Best-effort: the dev flows (`just run`/`qa`/`demo`) build the
    // daemon first and hit the sibling branch above, so this only matters for an out-of-tree harness.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(DAEMON_BIN_NAME);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Ok(PathBuf::from(DAEMON_BIN_NAME))
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
    /// monitor thread that respawns it on crash. The child gets a **cleared, minimal environment**
    /// ([`apply_child_env`]): `env_clear()` then only `DECKARD_SOCKET_PATH`/`DECKARD_RPC_URL`/
    /// `DECKARD_CHAIN_ID`/`DECKARD_CONFIG_DIR`, the inherited capability-channel fd
    /// (`DECKARD_RESOLVE_FD`), and the [`FORWARDED_ENV_ALLOWLIST`] toggles. The config dir is
    /// resolved HERE (in the app process, where `$HOME` is available) and passed explicitly, so the
    /// env-cleared daemon never needs `$HOME` to find its vault / policy / Helios cache.
    pub fn spawn(socket_path: PathBuf, rpc_url: String, chain_id: u64) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let control = ControlChannel::disconnected();
        let env = ChildEnv {
            socket_path: socket_path.clone(),
            rpc_url,
            chain_id,
            config_dir: resolve_config_dir(),
        };

        let sup = Self {
            shutdown: Arc::clone(&shutdown),
            child: Arc::clone(&child),
            socket_path,
            control: control.clone(),
        };

        std::thread::Builder::new()
            .name("deckard-signerd-sup".into())
            .spawn(move || monitor_loop(env, shutdown, child, control))
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

/// The environment the daemon child needs to agree with the app on socket + chain + config dir.
struct ChildEnv {
    socket_path: PathBuf,
    rpc_url: String,
    chain_id: u64,
    /// Absolute config dir (`vault.bin` / `policy.json` / Helios cache), resolved app-side so the
    /// env-cleared daemon never needs `$HOME` to find it. See [`resolve_config_dir`].
    config_dir: PathBuf,
}

/// Resolve the config directory to hand the daemon child, in the APP process where the full
/// environment (incl. `$HOME`) is still available, and **absolutized**, so the env-cleared daemon
/// ([`apply_child_env`]) never needs `$HOME` — and is robust to any CWD difference — to locate its
/// vault / policy / Helios cache.
///
/// `deckard_core::config_dir()` is the single canonical resolver (the `DECKARD_CONFIG_DIR` override,
/// empty-treated-as-unset, else the platform dir), shared with the app + the daemon's own
/// `Config::from_env`, so the three never drift. An empty result (no override, no platform dir) is
/// passed through as an empty `DECKARD_CONFIG_DIR`, which the daemon treats as unset and then fails
/// loudly — the app would be equally non-functional.
fn resolve_config_dir() -> PathBuf {
    let dir = deckard_core::config_dir().unwrap_or_default();
    std::path::absolute(&dir).unwrap_or(dir)
}

/// The Deckard-namespace env vars the daemon (or `deckard-core` running inside it) reads that the
/// supervisor does NOT compute itself — forwarded from the app's environment IFF present, so an
/// operator toggle / `just demo` setting still reaches the daemon after [`apply_child_env`] clears
/// the inherited environment. Each is parsed by the daemon as a bool/int/path-block and carries no
/// loader capability; the dangerous loader vars (`LD_PRELOAD`/`LD_AUDIT`/`DYLD_INSERT_LIBRARIES`/…)
/// and `$PATH` are deliberately absent. The control vars + `DECKARD_RESOLVE_FD` are set explicitly
/// (not via this list). `DECKARD_SIGNERD_BIN` is deliberately NOT here — it is the binary-
/// substitution vector (finding C1) and the daemon never reads it anyway.
///
/// Kept deliberately SMALL: only vars a real flow needs. `DECKARD_APPROVAL_TTL_SECS` (an
/// undocumented test/tuning knob no shipping flow sets — tests pass it straight to the daemon) is
/// NOT forwarded, so a poisoned app env can't lengthen the approval window through the supervisor.
/// `DECKARD_I_KNOW_THIS_IS_MAINNET` IS kept (status quo — it was inherited before `env_clear`):
/// against an attacker who controls the app's *launch* environment the guardrail override is moot,
/// because that same control loader-injects the *app* itself (which forwards the passphrase and
/// holds the resolver fd) — game-over for the app regardless. Restricting the override's reach is
/// tracked as the guardrail issue's "override env-var reconsideration" follow-up (#76).
const FORWARDED_ENV_ALLOWLIST: &[&str] = &[
    // Verified-reads toggle (`just demo` sets `0` to disable Helios on the Sepolia fork — required
    // for the local-fork demo; downgrades only READ verification, never signing).
    "DECKARD_VERIFIED_READS",
    // Pinned fork block for the demo's shield path (read by `deckard-core`; a benign integer).
    "DECKARD_DEMO_FORK_BLOCK",
    // Auto-approval guardrail override (human operator; documented only in THREAT-MODEL.md). Kept
    // for status-quo parity — see the rationale on this const's doc comment (#76 reconsiders it).
    "DECKARD_I_KNOW_THIS_IS_MAINNET",
];

/// Build the daemon child's MINIMAL, audited process environment on `cmd` (finding C1).
///
/// First [`Command::env_clear`]s, so a poisoned *app* environment — `LD_PRELOAD`, `LD_AUDIT`,
/// `DYLD_INSERT_LIBRARIES`, an injected `$PATH`, anything — can never flow into the one process
/// that holds the decrypted key. Then sets back ONLY:
/// - the control vars the supervisor computes (`DECKARD_SOCKET_PATH`, `DECKARD_RPC_URL`,
///   `DECKARD_CHAIN_ID`, `DECKARD_CONFIG_DIR`) plus this spawn's capability-channel fd number
///   (`DECKARD_RESOLVE_FD`); and
/// - the [`FORWARDED_ENV_ALLOWLIST`] Deckard-namespace toggles, copied from `lookup` when present.
///
/// `lookup` is `std::env::var_os` in production; tests inject a fake (hostile) parent environment
/// to prove the loader vars are dropped while the allowlist still passes through.
fn apply_child_env(
    cmd: &mut Command,
    env: &ChildEnv,
    resolve_fd: RawFd,
    lookup: impl Fn(&str) -> Option<OsString>,
) {
    cmd.env_clear();
    cmd.env("DECKARD_SOCKET_PATH", &env.socket_path);
    cmd.env("DECKARD_RPC_URL", &env.rpc_url);
    cmd.env("DECKARD_CHAIN_ID", env.chain_id.to_string());
    cmd.env("DECKARD_CONFIG_DIR", &env.config_dir);
    cmd.env(crate::server::RESOLVE_FD_ENV, resolve_fd.to_string());
    for &name in FORWARDED_ENV_ALLOWLIST {
        if let Some(val) = lookup(name) {
            cmd.env(name, val);
        }
    }
}

/// Sleep the current `backoff`, then double it (capped at 5s), and report whether the supervisor
/// should keep looping (`false` ⇒ shutdown was signalled, return now). Centralizes the retry cadence
/// so the three failure paths — binary refused, capability-channel mint failed, crash respawn — can
/// never drift apart (e.g. one path hot-looping faster than another).
fn backoff_and_continue(backoff: &mut Duration, shutdown: &AtomicBool) -> bool {
    if shutdown.load(Ordering::SeqCst) {
        return false;
    }
    std::thread::sleep(*backoff);
    *backoff = (*backoff * 2).min(Duration::from_secs(5));
    true
}

/// Spawn → poll-until-exit → backoff → respawn, until shutdown is signalled. Each spawn mints a
/// FRESH capability channel (an inherited fd serves exactly one daemon instance), publishes the
/// app end while the daemon is alive, and disconnects it when the daemon exits — so a restarted
/// daemon re-handshakes and there is never a window where `Resolve` is ungated.
fn monitor_loop(
    env: ChildEnv,
    shutdown: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    control: ControlChannel,
) {
    let mut backoff = Duration::from_millis(200);
    while !shutdown.load(Ordering::SeqCst) {
        // Resolve + provenance-check the daemon binary on every spawn attempt. In a release build
        // this is the ONE canonical bundled path (ownership/permission/symlink verified); a failure
        // means we refuse to exec rather than risk handing the unlock passphrase to a substituted
        // key-holder (finding C1). Retry with backoff in case it's a transient (e.g. a dev build
        // racing the daemon's compile); a real substitution stays refused.
        let bin = match resolve_binary() {
            Ok(b) => b,
            Err(e) => {
                eprintln!("deckard: refusing to launch signerd: {e}");
                if !backoff_and_continue(&mut backoff, &shutdown) {
                    return;
                }
                continue;
            }
        };
        // Mint this instance's capability channel before spawning so the child can inherit it.
        let spawned = match control_pair() {
            Ok((app_end, child_fd)) => {
                let mut cmd = Command::new(&bin);
                // Clear the inherited environment and set back only the audited allowlist BEFORE
                // spawn — this is the loader-injection half of finding C1.
                apply_child_env(&mut cmd, &env, child_fd.as_raw_fd(), |k| {
                    std::env::var_os(k)
                });
                cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
                let result = cmd.spawn();
                // The child inherited its own copy of `child_fd`; drop ours unconditionally so
                // the daemon end isn't held open by the parent.
                drop(child_fd);
                result.map(|child| (child, app_end))
            }
            Err(e) => {
                eprintln!("deckard: failed to create resolver capability channel: {e}");
                // Skip this spawn attempt; back off and retry.
                if !backoff_and_continue(&mut backoff, &shutdown) {
                    return;
                }
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

        if !backoff_and_continue(&mut backoff, &shutdown) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_env() -> ChildEnv {
        ChildEnv {
            socket_path: PathBuf::from("/tmp/deckard-test.sock"),
            rpc_url: "http://127.0.0.1:8545".to_string(),
            chain_id: 31337,
            config_dir: PathBuf::from("/tmp/deckard-test-cfg"),
        }
    }

    /// Locate a `print-the-environment` helper without needing `$PATH` (the child env is cleared,
    /// so a bare name wouldn't resolve). `/usr/bin/env` with no args prints `KEY=VALUE` lines.
    fn env_dumper() -> Option<&'static str> {
        ["/usr/bin/env", "/bin/env"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
    }

    /// THE finding-C1 proof: after [`apply_child_env`] the spawned child's REAL environment is
    /// cleared down to exactly the control allowlist — every inherited var, every loader-injection
    /// vector (`LD_PRELOAD`/`LD_AUDIT`/`DYLD_INSERT_LIBRARIES`), and an injected `$PATH` are gone.
    /// We pre-set the hostile vars as explicit overrides; `env_clear()` MUST wipe them. (No
    /// process-global env mutation, so this is race-free under the parallel test runner.)
    #[test]
    fn env_clear_strips_loader_vars_and_path_to_the_control_allowlist() {
        let Some(dumper) = env_dumper() else {
            eprintln!("skipping: no /usr/bin/env to dump the child environment");
            return;
        };
        let mut cmd = Command::new(dumper);
        // Hostile loader vars + an attacker PATH, set as overrides the clear must erase.
        cmd.env("LD_PRELOAD", "/tmp/evil.so")
            .env("LD_AUDIT", "/tmp/evil-audit.so")
            .env("DYLD_INSERT_LIBRARIES", "/tmp/evil.dylib")
            .env("PATH", "/attacker/bin");
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        // No allowlist forwarded → the child must end up with EXACTLY the five control vars.
        apply_child_env(&mut cmd, &child_env(), 7, |_| None);

        let out = cmd.output().expect("spawn env dumper");
        assert!(out.status.success(), "env dumper failed: {:?}", out.status);
        let printed = String::from_utf8_lossy(&out.stdout);

        let keys: std::collections::BTreeSet<&str> = printed
            .lines()
            .filter_map(|l| l.split('=').next())
            .filter(|k| !k.is_empty())
            .collect();
        let expected: std::collections::BTreeSet<&str> = [
            "DECKARD_SOCKET_PATH",
            "DECKARD_RPC_URL",
            "DECKARD_CHAIN_ID",
            "DECKARD_CONFIG_DIR",
            "DECKARD_RESOLVE_FD",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys, expected,
            "daemon child env must be cleared to exactly the control allowlist; loader vars / \
             PATH must be gone. Got:\n{printed}"
        );
        // Belt-and-suspenders: the hostile VALUES don't survive anywhere in the output.
        assert!(
            !printed.contains("/attacker/bin"),
            "injected PATH leaked into child:\n{printed}"
        );
        assert!(
            !printed.contains("/tmp/evil"),
            "loader-injection var leaked into child:\n{printed}"
        );
        // And the control values are carried through verbatim.
        assert!(printed.contains("DECKARD_SOCKET_PATH=/tmp/deckard-test.sock"));
        assert!(printed.contains("DECKARD_CONFIG_DIR=/tmp/deckard-test-cfg"));
        assert!(printed.contains("DECKARD_RESOLVE_FD=7"));
    }

    /// The allowlist is a *forward-if-present* filter: Deckard-namespace toggles the daemon reads
    /// pass through, but `DECKARD_SIGNERD_BIN` (the binary-substitution vector) and any unrelated
    /// var do NOT — even though they live in the (fake) parent environment.
    #[test]
    fn allowlist_forwards_toggles_but_not_the_bin_override_or_unrelated_vars() {
        let Some(dumper) = env_dumper() else {
            eprintln!("skipping: no /usr/bin/env to dump the child environment");
            return;
        };
        let parent: std::collections::HashMap<&str, OsString> = [
            ("DECKARD_VERIFIED_READS", "0"),         // allowlisted → forwarded
            ("DECKARD_I_KNOW_THIS_IS_MAINNET", "1"), // allowlisted → forwarded
            ("DECKARD_APPROVAL_TTL_SECS", "5"),      // NOT allowlisted → dropped
            ("DECKARD_SIGNERD_BIN", "/tmp/fake-signerd"), // must NOT be forwarded
            ("LD_PRELOAD", "/tmp/evil.so"),          // must NOT be forwarded
            ("SOME_UNRELATED_SECRET", "leak-me"),    // must NOT be forwarded
        ]
        .into_iter()
        .map(|(k, v)| (k, OsString::from(v)))
        .collect();

        let mut cmd = Command::new(dumper);
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        apply_child_env(&mut cmd, &child_env(), 9, |k| parent.get(k).cloned());

        let out = cmd.output().expect("spawn env dumper");
        let printed = String::from_utf8_lossy(&out.stdout);

        // Allowlisted toggles forwarded, value preserved.
        assert!(
            printed.contains("DECKARD_VERIFIED_READS=0"),
            "got:\n{printed}"
        );
        assert!(
            printed.contains("DECKARD_I_KNOW_THIS_IS_MAINNET=1"),
            "got:\n{printed}"
        );
        // The un-allowlisted TTL knob, the binary-substitution override, and unrelated/loader vars
        // are all dropped — a poisoned app env can't reach the daemon through them.
        for forbidden in [
            "DECKARD_APPROVAL_TTL_SECS",
            "DECKARD_SIGNERD_BIN",
            "LD_PRELOAD",
            "SOME_UNRELATED_SECRET",
        ] {
            assert!(
                !printed
                    .lines()
                    .any(|l| l.starts_with(&format!("{forbidden}="))),
                "{forbidden} must not reach the daemon child:\n{printed}"
            );
        }
    }

    /// Release-path provenance gate. Builds candidate binaries in a temp dir and asserts the
    /// ownership/permission/symlink rules. Compiled only when the dev override is NOT present
    /// (i.e. the path this gate actually guards).
    #[cfg(not(feature = "dev-signerd-bin"))]
    mod release_resolver {
        use super::super::verify_bundled_binary;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        fn fresh_dir(tag: &str) -> PathBuf {
            let dir =
                std::env::temp_dir().join(format!("deckard-prov-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("mkdir temp");
            dir
        }

        fn write_bin(dir: &std::path::Path, mode: u32) -> PathBuf {
            let bin = dir.join("deckard-signerd");
            std::fs::write(&bin, b"#!/bin/true\n").expect("write bin");
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(mode))
                .expect("chmod bin");
            bin
        }

        #[test]
        fn accepts_an_owned_nonwritable_regular_file() {
            let dir = fresh_dir("ok");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            let bin = write_bin(&dir, 0o755);
            assert_eq!(verify_bundled_binary(&bin).unwrap(), bin);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn rejects_a_group_or_world_writable_binary() {
            let dir = fresh_dir("ww");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            let bin = write_bin(&dir, 0o777);
            assert!(
                verify_bundled_binary(&bin).is_err(),
                "world-writable binary must be refused"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn rejects_a_symlinked_binary() {
            let dir = fresh_dir("sym");
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            let real = dir.join("real-signerd");
            std::fs::write(&real, b"x").unwrap();
            std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
            let link = dir.join("deckard-signerd");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert!(
                verify_bundled_binary(&link).is_err(),
                "a symlinked daemon binary (attacker redirect) must be refused"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn rejects_a_world_writable_parent_dir() {
            let dir = fresh_dir("wwdir");
            let bin = write_bin(&dir, 0o755);
            // World-writable → the file can be swapped out from under us.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
            assert!(
                verify_bundled_binary(&bin).is_err(),
                "a world-writable parent dir must be refused"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn rejects_a_sticky_world_writable_parent_dir() {
            // No sticky-bit carve-out: the sticky bit only fences OTHER uids, not a same-uid
            // attacker, and a release binary never legitimately lives in a world-writable dir.
            let dir = fresh_dir("sticky");
            let bin = write_bin(&dir, 0o755);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
            assert!(
                verify_bundled_binary(&bin).is_err(),
                "a world-writable parent dir must be refused even when sticky"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
