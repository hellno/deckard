//! Durable daily-cap (issue #108) end-to-end on a local anvil, driving the REAL daemon binary.
//!
//! The headline guarantee: the daily spend cap survives a daemon restart. Before #108 the cap was
//! in-memory and force-zeroed on every load, so a restart (crash, OOM, app update — or a same-uid
//! attacker crash-looping the auto-respawning daemon) reset the day's accounting and re-opened the
//! within-cap drain. These tests broadcast real txs, kill the daemon, respawn it against the SAME
//! config dir, and assert the persisted `spend.json` is recovered. Skips when `anvil` isn't on PATH.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    Allowlist, ApprovalMode, Decision, Effect, ExecuteResult, Intent, IntentKind, Policy,
    ProposalOrigin, Rule, POLICY_VERSION,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;

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

/// A tight v1 policy: per-tx `cap`, daily `cap`, any recipient, approval over cap. With per-tx
/// == daily, two within-per-tx sends that together exceed the daily cap let a test OBSERVE the
/// recovered spend behaviorally (the post-restart send is held, not auto-allowed).
fn tight_policy(cap: u128) -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(cap),
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128),
        spent_today_wei: U256::ZERO,
        rules: vec![Rule::Send {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: Some(U256::from(cap)),
            recipients: Allowlist::Any,
        }],
    }
}

/// Whether a within-per-tx send is HELD (the recovered spend ate the daily headroom) vs
/// auto-ALLOWED (headroom available). The daemon's `spent_today_wei` is `#[serde(skip)]`
/// runtime state and no longer crosses the `PolicyGet` wire, so the durable spend is observed
/// through this cap-enforcement decision — the actual security property — rather than read back
/// as a number.
async fn send_is_held(client: &SignerClient, recipient: Address, value: u128) -> bool {
    match client
        .propose(&send(recipient, value), ProposalOrigin::App)
        .await
        .unwrap()
    {
        Decision::Allow => false,
        Decision::NeedsApproval { .. } => true,
        other => panic!("expected Allow or NeedsApproval, got {other:?}"),
    }
}

/// Kill `d` and spawn a FRESH daemon against the same `dir` (a clean restart). Removes the stale
/// socket so the new daemon binds cleanly and the client doesn't connect to a dead socket.
fn restart(d: DaemonProc, dir: &std::path::Path, url: &str) -> DaemonProc {
    let socket = d.socket_path.clone();
    drop(d); // kills the child
    let _ = std::fs::remove_file(&socket);
    spawn_daemon(dir, url, CHAIN, &[])
}

#[tokio::test]
async fn honest_restart_recovers_the_daily_spend() {
    if !anvil_available() {
        eprintln!("SKIP honest_restart_recovers_the_daily_spend: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("durable-honest");
    let (_wallet, recipient) = seal_account0(dir.path());
    // Tight policy (per-tx == daily == 0.05) so the recovered spend is OBSERVABLE: pre-restart a
    // second 0.04 send is within both caps and auto-allows; once the first 0.04 is committed,
    // 0.04 + 0.04 exceeds the 0.05 daily cap, so it must be HELD.
    let cap: u128 = 50_000_000_000_000_000;
    write_policy(dir.path(), &tight_policy(cap));

    // Spend 0.04 ETH (within both caps) → broadcast → committed to spend.json.
    let value: u128 = 40_000_000_000_000_000;
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();
    let intent = send(recipient, value);
    assert_eq!(
        client.propose(&intent, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
    let id = SignerClient::request_id_for_intent(&intent);
    let tx = match client.execute(id).await.unwrap() {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };
    wait_receipt(&anvil.url(), tx).await.expect("receipt");
    assert!(
        dir.path().join("spend.json").exists(),
        "the durable counter file was written"
    );

    // Restart the daemon against the same dir — the in-memory cap is gone, but spend.json persists.
    let d2 = restart(d, dir.path(), &anvil.url());
    let client2 = SignerClient::new(d2.socket_path.clone());
    client2.unlock(PASS).await.unwrap();
    // The day's 0.04 spend is recovered across the restart (was 0 before #108): a second 0.04
    // send no longer fits the 0.05 daily cap, so it is HELD for approval rather than auto-allowed.
    assert!(
        send_is_held(&client2, recipient, value).await,
        "the recovered spend must hold a second 0.04 send (would Allow if the cap had reset)"
    );
}

#[tokio::test]
async fn restart_does_not_reset_the_cap() {
    // The security-relevant proof: a same-uid attacker who crash-loops the daemon to reset the cap
    // gains nothing — the persisted spend re-applies on boot, so a within-cap auto-allow that only
    // fits if the cap were reset is instead held for human approval.
    if !anvil_available() {
        eprintln!("SKIP restart_does_not_reset_the_cap: anvil not on PATH");
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("durable-cap");
    let (_wallet, recipient) = seal_account0(dir.path());
    // Tight policy: per-tx 0.05, daily 0.05. 0.04 then 0.03 each fit per-tx, but together exceed
    // the 0.05 daily cap.
    write_policy(dir.path(), &tight_policy(50_000_000_000_000_000u128));

    // Spend 0.04 ETH (within both caps) and broadcast it.
    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();
    let first = send(recipient, 40_000_000_000_000_000);
    assert_eq!(
        client.propose(&first, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
    let tx = match client
        .execute(SignerClient::request_id_for_intent(&first))
        .await
        .unwrap()
    {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };
    wait_receipt(&anvil.url(), tx).await.expect("receipt");

    // Crash-loop stand-in: restart the daemon. Before #108 this zeroed the cap.
    let d2 = restart(d, dir.path(), &anvil.url());
    let client2 = SignerClient::new(d2.socket_path.clone());
    client2.unlock(PASS).await.unwrap();

    // A 0.03 ETH send fits per-tx (0.05) and would auto-allow IF the cap had reset — but the
    // recovered 0.04 + 0.03 exceeds the 0.05 daily cap, so it is HELD for approval, not drained.
    // (The recovered spend is observed through this decision: `spent_today_wei` is `#[serde(skip)]`
    // runtime state and no longer crosses the `PolicyGet` wire.)
    assert!(
        send_is_held(&client2, recipient, 30_000_000_000_000_000).await,
        "the durable cap blocks the post-restart drain (would be Allow if the cap had reset)"
    );
}
