//! The spending fence the agent is allowed to READ (so it can stay inside the fence) but
//! never write. The daemon enforces it; `MockSigner` enforces the same rules in memory.
//!
//! [`evaluate`] is **the one decision function** — both `MockSigner` and the real
//! `deckard-signerd` call it, so there is no mock⇄daemon drift in the verdict logic.

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::intent::{Intent, IntentKind};

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
            reason: "revoked".into(),
        };
    }
    // 2. Allowlist (empty = any address).
    if !policy.allow_to.is_empty() && !policy.allow_to.contains(&intent.to) {
        return Decision::Deny {
            reason: "off_allowlist".into(),
        };
    }
    // 3. Calldata must be decodable for the kind.
    if !calldata_ok(intent) {
        return Decision::Deny {
            reason: "undecodable".into(),
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
                    reason: "over_cap".into(),
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

/// Shape check: does the calldata match the kind? The real Railgun adapter calldata is
/// validated downstream (`10-kohaku-shield.md`); this only enforces the coarse invariant
/// the policy gate relies on.
fn calldata_ok(intent: &Intent) -> bool {
    match intent.kind {
        // A plain send carries no calldata (the daemon builds the tx from to/value/token).
        IntentKind::Send => intent.calldata.is_empty(),
        // A generic contract write needs calldata to call.
        IntentKind::ContractCall => !intent.calldata.is_empty(),
        // Railgun deposit/withdraw: accept whatever calldata is handed over.
        IntentKind::Shield | IntentKind::Unshield => true,
    }
}
