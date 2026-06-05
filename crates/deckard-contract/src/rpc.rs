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
    /// Unlock the vault: the daemon reads the keystore, decrypts with `passphrase`, and
    /// holds the key for the session → [`SignerResponse::Unlock`]\([`UnlockOutcome`]\).
    ///
    /// The wire passphrase is a plain `String` because `zeroize::Zeroizing<String>` does
    /// not derive `Serialize`. The daemon moves it into `Zeroizing` the instant the frame
    /// is decoded and never retains the raw buffer; it never echoes the passphrase back.
    Unlock { passphrase: String },
    /// Lock: zeroize + drop the held key → `Locked`, and deny every in-flight approval.
    /// Re-arm only via a fresh [`Unlock`](Self::Unlock). → `Ack`.
    Lock,
    /// Close an approval loop opened by a `NeedsApproval`: flip the `Pending` record to
    /// `Allowed` (`approved: true`) or `Denied` (`approved: false`). → `Ack`.
    Resolve {
        request_id: RequestId,
        approved: bool,
    },
    /// Policy check, NO signing yet → [`Decision`].
    Propose { intent: Intent },
    /// Sign + broadcast, only if `Allow`/approved → [`ExecuteResult`].
    Execute { request_id: RequestId },
    /// Poll for the native-card result → [`ApprovalStatus`].
    Status { request_id: RequestId },
    /// STOP: zeroize the key, lock the daemon, drop in-flight approvals → `Ack`.
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
    /// Reply to `Unlock`.
    Unlock(UnlockOutcome),
    Decision(Decision),
    Execute(ExecuteResult),
    Status(ApprovalStatus),
    /// Reply to `Lock`, `Resolve`, and `RevokeAll`.
    Ack,
    Policy(Policy),
    Address(Address),
    Balance(BalanceReport),
}

/// Outcome of `Unlock`. Carries the wallet address on success — never any key material,
/// never the passphrase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnlockOutcome {
    /// Decrypted; the daemon now holds the key. `address` is the primary account.
    Unlocked { address: Address },
    /// The passphrase was wrong (or the vault was tampered with). No key is held.
    BadPassphrase,
    /// No keystore file exists yet — onboarding must create one first.
    NoVault,
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
    /// Approved (by a human, or auto within cap); `execute` will sign — subject to fresh
    /// re-checks at sign time (revoked, TTL expiry, and the spend caps for an auto-allow).
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
