//! The UDS server: accept connections, gate each on same-uid peer-cred, then serve a stream
//! of length-delimited CBOR request/response frames. All requests funnel through the single
//! shared [`Daemon`] behind a `tokio::sync::Mutex`, so they are serialized and can't race.

use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use zeroize::Zeroize;

use deckard_contract::{Decision, SignerRequest, SignerResponse};

use crate::auth;
use crate::daemon::Daemon;
use crate::frame;

/// Accept loop. Rejects (and drops) any connection whose peer uid differs from ours, then
/// spawns a per-connection task. Runs until the listener errors fatally.
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
            if let Err(e) = handle_conn(stream, daemon).await {
                eprintln!("signerd: connection closed: {e}");
            }
        });
    }
}

/// Serve one connection: read a frame, decode it, zeroize the raw bytes (they may hold an
/// `Unlock` passphrase), dispatch, write the response. A malformed/oversize frame gets one
/// error response and the connection is closed.
async fn handle_conn(mut stream: UnixStream, daemon: Arc<Mutex<Daemon>>) -> anyhow::Result<()> {
    loop {
        let mut buf = match frame::read_frame(&mut stream).await {
            Ok(Some(buf)) => buf,
            Ok(None) => return Ok(()), // peer closed cleanly between frames
            Err(e) => {
                // Oversize/short read: best-effort error, then close.
                let _ = reply_error(&mut stream, "malformed_request").await;
                return Err(e);
            }
        };

        let decoded: Result<SignerRequest, _> = frame::decode(&buf);
        // Always scrub the raw frame: an Unlock frame carries the passphrase bytes.
        buf.zeroize();

        let req = match decoded {
            Ok(req) => req,
            Err(e) => {
                let _ = reply_error(&mut stream, "malformed_request").await;
                return Err(e);
            }
        };

        // Dispatch behind the shared lock (serializes all requests). Note: we deliberately
        // never log the request contents — an Unlock passphrase must never reach a log line.
        let resp = daemon.lock().await.handle(req).await;

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
