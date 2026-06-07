//! Tier-1 (`--features railgun`): link Kohaku's **full `railgun` crate** and drive
//! its real read path against the live mainnet RAILGUN smart wallet THROUGH Helios.
//!
//! Proves, beyond the Tier-2 adapter check:
//!   * `RailgunBuilder::new(ChainConfig::mainnet(), provider)` accepts the
//!     Helios-backed alloy `DynProvider` and `.build()` links + constructs a real
//!     `RailgunProvider` (the whole ZK crate compiles into our dependency edge).
//!   * Railgun's real `RpcSyncer` (`UtxoSyncer::sync`) drives `eth_blockNumber` +
//!     `eth_getLogs` through Helios over a bounded tail window — the exact code the
//!     production sync path runs (Subsquid carries history; only the tail hits Helios).
//!
//! Read-only: no register/full-sync (that would scan from the 2022 deployment
//! block), no proving, no broadcast.

use std::sync::Arc;
use std::time::Duration;

use alloy::eips::BlockId;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use eyre::{eyre, Result};
use railgun::builder::RailgunBuilder;
use railgun::chain_config::ChainConfig;
use railgun::indexer::syncer::{RpcSyncer, UtxoSyncer};
use tracing::info;

use crate::proxy::MethodLog;

pub struct Tier1Out {
    pub summary: String,
}

pub async fn run(proxy_url: &str, head: u64, window: u64, _log: &MethodLog) -> Result<Tier1Out> {
    let chain = ChainConfig::mainnet();
    let wallet = chain.railgun_smart_wallet;

    // Fresh Helios-backed providers (DynProvider is Clone, but build fresh to keep
    // each consumer independent). `.with_default_block(latest)` is the same v1 fix as
    // Tier-2: it pins the adapter's eth_call (verify_root) to `latest` so it never
    // hits Helios's absent `pending` block.
    let p_build: DynProvider = ProviderBuilder::new()
        .with_default_block(BlockId::latest())
        .connect(proxy_url)
        .await?
        .erased();
    let p_sync: DynProvider = ProviderBuilder::new()
        .with_default_block(BlockId::latest())
        .connect(proxy_url)
        .await?
        .erased();

    // (1) Builder accepts the Helios-backed provider + the crate links. RPC-only
    // syncer so build()/usage never depends on Subsquid. build() is network-free
    // (MemoryDatabase), so this just proves construction + linkage.
    let rpc_syncer_for_builder = Arc::new(RpcSyncer::new(chain.clone(), p_build.clone()));
    let _railgun = RailgunBuilder::new(chain.clone(), p_build)
        .with_utxo_syncer(rpc_syncer_for_builder)
        .build()
        .await
        .map_err(|e| eyre!("RailgunBuilder::build: {e}"))?;
    info!("Tier-1: RailgunBuilder::new(ChainConfig::mainnet(), <Helios DynProvider>).build() OK");

    // (2) Drive the REAL RpcSyncer over a bounded tail window → eth_blockNumber
    // (latest_block) + eth_getLogs (events) through Helios. One getLogs call.
    let syncer = RpcSyncer::new(chain.clone(), p_sync)
        .with_batch_size(window.max(1))
        .with_batch_delay(Duration::ZERO);
    let from = head.saturating_sub(window);
    let events = UtxoSyncer::sync(&syncer, from, head)
        .await
        .map_err(|e| eyre!("RpcSyncer::sync: {e}"))?;
    info!(from, to = head, events = events.len(), "Tier-1: real RpcSyncer drove eth_getLogs through Helios");

    Ok(Tier1Out {
        summary: format!(
            "RailgunBuilder::build() OK; real RpcSyncer synced [{from},{head}] on {wallet} → {} SyncEvents via Helios",
            events.len()
        ),
    })
}
