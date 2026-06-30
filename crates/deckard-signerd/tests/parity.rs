//! #8 — the ONE decision function. Identical `(Intent, Policy)` vectors fed to `MockSigner`
//! and to the daemon's decision path must yield identical `Decision`s. Both route through
//! `deckard_contract::evaluate` (the daemon calls it directly after its process-level
//! pre-checks; the mock calls it in `propose`), so this pins that they never drift.
//!
//! The vectors all use an unlocked daemon, a matching chain id, and `kind = Send`, so the
//! daemon's pre-checks (`locked`/`chain_mismatch`/`unsupported_v1`/`shield_to_mismatch`)
//! don't fire and both sides reduce to `evaluate` — exactly the apples-to-apples parity
//! contract.
//!
//! Explicit carve-out: the daemon's auto-approval guardrail (auto-Allow → `NeedsApproval` on any
//! real-value chain — every chain except the exempt testnet/dev allowlist — `tests/guardrail.rs`)
//! is a PROCESS-level check on daemon state (the configured chain id and the operator override),
//! exactly like `locked` — it lives outside `evaluate` by design, so the mock is not expected to
//! mirror it. These vectors use chain 31337 (an exempt id), where it never fires.

use alloy_primitives::{Address, Bytes, B256, U256};
use deckard_contract::{
    evaluate, Allowlist, ApprovalMode, Decision, Effect, Intent, IntentKind, MockSigner, Policy,
    Rule, Signer, POLICY_VERSION,
};

/// Normalize away the (stateful, impl-specific) `NeedsApproval` request id so the comparison
/// is on the CLASSIFICATION, which is the parity contract.
fn norm(d: Decision) -> Decision {
    match d {
        Decision::NeedsApproval { .. } => Decision::NeedsApproval {
            request_id: B256::ZERO,
        },
        other => other,
    }
}

fn intent(kind: IntentKind, to: Address, value: u64, calldata: Bytes) -> Intent {
    Intent {
        chain_id: 31337,
        to,
        token: None,
        value: U256::from(value),
        calldata,
        kind,
    }
}

#[allow(clippy::too_many_arguments)]
fn policy(
    per_tx: u64,
    daily: u64,
    spent: u64,
    mode: ApprovalMode,
    allow: Vec<Address>,
    revoked: bool,
) -> Policy {
    // v1 shape: a `Send` rule carries the per-tx cap, approval mode, and recipient allowlist
    // (empty `allow` ⇒ `Any` to preserve the old "empty = any recipient" vector semantics; a
    // non-empty list ⇒ `Only`). The companion `Shield` rule keeps the fixture a realistic full
    // policy; these `Send` vectors never reach it.
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked,
        daily_cap_wei: U256::from(daily),
        auto_shield_min_wei: U256::from(10u64),
        spent_today_wei: U256::from(spent),
        rules: vec![
            Rule::Send {
                approval: mode,
                per_tx_cap_wei: Some(U256::from(per_tx)),
                recipients: if allow.is_empty() {
                    Allowlist::Any
                } else {
                    Allowlist::Only(allow)
                },
            },
            Rule::Shield { approval: mode },
        ],
    }
}

#[test]
fn mock_and_daemon_decision_logic_agree() {
    let a = Address::repeat_byte(0x22);
    let b = Address::repeat_byte(0x33);
    let send = |v| intent(IntentKind::Send, a, v, Bytes::new());

    let vectors: Vec<(&str, Intent, Policy)> = vec![
        (
            "within per-tx cap → Allow",
            send(20),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![], false),
        ),
        (
            "over per-tx cap → NeedsApproval",
            send(60),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![], false),
        ),
        (
            "over daily cap → NeedsApproval",
            send(20),
            policy(u64::MAX, 100, 90, ApprovalMode::OverCap, vec![], false),
        ),
        (
            "exact per-tx cap boundary → Allow",
            send(50),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![], false),
        ),
        (
            "Never + over cap → Deny over_cap",
            send(60),
            policy(50, 1000, 0, ApprovalMode::Never, vec![], false),
        ),
        (
            "Never + within cap → Allow",
            send(20),
            policy(50, 1000, 0, ApprovalMode::Never, vec![], false),
        ),
        (
            "Always + within cap → NeedsApproval",
            send(20),
            policy(50, 1000, 0, ApprovalMode::Always, vec![], false),
        ),
        (
            "off allowlist → Deny",
            send(20),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![b], false),
        ),
        (
            "on allowlist → Allow",
            send(20),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![a], false),
        ),
        (
            "revoked → Deny revoked",
            send(20),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![], true),
        ),
        (
            "undecodable (Send w/ calldata) → Deny",
            intent(IntentKind::Send, a, 20, Bytes::from_static(&[1, 2, 3])),
            policy(50, 1000, 0, ApprovalMode::OverCap, vec![], false),
        ),
    ];

    for (label, it, pol) in vectors {
        // The daemon's decision path IS `evaluate` (after pre-checks that don't apply here).
        let daemon_decision = norm(evaluate(&it, &pol));
        // MockSigner.propose routes through the same `evaluate` and mints a real id.
        let mock = MockSigner::new(pol.clone());
        let mock_decision = norm(mock.propose(&it));
        assert_eq!(
            daemon_decision, mock_decision,
            "decision diverged for: {label}"
        );
    }
}
