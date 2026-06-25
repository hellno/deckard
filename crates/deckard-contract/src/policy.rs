//! The spending fence the agent is allowed to READ (so it can stay inside the fence) but
//! never write. The daemon enforces it; `MockSigner` enforces the same rules in memory.
//!
//! [`evaluate`] is **the one decision function** — both `MockSigner` and the real
//! `deckard-signerd` call it, so there is no mock⇄daemon drift in the verdict logic.

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::deny_reasons;
use crate::intent::{Intent, IntentKind};
use crate::message_signing::{SignMessage, SignMessageKind};
use crate::swap_order::SwapOrder;

/// The agent-readable policy. All caps are in wei.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Per-transaction ceiling.
    pub per_tx_cap_wei: U256,
    /// Rolling daily ceiling.
    pub daily_cap_wei: U256,
    /// Spent so far today; the cap check compares `spent_today_wei + value`.
    pub spent_today_wei: U256,
    /// Allowed recipients. **EMPTY = any address allowed.**
    pub allow_to: Vec<Address>,
    /// Demo rule: auto-shield inbound ETH ≥ this. Read by the agent to decide *whether to
    /// propose a shield*; the policy gate itself does not switch on it.
    pub auto_shield_min_wei: U256,
    /// When a write needs a human approval card.
    pub require_approval: ApprovalMode,
    /// Set true by `revoke_all` / STOP. Re-checked at execute time (TOCTOU guard).
    pub revoked: bool,
    /// Per-token swap allowlist (sell+buy must both be present). **EMPTY = any token allowed.**
    /// Daemon config populates it from `tokens_for(chain_id)`; agent-readable via PolicyGet.
    #[serde(default)]
    pub allow_swap_tokens: Vec<Address>,
}

/// When the policy gate raises a native approval card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ApprovalMode {
    /// Never raise a card. Within cap → allow; over cap → deny (no card to override it).
    Never,
    /// Raise a card only when over a cap; within cap → allow.
    OverCap,
    /// Always raise a card, even within cap.
    Always,
}

/// **The** decision function. A *pure* `(Intent, Policy) -> Decision` with no I/O, no
/// signing, no state — both [`MockSigner`](crate::MockSigner) and `deckard-signerd` call
/// it so the verdict can never drift between the mock and the real daemon.
///
/// It owns the policy-level checks (`revoked`, allowlist, calldata shape, the caps × mode
/// matrix). Process-level pre-checks that the policy can't express — the daemon being
/// `Locked`, a `chain_id` mismatch, an unsupported `IntentKind` — are the daemon's job and
/// run *before* this function (the mock has none of those states, so feeding both the same
/// `(Intent, Policy)` yields identical `Decision`s; this is the parity contract).
///
/// For [`Decision::NeedsApproval`] the returned `request_id` is the **placeholder**
/// [`RequestId::ZERO`](alloy_primitives::B256::ZERO): minting a real, trackable id is the
/// stateful caller's job (it stores the pending record under that id). Callers must replace
/// it before returning the decision on the wire.
pub fn evaluate(intent: &Intent, policy: &Policy) -> Decision {
    // 1. STOP / revoked overrides everything.
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    // 2. Allowlist (empty = any address).
    if !policy.allow_to.is_empty() && !policy.allow_to.contains(&intent.to) {
        return Decision::Deny {
            reason: deny_reasons::OFF_ALLOWLIST.into(),
        };
    }
    // 3. Calldata must be decodable for the kind.
    if !calldata_ok(intent) {
        return Decision::Deny {
            reason: deny_reasons::UNDECODABLE.into(),
        };
    }
    // 4. Cap check: spent_today + value vs the per-tx and daily caps.
    let projected = policy.spent_today_wei.saturating_add(intent.value);
    let over = projected > policy.per_tx_cap_wei || projected > policy.daily_cap_wei;

    match policy.require_approval {
        // Never raises no card, so an over-cap write has nothing to authorise it → deny.
        ApprovalMode::Never => {
            if over {
                Decision::Deny {
                    reason: deny_reasons::OVER_CAP.into(),
                }
            } else {
                Decision::Allow
            }
        }
        ApprovalMode::OverCap => {
            if over {
                Decision::NeedsApproval {
                    request_id: RequestId::ZERO,
                }
            } else {
                Decision::Allow
            }
        }
        ApprovalMode::Always => Decision::NeedsApproval {
            request_id: RequestId::ZERO,
        },
    }
}

/// The swap-order decision function — pure, like [`evaluate`]. Swaps NEVER auto-allow in v1:
/// a valid order is ALWAYS `NeedsApproval`. `now` is unix-secs (injected so the fn stays pure).
/// `wallet` is the daemon's unlocked address (the receiver/owner binding).
pub fn evaluate_order(order: &SwapOrder, policy: &Policy, wallet: Address, now: u64) -> Decision {
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    if order.receiver == Address::ZERO {
        return Decision::Deny {
            reason: deny_reasons::RECEIVER_ZERO.into(),
        };
    }
    if order.receiver != wallet {
        return Decision::Deny {
            reason: deny_reasons::RECEIVER_NOT_WALLET.into(),
        };
    }
    // A zero sell amount is a garbage order (nothing to sell) and would let the shaped-approve
    // gate admit an `approve(relayer, 0)`; refuse it outright. (`buy_amount_min == 0` is left
    // valid: a max-slippage market sell is legitimate and the human sees it on the card.)
    if order.sell_amount.is_zero() {
        return Decision::Deny {
            reason: deny_reasons::ZERO_AMOUNT.into(),
        };
    }
    if !policy.allow_swap_tokens.is_empty()
        && (!policy.allow_swap_tokens.contains(&order.sell_token)
            || !policy.allow_swap_tokens.contains(&order.buy_token))
    {
        return Decision::Deny {
            reason: deny_reasons::OFF_SWAP_LIST.into(),
        };
    }
    if order.valid_to as u64 > now.saturating_add(86_400) {
        return Decision::Deny {
            reason: deny_reasons::VALID_TO_TOO_FAR.into(),
        };
    }
    Decision::NeedsApproval {
        request_id: RequestId::ZERO,
    }
}

/// The message-signing decision function — pure, like [`evaluate`] and [`evaluate_order`].
/// Messages never auto-allow in v1: any safe request is held for human approval. The `wallet`
/// argument is present so the parity signature has the daemon-bound account available as the
/// policy grows; it is intentionally unused by the first pass.
pub fn evaluate_message(message: &SignMessage, policy: &Policy, _wallet: Address) -> Decision {
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    match &message.kind {
        SignMessageKind::PersonalSign { .. } => Decision::NeedsApproval {
            request_id: RequestId::ZERO,
        },
        SignMessageKind::TypedDataV4(review) => {
            if review
                .domain_chain_id
                .is_some_and(|chain_id| chain_id != message.chain_id)
            {
                return Decision::Deny {
                    reason: deny_reasons::CHAINID_MISMATCH.into(),
                };
            }
            Decision::NeedsApproval {
                request_id: RequestId::ZERO,
            }
        }
        SignMessageKind::EthSign { .. } => Decision::Deny {
            reason: deny_reasons::ETH_SIGN_REFUSED.into(),
        },
        SignMessageKind::Authorization7702 { .. } => Decision::Deny {
            reason: deny_reasons::DELEGATION_REFUSED.into(),
        },
    }
}

/// Shape check: does the calldata match the kind? The real Railgun adapter calldata is
/// validated downstream (`10-kohaku-shield.md`); this only enforces the coarse invariant
/// the policy gate relies on.
///
/// The Shield invariant matters now that Shield routes to the signing path: a
/// `Shield`/`Unshield` MUST carry non-empty calldata. Without it, an `Intent{kind:Shield,
/// calldata: empty}` would fall through the daemon's broadcast as a **plain native ETH send**
/// to `intent.to` (no private note ever created) while wire-labelled "Shield" — a key-less
/// client could thereby move ETH to an arbitrary address under the Shield label. Requiring
/// calldata closes that. (The deeper `to == RelayAdapt(chain)` check lives downstream — the
/// contract crate is pure policy with zero chain knowledge and no railgun dep, by charter.)
fn calldata_ok(intent: &Intent) -> bool {
    match intent.kind {
        // A plain send carries no calldata (the daemon builds the tx from to/value/token).
        IntentKind::Send => intent.calldata.is_empty(),
        // A contract write / Railgun deposit / withdraw all carry an encoded call. An empty
        // payload for any of these would degrade into a bare native send — reject it.
        IntentKind::ContractCall | IntentKind::Shield | IntentKind::Unshield => {
            !intent.calldata.is_empty()
        }
    }
}

#[cfg(test)]
mod evaluate_order_tests {
    use super::*;
    use alloy_primitives::B256;

    const NOW: u64 = 1_700_000_000;
    const WALLET_BYTE: u8 = 0x11;

    /// The wallet the daemon binds owner/receiver to in these vectors.
    fn wallet() -> Address {
        Address::repeat_byte(WALLET_BYTE)
    }

    /// A base policy: not revoked, empty swap allowlist (any token allowed). The other
    /// caps are irrelevant to `evaluate_order` (it never inspects them).
    fn base_policy() -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(50u64),
            daily_cap_wei: U256::from(1000u64),
            spent_today_wei: U256::ZERO,
            allow_to: vec![],
            auto_shield_min_wei: U256::from(10u64),
            require_approval: ApprovalMode::OverCap,
            revoked: false,
            allow_swap_tokens: vec![],
        }
    }

    /// A well-formed order whose receiver == wallet and whose `valid_to` sits inside the
    /// 24h horizon. Sub-tests mutate one field at a time off this baseline.
    fn base_order() -> SwapOrder {
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
    fn revoked_denies() {
        let mut p = base_policy();
        p.revoked = true;
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::REVOKED.into()
            }
        );
    }

    #[test]
    fn receiver_zero_denies() {
        let order = SwapOrder {
            receiver: Address::ZERO,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::RECEIVER_ZERO.into()
            }
        );
    }

    #[test]
    fn receiver_not_wallet_denies() {
        let order = SwapOrder {
            receiver: Address::repeat_byte(0x22),
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::RECEIVER_NOT_WALLET.into()
            }
        );
    }

    #[test]
    fn zero_sell_amount_denies() {
        let order = SwapOrder {
            sell_amount: alloy_primitives::U256::ZERO,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::ZERO_AMOUNT.into()
            }
        );
    }

    #[test]
    fn empty_swap_list_allows_any_token() {
        // Empty allowlist = any token: a well-formed order needs approval, never denied.
        assert!(matches!(
            evaluate_order(&base_order(), &base_policy(), wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn sell_off_list_denies() {
        let mut p = base_policy();
        // buy_token present, sell_token absent.
        p.allow_swap_tokens = vec![Address::repeat_byte(0xB2)];
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn buy_off_list_denies() {
        let mut p = base_policy();
        // sell_token present, buy_token absent.
        p.allow_swap_tokens = vec![Address::repeat_byte(0xA1)];
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn both_off_list_denies() {
        let mut p = base_policy();
        p.allow_swap_tokens = vec![Address::repeat_byte(0xEE)];
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn both_on_list_needs_approval() {
        let mut p = base_policy();
        p.allow_swap_tokens = vec![Address::repeat_byte(0xA1), Address::repeat_byte(0xB2)];
        assert!(matches!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn valid_to_at_horizon_is_allowed() {
        // Boundary: valid_to == now + 86_400 (exactly 24h) is INSIDE the horizon.
        let order = SwapOrder {
            valid_to: (NOW + 86_400) as u32,
            ..base_order()
        };
        assert!(matches!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn valid_to_one_past_horizon_denies() {
        // Boundary: valid_to == now + 86_401 is one second too far.
        let order = SwapOrder {
            valid_to: (NOW + 86_401) as u32,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::VALID_TO_TOO_FAR.into()
            }
        );
    }

    #[test]
    fn well_formed_order_needs_approval_with_zero_placeholder() {
        // A valid order never auto-allows: it is ALWAYS NeedsApproval, and the pure fn
        // returns the ZERO placeholder id (the stateful caller mints the real one).
        assert_eq!(
            evaluate_order(&base_order(), &base_policy(), wallet(), NOW),
            Decision::NeedsApproval {
                request_id: RequestId::ZERO
            }
        );
    }
}

#[cfg(test)]
mod message_signing_tests {
    use super::*;
    use crate::{MessageSigningRisk, SignMessage, SignMessageKind, TypedDataReview};
    use alloy_primitives::B256;

    const CHAIN_ID: u64 = 11155111;

    fn wallet() -> Address {
        Address::repeat_byte(0x11)
    }

    fn base_policy() -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(50u64),
            daily_cap_wei: U256::from(1000u64),
            spent_today_wei: U256::ZERO,
            allow_to: vec![],
            auto_shield_min_wei: U256::from(10u64),
            require_approval: ApprovalMode::Never,
            revoked: false,
            allow_swap_tokens: vec![],
        }
    }

    fn personal_message() -> SignMessage {
        SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::PersonalSign {
                message: b"Sign in to Deckard".as_slice().into(),
            },
        }
    }

    fn typed_message(chain_id: u64) -> SignMessage {
        SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::TypedDataV4(TypedDataReview {
                domain_name: Some("Permit2".into()),
                domain_version: Some("1".into()),
                domain_chain_id: Some(chain_id),
                verifying_contract: Some(Address::repeat_byte(0x22)),
                primary_type: "PermitSingle".into(),
                digest: B256::repeat_byte(0x42),
                risks: vec![MessageSigningRisk::PermitLike],
                permit: None,
            }),
        }
    }

    #[test]
    fn personal_sign_always_needs_approval() {
        assert_eq!(
            evaluate_message(&personal_message(), &base_policy(), wallet()),
            Decision::NeedsApproval {
                request_id: RequestId::ZERO
            }
        );
    }

    #[test]
    fn typed_data_chainid_mismatch_denies() {
        assert_eq!(
            evaluate_message(&typed_message(1), &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::CHAINID_MISMATCH.into()
            }
        );
    }

    #[test]
    fn eth_sign_is_refused() {
        let message = SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::EthSign {
                digest: B256::repeat_byte(0x33),
            },
        };
        assert_eq!(
            evaluate_message(&message, &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::ETH_SIGN_REFUSED.into()
            }
        );
    }

    #[test]
    fn eip7702_delegation_refused() {
        let message = SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::Authorization7702 {
                delegate: Address::repeat_byte(0x44),
                nonce: 7,
            },
        };
        assert_eq!(
            evaluate_message(&message, &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::DELEGATION_REFUSED.into()
            }
        );
    }
}
