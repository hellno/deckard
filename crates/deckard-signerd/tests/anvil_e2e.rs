//! Broadcast tests on a local anvil node (the only assertions that need a chain): a within-cap
//! send signs + broadcasts with a real receipt (#4), and an over-cap send broadcasts after
//! approval (#5). Native ETH sends don't need a mainnet fork, so a plain local anvil suffices
//! and CI needs no RPC secret. Skips gracefully when `anvil` isn't installed.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ActivityLifecycle, ApprovalMode, ApprovalStatus, BreachedLimit, Decision, ExecuteResult,
    Intent, IntentKind, Policy, ProposalOrigin, SignerRequest, SignerResponse, SwapOrder,
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
    assert_eq!(
        client.propose(&intent, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
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
    let id = match client.propose(&intent, ProposalOrigin::App).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    };

    // Approve (the native card / a test), then execute → broadcast.
    d.resolve(id, true);
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
        allow_swap_tokens: vec![],
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
    assert_eq!(
        client.propose(&first, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
    assert_eq!(
        client.propose(&second, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );

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

/// Build `approve(address,uint256)` calldata (selector + two 32-byte words).
fn approve_calldata(spender: Address, amount: U256) -> Bytes {
    let mut data = vec![0x09, 0x5e, 0xa7, 0xb3];
    let mut spender_word = [0u8; 32];
    spender_word[12..].copy_from_slice(spender.as_slice());
    data.extend_from_slice(&spender_word);
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(data)
}

/// A value-0 `ContractCall` to `to` carrying `calldata` — the shaped-approve wire shape.
fn contract_call(to: Address, calldata: Bytes) -> Intent {
    Intent {
        chain_id: CHAIN,
        to,
        token: None,
        value: U256::ZERO,
        calldata,
        kind: IntentKind::ContractCall,
    }
}

#[tokio::test]
async fn activity_feed_executed_carries_tx_hash() {
    // #60 acceptance 1: an auto-allowed within-cap AGENT action, once broadcast, appears in the
    // activity feed as `Executed` with the real `tx_hash` + a daemon timestamp + actor=agent +
    // no breached cap. (A native send stands in for the shield: it exercises the identical feed
    // path — propose → auto-allow → execute → broadcast — without needing a Railgun fork RPC.)
    if !anvil_available() {
        eprintln!("SKIP activity_feed_executed_carries_tx_hash: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("anvil-feed-exec");
    let (_wallet, recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let intent = send(recipient, 10_000_000_000_000_000); // 0.01 ETH, within cap → auto-allow
    assert_eq!(
        client
            .propose(&intent, ProposalOrigin::Agent)
            .await
            .unwrap(),
        Decision::Allow
    );
    let id = SignerClient::request_id_for_intent(&intent);
    let tx_hash = match client.execute(id).await.unwrap() {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };
    wait_receipt(&anvil.url(), tx_hash)
        .await
        .expect("a receipt");

    let feed = client.activity_feed().await.unwrap();
    let rec = feed
        .iter()
        .find(|r| r.request_id == id)
        .expect("the executed action must appear in the feed");
    assert_eq!(
        rec.lifecycle,
        ActivityLifecycle::Executed,
        "broadcast → Executed"
    );
    assert_eq!(
        rec.tx_hash,
        Some(tx_hash),
        "the feed surfaces the real tx hash"
    );
    assert_eq!(rec.origin, ProposalOrigin::Agent, "actor=agent");
    assert_eq!(
        rec.reason,
        BreachedLimit::None,
        "within cap → no breached fence"
    );
    assert!(rec.timestamp_ms > 0, "the row carries a daemon timestamp");
}

/// Regression: a user must be able to swap the SAME amount any number of times. Each swap
/// re-issues an identical exact-gross relayer approve, which hashes to the same request id; the
/// daemon must NOT treat the second one as an `already_executed` replay (an `approve` moves no
/// funds and is gated by a matching pending order + the human hold). The strict replay guard
/// stays for fund-moving sends — asserted in the same test.
#[tokio::test]
async fn repeat_same_amount_swap_approve_is_not_replay_blocked() {
    if !anvil_available() {
        eprintln!("SKIP repeat_same_amount_swap_approve_is_not_replay_blocked: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("anvil-repeat-approve");
    let (wallet, recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let relayer = deckard_core::GPV2_VAULT_RELAYER;
    let sell_token = Address::repeat_byte(0x77);
    let sell_amount = U256::from(1_000_000u64);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // A pending order admits the shaped approve for its sell token + exact amount.
    let ord = SwapOrder {
        chain_id: CHAIN,
        owner: wallet,
        sell_token,
        buy_token: Address::repeat_byte(0x88),
        sell_amount,
        buy_amount_min: U256::from(900_000u64),
        receiver: wallet,
        valid_to: (now + 3_600) as u32,
        app_data: deckard_core::APP_DATA_HASH,
    };
    assert!(matches!(
        client
            .propose_order(&ord, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::NeedsApproval { .. }
    ));

    // First swap's approve: propose → resolve → execute → on-chain (broadcast recorded).
    let approve = contract_call(sell_token, approve_calldata(relayer, sell_amount));
    let approve_id = match client
        .propose(&approve, ProposalOrigin::Agent)
        .await
        .unwrap()
    {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval for the first approve, got {other:?}"),
    };
    d.resolve(approve_id, true);
    assert!(matches!(
        client.execute(approve_id).await.unwrap(),
        ExecuteResult::Broadcast { .. }
    ));

    // THE FIX: re-proposing the identical approve (the order is still pending) must NOT be an
    // `already_executed` replay — it starts a fresh approval cycle (NeedsApproval again).
    assert!(
        matches!(
            client
                .propose(&approve, ProposalOrigin::Agent)
                .await
                .unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "repeating the same-amount swap approve must be allowed, not replay-blocked"
    );

    // ...and the carve-out resets the record to Pending — it does NOT auto-allow. Executing the
    // re-proposed approve before a fresh control-channel hold is refused: a repeat swap still
    // requires a new human approval (no funds move on the relaxed replay without a new hold).
    assert_eq!(
        client.execute(approve_id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "not_approved".into()
        }
    );

    // The strict replay guard is PRESERVED for fund-moving sends: re-proposing a broadcast send
    // is still refused as `already_executed`.
    let s = send(recipient, 10_000_000_000_000_000); // 0.01 ETH, within cap → Allow
    assert_eq!(
        client.propose(&s, ProposalOrigin::Agent).await.unwrap(),
        Decision::Allow
    );
    let send_id = SignerClient::request_id_for_intent(&s);
    assert!(matches!(
        client.execute(send_id).await.unwrap(),
        ExecuteResult::Broadcast { .. }
    ));
    assert_eq!(
        client.propose(&s, ProposalOrigin::Agent).await.unwrap(),
        Decision::Deny {
            reason: "already_executed".into()
        }
    );
}
