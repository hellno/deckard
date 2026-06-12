//! # Deny-reason vocabulary — the frozen tag set
//!
//! The single source of truth for every machine-readable tag that fills a
//! `Decision::Deny { reason }`, an [`ExecuteResult::Denied`](crate::ExecuteResult),
//! a [`SignOrderResult::Denied`](crate::SignOrderResult), or a wire-level `reply_error`
//! across `deckard-contract`, `deckard-signerd`, and `deckard-mcp`.
//!
//! ## Why a consts module, not an enum
//!
//! The wire shape stays `reason: String` — the byte-stable serde round-trip of
//! [`Decision`](crate::Decision) is frozen and worth more than enum exhaustiveness. So we
//! freeze the *vocabulary* in code instead: every production construction site references a
//! const here, and `tests/deny_vocabulary.rs` fails the build if a `Deny`/`Denied`/
//! `reply_error` site is constructed from a raw string literal.
//!
//! ## Adding a tag is deliberate
//!
//! Minting a new refusal means two edits, on purpose:
//!   1. a `pub const` here with a doc comment (meaning + when it fires), and
//!   2. a row in `docs/build/31-agent-quickstart.md` (the agent-facing remediation table).
//!
//! Remediation guidance lives docs-side, never on the wire. An agent retrying against a
//! *stable* vocabulary can recover; one flailing against typos and synonyms cannot.
//!
//! ## Dynamic-prefix tags
//!
//! Four reasons carry a redacted one-line detail: `"<prefix>: <detail>"`. Each has a
//! dedicated builder ([`railgun_keys`], [`signer_error`], [`sign_failed`],
//! [`broadcast_failed`]) — there is deliberately no open `with_detail(prefix, …)`, so an
//! arbitrary string can never become a reason prefix. Consumers match on the prefix const
//! (e.g. `reason.starts_with(BROADCAST_FAILED)`), never the full string.

// ───────────────────────── Policy gate ─────────────────────────
// Minted by the pure decision functions in `policy.rs` (`evaluate`, `evaluate_order`); both
// `MockSigner` and the daemon route through them, so these are the parity-shared verdicts.

/// STOP / `revoke_all` is engaged: the panic brake denied this request. Fires whenever
/// `policy.revoked` is set — at propose and at the execute/sign TOCTOU re-check.
pub const REVOKED: &str = "revoked";
/// The recipient is not in the (non-empty) `allow_to` allowlist.
pub const OFF_ALLOWLIST: &str = "off_allowlist";
/// The intent's calldata does not match its `IntentKind` (e.g. a Shield with empty calldata,
/// or a Send carrying calldata).
pub const UNDECODABLE: &str = "undecodable";
/// Over a spending cap while `require_approval = Never` — no card exists to authorise it.
pub const OVER_CAP: &str = "over_cap";
/// Swap order receiver is the zero address.
pub const RECEIVER_ZERO: &str = "receiver_zero";
/// Swap order receiver is not the daemon's unlocked wallet (funds would leave the operator).
pub const RECEIVER_NOT_WALLET: &str = "receiver_not_wallet";
/// Swap order sell amount is zero (a garbage order).
pub const ZERO_AMOUNT: &str = "zero_amount";
/// Swap sell or buy token is not in the (non-empty) `allow_swap_tokens` list.
pub const OFF_SWAP_LIST: &str = "off_swap_list";
/// Swap order `valid_to` is more than 24h in the future.
pub const VALID_TO_TOO_FAR: &str = "valid_to_too_far";

// ─────────────────────── Daemon process-level ───────────────────────
// Process-state pre-checks the pure policy can't express (`deckard-signerd`); they run
// before / around `evaluate` and are NOT part of the MockSigner parity contract.

/// The daemon holds no key (it starts locked; lock/STOP zeroize the key). The `from_deny_reason`
/// catalog also distinguishes the "no vault yet" case under this same tag.
pub const LOCKED: &str = "locked";
/// The sidecar and the daemon disagree on the chain id (key-less pre-check, conclusive even
/// while locked).
pub const CHAIN_MISMATCH: &str = "chain_mismatch";
/// `IntentKind` unsupported in v0.1 (Unshield, or a non-shaped ContractCall). Reachable: an
/// Unshield or arbitrary ContractCall hits this (`daemon_e2e` asserts it for Unshield).
pub const UNSUPPORTED_V1: &str = "unsupported_v1";
/// An ERC-20 (`token = Some`) Send — v0.1 signs native-ETH sends only.
pub const ERC20_UNSUPPORTED_V1: &str = "erc20_unsupported_v1";
/// A Shield intent that does not target the chain's Railgun RelayAdapt contract.
pub const SHIELD_TO_MISMATCH: &str = "shield_to_mismatch";
/// Sign-time caps re-check failed for an auto-allowed request (the spend TOCTOU guard: two
/// within-cap proposals can't both broadcast past the daily cap).
pub const CAP_EXCEEDED: &str = "cap_exceeded";
/// The request is `Pending` — it needs a human approval that hasn't happened yet.
pub const NOT_APPROVED: &str = "not_approved";
/// A human answered the approval card with Deny.
pub const USER_DENIED: &str = "user_denied";
/// The request outlived its approval TTL before it was executed.
pub const EXPIRED: &str = "expired";
/// No request is stored under this id (a re-unlock or daemon restart starts a clean session).
pub const UNKNOWN_REQUEST: &str = "unknown_request";
/// This exact request was already broadcast (ids are deterministic per intent; do not retry).
pub const ALREADY_EXECUTED: &str = "already_executed";
/// The RPC did not answer within the broadcast window — transaction status UNKNOWN, do not retry.
pub const BROADCAST_TIMEOUT: &str = "broadcast_timeout";
/// The request frame could not be decoded at all (wire-level `reply_error`).
pub const MALFORMED_REQUEST: &str = "malformed_request";
/// The Railgun derivation known-answer test failed, so a view grant is refused. Only on the
/// `#[cfg(feature = "shield")]` path.
pub const DERIVATION_UNVERIFIED: &str = "derivation_unverified";
/// Built without the `shield` feature, so there is no Railgun derivation to grant. Only on the
/// `#[cfg(not(feature = "shield"))]` path.
pub const SHIELD_UNAVAILABLE: &str = "shield_unavailable";

// ───────────────────────── Swap v1 (CoW) ─────────────────────────
// Shaped-approve admission + order sign/cancel guards in the daemon, and the swap mock.

/// A swap `approve` carrying a non-zero ETH value (would move ETH invisibly).
pub const APPROVE_WITH_VALUE: &str = "approve_with_value";
/// A swap `approve` whose spender is not the GPv2 vault relayer.
pub const APPROVE_WRONG_SPENDER: &str = "approve_wrong_spender";
/// A swap `approve` with no stored order matching its (sell_token, sell_amount).
pub const APPROVE_NO_MATCHING_ORDER: &str = "approve_no_matching_order";
/// The stored order was already signed (idempotency guard against double-signing).
pub const ALREADY_SIGNED: &str = "already_signed";
/// The request id refers to a Tx payload where an Order was required (or vice versa).
pub const NOT_AN_ORDER: &str = "not_an_order";
/// The `MockSigner` used by the MCP test harness does not implement swaps. **Test surface
/// only** — never minted by a real daemon, so it is intentionally absent from the agent docs
/// table.
pub const SWAP_UNSUPPORTED_IN_MOCK: &str = "swap_unsupported_in_mock";

// ─────────────────── Dynamic-prefix reasons ───────────────────
// Rendered as `"<prefix>: <detail>"` where `<detail>` is an already-redacted one-line error.
// Each prefix has a dedicated builder below; consumers match the prefix const, never the
// whole string. There is intentionally no `with_detail(prefix, …)` taking an arbitrary
// prefix — that would reopen the free-form hole this module exists to close.

/// Railgun key derivation/grant error (shield feature). Built by [`railgun_keys`].
pub const RAILGUN_KEYS: &str = "railgun_keys";
/// The daemon could not obtain an account signer for the unlocked wallet. Built by [`signer_error`].
pub const SIGNER_ERROR: &str = "signer_error";
/// Offline EIP-712 order-digest signing failed. Built by [`sign_failed`].
pub const SIGN_FAILED: &str = "sign_failed";
/// The RPC refused the broadcast (nothing was consumed). Built by [`broadcast_failed`].
pub const BROADCAST_FAILED: &str = "broadcast_failed";

/// The `": "` separator that joins a prefix to its detail — single-sourced so every
/// dynamic-prefix reason renders byte-identically and consumer `starts_with(PREFIX)` checks
/// keep matching.
fn prefixed(prefix: &str, detail: impl core::fmt::Display) -> String {
    format!("{prefix}: {detail}")
}

/// `"railgun_keys: <detail>"` — see [`RAILGUN_KEYS`].
#[must_use]
pub fn railgun_keys(detail: impl core::fmt::Display) -> String {
    prefixed(RAILGUN_KEYS, detail)
}

/// `"signer_error: <detail>"` — see [`SIGNER_ERROR`].
#[must_use]
pub fn signer_error(detail: impl core::fmt::Display) -> String {
    prefixed(SIGNER_ERROR, detail)
}

/// `"sign_failed: <detail>"` — see [`SIGN_FAILED`].
#[must_use]
pub fn sign_failed(detail: impl core::fmt::Display) -> String {
    prefixed(SIGN_FAILED, detail)
}

/// `"broadcast_failed: <detail>"` — see [`BROADCAST_FAILED`].
#[must_use]
pub fn broadcast_failed(detail: impl core::fmt::Display) -> String {
    prefixed(BROADCAST_FAILED, detail)
}
