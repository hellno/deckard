//! `ShieldStatus` — the lifecycle of a shield (public → private) deposit.
//!
//! A shield moves funds from a public Ethereum balance into a Railgun private note.
//! That is not a single instant: the tx must be broadcast, included + confirmed on
//! chain, and then the Railgun UTXO sync must catch up before the private note is
//! visible and spendable. This enum is the **spec-complete map of those steps**, plus
//! the per-state reassurance copy ("where's my money?") and a status-glyph hook the UI
//! renders. Wave 2 drives the transitions; v1 builds the minimal path
//! (`Sending` → `ConfirmingOnChain` → `PrivateSpendable`) and leaves the rest as
//! specced-but-stubbed variants.
//!
//! ## Portability
//!
//! Like every other wire type in this crate, `ShieldStatus` carries the standard
//! `serde` derives so it round-trips byte-stably across JSON (the MCP surface) and
//! CBOR (the daemon UDS), and it leans only on `core::fmt` + `alloc`-available types
//! (`String`) so a future `#![no_std]` flip would be mechanical. The glyph hook
//! returns a plain `&'static str` *semantic* token (never a GPUI `IconName`) so the
//! key-less contract crate stays free of any UI dependency; the app maps the token to
//! the circular status glyph defined in `DESIGN.md`.

use core::fmt;

use alloy_primitives::{B256, U256};
use serde::{Deserialize, Serialize};

/// The lifecycle of a shield deposit, from broadcast to spendable private balance.
///
/// The happy path is `Sending` → `ConfirmingOnChain` → `SyncingPrivate` →
/// `PrivateSpendable`; any step may instead terminate in `Failed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShieldStatus {
    /// Tx signed and broadcast; awaiting first inclusion in a block.
    Sending,
    /// Included on chain; waiting for `confirmed`/`target` confirmations before the
    /// note is sync-visible.
    ConfirmingOnChain {
        tx_hash: B256,
        confirmed: u32,
        target: u32,
    },
    /// Confirmed; the Railgun UTXO sync is catching up so the private note appears.
    SyncingPrivate { tx_hash: B256 },
    /// The note is synced and spendable. `shielded_wei` is the private balance now
    /// available (net of the on-chain Railgun fee).
    PrivateSpendable { shielded_wei: U256 },
    /// Terminal failure at any step. `reason` is a short, non-secret tag
    /// (e.g. `broadcast_failed`, `reverted`, `sync_failed`).
    Failed { reason: String },
}

impl ShieldStatus {
    /// A short, stable semantic token for the circular status glyph the UI renders
    /// (see `DESIGN.md`: filled check = confirmed/done, amber clock ring = pending,
    /// error x-ring = failed). The app maps this token to its icon kit; the contract
    /// crate stays UI-free. The strings are stable wire-adjacent identifiers.
    pub fn glyph(&self) -> &'static str {
        match self {
            // In-flight: the amber clock-ring "pending" glyph.
            ShieldStatus::Sending
            | ShieldStatus::ConfirmingOnChain { .. }
            | ShieldStatus::SyncingPrivate { .. } => "clock-ring",
            // Done: the filled-check "confirmed" glyph.
            ShieldStatus::PrivateSpendable { .. } => "check-filled",
            // Terminal failure: the error x-ring glyph.
            ShieldStatus::Failed { .. } => "x-ring",
        }
    }

    /// True once the shielded note is synced and spendable — the only terminal-success
    /// state.
    pub fn is_spendable(&self) -> bool {
        matches!(self, ShieldStatus::PrivateSpendable { .. })
    }

    /// True for any terminal state (spendable or failed) — nothing more will transition.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ShieldStatus::PrivateSpendable { .. } | ShieldStatus::Failed { .. }
        )
    }
}

impl fmt::Display for ShieldStatus {
    /// The "where's my money?" reassurance line shown in the status strip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShieldStatus::Sending => write!(f, "Broadcasting your deposit…"),
            ShieldStatus::ConfirmingOnChain {
                confirmed, target, ..
            } => write!(
                f,
                "On-chain. Waiting for {confirmed}/{target} confirmations — your funds are safe."
            ),
            ShieldStatus::SyncingPrivate { .. } => {
                write!(f, "Confirmed. Syncing your private balance…")
            }
            ShieldStatus::PrivateSpendable { .. } => write!(f, "Private. Spendable now."),
            ShieldStatus::Failed { reason } => write!(f, "Shield failed ({reason})."),
        }
    }
}
