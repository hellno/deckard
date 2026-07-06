//! The daemon-free slice of the `docs/build/30-mcp-shape.md` acceptance scenario
//! ("MCP surface: read-free, write-gated, secret-tight"), run against `MockSigner`.
//!
//! This exercises the **signer half** of T1–T8 — every step that is a daemon RPC. The
//! steps that live in the (out-of-scope) MCP server are called out inline:
//!   * T1 `list_tools` — MCP tool registry, not a signer op.
//!   * T5 `simulate`   — Helios eth_call preview, not a signer op.
//!   * T7 secret-refusal of `--passphrase`/`--key` flags — MCP-mode CLI parsing.
//!   * T9 transcript key-leak scan — asserted over the MCP JSON-RPC transcript, where a
//!     64-hex private key would leak; note a `tx_hash`/`request_id` is legitimately 64-hex,
//!     so that gate belongs to the MCP server ticket, not this contract.

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    Allowlist, ApprovalMode, ApprovalStatus, Decision, Effect, ExecuteResult, Intent, IntentKind,
    MockSigner, Policy, Rule, Signer, POLICY_VERSION,
};

const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH
const DAILY_CAP: u64 = 1_000_000_000_000_000_000; // 1 ETH
const AUTO_SHIELD_MIN: u64 = 10_000_000_000_000_000; // 0.01 ETH
const OVER_CAP_VALUE: u64 = 200_000_000_000_000_000; // 0.2 ETH (> per-tx cap)
const WITHIN_CAP_VALUE: u64 = 20_000_000_000_000_000; // 0.02 ETH (< per-tx cap)

fn demo_signer() -> MockSigner {
    MockSigner::new(Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(DAILY_CAP),
        auto_shield_min_wei: U256::from(AUTO_SHIELD_MIN),
        spent_today_wei: U256::ZERO,
        rules: vec![
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(PER_TX_CAP)),
                recipients: Allowlist::Any, // any address (this harness exercises sends)
            },
            // T6 proposes a within-cap Shield and expects it to auto-allow → Allow, so the
            // Shield rule MUST be `OverCap` (within cap ⇒ no card ⇒ Allow), not Always.
            Rule::Shield {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: None,
            },
            Rule::Swap {
                tokens: Allowlist::Any, // any token (this harness exercises sends, not swaps)
            },
        ],
    })
}

fn intent(kind: IntentKind, value: u64) -> Intent {
    // Send carries no calldata; every other kind (Shield/Unshield/ContractCall) must carry
    // its encoded call — the policy gate now rejects an empty payload for those (an empty
    // "Shield" would otherwise degrade into a bare native send). Stand-in bytes for non-Send.
    let calldata = match kind {
        IntentKind::Send => Bytes::new(),
        _ => Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
    };
    Intent {
        chain_id: 1,
        to: Address::repeat_byte(0x22),
        token: None,
        value: U256::from(value),
        calldata,
        kind,
    }
}

#[test]
fn mcp_surface_daemon_free_slice() {
    let s = demo_signer();

    // ---- T2: read tools succeed, deterministic, carry no secret -----------------------
    assert_eq!(s.address(), MockSigner::mock_address());
    let pol = s.policy();
    assert_eq!(
        pol.per_tx_cap_for(IntentKind::Send),
        Some(U256::from(PER_TX_CAP))
    );
    assert!(!pol.revoked);
    let bal = s.balance(false);
    assert_eq!(bal.public_wei, U256::ZERO); // unset → zero, never a key

    // The read responses serialize without any "passphrase"/secret field.
    let json = serde_json::to_string(&pol).unwrap();
    assert!(!json.contains("passphrase"));

    // ---- T3: propose an over-cap Send → NeedsApproval (NOT Allow) ----------------------
    let over = s.propose(&intent(IntentKind::Send, OVER_CAP_VALUE));
    let req_id = match over {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("T3 expected NeedsApproval, got {other:?}"),
    };

    // ---- T4: execute before approval → Denied (never signs on Pending) -----------------
    assert!(matches!(s.execute(req_id), ExecuteResult::Denied { .. }));
    assert_eq!(s.status(req_id), ApprovalStatus::Pending);

    // ---- T6: shield within cap with OverCap → Allow; execute → broadcast ---------------
    let shield = s.propose(&intent(IntentKind::Shield, WITHIN_CAP_VALUE));
    assert_eq!(shield, Decision::Allow);
    let shield_id = s.last_request_id().expect("Allow minted a request id");
    assert_eq!(
        s.execute(shield_id),
        ExecuteResult::Broadcast {
            tx_hash: MockSigner::broadcast_tx_hash()
        }
    );
    // the shield spend is now reflected in policy
    assert_eq!(s.policy().spent_today_wei, U256::from(WITHIN_CAP_VALUE));

    // ---- T8: approve an over-cap write, STOP, then execute → Denied{revoked} (TOCTOU) --
    let pending = match s.propose(&intent(IntentKind::Send, OVER_CAP_VALUE)) {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("T8 setup expected NeedsApproval, got {other:?}"),
    };
    s.approve(pending); // human approved BEFORE the STOP
    assert_eq!(s.status(pending), ApprovalStatus::Allowed);
    s.revoke_all();
    assert_eq!(
        s.execute(pending),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );
    // STOP is sticky: further proposes are denied too.
    assert_eq!(
        s.propose(&intent(IntentKind::Send, WITHIN_CAP_VALUE)),
        Decision::Deny {
            reason: "revoked".into()
        }
    );
}
