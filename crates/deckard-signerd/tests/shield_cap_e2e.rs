//! #185 regression (TRUST-CRITICAL): the per-transaction cap is enforced on the SHIELD path
//! end-to-end through the REAL daemon — a shield OVER the stated per-move cap is HELD for human
//! approval, never auto-broadcast.
//!
//! The bug this pins shut: `policy.demo.json`'s shield rule carried no `per_tx_cap_wei`, and
//! `per_tx_cap_for(Shield)` returned `None`, so a 0.15 ETH deposit auto-allowed under a stated
//! 0.1 ETH per-move cap. The contract-crate unit tests prove `evaluate` (the shared gate) now
//! enforces it; THIS test proves the daemon's shield propose path actually routes through that
//! gate with the durable spend counter synced — no daemon-side bypass.
//!
//! Hermetic: `propose` never broadcasts, so a dummy RPC is enough (no anvil, no network). The
//! shield intent is shaped (targets the chain's RelayAdapt, non-empty calldata) so it clears the
//! daemon's `shield_to_mismatch` / `undecodable` pre-checks and reaches the policy gate.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalMode, Decision, Effect, Intent, IntentKind, Policy, ProposalOrigin, Rule,
    POLICY_VERSION,
};
use deckard_signerd::SignerClient;

use common::*;

const DUMMY_RPC: &str = "http://127.0.0.1:1"; // propose never broadcasts
const SEPOLIA: u64 = 11_155_111;
/// The Sepolia RelayAdapt (pinned in `shield_target.rs` against `railgun::chain_config`). A shield
/// is only admitted when it targets this address, so the intent must use it to reach the gate.
const RELAY_ADAPT: &str = "0x7e3d929EbD5bDC84d02Bd3205c777578f33A214D";

/// A shield intent with the non-empty stand-in calldata `calldata_ok` requires (the real Railgun
/// adapter call is validated downstream; `propose` only needs a decodable shape). Only `value`
/// varies across the sub-assertions.
fn shield(to: Address, value: u128) -> Intent {
    Intent {
        chain_id: SEPOLIA,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
        kind: IntentKind::Shield,
    }
}

/// A policy whose SHIELD rule carries a per-tx cap (the #185 fix), under a daily wall set far
/// above the cap so the per-tx cap is the ONLY fence that can trip — the assertions below prove
/// per-tx enforcement on the shield path, not the daily wall.
fn shield_cap_policy(per_tx: u128) -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(10_000_000_000_000_000_000u128), // 10 ETH — never the binding fence
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128),
        spent_today_wei: U256::ZERO,
        rules: vec![Rule::Shield {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: Some(U256::from(per_tx)),
        }],
    }
}

#[tokio::test]
async fn shield_over_per_tx_cap_is_held_not_broadcast() {
    let relay_adapt: Address = RELAY_ADAPT.parse().unwrap();
    let dir = TempDir::new("shield-cap");
    let _ = seal_account0(dir.path());
    // The stated per-move cap the demo advertises: 0.1 ETH.
    let per_tx: u128 = 100_000_000_000_000_000;
    write_policy(dir.path(), &shield_cap_policy(per_tx));
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // A 0.05 ETH shield is WITHIN the 0.1 per-tx cap → auto-allowed (Sepolia is guardrail-exempt,
    // so a within-cap auto-allow is not downgraded to a hold).
    assert_eq!(
        client
            .propose(
                &shield(relay_adapt, 50_000_000_000_000_000),
                ProposalOrigin::App
            )
            .await
            .unwrap(),
        Decision::Allow,
        "a within-cap shield still auto-allows (the fix must not over-block)"
    );

    // A 0.15 ETH shield is OVER the 0.1 per-tx cap → HELD for human approval, NOT auto-broadcast.
    // Before #185 this returned `Allow` (per_tx_cap_for(Shield) was None) — the exact bug.
    assert!(
        matches!(
            client
                .propose(
                    &shield(relay_adapt, 150_000_000_000_000_000),
                    ProposalOrigin::App
                )
                .await
                .unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "a shield over the per-tx cap must ASK, never auto-broadcast (#185)"
    );

    // Boundary: exactly at the cap is within (the check is strictly greater-than).
    assert_eq!(
        client
            .propose(&shield(relay_adapt, per_tx), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Allow,
        "a shield exactly at the per-tx cap is within it"
    );
}
