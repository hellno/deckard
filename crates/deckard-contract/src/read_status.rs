//! `ReadStatus` — Deckard-owned trust label attached to every chain read.
//!
//! This is the contract the UI and the MCP agent surface see. The hard rule:
//! **never silently serve an untrusted read.** A read is either verified, or
//! visibly degraded/unsynced — never quietly trusted.
//!
//! The three states map onto *observable* Helios behavior (verified against
//! a16z/helios @ 0.11.1, `core/src/client/node.rs`):
//!
//! - `Verified` — Helios head is fresh (age ≤ 60s, the hard `check_head_age` gate)
//!   and the read came back from the verified light client.
//! - `Degraded` — still cryptographically verified, but off the happy path: we
//!   failed over to a secondary EL, or we're on a community fallback checkpoint.
//!   Trust note shown. Rarely emitted in v1.
//! - `Unsynced` — cannot produce a verified read right now: sync not finished, head
//!   stale past the 60s gate, the read failed, or verification is disabled. The UI
//!   shows a hard "NOT VERIFIED" state. Deckard MUST NOT fall back to a raw
//!   untrusted RPC and still claim it is verified.
//!
//! ## Portability
//!
//! `deckard-contract` is a **std** crate today (no `#![no_std]`), so `String`
//! here resolves to `std::string::String`. The type is written to be no_std-
//! *ready* — it leans only on `alloc`-available types (`String`) and `core::fmt`
//! for `Display` — so a future `#![no_std]` + `extern crate alloc` flip would be
//! mechanical, not a rewrite. Like every other wire type it carries the same
//! `serde` derives, so it round-trips byte-stably across JSON (the MCP surface)
//! and CBOR (the daemon UDS).

use core::fmt;

use serde::{Deserialize, Serialize};

/// Trust label attached to every chain read. Maps onto observable Helios state
/// (see deckard-core's verified read path).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadStatus {
    /// Helios head fresh and the read succeeded against the verified light client.
    /// Fully trustless.
    Verified,
    /// Still cryptographically verified, but off the happy path (EL failover or
    /// community-fallback checkpoint). `reason` is shown to the user. Rarely
    /// emitted in v1.
    Degraded { reason: String },
    /// No verified read is possible right now (Helios unsynced, head stale past
    /// the 60s gate, the read failed, or verification is disabled). `reason` is
    /// shown to the user. Deckard MUST NOT fall back to a raw untrusted RPC and
    /// still claim Verified.
    Unsynced { reason: String },
}

impl ReadStatus {
    /// Off-the-happy-path-but-still-verified label.
    pub fn degraded(reason: impl Into<String>) -> Self {
        ReadStatus::Degraded {
            reason: reason.into(),
        }
    }

    /// No-verified-read-possible label.
    pub fn unsynced(reason: impl Into<String>) -> Self {
        ReadStatus::Unsynced {
            reason: reason.into(),
        }
    }

    /// True only when a real, verified value backs the read.
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
