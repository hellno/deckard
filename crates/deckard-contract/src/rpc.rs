//! The daemon socket API — the wire `deckard-mcp` (key-less) speaks to `deckard-signerd`.
//! serde-derived so it frames as CBOR (ciborium) on the UDS and JSON for MCP. One request
//! per frame, one response per frame.

use alloy_primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::policy::Policy;
use crate::read_status::ReadStatus;
use crate::swap_order::SwapOrder;

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
    /// Export the read-only Railgun view grant (0zk address + viewing key) for shielded-balance
    /// sync → [`SignerResponse::RailgunView`]. The daemon refuses unless it's unlocked AND the
    /// derivation known-answer test passes (no grant from an unverified derivation).
    RailgunViewGrant { chain_id: u64, index: u32 },
    /// Propose a swap order (policy check only, NO signing) → [`Decision`].
    ProposeOrder { order: SwapOrder },
    /// Sign a stored, approved order's EIP-712 digest → [`SignOrderResult`]. No HTTP.
    SignOrder { request_id: RequestId },
    /// Broadcast an `invalidateOrder` cancel for a stored order → [`ExecuteResult`].
    CancelOrder { request_id: RequestId },
    /// List all in-flight pending records WITH payloads (the GUI approval inbox) → [`SignerResponse::Pending`].
    PendingList,
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
    /// Reply to `RailgunViewGrant`, or a `Decision::Deny` when locked / the gate fails.
    RailgunView(RailgunViewGrant),
    /// Reply to `SignOrder`.
    SignOrder(SignOrderResult),
    /// Reply to `PendingList`: every in-flight record with its full payload.
    Pending(Vec<PendingRecord>),
}

/// A read-only Railgun grant: the 0zk `address` + the `viewing_key` (hex). NOT the spending
/// key — the app can SEE private balances but cannot spend them (spending stays in the
/// daemon). The viewing key reveals private note history, so it's a secret: `Debug` is
/// redacted and callers must treat it accordingly.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct RailgunViewGrant {
    pub address: String,
    pub viewing_key: String,
}

impl core::fmt::Debug for RailgunViewGrant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RailgunViewGrant")
            .field("address", &self.address)
            .field("viewing_key", &"<redacted>")
            .finish()
    }
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
    /// Trust label for this read (Helios-verified vs unsynced/degraded). The hard
    /// rule: a balance is `Verified` only when a fresh Helios-verified read backs
    /// it; otherwise it is visibly `Unsynced`/`Degraded`, never quietly trusted.
    pub read_status: ReadStatus,
}

/// Outcome of `sign_order`. `signature` is the 65-byte r||s||v EIP-712 ECDSA signature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignOrderResult {
    Signed { signature: Bytes },
    Denied { reason: String },
}

/// One pending record for the GUI inbox (child #25 renders these). Carries the full payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingRecord {
    pub request_id: RequestId,
    pub status: ApprovalStatus,
    pub payload: PendingPayloadView,
}

/// The wire view of a pending record's payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PendingPayloadView {
    Tx(Intent),
    Order(SwapOrder),
    Approve {
        token: Address,
        spender: Address,
        amount: U256,
    },
}
