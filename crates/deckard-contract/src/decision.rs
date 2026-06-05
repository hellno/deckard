//! The daemon's verdict, returned by `propose`. The agent cannot forge `Allow`.

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

/// What `propose` decided about an [`crate::Intent`]. A `Decision::Allow` or an approved
/// `RequestId` is the *only* token that lets `execute` sign.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Decision {
    /// Within policy → safe to `execute`.
    Allow,
    /// Policy violation; terminal. `reason` is a short machine-readable tag
    /// (e.g. `revoked`, `off_allowlist`, `undecodable`, `over_cap`).
    Deny { reason: String },
    /// A human must approve via the native card before `execute` will sign.
    NeedsApproval { request_id: RequestId },
}

/// Opaque approval handle; the agent polls `status` on it. (A 32-byte hash so the daemon
/// can make it unguessable in production.)
pub type RequestId = B256;
