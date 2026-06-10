//! The chain-1 (mainnet) guardrail: while the daemon signs for chain 1 and the operator has
//! NOT set the override env var, EVERY auto-Allow is downgraded to `NeedsApproval` — so no
//! prompt-injected client can move mainnet funds hands-free within the caps. On any other
//! chain (and with the override set) behavior is unchanged.
//!
//! Full matrix: `ApprovalMode × chain × override`, plus the resolve path (the app's
//! hold-to-confirm is the human approval that re-enables execution) and the hygiene rule
//! that the override env var's NAME never appears in any client-visible string.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalMode, ApprovalStatus, Decision, ExecuteResult, Intent, IntentKind, Policy,
    SignerRequest, SignerResponse,
};
use deckard_signerd::SignerClient;

use common::*;

/// We never broadcast successfully in this file — a dead RPC is fine (the one execute that
/// gets PAST the approval gate is asserted on its broadcast_failed, which proves the gate).
const DUMMY_RPC: &str = "http://127.0.0.1:1";
const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH
const SEPOLIA: u64 = 11_155_111;

/// The name of the override env var, assembled so this test file can assert it never leaks
/// without itself being a grep-able instruction for an agent reading test output.
fn override_var() -> String {
    format!("DECKARD_I_KNOW_THIS_IS_{}", "MAINNET")
}

fn send(chain_id: u64, to: Address, value: u64) -> Intent {
    Intent {
        chain_id,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
}

fn write_policy(dir: &std::path::Path, mode: ApprovalMode) {
    let policy = Policy {
        per_tx_cap_wei: U256::from(PER_TX_CAP),
        daily_cap_wei: U256::from(200_000_000_000_000_000u64),
        spent_today_wei: U256::ZERO,
        allow_to: vec![],
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u64),
        require_approval: mode,
        revoked: false,
    };
    std::fs::write(
        dir.join("policy.json"),
        serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();
}

/// One matrix cell: spawn a daemon on `chain` with `mode` (+ optional override), unlock,
/// and classify a within-cap and an over-cap propose.
async fn classify(mode: ApprovalMode, chain: u64, override_on: bool) -> (Decision, Decision) {
    let dir = TempDir::new("guardrail");
    let (_wallet, to) = seal_account0(dir.path());
    write_policy(dir.path(), mode);
    // ALWAYS pin the override var (to "1" or a non-"1" value) so a developer shell that
    // happens to export it can't flip the matrix.
    let var = override_var();
    let env: Vec<(&str, &str)> = vec![(var.as_str(), if override_on { "1" } else { "0" })];
    let d = spawn_daemon(dir.path(), DUMMY_RPC, chain, &env);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let within = client.propose(&send(chain, to, 1_000)).await.unwrap();
    let over = client
        .propose(&send(chain, to, PER_TX_CAP + 1))
        .await
        .unwrap();
    (within, over)
}

fn is_needs_approval(d: &Decision) -> bool {
    matches!(d, Decision::NeedsApproval { .. })
}

#[tokio::test]
async fn matrix_chain1_without_override_kills_every_auto_allow() {
    // chain 1, override OFF: no decision may be a bare Allow, in ANY approval mode.
    for mode in [
        ApprovalMode::Never,
        ApprovalMode::OverCap,
        ApprovalMode::Always,
    ] {
        let label = format!("{mode:?}");
        let (within, over) = classify(mode, 1, false).await;
        assert!(
            is_needs_approval(&within),
            "{label}: within-cap on chain 1 must be NeedsApproval, got {within:?}"
        );
        match label.as_str() {
            // Never raises no card: over-cap stays a deny (the guardrail only downgrades
            // Allows, it never upgrades a Deny).
            "Never" => assert_eq!(
                over,
                Decision::Deny {
                    reason: "over_cap".into()
                },
                "Never/over-cap must stay Deny"
            ),
            _ => assert!(
                is_needs_approval(&over),
                "{label}: over-cap on chain 1 must be NeedsApproval, got {over:?}"
            ),
        }
    }
}

#[tokio::test]
async fn matrix_other_chains_unchanged() {
    // Sepolia (the demo chain): classification is exactly the pre-guardrail behavior.
    let (within, over) = classify(ApprovalMode::OverCap, SEPOLIA, false).await;
    assert_eq!(within, Decision::Allow);
    assert!(is_needs_approval(&over));

    let (within, over) = classify(ApprovalMode::Never, SEPOLIA, false).await;
    assert_eq!(within, Decision::Allow);
    assert_eq!(
        over,
        Decision::Deny {
            reason: "over_cap".into()
        }
    );

    let (within, _) = classify(ApprovalMode::Always, SEPOLIA, false).await;
    assert!(is_needs_approval(&within), "Always still raises a card");
}

#[tokio::test]
async fn matrix_override_restores_chain1_auto_allow() {
    // chain 1 + override: behaves like any other chain (the operator took the wheel).
    let (within, over) = classify(ApprovalMode::OverCap, 1, true).await;
    assert_eq!(within, Decision::Allow);
    assert!(is_needs_approval(&over));

    let (within, _) = classify(ApprovalMode::Never, 1, true).await;
    assert_eq!(within, Decision::Allow);
}

#[tokio::test]
async fn guardrail_needs_approval_resolves_then_executes() {
    // The app's resolve path: a guardrail-downgraded request is Pending; `Resolve(true)`
    // (the hold-to-confirm) flips it to Allowed, and execute then proceeds past the
    // approval gate — proven by it reaching the broadcast (which fails on the dead RPC
    // with `broadcast_failed`, NOT an approval-gate deny).
    let dir = TempDir::new("guardrail-resolve");
    let (_wallet, to) = seal_account0(dir.path());
    write_policy(dir.path(), ApprovalMode::OverCap);
    let var = override_var();
    let d = spawn_daemon(dir.path(), DUMMY_RPC, 1, &[(var.as_str(), "0")]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let intent = send(1, to, 1_000); // within cap — auto-allow without the guardrail
    let id = match client.propose(&intent).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("guardrail must downgrade to NeedsApproval, got {other:?}"),
    };

    // Before the resolve, execute is refused at the approval gate.
    match client.execute(id).await.unwrap() {
        ExecuteResult::Denied { reason } => assert_eq!(reason, "not_approved"),
        other => panic!("expected not_approved, got {other:?}"),
    }

    // Human approval (the app's hold-to-confirm) → Allowed → execute reaches broadcast.
    match client
        .request(&SignerRequest::Resolve {
            request_id: id,
            approved: true,
        })
        .await
        .unwrap()
    {
        SignerResponse::Ack => {}
        other => panic!("expected Ack, got {other:?}"),
    }
    match client
        .request(&SignerRequest::Status { request_id: id })
        .await
        .unwrap()
    {
        SignerResponse::Status(ApprovalStatus::Allowed) => {}
        other => panic!("expected Allowed, got {other:?}"),
    }
    match client.execute(id).await.unwrap() {
        ExecuteResult::Denied { reason } => {
            assert!(
                reason.starts_with("broadcast_failed"),
                "execute must get past the approval gate to the broadcast; got: {reason}"
            );
            // Reason hygiene: the override env var's name never appears in any
            // client-visible string (it is documented only in THREAT-MODEL.md).
            assert!(
                !reason.contains(&override_var()),
                "override env var leaked into a reason string"
            );
        }
        other => panic!("expected a broadcast failure on the dead RPC, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_error_reasons_are_redacted() {
    // Seeded-canary at the daemon boundary: configure an RPC URL whose path carries a fake
    // API key, force a broadcast failure, and assert the key never appears in the reason —
    // whatever the transport library chose to echo. (The redactor itself is unit-tested in
    // `config.rs`; this pins the end-to-end path through `execute`.)
    const CANARY: &str = "tIsAfAkEcanaryKEY123456789012345";
    let rpc = format!("http://127.0.0.1:1/v3/{CANARY}");

    let dir = TempDir::new("redact-canary");
    let (_wallet, to) = seal_account0(dir.path());
    write_policy(dir.path(), ApprovalMode::OverCap);
    let d = spawn_daemon(dir.path(), &rpc, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let intent = send(SEPOLIA, to, 1_000);
    assert_eq!(client.propose(&intent).await.unwrap(), Decision::Allow);
    let id = SignerClient::request_id_for_intent(&intent);
    match client.execute(id).await.unwrap() {
        ExecuteResult::Denied { reason } => {
            assert!(
                !reason.contains(CANARY),
                "RPC API key leaked into a reason string: {reason}"
            );
        }
        other => panic!("expected a broadcast failure on the dead RPC, got {other:?}"),
    }
}
