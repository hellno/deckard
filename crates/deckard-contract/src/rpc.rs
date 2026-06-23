//! The daemon socket API — the wire `deckard-mcp` (key-less) speaks to `deckard-signerd`.
//! serde-derived so it frames as CBOR (ciborium) on the UDS and JSON for MCP. One request
//! per frame, one response per frame.

use alloy_primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::message_signing::SignMessage;
use crate::policy::Policy;
use crate::read_status::ReadStatus;
use crate::swap_order::SwapOrder;

/// WHO proposed a pending record: a foreground human action in the app (`App`), or an
/// autonomous agent (the MCP sidecar, `Agent`). Drives the Approvals agent header band and
/// Activity's two-actor chain. `App` is the safe default (the `#[default]` variant) so an
/// un-tagged proposal never masquerades as an agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalOrigin {
    #[default]
    App,
    Agent,
}

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
    /// Policy check, NO signing yet → [`Decision`]. `origin` records WHO proposed (a
    /// foreground human app action vs an autonomous agent) so the GUI inbox can render the
    /// agent band / two-actor chain; it never affects the policy verdict.
    Propose {
        intent: Intent,
        origin: ProposalOrigin,
    },
    /// Sign + broadcast, only if `Allow`/approved → [`ExecuteResult`].
    Execute { request_id: RequestId },
    /// Poll for the native-card result → [`ApprovalStatus`].
    Status { request_id: RequestId },
    /// Rich per-request status for the agent poll loop (`deckard_status`). Read-only, no signing.
    StatusView { request_id: RequestId },
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
    /// Propose a swap order (policy check only, NO signing) → [`Decision`]. `origin` records WHO
    /// proposed (a foreground human GUI swap vs an autonomous agent), mirroring [`Propose`]'s
    /// origin — display-only (drives the feed's two-actor chain), never affects the verdict. A
    /// user-driven GUI swap MUST pass `App` so the order row doesn't masquerade as the agent.
    ProposeOrder {
        order: SwapOrder,
        origin: ProposalOrigin,
    },
    /// Propose an off-chain message signature (policy check only, NO signing) → [`Decision`].
    ProposeMessage {
        message: SignMessage,
        origin: ProposalOrigin,
    },
    /// Sign a stored, approved order's EIP-712 digest → [`SignOrderResult`]. No HTTP.
    SignOrder { request_id: RequestId },
    /// Sign a stored, approved message → [`SignMessageResult`]. No HTTP, no broadcast.
    SignMessage { request_id: RequestId },
    /// Broadcast an `invalidateOrder` cancel for a stored order → [`ExecuteResult`].
    CancelOrder { request_id: RequestId },
    /// List all in-flight pending records WITH payloads (the GUI approval inbox) → [`SignerResponse::Pending`].
    PendingList,
    /// Read the **activity feed**: every tracked action (auto-allowed, pending, denied, and
    /// executed) as an [`ActivityRecord`], newest-first → [`SignerResponse::Activity`]. Unlike
    /// `PendingList` this retains auto-allowed/executed rows (with their `tx_hash` + timestamp),
    /// so the GUI can show what the agent *did*, not only what is pending. The handler expires
    /// stale rows first, so the feed never shows a lapsed card as still pending.
    ActivityFeed,
}

/// `deckard-signerd` → `deckard-mcp`. One variant per request shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignerResponse {
    /// Reply to `Unlock`.
    Unlock(UnlockOutcome),
    Decision(Decision),
    Execute(ExecuteResult),
    Status(ApprovalStatus),
    /// Reply to `StatusView`.
    StatusView(StatusView),
    /// Reply to `Lock`, `Resolve`, and `RevokeAll`.
    Ack,
    Policy(Policy),
    Address(Address),
    Balance(BalanceReport),
    /// Reply to `RailgunViewGrant`, or a `Decision::Deny` when locked / the gate fails.
    RailgunView(RailgunViewGrant),
    /// Reply to `SignOrder`.
    SignOrder(SignOrderResult),
    /// Reply to `SignMessage`.
    SignMessage(SignMessageResult),
    /// Reply to `PendingList`: every in-flight record with its full payload.
    Pending(Vec<PendingRecord>),
    /// Reply to `ActivityFeed`: the activity ledger, newest-first.
    Activity(Vec<ActivityRecord>),
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

/// Result of `sign_message`. `signature` is a 65-byte ECDSA signature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignMessageResult {
    Signed { signature: Bytes },
    Denied { reason: String },
}

/// One pending record for the GUI inbox (child #25 renders these). Carries the full payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingRecord {
    pub request_id: RequestId,
    pub status: ApprovalStatus,
    pub payload: PendingPayloadView,
    /// Millis until the approval TTL elapses, computed by the daemon from `expires_at: Instant`
    /// at list time. `0` for terminal (Allowed/Denied/Expired) or already past TTL. A snapshot.
    pub remaining_ms: u64,
    /// Who proposed this — drives the agent band + Activity two-actor chain.
    pub origin: ProposalOrigin,
}

/// Rich per-request status for the agent's poll loop (the `deckard_status` MCP tool).
/// Assembled by the daemon from the request's pending record. Additive — leaves
/// [`ApprovalStatus`] and `SignerRequest::Status` untouched (#31 additive-evolution rule),
/// so the existing app/daemon status path is unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatusView {
    pub request_id: RequestId,
    /// Approval state — `Denied { reason }` carries the deny tag the agent needs to recover
    /// (user_denied / revoked / unknown_request / expired / ...). NEVER collapse the reason.
    pub status: ApprovalStatus,
    /// Millis until the approval TTL elapses (Pending/Allowed); `0` when terminal or past TTL.
    pub remaining_ms: u64,
    /// `Some` once the request was signed + broadcast.
    pub tx_hash: Option<B256>,
    /// Lifecycle position. For an unknown request id: `Expired`.
    pub lifecycle: ActivityLifecycle,
}

/// The wire view of a pending record's payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PendingPayloadView {
    Tx(Intent),
    Order(SwapOrder),
    Message(SignMessage),
    Approve {
        token: Address,
        spender: Address,
        amount: U256,
    },
}

/// Where an [`ActivityRecord`] sits in its lifecycle: `Proposed` (stored, awaiting a human
/// decision — an over-cap or auto-approval-guardrail card, still approvable), `Decided` (a verdict
/// landed — `approved: true` for an auto-allow-within-cap or a human approval; `approved:
/// false` for a denial or a STOP revoke — both cases where **a human acted**), `Expired` (the
/// approval window lapsed with **no human action**), or `Executed` (signed + broadcast, so the
/// record's `tx_hash` is `Some`).
///
/// `Expired` is split out from `Decided{approved:false}` on purpose: the feed's amber tint means
/// "a human acted here" (DESIGN §the actor model), and a lapsed window is the one closed state
/// where nobody acted — so it must render neutral, never amber. A human denial and a STOP revoke
/// stay `Decided{approved:false}` (pressing deny / STOP *is* a human action).
///
/// This is its OWN enum, deliberately NOT extra [`ApprovalStatus`] variants: the feed needs a
/// distinct shape (auto-allowed/executed rows that never wait in `PendingList`), and new
/// `ApprovalStatus` variants would ripple through every exhaustive match in the daemon + app.
/// `ApprovalStatus` and `PendingRecord` stay untouched (#28/#31 additive-evolution rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityLifecycle {
    /// Stored and waiting on a human — approvable from the feed (and the Approvals queue).
    Proposed,
    /// A decision landed by a human (or an auto-allow). `approved == true`: auto-allowed within
    /// cap, or a human approval. `approved == false`: a human denial or a STOP/`revoke_all` — in
    /// both a human acted.
    Decided { approved: bool },
    /// The approval window lapsed before anyone acted — a closed, never-approved card with NO
    /// human in the loop. Rendered neutral (never the amber "you acted" tint).
    Expired,
    /// Signed + broadcast — the record's `tx_hash` is `Some`.
    Executed,
}

/// Which spending fence a proposal breached, recomputed by the daemon at record-write time
/// (a read of data it already holds) so the feed can cite the **actual** cap hit — never a
/// hardcoded "over per-tx cap". `None` for a within-cap auto-allow or an auto-approval-guardrail
/// hold (no cap was breached; the hold is the guardrail, not a cap). This is display-only and lives
/// OFF the verdict path: [`evaluate`](crate::evaluate) still collapses both caps into one
/// `over` bool and returns no reason — that frozen function is unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreachedLimit {
    /// No cap breached — a within-cap auto-allow, or an auto-approval-guardrail hold.
    #[default]
    None,
    /// The per-transaction ceiling.
    PerTxCap,
    /// The rolling daily ceiling.
    DailyCap,
    /// The recipient is not in the (non-empty) allow-list.
    OffAllowlist,
}

/// One row in the **activity feed** — what an actor (agent or human) *did*, not only what is
/// pending. Unlike [`PendingRecord`], the feed retains auto-allowed and executed actions that
/// never wait in `PendingList`, so it is a true session ledger.
///
/// - `origin` is the two-signal actor (agent = cyan, human = amber).
/// - `timestamp_ms` is daemon-stamped unix **millis** at propose time (consistent with
///   `remaining_ms`; supports a clock/relative-time render).
/// - `tx_hash` is `Some` once the action was signed + broadcast.
/// - `reason` cites the breached fence for a pending/decided card (display-only).
///
/// Reuses the generic [`PendingPayloadView`] so non-shield activity (send/swap/approve) drops
/// in later without a record-shape change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActivityRecord {
    pub request_id: RequestId,
    /// WHO acted — drives the feed's two-actor chain (agent cyan, human amber).
    pub origin: ProposalOrigin,
    pub payload: PendingPayloadView,
    /// Unix epoch **millis**, daemon-stamped when the record was first proposed.
    pub timestamp_ms: u64,
    /// `Some` once the action was signed + broadcast.
    pub tx_hash: Option<B256>,
    pub lifecycle: ActivityLifecycle,
    /// The breached fence (display-only; `None` for a within-cap auto-allow / guardrail hold).
    pub reason: BreachedLimit,
    /// `true` only when the daemon auto-allowed this hands-free at propose (within cap, off
    /// mainnet). A mainnet-guardrail hold and an over-cap card are both `false` even though
    /// neither breached a cap, so the feed can honestly say "auto-approved within cap" vs
    /// "you approved" instead of inferring it from the absent breach `reason`. `#[serde(default)]`
    /// keeps an older producer (no field) decoding to the safe `false` (= a human was involved).
    #[serde(default)]
    pub auto_allowed: bool,
}
