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
/// CoW Protocol orderbook REST client (quote / submit / status). Gated behind the default-on
/// `cow-client` feature so the daemon never compiles `reqwest`. When the feature is off, this
/// module is simply not named — there is no stub (callers reach for `cow_types` instead).
#[cfg(feature = "cow-client")]
pub mod cow_client;
/// CoW Protocol (GPv2) order types + EIP-712 machinery (digest, uid, slippage, approve decode,
/// cancel calldata). UNFEATURED on purpose: the signer daemon builds core with
/// `default-features = false` (no HTTP) but still needs to compute/sign/cancel orders, so this
/// module must always compile. No network code lives here — see `cow_client` for that.
pub mod cow_types;
/// Runtime env knobs for local-fork / demo mode (verified-reads toggle, shielded-sync fork
/// pin). Unfeatured so the daemon (built with `default-features = false`) can read them too.
pub mod env;
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
/// Read-only Railgun shielded-balance sync actor (Wave-2 T9). Gated behind `shield`.
#[cfg(feature = "shield")]
pub mod shielded;
pub mod tokens;

pub use balances::{fetch_portfolio, format_amount, Portfolio, TokenBalance};
// CoW order machinery, re-exported so the daemon + app + MCP can build/sign/cancel orders and
// decode shaped approvals through core without naming the `cow_types` path directly.
pub use cow_types::{
    apply_slippage, build_invalidate_order_calldata, cow_api_base, decode_approve, order_digest,
    order_uid, APPROVE_SELECTOR, APP_DATA_DOC, APP_DATA_HASH, GPV2_SETTLEMENT, GPV2_VAULT_RELAYER,
    ORDER_TYPE_HASH,
};
// The orderbook REST client + its serde types + pure parse helpers, re-exported only when the
// `cow-client` feature is on (the daemon, built without it, never sees these symbols).
pub use config::{config_dir, policy_path, vault_path};
#[cfg(feature = "cow-client")]
pub use cow_client::{
    get_account_orders, get_order_status, parse_account_orders, parse_error_body,
    parse_order_status, parse_order_uid, parse_quote_response, post_order, post_quote,
    put_app_data, swap_order_from_quote, AccountOrder, AppDataDoc, CowError, CowOrderbook,
    OrderCreation, OrderStatusResponse, QuoteOrderParameters, QuoteRequest, QuoteResponse,
    DEFAULT_SLIPPAGE_BPS,
};
pub use env::{demo_fork_block, screen_capture_allowed, verified_reads_enabled};
pub use eth::{EthProvider, Read, DEFAULT_RPC};
#[cfg(feature = "verified-reads")]
pub use helios::{launch_verified, VerifiedReader, DEFAULT_CONSENSUS_RPC};
pub use keystore::{
    atomic_write, random_word_positions, KdfParams, SecretKind, UnlockedVault, Vault, WordCount,
};
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
#[cfg(feature = "shield")]
pub use shielded::{ShieldedHandle, ShieldedSnapshot};
pub use tokens::{tokens_for, TokenInfo, DEFAULT_TOKENS, SEPOLIA_TOKENS};

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
