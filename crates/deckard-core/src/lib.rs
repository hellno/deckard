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
// (allow-{unwrap,expect,panic,indexing-slicing}-in-tests). The two legitimate startup-fatal
// `expect`s in eth.rs carry a local `#[allow(clippy::expect_used)]` with a documented reason.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::get_unwrap,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::mem_forget
)]

pub mod balances;
pub mod eth;
pub mod keystore;
pub mod tokens;

pub use balances::{fetch_portfolio, format_amount, Portfolio, TokenBalance};
pub use eth::{EthProvider, DEFAULT_RPC};
pub use keystore::{random_word_positions, KdfParams, SecretKind, UnlockedVault, Vault, WordCount};
pub use tokens::{TokenInfo, DEFAULT_TOKENS};

// Re-export the alloy primitive types the UI renders, so the app layer doesn't
// need a direct alloy dependency just to name an `Address` or a `U256`.
pub use alloy_primitives::{Address, U256};
