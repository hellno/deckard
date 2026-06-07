//! Stand up a **verified** Helios light client whose localhost JSON-RPC server
//! is the endpoint Railgun (or a generic alloy provider) reads through.
//!
//! Verified against `a16z/helios @ 0.11.1` source:
//!   * `EthereumClientBuilder::rpc_address(SocketAddr)` records a bind addr;
//!   * on `.build()`, `HeliosClient::new` does `tokio::spawn(jsonrpc::start(inner,
//!     addr))` (held alive by a `pending()`), serving the `eth_*` subset in
//!     `core/src/jsonrpc/mod.rs` at `http://<addr>` — every read proof-checked.
//!   * `.build()` is sync but MUST run inside a tokio runtime (it spawns the
//!     server task). We are, via `#[tokio::main]`.
//!
//! `wait_synced()` ≠ ready: after it returns, the first execution head lands
//! ~1 slot later (≤12s); until then every `Latest` read fails the 60s
//! `check_head_age` gate. So we poll `get_block_number()` until `Ok`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alloy::primitives::{B256, U256};
use eyre::{eyre, Result};
use helios_ethereum::config::networks::Network;
use helios_ethereum::database::FileDB;
use helios_ethereum::{EthereumClient, EthereumClientBuilder};
use tracing::info;

/// Build a verified Helios **mainnet** client whose localhost JSON-RPC server is
/// bound at `rpc_addr`. FileDB → warm starts from a cached checkpoint.
pub fn build_with_server(
    cl: &str,
    el: &str,
    checkpoint: Option<B256>,
    data_dir: PathBuf,
    rpc_addr: SocketAddr,
) -> Result<EthereumClient> {
    let b = EthereumClientBuilder::<FileDB>::new()
        .network(Network::Mainnet)
        .consensus_rpc(cl)?
        .execution_rpc(el)?
        .data_dir(data_dir)
        // strict: refuse a too-old checkpoint (hard failure, never a silent stale read).
        .strict_checkpoint_age()
        // THE mechanism the whole v1 path rests on: spawn the localhost JSON-RPC
        // server on build() so an alloy HTTP provider can read through it.
        .rpc_address(rpc_addr);
    let b = match checkpoint {
        Some(cp) => b.checkpoint(cp),
        // No user-pinned checkpoint → community fallback (ethPandaOps). Honest
        // spike default; in Deckard this read path is labeled Degraded.
        None => b.load_external_fallback(),
    };
    b.with_file_db().build()
}

/// Block until the client serves a fresh verified head (the honest "ready to
/// serve verified reads" moment — see module docs on why `wait_synced` isn't it).
pub async fn wait_until_serving(
    client: &EthereumClient,
    timeout: Duration,
) -> Result<U256> {
    let deadline = Instant::now() + timeout;
    loop {
        match client.get_block_number().await {
            Ok(h) => return Ok(h),
            Err(e) => {
                if Instant::now() > deadline {
                    return Err(eyre!("no fresh head within {timeout:?}: {e}"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Poll the localhost JSON-RPC **server** (not the typed client) over HTTP until
/// it answers `eth_chainId` — proving the spawned `jsonrpc::start` task has bound
/// and is serving verified reads at `url`.
pub async fn wait_server_live(url: &str, timeout: Duration) -> Result<u64> {
    let http = reqwest::Client::new();
    let deadline = Instant::now() + timeout;
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]});
    loop {
        let got = async {
            let resp = http.post(url).json(&req).send().await.ok()?;
            let v: serde_json::Value = resp.json().await.ok()?;
            let hex = v.get("result")?.as_str()?;
            u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok()
        }
        .await;
        if let Some(chain_id) = got {
            return Ok(chain_id);
        }
        if Instant::now() > deadline {
            return Err(eyre!("Helios localhost server at {url} never answered eth_chainId"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Grab a free loopback port by binding an ephemeral socket and dropping it.
/// (Helios discards the jsonrpsee `ServerHandle`, so we can't recover a `:0`
/// port after the fact — pick a concrete one up front. Small TOCTOU window,
/// fine for a spike.)
pub fn free_loopback_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    drop(l);
    Ok(port)
}

#[allow(dead_code)]
pub fn log_ready(head: U256, url: &str) {
    info!(head = %head, server = %url, "Helios verified + localhost server live");
}
