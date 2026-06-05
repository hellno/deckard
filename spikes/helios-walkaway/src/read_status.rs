//! `ReadStatus` — Deckard-owned trust label attached to every chain read.
//!
//! This is the contract the UI and the MCP agent surface see. The hard rule:
//! **never silently serve an untrusted read.** A read is either verified, or
//! visibly degraded/unsynced — never quietly trusted.
//!
//! The three states map onto *observable* Helios behavior (verified against
//! a16z/helios @ 0.11.1, `core/src/client/node.rs`):
//!
//! - `Verified`   — `syncing()` returns `SyncStatus::None` (head age ≤ 60s, the
//!                  hard `check_head_age` gate) and the read came back from the
//!                  primary upstream.
//! - `Degraded`   — still cryptographically verified, but off the happy path:
//!                  we failed over to a secondary EL, or we're on a community
//!                  fallback checkpoint. Trust note shown.
//! - `Unsynced`   — cannot produce a verified read: sync not finished, head
//!                  stale past the 60s gate (CL dark / "head frozen"), or all
//!                  upstreams down. The UI shows a hard "NOT VERIFIED" state.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadStatus {
    /// Helios head fresh, reading from the primary upstream. Fully trustless.
    Verified,
    /// Still verified, but not on the primary path. `reason` is shown to the user.
    Degraded { reason: String },
    /// No verified read is possible right now. `reason` is shown to the user.
    /// Deckard MUST NOT fall back to a raw untrusted RPC to fill this gap.
    Unsynced { reason: String },
}

impl ReadStatus {
    pub fn degraded(reason: impl Into<String>) -> Self {
        ReadStatus::Degraded { reason: reason.into() }
    }
    pub fn unsynced(reason: impl Into<String>) -> Self {
        ReadStatus::Unsynced { reason: reason.into() }
    }
    /// True only when a real, verified value backs the read. (Deckard-facing API.)
    #[allow(dead_code)]
    pub fn is_trustworthy(&self) -> bool {
        matches!(self, ReadStatus::Verified | ReadStatus::Degraded { .. })
    }
}

impl fmt::Display for ReadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadStatus::Verified => write!(f, "VERIFIED"),
            ReadStatus::Degraded { reason } => write!(f, "DEGRADED ({reason})"),
            ReadStatus::Unsynced { reason } => write!(f, "NOT VERIFIED ({reason})"),
        }
    }
}
