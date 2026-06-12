//! #5 — the ONE swap decision function. Identical `(SwapOrder, Policy, wallet, now)` vectors
//! fed to `deckard_contract::evaluate_order` directly AND via `MockSigner::propose_order` must
//! yield identical (normalized) `Decision`s. Both route through the same pure `evaluate_order`,
//! so this pins that the mock (and, by extension, the daemon, which calls the same fn) never
//! drifts from the contract's classification.
//!
//! The mock binds the order's owner/receiver decision to `MockSigner::mock_address()` and feeds
//! `evaluate_order` an injected clock via `set_now`, so the parity vectors use that same wallet
//! and `now` on the direct call. We normalize `NeedsApproval`'s request id to ZERO (it is a
//! stateful, impl-specific mint) exactly like `parity.rs::norm`.

use alloy_primitives::{Address, B256, U256};
use deckard_contract::{
    evaluate_order, ApprovalMode, Decision, MockSigner, Policy, Signer, SwapOrder,
};

/// Normalize away the stateful `NeedsApproval` request id — the comparison is on the
/// CLASSIFICATION, which is the parity contract (mirrors `parity.rs::norm`).
fn norm(d: Decision) -> Decision {
    match d {
        Decision::NeedsApproval { .. } => Decision::NeedsApproval {
            request_id: B256::ZERO,
        },
        other => other,
    }
}

/// The fixed wallet both sides bind against (the mock's pinned `0x11…11`).
fn wallet() -> Address {
    MockSigner::mock_address()
}

/// The injected unix-secs clock both sides use (the mock's default).
const NOW: u64 = 1_700_000_000;

/// A policy with the given swap allowlist + revoked flag. The cap/mode fields are inert for
/// `evaluate_order` (swaps never touch the spend caps), but we set sane values for clarity.
fn policy(allow_swap_tokens: Vec<Address>, revoked: bool) -> Policy {
    Policy {
        per_tx_cap_wei: U256::from(50u64),
        daily_cap_wei: U256::from(1000u64),
        spent_today_wei: U256::ZERO,
        allow_to: vec![],
        auto_shield_min_wei: U256::from(10u64),
        require_approval: ApprovalMode::OverCap,
        revoked,
        allow_swap_tokens,
    }
}

/// A base well-formed order: owner+receiver bound to `wallet()`, `valid_to` one hour out
/// (inside the 24h horizon), tokens `0xA1`/`0xB2`.
fn order() -> SwapOrder {
    SwapOrder {
        chain_id: 11155111,
        owner: wallet(),
        sell_token: Address::repeat_byte(0xA1),
        buy_token: Address::repeat_byte(0xB2),
        sell_amount: U256::from(1_000_000u64),
        buy_amount_min: U256::from(900_000u64),
        receiver: wallet(),
        valid_to: (NOW + 3600) as u32,
        app_data: B256::repeat_byte(0xCD),
    }
}

#[test]
fn mock_and_contract_swap_decision_logic_agree() {
    let sell = Address::repeat_byte(0xA1);
    let buy = Address::repeat_byte(0xB2);

    let vectors: Vec<(&str, SwapOrder, Policy)> = vec![
        ("revoked → Deny revoked", order(), policy(vec![], true)),
        (
            "receiver_zero → Deny",
            SwapOrder {
                receiver: Address::ZERO,
                ..order()
            },
            policy(vec![], false),
        ),
        (
            "receiver_not_wallet → Deny",
            SwapOrder {
                receiver: Address::repeat_byte(0x22),
                ..order()
            },
            policy(vec![], false),
        ),
        (
            "zero sell_amount → Deny zero_amount",
            SwapOrder {
                sell_amount: U256::ZERO,
                ..order()
            },
            policy(vec![], false),
        ),
        (
            "sell-off swap list → Deny off_swap_list",
            order(),
            // only the buy token is listed; the sell token is off-list.
            policy(vec![buy], false),
        ),
        (
            "buy-off swap list → Deny off_swap_list",
            order(),
            // only the sell token is listed; the buy token is off-list.
            policy(vec![sell], false),
        ),
        (
            "both-off swap list → Deny off_swap_list",
            order(),
            policy(vec![Address::repeat_byte(0xFE)], false),
        ),
        (
            "on swap list → NeedsApproval",
            order(),
            policy(vec![sell, buy], false),
        ),
        (
            "empty swap list (any token) → NeedsApproval",
            order(),
            policy(vec![], false),
        ),
        (
            "valid_to == now+86400 boundary → NeedsApproval (allowed)",
            SwapOrder {
                valid_to: (NOW + 86_400) as u32,
                ..order()
            },
            policy(vec![], false),
        ),
        (
            "valid_to == now+86401 → Deny valid_to_too_far",
            SwapOrder {
                valid_to: (NOW + 86_401) as u32,
                ..order()
            },
            policy(vec![], false),
        ),
    ];

    for (label, ord, pol) in vectors {
        // Direct call to the contract's pure decision fn, with the SAME wallet + now the mock
        // injects.
        let contract_decision = norm(evaluate_order(&ord, &pol, wallet(), NOW));

        // Via the mock: pin its clock to NOW, then propose_order routes through the same
        // `evaluate_order(order, policy, mock_address(), now)`.
        let mock = MockSigner::new(pol.clone());
        mock.set_now(NOW);
        let mock_decision = norm(mock.propose_order(&ord));

        assert_eq!(
            contract_decision, mock_decision,
            "swap decision diverged for: {label}"
        );
    }
}
