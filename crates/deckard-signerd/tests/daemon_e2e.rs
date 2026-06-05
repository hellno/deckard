//! End-to-end daemon tests that drive the real `deckard-signerd` binary over the socket, but
//! do NOT need a chain (no transaction is broadcast). Covers acceptance #2 (socket perms),
//! #3 (unlock outcomes), the propose-decision half of #4 (chain mismatch / unsupported kind /
//! allowlist / cap classification), #5 (resolve-false + TTL deny execute), #6 (STOP zeroize +
//! re-arm), and #7 (TOCTOU). The successful broadcasts live in `anvil_e2e.rs`.

mod common;

use std::os::unix::fs::PermissionsExt;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalMode, ApprovalStatus, Decision, ExecuteResult, Intent, IntentKind, Policy,
    SignerRequest, SignerResponse, UnlockOutcome,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
/// We never broadcast in this file, so the RPC is never contacted — a dead address is fine.
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

#[tokio::test]
async fn unlock_outcomes() {
    // NoVault: empty config dir.
    let dir = TempDir::new("novault");
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    assert_eq!(
        client.unlock("anything").await.unwrap(),
        UnlockOutcome::NoVault
    );
    drop(d);

    // Sealed vault: wrong → BadPassphrase; correct → Unlocked{account0}.
    let dir = TempDir::new("unlock");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    assert_eq!(
        client.unlock("wrong-pass").await.unwrap(),
        UnlockOutcome::BadPassphrase
    );
    assert_eq!(
        client.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
}

#[tokio::test]
async fn propose_decision_matrix() {
    let dir = TempDir::new("propose");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    // Locked → Deny{locked}.
    assert_eq!(
        client.propose(&send(to, 1_000)).await.unwrap(),
        Decision::Deny {
            reason: "locked".into()
        }
    );
    client.unlock(PASS).await.unwrap();

    // Within cap → Allow.
    assert_eq!(
        client.propose(&send(to, 1_000)).await.unwrap(),
        Decision::Allow
    );

    // Over per-tx cap → NeedsApproval.
    assert!(matches!(
        client.propose(&send(to, PER_TX_CAP + 1)).await.unwrap(),
        Decision::NeedsApproval { .. }
    ));

    // chain mismatch.
    let mut wrong_chain = send(to, 1_000);
    wrong_chain.chain_id = 1;
    assert_eq!(
        client.propose(&wrong_chain).await.unwrap(),
        Decision::Deny {
            reason: "chain_mismatch".into()
        }
    );

    // unsupported kind (Shield is T-Privacy).
    let mut shield = send(to, 1_000);
    shield.kind = IntentKind::Shield;
    assert_eq!(
        client.propose(&shield).await.unwrap(),
        Decision::Deny {
            reason: "unsupported_v1".into()
        }
    );

    // ERC-20 send (token = Some) is a fast-follow.
    let mut erc20 = send(to, 1_000);
    erc20.token = Some(Address::repeat_byte(0xEE));
    assert_eq!(
        client.propose(&erc20).await.unwrap(),
        Decision::Deny {
            reason: "erc20_unsupported_v1".into()
        }
    );
}

#[tokio::test]
async fn off_allowlist_denies() {
    let dir = TempDir::new("allowlist");
    let (_wallet, to) = seal_account0(dir.path());
    // A policy whose allowlist excludes `to`.
    let policy = Policy {
        per_tx_cap_wei: U256::from(PER_TX_CAP),
        daily_cap_wei: U256::from(200_000_000_000_000_000u64),
        spent_today_wei: U256::ZERO,
        allow_to: vec![Address::repeat_byte(0x99)],
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u64),
        require_approval: ApprovalMode::OverCap,
        revoked: false,
    };
    std::fs::write(
        dir.path().join("policy.json"),
        serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();

    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();
    assert_eq!(
        client.propose(&send(to, 1_000)).await.unwrap(),
        Decision::Deny {
            reason: "off_allowlist".into()
        }
    );
}

#[tokio::test]
async fn resolve_false_and_ttl_deny_execute() {
    let dir = TempDir::new("ttl");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // resolve(false) → execute Denied{user_denied}.
    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    ack(
        &client,
        SignerRequest::Resolve {
            request_id: id,
            approved: false,
        },
    )
    .await;
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "user_denied".into()
        }
    );

    // TTL expiry → status Expired, execute Denied{expired}. (Different value → different id.)
    let over2 = send(to, PER_TX_CAP + 2);
    let id2 = needs_approval_id(&client, &over2).await;
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    assert_eq!(status(&client, id2).await, ApprovalStatus::Expired);
    assert_eq!(
        client.execute(id2).await.unwrap(),
        ExecuteResult::Denied {
            reason: "expired".into()
        }
    );
}

#[tokio::test]
async fn stop_zeroizes_and_re_arms() {
    let dir = TempDir::new("stop");
    let (wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // An Allowed within-cap request, then STOP.
    let intent = send(to, 1_000);
    assert_eq!(client.propose(&intent).await.unwrap(), Decision::Allow);
    let id = SignerClient::request_id_for_intent(&intent);
    ack(&client, SignerRequest::RevokeAll).await;

    // Address now reports locked (Deny-style, per the schema).
    match client.request(&SignerRequest::Address).await.unwrap() {
        SignerResponse::Decision(Decision::Deny { reason }) => assert_eq!(reason, "locked"),
        other => panic!("expected locked Deny for Address, got {other:?}"),
    }
    // execute on the pre-STOP Allow → Denied{revoked}.
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );

    // A fresh unlock re-arms (and starts a clean session).
    assert_eq!(
        client.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
    assert_eq!(
        client.propose(&send(to, 1_000)).await.unwrap(),
        Decision::Allow
    );
}

#[tokio::test]
async fn toctou_resolve_then_revoke_then_execute_denied() {
    let dir = TempDir::new("toctou");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    ack(
        &client,
        SignerRequest::Resolve {
            request_id: id,
            approved: true,
        },
    )
    .await;
    assert_eq!(status(&client, id).await, ApprovalStatus::Allowed);
    // STOP after approval but before execute.
    ack(&client, SignerRequest::RevokeAll).await;
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );
}

#[tokio::test]
async fn socket_is_0600_in_0700_dir() {
    let dir = TempDir::new("perms");
    let _ = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);

    let sock_mode = std::fs::metadata(&d.socket_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(sock_mode, 0o600, "socket must be 0600");
    let parent = d.socket_path.parent().unwrap();
    let dir_mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "socket dir must be 0700");
}

#[tokio::test]
async fn allowed_request_expires() {
    // #2 regression: an APPROVED (Allowed) request goes stale after the TTL and can't execute.
    let dir = TempDir::new("allowed-ttl");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    ack(
        &client,
        SignerRequest::Resolve {
            request_id: id,
            approved: true,
        },
    )
    .await;
    assert_eq!(status(&client, id).await, ApprovalStatus::Allowed);

    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    assert_eq!(status(&client, id).await, ApprovalStatus::Expired);
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "expired".into()
        }
    );
}

#[tokio::test]
async fn re_propose_is_idempotent() {
    // #3 regression: a re-propose never resets a live card, re-raises a Deny, or downgrades
    // an approval.
    let dir = TempDir::new("idempotent");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // (a) within-cap: re-propose returns the same Allow.
    let within = send(to, 1_000);
    assert_eq!(client.propose(&within).await.unwrap(), Decision::Allow);
    assert_eq!(client.propose(&within).await.unwrap(), Decision::Allow);

    // (b) a user Deny is sticky — re-propose does NOT re-raise the card.
    let denied = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &denied).await;
    ack(
        &client,
        SignerRequest::Resolve {
            request_id: id,
            approved: false,
        },
    )
    .await;
    assert_eq!(
        client.propose(&denied).await.unwrap(),
        Decision::Deny {
            reason: "user_denied".into()
        }
    );

    // (c) an approval is not downgraded back to Pending by a re-propose.
    let approved = send(to, PER_TX_CAP + 2);
    let id2 = needs_approval_id(&client, &approved).await;
    ack(
        &client,
        SignerRequest::Resolve {
            request_id: id2,
            approved: true,
        },
    )
    .await;
    assert_eq!(client.propose(&approved).await.unwrap(), Decision::Allow);
}

// --- small request helpers -----------------------------------------------------------------

async fn needs_approval_id(client: &SignerClient, intent: &Intent) -> deckard_contract::RequestId {
    match client.propose(intent).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

async fn ack(client: &SignerClient, req: SignerRequest) {
    assert_eq!(client.request(&req).await.unwrap(), SignerResponse::Ack);
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
