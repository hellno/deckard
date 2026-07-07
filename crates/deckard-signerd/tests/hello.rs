//! The daemon-socket half of the wire-evolution rules (#31). The contract-layer proofs live in
//! `deckard-contract` (`wire_evolution`: E1 shape, E2 byte-identity, E3 unknown-variant rejection,
//! E4 unknown-key tolerance). Here we prove the two properties that only a *running daemon* can
//! show:
//!
//!   1. `Hello` is answered in **every** state, including `Locked` — capability discovery reveals
//!      the capability names but no vault state, and it does so from the single-source registry so
//!      the daemon's answer is byte-identical to what the mocks return (parity by construction).
//!   2. An unknown request *kind* on the wire is the backward-compat valve in action: the daemon
//!      rejects the frame LOUDLY (`malformed_request`), never panics, signs NOTHING, and keeps
//!      serving — a new peer talking to an old daemon degrades safely.

mod common;

use std::path::Path;

use deckard_contract::{
    capabilities, deny_reasons, Decision, HelloInfo, SignerRequest, SignerResponse, UnlockOutcome,
};
use deckard_signerd::{frame, SignerClient};

use common::*;

const CHAIN: u64 = 31337;
/// Hello + the bad-frame reject never touch a chain, so a dead RPC is fine.
const DUMMY_RPC: &str = "http://127.0.0.1:1";

/// `spec_version` is a real `YYYY-MM-DD` (validated without a regex dep).
fn is_iso_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    matches!(parts.as_slice(), [y, m, d]
        if y.len() == 4 && m.len() == 2 && d.len() == 2
            && [y, m, d].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())))
}

/// Round-trip a `Hello` over the public socket and unwrap the [`HelloInfo`].
async fn hello(client: &SignerClient) -> HelloInfo {
    match client.request(&SignerRequest::Hello).await.expect("hello") {
        SignerResponse::Hello(info) => info,
        other => panic!("expected a Hello reply, got {other:?}"),
    }
}

/// Send a raw frame carrying a request *kind* the daemon has never heard of, and return the
/// daemon's reply. Simulates a newer peer proposing a future variant to today's daemon.
async fn send_unknown_kind(socket: &Path) -> SignerResponse {
    use tokio::net::UnixStream;

    // A future wire superset. Its `WarpDrive` tag is not a `SignerRequest` variant, so the daemon
    // fails to decode the frame — exactly the old-daemon-meets-new-request path (#31 rules #1/#3).
    #[derive(serde::Serialize)]
    enum FutureRequest {
        WarpDrive,
    }

    let mut stream = UnixStream::connect(socket).await.expect("connect socket");
    let body = frame::encode(&FutureRequest::WarpDrive).expect("encode future frame");
    frame::write_frame(&mut stream, &body)
        .await
        .expect("write future frame");
    let resp = frame::read_frame(&mut stream)
        .await
        .expect("read reply")
        .expect("daemon replied before closing");
    frame::decode(&resp).expect("decode reply")
}

#[tokio::test]
async fn hello_answers_in_every_state_and_survives_a_bad_frame() {
    let dir = TempDir::new("hello");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    // (1) Answered while LOCKED (the daemon starts locked; we have not unlocked). The reply carries
    // the capability names + spec_version + impl_name and nothing else, and it equals the
    // single-source registry — so the daemon can never drift from the mocks (parity contract).
    let locked = hello(&client).await;
    assert!(is_iso_date(&locked.spec_version), "spec_version not a date");
    assert!(
        locked
            .capabilities
            .iter()
            .any(|c| c == capabilities::CAP_CORE),
        "capabilities must include core"
    );
    assert!(
        locked
            .capabilities
            .iter()
            .any(|c| c == capabilities::CAP_MCP_V0_1),
        "capabilities must include mcp.v0.1"
    );
    assert_eq!(locked.impl_name, capabilities::IMPL_SIGNERD);
    assert_eq!(
        locked,
        capabilities::hello_info(capabilities::IMPL_SIGNERD),
        "the daemon's Hello must equal the single-source registry"
    );

    // (2) Unlock, then Hello again → identical: it is answered in every state, and unlocking never
    // changes the advertised capabilities (discovery is not state-dependent).
    assert_eq!(
        client.unlock(PASS).await.expect("unlock"),
        UnlockOutcome::Unlocked { address: wallet }
    );
    let unlocked = hello(&client).await;
    assert_eq!(locked, unlocked, "Hello must be identical across states");

    // (3) A future request kind → the daemon rejects the frame LOUDLY and does not panic. This is
    // the compat valve: an old daemon answers a new kind with the existing frame-decode error.
    match send_unknown_kind(&d.socket_path).await {
        SignerResponse::Decision(Decision::Deny { reason }) => {
            assert_eq!(reason, deny_reasons::MALFORMED_REQUEST);
        }
        other => panic!("expected Deny{{malformed_request}}, got {other:?}"),
    }

    // (4) The daemon survived: a fresh Hello still answers identically, and — the security-critical
    // part — the malformed frame signed and recorded NOTHING (the activity ledger is still empty
    // even though the daemon is unlocked and *could* have signed).
    assert_eq!(
        hello(&client).await,
        unlocked,
        "daemon must keep serving after a bad frame"
    );
    assert!(
        client
            .activity_feed()
            .await
            .expect("activity feed")
            .is_empty(),
        "a malformed frame must sign and record nothing"
    );
}
