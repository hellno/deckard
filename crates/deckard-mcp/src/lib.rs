//! # deckard-mcp
//!
//! Deckard's **key-less agent surface**: one binary that is both a CLI and (with `--mcp`)
//! an MCP stdio server — the `mcp.v0.1` launch profile of `docs/build/30-mcp-shape.md`
//! (amended at launch: 6 `deckard_`-prefixed tools, raw `propose` and `simulate` cut; later
//! grown to 7 with the read-only `deckard_status` approval-poll tool).
//!
//! ## Security model
//! - **No key material, ever.** This process holds no seed, no spending key, no passphrase;
//!   there is no code path that could sign. Writes are [`deckard_contract::Intent`]s
//!   proposed over the same-uid Unix socket to `deckard-signerd`, which enforces the
//!   policy and signs in its own address space.
//! - **One transient secret:** the Railgun *viewing* key arrives alongside the wallet's own
//!   0zk address in `RailgunViewGrant` (recipient derivation). It is moved into `Zeroizing`
//!   on receipt, dropped immediately, and never logged or echoed ([`sidecar`]).
//! - **No secret in any response or transcript:** daemon reasons are URL-redacted at the
//!   daemon boundary; secret-shaped CLI flags are hard-rejected without echoing their
//!   values ([`secrets`]); the acceptance suite walks the full transcript (T9).
//! - **STOP is always available:** `deckard_revoke_all` / `deckard-mcp stop`.

pub mod amount;
pub use deckard_wallet_client::failure;
pub mod install;
pub mod secrets;
pub mod server;
pub mod sidecar;

pub use failure::Failure;
pub use sidecar::Sidecar;
