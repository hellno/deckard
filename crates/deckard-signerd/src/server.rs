//! The UDS server: accept connections, gate each on same-uid peer-cred, then serve a stream
//! of length-delimited CBOR request/response frames. All requests funnel through the single
//! shared [`Daemon`] behind a `tokio::sync::Mutex`, so they are serialized and can't race.
//!
//! Two channels reach the one daemon (PRD-01):
//! - the **public** [`serve`] accept-loop on the `0600` socket — propose/read/execute/STOP,
//!   gated on same-uid peer-cred (defense-in-depth + logging), but never `Resolve`;
//! - the **private** [`serve_control`] single connection over the [`socketpair`] end the app
//!   handed us by fd inheritance ([`adopt_control_fd`]) — the only channel that may `Resolve`.

use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use zeroize::Zeroize;

use deckard_contract::{deny_reasons, Decision, SignerRequest, SignerResponse};

use crate::auth;
use crate::daemon::{Channel, Daemon};
use crate::frame;

/// Env var naming the inherited capability-channel fd (set by [`crate::supervise`] on the
/// daemon child). Its presence is what arms `Resolve` on the control channel; without it the
/// daemon serves only the public socket and `Resolve` is refused everywhere (fail-closed).
pub const RESOLVE_FD_ENV: &str = "DECKARD_RESOLVE_FD";

/// Accept loop for the **public** proposer socket. Rejects (and drops) any connection whose
/// peer uid differs from ours, then spawns a per-connection task tagged [`Channel::Public`].
/// Runs until the listener errors fatally.
pub async fn serve(listener: UnixListener, daemon: Arc<Mutex<Daemon>>) -> anyhow::Result<()> {
    let our = auth::our_uid();
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _addr)) => stream,
            Err(e) => {
                eprintln!("signerd: accept error: {e}");
                continue;
            }
        };

        match auth::peer_uid(&stream) {
            Ok(uid) if auth::same_uid(uid, our) => {}
            Ok(uid) => {
                eprintln!("signerd: rejecting connection from uid {uid} (daemon uid {our})");
                continue; // drop the stream → connection refused
            }
            Err(e) => {
                eprintln!("signerd: peer-cred check failed, dropping connection: {e}");
                continue;
            }
        }

        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, daemon, Channel::Public).await {
                eprintln!("signerd: connection closed: {e}");
            }
        });
    }
}

/// Serve the **private** capability channel: a single, already-connected `socketpair` end
/// inherited from the supervising app. There is no `accept`/peer-cred here — the channel is
/// trusted by construction (a same-process-tree fd the app never shares), which is precisely
/// the unforgeable role proof same-uid peer-cred cannot give. Requests are tagged
/// [`Channel::Control`], so this is the only place a `Resolve` is honoured.
pub async fn serve_control(stream: UnixStream, daemon: Arc<Mutex<Daemon>>) -> anyhow::Result<()> {
    handle_conn(stream, daemon, Channel::Control).await
}

/// Adopt the inherited capability-channel fd named by [`RESOLVE_FD_ENV`], if present, into a
/// tokio [`UnixStream`]. Returns `Ok(None)` when the env var is absent (the daemon then has no
/// control channel and refuses every `Resolve`). A malformed/unusable value is surfaced as an
/// error so a misconfiguration is loud, not silent.
pub fn adopt_control_fd() -> anyhow::Result<Option<UnixStream>> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::net::UnixStream as StdUnixStream;

    let Some(val) = std::env::var_os(RESOLVE_FD_ENV) else {
        return Ok(None);
    };
    let fd: RawFd = val
        .to_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("{RESOLVE_FD_ENV} is not a valid fd number"))?;

    // SAFETY: the supervising parent (`crate::supervise`) created an AF_UNIX `socketpair`,
    // left this end without close-on-exec, and named it in `{RESOLVE_FD_ENV}` for us to
    // inherit across exec. We are the sole owner of this fd in this process — the parent
    // closed its copy of the child end right after spawn — so adopting it into an `OwnedFd`
    // (whose `Drop` then closes it exactly once) is sound. reason: there is no safe std API
    // to reclaim an inherited fd by number; this is the boundary the issue scopes the one
    // `unsafe` to (daemon-side), keeping `deckard-core`'s `#![forbid(unsafe_code)]` intact.
    #[allow(unsafe_code)]
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };

    // Validate before trusting: `{RESOLVE_FD_ENV}` is just an integer in the environment, so a
    // stale/crafted value could name an fd we already own (the public listener, stdout, …).
    // Confirm it really is a stream socket — otherwise refuse loudly rather than (a) treating an
    // arbitrary inherited fd as the approval-authorised channel or (b) double-closing a live fd
    // on `OwnedFd`'s `Drop`. Then set close-on-exec so the capability never leaks further.
    let sock_type = nix::sys::socket::getsockopt(&owned, nix::sys::socket::sockopt::SockType)
        .map_err(|e| anyhow::anyhow!("{RESOLVE_FD_ENV} is not a socket: {e}"))?;
    anyhow::ensure!(
        sock_type == nix::sys::socket::SockType::Stream,
        "{RESOLVE_FD_ENV} fd is not a stream socket"
    );
    nix::fcntl::fcntl(
        owned.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )
    .map_err(|e| anyhow::anyhow!("set close-on-exec on control fd: {e}"))?;

    let std_stream = StdUnixStream::from(owned);
    std_stream.set_nonblocking(true)?;
    Ok(Some(UnixStream::from_std(std_stream)?))
}

/// Serve one connection: read a frame, decode it, zeroize the raw bytes (they may hold an
/// `Unlock` passphrase), dispatch on `channel`, write the response. A malformed/oversize frame
/// gets one error response and the connection is closed.
async fn handle_conn(
    mut stream: UnixStream,
    daemon: Arc<Mutex<Daemon>>,
    channel: Channel,
) -> anyhow::Result<()> {
    loop {
        let mut buf = match frame::read_frame(&mut stream).await {
            Ok(Some(buf)) => buf,
            Ok(None) => return Ok(()), // peer closed cleanly between frames
            Err(e) => {
                // Oversize/short read: best-effort error, then close.
                let _ = reply_error(&mut stream, deny_reasons::MALFORMED_REQUEST).await;
                return Err(e);
            }
        };

        let decoded: Result<SignerRequest, _> = frame::decode(&buf);
        // Always scrub the raw frame: an Unlock frame carries the passphrase bytes.
        buf.zeroize();

        let req = match decoded {
            Ok(req) => req,
            Err(e) => {
                let _ = reply_error(&mut stream, deny_reasons::MALFORMED_REQUEST).await;
                return Err(e);
            }
        };

        // A `Balance` read needs the embedded Helios light client. Its first-time bootstrap
        // (`launch_verified`) can take seconds-to-90s; prime it HERE, OFF the daemon lock, so
        // the long bootstrap never serializes ahead of the security brake (STOP/Lock) or any
        // other request. The cell is idempotent and separately locked — after this returns,
        // the daemon's `balance` handler does only the quick verified read under its mutex.
        // Skipped in demo/local-fork mode (`DECKARD_VERIFIED_READS=0`): Helios is mainnet-only,
        // so priming it against a fork would stall here for nothing — the handler reads raw.
        #[cfg(feature = "verified-reads")]
        if matches!(req, SignerRequest::Balance { .. }) && deckard_core::verified_reads_enabled() {
            let (cell, (cl, el, data_dir)) = {
                let d = daemon.lock().await;
                (d.helios_cell(), d.helios_bootstrap_args())
            };
            cell.ensure(cl, &el, data_dir).await;
        }

        // Dispatch behind the shared lock (serializes all requests). Note: we deliberately
        // never log the request contents — an Unlock passphrase must never reach a log line.
        let resp = daemon.lock().await.handle(req, channel).await;

        let body = frame::encode(&resp)?;
        frame::write_frame(&mut stream, &body).await?;
    }
}

/// Send a generic Deny-style error response (used when we can't even decode the request, so
/// the precise response variant is unknown).
async fn reply_error(stream: &mut UnixStream, reason: &str) -> anyhow::Result<()> {
    let resp = SignerResponse::Decision(Decision::Deny {
        reason: reason.to_string(),
    });
    let body = frame::encode(&resp)?;
    frame::write_frame(stream, &body).await
}
