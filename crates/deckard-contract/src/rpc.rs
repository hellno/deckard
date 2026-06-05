//! The daemon socket API — the wire `deckard-mcp` (key-less) speaks to `deckard-signerd`.
//! serde-derived so it frames as CBOR (ciborium) on the UDS and JSON for MCP. One request
//! per frame, one response per frame.

use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::policy::Policy;

/// `deckard-mcp` → `deckard-signerd`. The key-less client only proposes; it never signs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignerRequest {
    /// Policy check, NO signing yet → [`Decision`].
    Propose { intent: Intent },
    /// Sign + broadcast, only if `Allow`/approved → [`ExecuteResult`].
    Execute { request_id: RequestId },
    /// Poll for the native-card result → [`ApprovalStatus`].
    Status { request_id: RequestId },
    /// STOP: set `policy.revoked`, drop in-flight approvals → `Ack`.
    RevokeAll,
    /// Read-only snapshot for the agent → [`Policy`].
    PolicyGet,
    /// → [`Address`](alloy_primitives::Address).
    Address,
    /// → [`BalanceReport`].
    Balance { shielded: bool },
}

/// `deckard-signerd` → `deckard-mcp`. One variant per request shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignerResponse {
    Decision(Decision),
    Execute(ExecuteResult),
    Status(ApprovalStatus),
    /// Reply to `RevokeAll`.
    Ack,
    Policy(Policy),
    Address(Address),
    Balance(BalanceReport),
}

/// Outcome of `execute`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExecuteResult {
    /// Signed + broadcast.
    Broadcast { tx_hash: B256 },
    /// Refused at sign time (e.g. `revoked`, `already_executed`, `unknown_request`).
    Denied { reason: String },
}

/// Result of polling a `RequestId`. Approvals expire so a stale id can't be executed later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    /// Awaiting the human on the native card.
    Pending,
    /// The human approved; `execute` will sign (subject to a fresh `revoked` re-check).
    Allowed,
    /// Terminal denial.
    Denied { reason: String },
    /// The approval window elapsed.
    Expired,
}

/// Public + shielded balances, both in wei.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BalanceReport {
    pub public_wei: U256,
    pub shielded_wei: U256,
}
