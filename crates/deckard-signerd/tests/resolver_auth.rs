//! PRD-01 — resolver authentication. The daemon honours `Resolve` (approval) ONLY on the
//! private capability channel the supervising app inherits (`supervise.rs` → `socketpair`);
//! a `Resolve` on the public same-uid proposer socket is refused with a typed denial. This
//! closes `THREAT-MODEL.md` residual-risk #1 (same-uid self-approval) and is the prerequisite
//! for any external proposer.
//!
//! These run under plain `cargo test` (none `#[ignore]`); no chain is needed (nothing is
//! broadcast — every assertion is about the approval gate, not signing).

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalStatus, Decision, Intent, IntentKind, SignerRequest, SignerResponse,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
/// Nothing is broadcast here, so the RPC is never contacted.
const DUMMY_RPC: &str = "http://127.0.0.1:1";
const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH (the default policy cap)

fn send(to: Address, value: u64) -> Intent {
    Intent {
        chain_id: CHAIN,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
}

/// Propose an over-cap Send → its `NeedsApproval` request id (a `Pending` card to approve).
async fn pending_id(client: &SignerClient, to: Address) -> deckard_contract::RequestId {
    match client.propose(&send(to, PER_TX_CAP + 1)).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

async fn status(client: &SignerClient, id: deckard_contract::RequestId) -> ApprovalStatus {
    match client
        .request(&SignerRequest::Status { request_id: id })
        .await
        .unwrap()
    {
        SignerResponse::Status(s) => s,
        other => panic!("expected Status, got {other:?}"),
    }
}

#[tokio::test]
async fn resolve_rejected_on_public_socket() {
    // A `Resolve` on the public proposer socket is refused with a typed, payload-free denial,
    // and the pending record is left untouched (still `Pending`, still approvable by the app).
    let dir = TempDir::new("resolver-public-reject");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to).await;

    // Public-socket Resolve → Deny{resolve_not_authorized}.
    match client
        .request(&SignerRequest::Resolve {
            request_id: id,
            approved: true,
        })
        .await
        .unwrap()
    {
        SignerResponse::Decision(Decision::Deny { reason }) => {
            assert_eq!(reason, "resolve_not_authorized")
        }
        other => panic!("public Resolve must be denied, got {other:?}"),
    }

    // The record was NOT approved — it is still Pending.
    assert_eq!(status(&client, id).await, ApprovalStatus::Pending);
}

#[tokio::test]
async fn resolve_accepted_on_control_channel() {
    // Over the inherited capability channel, the same `Resolve` flips Pending → Allowed.
    let dir = TempDir::new("resolver-control-accept");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to).await;
    d.resolve(id, true); // the authenticated channel
    assert_eq!(status(&client, id).await, ApprovalStatus::Allowed);
}

#[tokio::test]
async fn stop_still_works_on_public_socket() {
    // STOP only REDUCES authority, so it stays reachable on the public socket (it must never
    // depend on the capability handshake).
    let dir = TempDir::new("resolver-stop-public");
    let (_wallet, _to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    assert_eq!(
        client.request(&SignerRequest::RevokeAll).await.unwrap(),
        SignerResponse::Ack
    );
    // The key is gone: Address reports locked Deny-style.
    match client.request(&SignerRequest::Address).await.unwrap() {
        SignerResponse::Decision(Decision::Deny { reason }) => assert_eq!(reason, "locked"),
        other => panic!("expected locked Deny after public STOP, got {other:?}"),
    }
}

#[tokio::test]
async fn second_proposer_cannot_self_approve() {
    // The red-team scenario (issue #19 / residual #1): a SECOND same-uid proposer opens its own
    // connection, sees the NeedsApproval, and rubber-stamps it — exactly the ~20-line bypass.
    // Post-PRD-01 it is refused: a proposer can propose, but it cannot approve. Only the app's
    // inherited control channel can.
    let dir = TempDir::new("resolver-red-team");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);

    // The honest proposer (e.g. the MCP sidecar) raises a card.
    let proposer = SignerClient::new(d.socket_path.clone());
    proposer.unlock(PASS).await.unwrap();
    let id = pending_id(&proposer, to).await;

    // The attacker: a second same-uid client speaking the documented wire. It can recompute the
    // id (deterministic) and tries to self-approve over its own public connection.
    let attacker = SignerClient::new(d.socket_path.clone());
    match attacker
        .request(&SignerRequest::Resolve {
            request_id: id,
            approved: true,
        })
        .await
        .unwrap()
    {
        SignerResponse::Decision(Decision::Deny { reason }) => {
            assert_eq!(reason, "resolve_not_authorized")
        }
        other => panic!("the attacker's self-approve must be denied, got {other:?}"),
    }
    // Still Pending — the bypass failed, and execute is refused at the approval gate.
    assert_eq!(status(&proposer, id).await, ApprovalStatus::Pending);
    match proposer.execute(id).await.unwrap() {
        deckard_contract::ExecuteResult::Denied { reason } => assert_eq!(reason, "not_approved"),
        other => panic!("an unapproved request must not execute, got {other:?}"),
    }

    // The legitimate resolver (the app's capability channel) CAN approve.
    d.resolve(id, true);
    assert_eq!(status(&proposer, id).await, ApprovalStatus::Allowed);
}
