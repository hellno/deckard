//! `Upstreams` — Deckard's own multi-instance supervisor over N Helios clients.
//!
//! Helios has **no native multi-EL failover**: one `EthereumClient` is wired to
//! exactly one execution RPC and one consensus RPC (verified: `execution_rpc`/
//! `consensus_rpc` on `EthereumClientBuilder` are single `Url`s). So the walkaway
//! beat ("cut the centralized EL RPC and keep serving verified reads") is *our*
//! logic, not Helios's.
//!
//! Design — **Shape A (multi-instance + supervisor)**, chosen because of how
//! Helios is built (verified against `core/src/client/node.rs`):
//!
//!   * The **consensus** client pushes each sync-committee-verified execution
//!     header into the execution provider's in-memory cache. So `get_block_number`
//!     / head is **CL-driven and served from cache — it does NOT touch the EL RPC.**
//!   * Only **state reads** (`get_balance` → `get_account` → `eth_getProof`) hit
//!     the EL RPC, then verify the returned account against the cached header's
//!     state root.
//!
//! Therefore cutting the EL leaves the head live and advancing while only state
//! reads fail — and a *second, already-synced* client recovers them instantly by
//! re-deriving the proof from an independent untrusted EL and re-verifying against
//! the same CL+checkpoint. Both clients are equally trustless; failover is honest
//! re-verification, not a cached stale value.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{Address, U256};
use alloy::rpc::types::SyncStatus;
use helios_ethereum::EthereumClient;

use crate::read_status::ReadStatus;

/// A labeled Helios client + the human name of the EL it talks to.
pub struct Upstream {
    pub label: String,
    pub client: EthereumClient,
}

pub struct Upstreams {
    upstreams: Vec<Upstream>,
    active: AtomicUsize,
    /// Per-read timeout. After this, a hung EL is treated as down and we fail over.
    read_timeout: Duration,
}

/// What a supervised read returns: the value (if any), its trust label, and the
/// upstream that actually served it.
pub struct Read {
    pub value: Option<U256>,
    pub status: ReadStatus,
    pub served_by: Option<String>,
}

impl Upstreams {
    pub fn new(upstreams: Vec<Upstream>, read_timeout: Duration) -> Self {
        Self {
            upstreams,
            active: AtomicUsize::new(0),
            read_timeout,
        }
    }

    #[allow(dead_code)] // Deckard-facing API (which upstream is live), shown in UI.
    pub fn active_label(&self) -> &str {
        &self.upstreams[self.active.load(Ordering::SeqCst)].label
    }

    /// Read a balance with failover. Tries the active upstream first; on error or
    /// timeout, walks the remaining upstreams. The first success wins and becomes
    /// the new active upstream. If every upstream fails, classifies the outage as
    /// `Unsynced` — and never returns an untrusted value.
    pub async fn get_balance(&self, addr: Address) -> Read {
        let n = self.upstreams.len();
        let start = self.active.load(Ordering::SeqCst);
        let block = BlockNumberOrTag::Latest.into();

        for offset in 0..n {
            let idx = (start + offset) % n;
            let up = &self.upstreams[idx];

            let attempt = tokio::time::timeout(self.read_timeout, up.client.get_balance(addr, block)).await;

            match attempt {
                Ok(Ok(value)) => {
                    self.active.store(idx, Ordering::SeqCst);
                    let status = if offset == 0 && idx == 0 {
                        ReadStatus::Verified
                    } else {
                        ReadStatus::degraded(format!("failover→{}", up.label))
                    };
                    return Read { value: Some(value), status, served_by: Some(up.label.clone()) };
                }
                Ok(Err(e)) => {
                    tracing::warn!(upstream = %up.label, error = %e, "read failed, trying next upstream");
                }
                Err(_) => {
                    tracing::warn!(upstream = %up.label, timeout_ms = self.read_timeout.as_millis() as u64, "read timed out, trying next upstream");
                }
            }
        }

        // Every EL upstream failed. Classify the outage honestly using the
        // consensus-side observable: is the head itself stale (CL dark / frozen),
        // or are just the ELs down?
        let reason = self.classify_outage().await;
        Read { value: None, status: ReadStatus::unsynced(reason), served_by: None }
    }

    /// Current verified head (block number), with failover — the daemon's UI head.
    /// (The spike proves EL-independence with `head_of_primary` instead; this is the
    /// general failover variant the real app would use.)
    #[allow(dead_code)]
    pub async fn head(&self) -> Option<U256> {
        let n = self.upstreams.len();
        let start = self.active.load(Ordering::SeqCst);
        for offset in 0..n {
            let up = &self.upstreams[(start + offset) % n];
            if let Ok(Ok(h)) = tokio::time::timeout(self.read_timeout, up.client.get_block_number()).await {
                return Some(h);
            }
        }
        None
    }

    /// Head from the **primary** (index 0) client specifically, regardless of which
    /// upstream is currently active. After the EL cut this STILL returns — because
    /// `get_block_number` reads the CL-pushed cache, not the EL — which is the honest
    /// proof that the head is EL-independent (the dead-EL client still knows the head).
    pub async fn head_of_primary(&self) -> Option<U256> {
        let up = self.upstreams.first()?;
        tokio::time::timeout(self.read_timeout, up.client.get_block_number())
            .await
            .ok()?
            .ok()
    }

    /// Distinguish "head frozen" (CL not delivering → `syncing()` reports Info,
    /// `check_head_age` past its 60s gate) from "all ELs down but head still fresh".
    async fn classify_outage(&self) -> String {
        for up in &self.upstreams {
            if let Ok(Ok(status)) = tokio::time::timeout(self.read_timeout, up.client.syncing()).await {
                if let SyncStatus::Info(_) = status {
                    return "head frozen (consensus upstream not delivering)".to_string();
                }
            }
        }
        "all execution upstreams down".to_string()
    }
}
