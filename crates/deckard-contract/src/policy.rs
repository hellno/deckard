//! The spending fence the agent is allowed to READ (so it can stay inside the fence) but
//! never write. The daemon enforces it; `MockSigner` enforces the same rules in memory.

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

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
