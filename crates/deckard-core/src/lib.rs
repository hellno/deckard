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

pub mod balances;
pub mod config;
pub mod eth;
/// Embedded Helios light client → verified localhost reads. Gated behind the
/// default-on `verified-reads` feature so the heavy revm/bls build is toggleable.
#[cfg(feature = "verified-reads")]
pub mod helios;
pub mod keystore;
pub mod tokens;

pub use balances::{fetch_portfolio, format_amount, Portfolio, TokenBalance};
pub use config::{config_dir, policy_path, vault_path};
pub use eth::{EthProvider, Read, DEFAULT_RPC};
#[cfg(feature = "verified-reads")]
pub use helios::{launch_verified, VerifiedReader, DEFAULT_CONSENSUS_RPC};
pub use keystore::{random_word_positions, KdfParams, SecretKind, UnlockedVault, Vault, WordCount};
pub use tokens::{TokenInfo, DEFAULT_TOKENS};

// The shared trust label, re-exported so the app + daemon can name it through core
// without a direct deckard-contract dependency just to render a read status.
pub use deckard_contract::ReadStatus;

// Re-export the alloy primitive types the UI renders, so the app layer doesn't
// need a direct alloy dependency just to name an `Address` or a `U256`.
pub use alloy_primitives::{Address, U256};
