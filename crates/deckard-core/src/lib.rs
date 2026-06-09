//! deckard-core — Deckard's headless engine.
//!
//! Zero GPUI dependency: the Ethereum provider, balances, HD key derivation, and
//! the encrypted keystore all live here so they are unit-testable in isolation and
//! portable if the view layer ever changes. The GPUI app is a thin shell over this.
//!
//! Threading model (eng-review decision): a *single* background tokio runtime
//! thread owns every network call. Results bridge back to the GUI's own executor
//! over `flume` channels, whose `recv_async()` future is runtime-agnostic — so the
//! GUI thread never blocks and never touches tokio.

// The security engine is pure-safe by construction. `forbid` (stronger than the workspace-wide
// `deny`) makes that a hard compile-time guarantee that cannot be locally overridden — no agent or
// future edit can introduce an `unsafe` block in the crate that touches keys and untrusted bytes.
#![forbid(unsafe_code)]
// Panic-class restriction lints for the trust core (issue #7, hardened path): production code in
// deckard-core propagates errors, it does not panic on bad input. Tests are exempt via clippy.toml
// (allow-{unwrap,expect,panic,indexing-slicing}-in-tests). The few legitimate startup/FFI-boundary
// panics carry a local `#[allow(clippy::…)]` with a documented reason.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::get_unwrap,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::mem_forget
)]

pub mod balances;
pub mod config;
pub mod eth;
/// Embedded Helios light client → verified localhost reads. Gated behind the
/// default-on `verified-reads` feature so the heavy revm/bls build is toggleable.
#[cfg(feature = "verified-reads")]
pub mod helios;
pub mod keystore;
/// Railgun seed→0zk-key derivation (SLIP-0010 ed25519), gated behind `shield`. The
/// consensus-critical path: KAT-verified against Railgun's own engine vector so a wrong
/// derivation can't silently show a $0 shielded balance.
#[cfg(feature = "shield")]
pub mod railgun_keys;
/// Key-less Railgun shield-calldata builder. Gated behind the default-on `shield` feature
/// so the heavy ZK `railgun` crate is toggleable. When the feature is off, the
/// `build_shield_native_intent` stub below returns a clear error (never a fake success).
#[cfg(feature = "shield")]
pub mod shield;
pub mod tokens;

pub use balances::{fetch_portfolio, format_amount, Portfolio, TokenBalance};
pub use config::{config_dir, policy_path, vault_path};
pub use eth::{EthProvider, Read, DEFAULT_RPC};
#[cfg(feature = "verified-reads")]
pub use helios::{launch_verified, VerifiedReader, DEFAULT_CONSENSUS_RPC};
pub use keystore::{random_word_positions, KdfParams, SecretKind, UnlockedVault, Vault, WordCount};
// The key-less shield-calldata builder + the 0zk recipient type, re-exported so the daemon
// and its tests can name them through core without a direct `railgun` dependency.
#[cfg(feature = "shield")]
pub use shield::{build_shield_native_intent, RailgunAddress};
// Railgun key derivation + the runtime known-answer gate (`known_answer_ok`), re-exported so
// the app can derive the user's own 0zk address and refuse to show shielded balances until the
// gate passes.
#[cfg(feature = "shield")]
pub use railgun_keys::{
    known_answer_ok, railgun_address_from_entropy, railgun_keys_from_entropy, RailgunKeys,
};
pub use tokens::{TokenInfo, DEFAULT_TOKENS};

/// Feature-off stub: when `shield` is compiled out, the symbol still exists so the daemon
/// and tests build, but it returns a clear error — NEVER a fake success. Mirrors the
/// honest-failure pattern the `verified-reads`-off read path uses (a Deny/Unsynced label
/// rather than a silent fabricated value).
#[cfg(not(feature = "shield"))]
pub fn build_shield_native_intent(
    _chain_id: u64,
    _recipient: (),
    _value: alloy_primitives::U256,
) -> anyhow::Result<deckard_contract::Intent> {
    anyhow::bail!("shield unavailable (feature off)")
}

// The shared trust label, re-exported so the app + daemon can name it through core
// without a direct deckard-contract dependency just to render a read status.
pub use deckard_contract::ReadStatus;

// Re-export the alloy primitive types the UI renders, so the app layer doesn't
// need a direct alloy dependency just to name an `Address` or a `U256`.
pub use alloy_primitives::{Address, U256};
