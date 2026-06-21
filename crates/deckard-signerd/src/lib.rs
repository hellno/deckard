//! # deckard-signerd
//!
//! The process-isolated signer daemon — Deckard's operator spine. It owns the decrypted key
//! in its own address space, runs the real policy gate ([`deckard_contract::evaluate`]),
//! signs + broadcasts `Send` transactions, and answers STOP. The GUI app and the future
//! `deckard-mcp` are **key-less clients** that reach it over a same-uid Unix-domain socket
//! (4-byte-BE-length CBOR frames). This crate is a `lib` + `bin`: the library carries the
//! wire framing, socket/peer-cred plumbing, the daemon state machine, and the client +
//! supervisor the app reuses; `main.rs` is the daemon entry point.
//!
//! ## Security model
//! - **Key isolation:** only this process ever holds an [`deckard_core::UnlockedVault`]. The
//!   app/MCP never receive key bytes — only an [`deckard_contract::Address`] and decisions.
//! - **Caller auth:** every connection is gated on `SO_PEERCRED`/`LOCAL_PEERCRED` same-uid
//!   ([`auth`]); the socket is `0600` inside a `0700` dir ([`socket`]).
//! - **STOP = zeroize:** `Lock`/`RevokeAll` drop the `UnlockedVault` (zeroizing the secret)
//!   → `Locked`; re-arm only via a fresh `Unlock` ([`daemon`]).
//! - **TOCTOU:** `execute` re-checks `Locked` at sign time, so an approval granted before a
//!   STOP is still refused.

pub mod auth;
pub mod client;
pub mod config;
pub mod daemon;
pub mod frame;
pub mod policy_store;
pub mod request_id;
pub mod server;
pub mod signing;
pub mod socket;
pub mod spend_store;
pub mod supervise;

pub use client::SignerClient;
pub use config::Config;
pub use daemon::{Channel, Daemon};
pub use request_id::{request_id_for, request_id_for_order};
pub use supervise::{ControlChannel, DaemonSupervisor};
