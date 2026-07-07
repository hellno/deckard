//! # Wire capability registry — the single source of truth (issue #31)
//!
//! What *kinds* of request does this build of the Deckard socket API understand? This module
//! answers that, and is the one place the answer lives. Every implementation builds its `Hello`
//! answer from [`hello_info`] — the daemon and the MCP acceptance mock over the wire
//! ([`SignerRequest::Hello`] → [`SignerResponse::Hello`]), and the contract
//! [`MockSigner`](crate::MockSigner) via its `hello()` — so a capability name, or the wire
//! `spec_version`, can never drift between implementations. That parity is the whole point of
//! capability discovery: an agent asks *does this build understand request kind X?* and gets the
//! same answer no matter which implementation is behind the socket.
//!
//! [`SignerRequest::Hello`]: crate::rpc::SignerRequest::Hello
//! [`SignerResponse::Hello`]: crate::rpc::SignerResponse::Hello
//! [`deckard-signerd`]: https://docs.rs/deckard-signerd
//!
//! ## The five evolution rules (normative — full text in `docs/build/40-wire-evolution.md`)
//!
//! 1. **Capability discovery, not version negotiation.** `Hello` returns [`HelloInfo`]; an old
//!    peer that predates a variant answers with the existing decode error — that IS the compat
//!    valve.
//! 2. **Date-version, bump only on a breaking change.** [`SPEC_VERSION`] is `YYYY-MM-DD`.
//!    Additions ship under a new capability NAME (below), *never* a version bump.
//! 3. **Protobuf discipline on the CBOR.** A capability name is forever: never reused, renamed,
//!    or retyped. Decoders ignore unknown struct keys and reject unknown enum variants.
//! 4. **Two distinguishable failures.** "unsupported message/capability" = a frame-decode error;
//!    "supported but refused" = a [`Decision::Deny`](crate::Decision) from the frozen
//!    [`deny_reasons`](crate::deny_reasons) vocabulary.
//! 5. **In-repo home.** This module + the build doc are that home.
//!
//! ## Adding a capability (the extension point #198 / #204 use)
//!
//! Registering a new request *kind* or origin variant is one edit here plus one row in the
//! `docs/build/40-wire-evolution.md` table — mirroring the deny-vocabulary discipline in
//! [`deny_reasons`](crate::deny_reasons):
//!
//!   1. add a `pub const CAP_… : &str = "…";` with a doc comment (what it means, since when),
//!   2. push it into [`capabilities`] in registry order (append; never reorder or reuse a name),
//!   3. add its row to the build-doc registry table.
//!
//! `spec_version` does **not** change (rule #2). `baseline_capabilities_present_and_wellformed`
//! guards the shape.

use crate::rpc::HelloInfo;

/// The wire spec's date-version, `YYYY-MM-DD` (rule #2). This is the date the capability-discovery
/// mechanism + these evolution rules were introduced — the wire's baseline. It is bumped **only**
/// on a genuinely breaking change (a removed/renamed/retyped map key, or changed semantics of an
/// existing frame). Additive capabilities never touch it.
pub const SPEC_VERSION: &str = "2026-07-07";

// ───────────────────────── Capability names ─────────────────────────
// A name is forever (rule #3): never reuse, rename, or retype one. A retired capability's name is
// left retired, never recycled. Keep these in registry order and mirror the build-doc table.

/// The shipped socket API — the frozen `deckard-contract` request/response set (unlock, propose,
/// execute, status, revoke_all, policy_get, address, balance, pending/activity, …). Since the
/// contract freeze; defining doc `docs/build/30-mcp-shape.md`.
pub const CAP_CORE: &str = "core";

/// The `mcp.v0.1` agent-tool profile — the key-less MCP sidecar's tool surface over `core`.
/// Defining doc `docs/build/31-agent-quickstart.md` (the tool list is drift-guarded there).
pub const CAP_MCP_V0_1: &str = "mcp.v0.1";

// ───────────────────────── Implementation names ─────────────────────────
// `impl_name` is informational only (rule #1: no code path branches on it). Each implementation
// reports its own honest name; the capabilities + spec_version are what must match across them.

/// `impl_name` the production daemon reports.
pub const IMPL_SIGNERD: &str = "deckard-signerd";
/// `impl_name` the in-memory mocks report (the contract `MockSigner`, the MCP acceptance mock).
pub const IMPL_MOCK: &str = "deckard-mock";

/// The capability names this build understands, in registry order. Building the `Hello` reply
/// from this one function is what keeps every implementation's answer identical.
#[must_use]
pub fn capabilities() -> Vec<String> {
    vec![CAP_CORE.to_string(), CAP_MCP_V0_1.to_string()]
}

/// Build the [`HelloInfo`] this build answers `Hello` with, tagged with the caller's `impl_name`.
///
/// The daemon calls this with [`IMPL_SIGNERD`]; the mocks call it with [`IMPL_MOCK`]. Only
/// `impl_name` differs between them — `spec_version` and `capabilities` are byte-identical, which
/// is the parity contract capability discovery exists to guarantee.
#[must_use]
pub fn hello_info(impl_name: &str) -> HelloInfo {
    HelloInfo {
        spec_version: SPEC_VERSION.to_string(),
        capabilities: capabilities(),
        impl_name: impl_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SPEC_VERSION` is a real `YYYY-MM-DD` date-string (rule #2). Validated without a regex dep:
    /// three `-`-separated groups of 4/2/2 ASCII digits.
    fn is_iso_date(s: &str) -> bool {
        let parts: Vec<&str> = s.split('-').collect();
        matches!(parts.as_slice(), [y, m, d]
            if y.len() == 4 && m.len() == 2 && d.len() == 2
                && [y, m, d].iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())))
    }

    #[test]
    fn spec_version_is_a_date() {
        assert!(
            is_iso_date(SPEC_VERSION),
            "SPEC_VERSION must be YYYY-MM-DD, got {SPEC_VERSION:?}"
        );
    }

    #[test]
    fn baseline_capabilities_present_and_wellformed() {
        let caps = capabilities();
        // The baseline registry the issue pins: core + the mcp.v0.1 profile.
        assert!(caps.iter().any(|c| c == CAP_CORE), "core missing");
        assert!(caps.iter().any(|c| c == CAP_MCP_V0_1), "mcp.v0.1 missing");

        // Rule #3 hygiene: names are lowercase, whitespace-free, non-empty, and unique.
        for c in &caps {
            assert!(!c.is_empty(), "empty capability name");
            assert!(
                c.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.'),
                "capability {c:?} must be lowercase [a-z0-9.]"
            );
        }
        let mut sorted = caps.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), caps.len(), "duplicate capability name");
    }

    #[test]
    fn hello_info_is_single_sourced() {
        // Whoever calls it, spec_version + capabilities are identical; only impl_name varies.
        let daemon = hello_info(IMPL_SIGNERD);
        let mock = hello_info(IMPL_MOCK);
        assert_eq!(daemon.spec_version, mock.spec_version);
        assert_eq!(daemon.capabilities, mock.capabilities);
        assert_eq!(daemon.spec_version, SPEC_VERSION);
        assert_eq!(daemon.capabilities, capabilities());
        assert_eq!(daemon.impl_name, IMPL_SIGNERD);
        assert_eq!(mock.impl_name, IMPL_MOCK);
        assert_ne!(daemon.impl_name, mock.impl_name);
    }
}
