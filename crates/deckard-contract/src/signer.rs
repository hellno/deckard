//! The signer abstraction. **Sync on purpose**: it keeps this crate runtime-free. The real
//! UDS client does a fast blocking round-trip off the UI thread; wrapping it in async is the
//! daemon ticket's call, not this contract's. `MockSigner` is the in-memory implementation.

use alloy_primitives::Address;

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::policy::Policy;
use crate::rpc::{ApprovalStatus, BalanceReport, ExecuteResult, SignOrderResult, UnlockOutcome};
use crate::swap_order::SwapOrder;

/// The daemon-socket API expressed as a trait, so callers can hold a `Box<dyn Signer>` and
/// swap the mock for the real UDS client without changing a line. Object-safe: every method
/// takes `&self` and returns owned values.
pub trait Signer {
    /// Unlock the vault for the session (the daemon decrypts + holds the key). Returns the
    /// wallet address on success — never key material.
    fn unlock(&self, passphrase: &str) -> UnlockOutcome;
    /// Lock: zeroize + drop the held key and deny in-flight approvals. Re-arm via `unlock`.
    fn lock(&self);
    /// Close an approval loop: flip a `Pending` request to `Allowed`/`Denied`.
    fn resolve(&self, request_id: RequestId, approved: bool);
    /// The wallet's public address (key-less to read).
    fn address(&self) -> Address;
    /// Public + shielded balances. `shielded` mirrors the wire request; the report carries
    /// both fields regardless.
    fn balance(&self, shielded: bool) -> BalanceReport;
    /// A read-only snapshot of the spending fence.
    fn policy(&self) -> Policy;
    /// Policy check only — NEVER signs, NEVER broadcasts. Returns a [`Decision`].
    fn propose(&self, intent: &Intent) -> Decision;
    /// Sign + broadcast, only for an allow-equivalent or approved request. Re-checks
    /// `revoked` at sign time (TOCTOU guard).
    fn execute(&self, request_id: RequestId) -> ExecuteResult;
    /// Poll an approval handle.
    fn status(&self, request_id: RequestId) -> ApprovalStatus;
    /// STOP: revoke all agent authority for the session and drop in-flight approvals.
    fn revoke_all(&self);
    /// Swap-order policy check only — NEVER signs. Returns a [`Decision`].
    fn propose_order(&self, order: &SwapOrder) -> Decision;
    /// Sign a stored, approved order (EIP-712). Re-checks `revoked` at sign time (TOCTOU).
    fn sign_order(&self, request_id: RequestId) -> SignOrderResult;
    /// Broadcast an `invalidateOrder` cancel for a stored order.
    fn cancel_order(&self, request_id: RequestId) -> ExecuteResult;
}
