//! Broadcast tests on a local anvil node (the only assertions that need a chain): a within-cap
//! send signs + broadcasts with a real receipt (#4), and an over-cap send broadcasts after
//! approval (#5). Native ETH sends don't need a mainnet fork, so a plain local anvil suffices
//! and CI needs no RPC secret. Skips gracefully when `anvil` isn't installed.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalMode, ApprovalStatus, Decision, ExecuteResult, Intent, IntentKind, Policy,
    SignerRequest, SignerResponse,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
const PER_TX_CAP: u128 = 50_000_000_000_000_000; // 0.05 ETH

fn send(to: Address, value: u128) -> Intent {
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
async fn within_cap_send_broadcasts_with_receipt() {
    if !anvil_available() {
        eprintln!("SKIP within_cap_send_broadcasts_with_receipt: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("anvil-send");
    let (_wallet, recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let value: u128 = 10_000_000_000_000_000; // 0.01 ETH, within cap
    let intent = send(recipient, value);
    assert_eq!(client.propose(&intent).await.unwrap(), Decision::Allow);
    let id = SignerClient::request_id_for_intent(&intent);

    let before = balance(&anvil.url(), recipient).await;
    let tx_hash = match client.execute(id).await.unwrap() {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };

    let receipt = wait_receipt(&anvil.url(), tx_hash)
        .await
        .expect("a real receipt");
    assert!(receipt.status(), "tx should have succeeded");
    let after = balance(&anvil.url(), recipient).await;
    assert_eq!(
        after - before,
        U256::from(value),
        "recipient credited exactly the sent value"
    );

    // Idempotency: a second execute of the same id is refused.
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "already_executed".into()
        }
    );
}

#[tokio::test]
async fn over_cap_approve_then_execute_broadcasts() {
    if !anvil_available() {
        eprintln!("SKIP over_cap_approve_then_execute_broadcasts: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("anvil-approve");
    let (_wallet, recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let value: u128 = PER_TX_CAP + 10_000_000_000_000_000; // 0.06 ETH > cap
    let intent = send(recipient, value);
    let id = match client.propose(&intent).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    };

    // Approve (the native card / a test), then execute → broadcast.
    assert_eq!(
        client
            .request(&SignerRequest::Resolve {
                request_id: id,
                approved: true
            })
            .await
            .unwrap(),
        SignerResponse::Ack
    );
    assert_eq!(
        client
            .request(&SignerRequest::Status { request_id: id })
            .await
            .unwrap(),
        SignerResponse::Status(ApprovalStatus::Allowed)
    );

    let before = balance(&anvil.url(), recipient).await;
    let tx_hash = match client.execute(id).await.unwrap() {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };
    let receipt = wait_receipt(&anvil.url(), tx_hash)
        .await
        .expect("a real receipt");
    assert!(receipt.status());
    let after = balance(&anvil.url(), recipient).await;
    assert_eq!(after - before, U256::from(value));
}

#[tokio::test]
async fn daily_cap_enforced_at_execute() {
    // #1 regression: two within-cap proposals both Allow (spent=0 at propose), but once the
    // first broadcasts, the second can't execute past the daily cap — the auto-allow is
    // re-checked against the caps at sign time.
    if !anvil_available() {
        eprintln!("SKIP daily_cap_enforced_at_execute: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("anvil-dailycap");
    let (_wallet, recipient) = seal_account0(dir.path());
    // Tight policy: per-tx 0.05, daily 0.05. 0.04 + 0.039 each pass at propose, but together
    // exceed the 0.05 daily cap.
    let policy = Policy {
        per_tx_cap_wei: U256::from(50_000_000_000_000_000u128),
        daily_cap_wei: U256::from(50_000_000_000_000_000u128),
        spent_today_wei: U256::ZERO,
        allow_to: vec![],
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128),
        require_approval: ApprovalMode::OverCap,
        revoked: false,
    };
    std::fs::write(
        dir.path().join("policy.json"),
        serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();

    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let first = send(recipient, 40_000_000_000_000_000); // 0.04 ETH
    let second = send(recipient, 39_000_000_000_000_000); // 0.039 ETH (distinct id)
    assert_eq!(client.propose(&first).await.unwrap(), Decision::Allow);
    assert_eq!(client.propose(&second).await.unwrap(), Decision::Allow);

    // First executes (spends 0.04); the second now exceeds the 0.05 daily cap at sign time.
    assert!(matches!(
        client
            .execute(SignerClient::request_id_for_intent(&first))
            .await
            .unwrap(),
        ExecuteResult::Broadcast { .. }
    ));
    assert_eq!(
        client
            .execute(SignerClient::request_id_for_intent(&second))
            .await
            .unwrap(),
        ExecuteResult::Denied {
            reason: "cap_exceeded".into()
        }
    );
}
