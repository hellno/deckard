//! Single-session double-spend (TOCTOU) regression for the value-bearing auto-allow path (#135 PR2).
//!
//! PR1 lets a within-cap native-ETH Send auto-allow at PROPOSE time (no human). Proposing only
//! decides; the durable spend is reserved at EXECUTE time (daemon.rs:1355). So two DISTINCT
//! within-cap Sends can BOTH return `Decision::Allow` while `spent_today` is still 0 — that is the
//! Time-Of-Check / Time-Of-Use window. The property this test PROVES (verify, don't rebuild): the
//! daemon serializes `execute` under its mutex and re-evaluates an auto-allow against the CURRENT
//! `spent_today` BEFORE reserving (daemon.rs:1312 — `!req.approved && evaluate(..) != Allow →
//! Denied{cap_exceeded}`). Once the FIRST Send commits and grows `spent_today`, the SECOND's
//! re-evaluate no longer fits the daily cap, so it is Denied at sign time and never reaches the
//! chain. The two within-cap auto-allows therefore cannot both drain past the daily cap.
//!
//! `durable_cap_e2e.rs` covers only the RESTART path (a recovered spend held at *propose* time);
//! this covers the single-session execute-time close that no restart is involved in. Skips when
//! `anvil` isn't on PATH.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    deny_reasons, Allowlist, ApprovalMode, Decision, Effect, ExecuteResult, Intent, IntentKind,
    Policy, ProposalOrigin, Rule, POLICY_VERSION,
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

#[tokio::test]
async fn two_within_cap_autoallows_cannot_both_execute_past_the_daily_cap() {
    if !anvil_available() {
        eprintln!(
            "SKIP two_within_cap_autoallows_cannot_both_execute_past_the_daily_cap: anvil not on PATH"
        );
        return;
    }
    let anvil = start_anvil();
    wait_anvil_ready(&anvil.url()).await;

    let dir = TempDir::new("send-toctou");
    let (_wallet, recip1) = seal_account0(dir.path());

    // DISTINCT caps so the daily cap — not the per-tx cap — is what closes the window: each 0.03
    // send fits per-tx (0.03 < 0.04) and fits daily on its own (0.03 < 0.05), but 0.03 + 0.03 =
    // 0.06 exceeds the 0.05 daily cap. (Field shape mirrors `durable_cap_e2e.rs::tight_policy`,
    // built inline here with per-tx vs daily split.)
    let policy = Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(50_000_000_000_000_000u128), // 0.05 ETH
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128),
        spent_today_wei: U256::ZERO,
        rules: vec![Rule::Send {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: Some(U256::from(40_000_000_000_000_000u128)), // 0.04 ETH
            recipients: Allowlist::Any,
        }],
    };
    write_policy(dir.path(), &policy);

    // A distinct, NON-funded destination so we can assert it never received anything (its anvil
    // starting balance is 0 — the second send must not drain to it).
    let recip2 = Address::repeat_byte(0x99);

    let d = spawn_daemon(dir.path(), &anvil.url(), CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // Two distinct within-cap sends, each 0.03 ETH (distinct recipients ⇒ distinct request ids).
    let value: u128 = 30_000_000_000_000_000;
    let a = send(recip1, value);
    let b = send(recip2, value);

    // THE TOCTOU WINDOW: both auto-allow at propose time because `spent_today` is still 0 — the
    // reservation that would gate the second one happens at EXECUTE, not here.
    assert_eq!(
        client.propose(&a, ProposalOrigin::App).await.unwrap(),
        Decision::Allow,
        "send A is within both caps ⇒ auto-allow"
    );
    assert_eq!(
        client.propose(&b, ProposalOrigin::App).await.unwrap(),
        Decision::Allow,
        "send B also auto-allows while spent_today is still 0 — this is the TOCTOU window"
    );

    // Execute A: signs, reserves 0.03 durably, broadcasts, commits ⇒ `spent_today` becomes 0.03.
    let tx_a = match client
        .execute(SignerClient::request_id_for_intent(&a))
        .await
        .unwrap()
    {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected A to Broadcast, got {other:?}"),
    };
    wait_receipt(&anvil.url(), tx_a).await.expect("receipt");

    // Execute B: the execute-time re-evaluate now sees 0.03 + 0.03 = 0.06 > 0.05 daily cap, so the
    // auto-allow no longer holds and B is DENIED before any reserve/sign — this is the close.
    match client
        .execute(SignerClient::request_id_for_intent(&b))
        .await
        .unwrap()
    {
        ExecuteResult::Denied { reason } => assert_eq!(
            reason,
            deny_reasons::CAP_EXCEEDED, // "cap_exceeded"
            "B must be denied with the cap tag, not some other refusal"
        ),
        ExecuteResult::Broadcast { tx_hash } => {
            panic!("DOUBLE-SPEND: B broadcast {tx_hash:?} past the daily cap — the close failed")
        }
    }

    // No double-spend reached the chain: recip2 (the second, non-funded destination) never got the
    // 0.03 ETH, so the two within-cap auto-allows did not both drain.
    assert_eq!(
        balance(&anvil.url(), recip2).await,
        U256::ZERO,
        "recip2 must have received nothing — B never broadcast"
    );
}
