//! Stand up an embedded **verified** Helios light client whose localhost JSON-RPC
//! server is the endpoint an alloy provider reads through — so every chain read is
//! proof-checked instead of trusting a raw vendor RPC.
//!
//! Lifted from the verified `eip1193-railgun` spike (`spikes/eip1193-railgun/src/helios.rs`),
//! ported from `eyre` → `anyhow` and trimmed to the helios-only consumer Deckard needs.
//!
//! Verified against `a16z/helios @ 0.11.1`:
//!   * `EthereumClientBuilder::rpc_address(SocketAddr)` records a bind addr;
//!   * on `.build()`, the client `tokio::spawn`s the localhost JSON-RPC server,
//!     serving the `eth_*` subset — every read proof-checked.
//!   * `.build()` is sync but MUST run inside a tokio runtime (it spawns the server
//!     task). Both callers (deckard-core's EthProvider worker and deckard-signerd's
//!     daemon) own a tokio runtime, so this holds.
//!
//! `wait_synced()` ≠ ready: after it returns, the first execution head lands ~1 slot
//! later (≤12s); until then every `Latest` read fails the 60s `check_head_age` gate.
//! So we poll `get_block_number()` until `Ok` before declaring the read path live.
//!
//! THE one-line consumer fix (see [`connect_verified_provider`]): build the alloy
//! provider with `.with_default_block(BlockId::latest())`. alloy's `Provider::call`
//! defaults the block tag to `pending`, which a light client cannot serve
//! ("block not found: pending") — this rewrites the default to `latest` so the
//! Multicall3 / ENS `eth_call` reads work.
//!
//! TODO(post-v1): v1 runs an INDEPENDENT Helios instance per reader (one behind
//! deckard-core::EthProvider, one in the daemon). The "consolidate all reads into
//! the daemon" refactor is deferred. The failover/community-checkpoint supervisor
//! (spikes/helios-walkaway/src/upstreams.rs) that would emit `ReadStatus::Degraded`
//! is also deferred — v1 runs a single client per reader.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alloy::eips::BlockId;
use alloy::primitives::U256;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use anyhow::{anyhow, Result};
use deckard_contract::ReadStatus;
use helios_ethereum::config::networks::Network;
use helios_ethereum::database::FileDB;
use helios_ethereum::{EthereumClient, EthereumClientBuilder};

/// A consensus-layer (beacon) endpoint that actually drives a Helios sync. Nimbus's
/// public testing beacon API is the spike's proven default; dRPC
/// (`https://eth-beacon-chain.drpc.org`) is the documented alternate.
pub const DEFAULT_CONSENSUS_RPC: &str = "http://testing.mainnet.beacon-api.nimbus.team";

/// A live, verified Helios read path: the localhost provider an alloy consumer reads
/// through, plus the owning `EthereumClient` whose Drop tears down the spawned server.
///
/// **The `_client` field is load-bearing**: dropping it kills the spawned localhost
/// JSON-RPC server task. Keep this struct alive for as long as reads are served.
pub struct VerifiedReader {
    /// The localhost provider, already built with the `with_default_block(latest)` fix.
    provider: DynProvider,
    /// The `http://127.0.0.1:<port>` URL the localhost JSON-RPC server is bound at — so a
    /// caller (e.g. the daemon's `signing::read_balance`) can build its OWN consumer
    /// provider against the same verified server.
    localhost_url: String,
    /// Owns the spawned localhost JSON-RPC server task; must outlive `provider`.
    _client: EthereumClient,
}

impl VerifiedReader {
    /// Borrow the verified localhost provider (alloy, `with_default_block(latest)`).
    pub fn provider(&self) -> &DynProvider {
        &self.provider
    }

    /// The verified localhost JSON-RPC URL (`http://127.0.0.1:<port>`). Reads through
    /// this are proof-checked by Helios.
    pub fn localhost_url(&self) -> &str {
        &self.localhost_url
    }

    /// Compute the trust label for a read taken *now*: `Verified` only when the Helios
    /// head is fresh (age ≤ 60s), else `Unsynced`. v1 never emits `Degraded` here — that
    /// is the deferred failover/community-checkpoint path (see the module TODO).
    ///
    /// Called once per read so a head that goes stale mid-session is caught: a value is
    /// only ever labelled `Verified` when a fresh verified head actually backs it.
    ///
    /// We fetch the *latest block by tag* (not a bare `eth_blockNumber`): fetching the
    /// `Latest` block exercises Helios's own `check_head_age` (60s) gate AND lets us derive
    /// freshness from the block's timestamp directly, rather than trusting that a stored
    /// height implies a fresh head. A bare height call can be answered by a stalled-but-
    /// not-yet-expired client and would over-report `Verified`; the timestamp check closes
    /// that gap.
    pub async fn head_status(&self) -> ReadStatus {
        let block = match self.provider.get_block(BlockId::latest()).await {
            Ok(Some(b)) => b,
            // No latest block: either still syncing or the head aged out of the gate.
            Ok(None) => return ReadStatus::unsynced("helios head unavailable: no latest block"),
            Err(e) => {
                return ReadStatus::unsynced(format!("helios head unavailable: {}", one_line(&e)))
            }
        };

        let head_ts = block.header.timestamp;
        let now = now_unix();
        // `now` can legitimately be < head_ts by a few seconds (clock skew / a head minted
        // slightly ahead); saturating_sub treats that as age 0, never as a stale read.
        let age = now.saturating_sub(head_ts);
        if age <= MAX_HEAD_AGE_SECS {
            ReadStatus::Verified
        } else {
            ReadStatus::unsynced(format!("helios head stale ({age}s > {MAX_HEAD_AGE_SECS}s)"))
        }
    }
}

/// Helios's own hard freshness gate for `Latest` reads (`check_head_age`, 60s). We mirror
/// it here so a value is labelled `Verified` only when its backing head is within the gate.
const MAX_HEAD_AGE_SECS: u64 = 60;

/// Current wall-clock UNIX time in seconds. Used only to compare against the verified
/// head's block timestamp for the freshness label.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a verified Helios **mainnet** client, launch its localhost JSON-RPC server,
/// and return a [`VerifiedReader`] only once the server is actually serving a fresh
/// verified head.
///
/// * `consensus_rpc` — the beacon (CL) endpoint that drives the sync (e.g. Nimbus).
/// * `execution_rpc` — the EL endpoint Helios proves against (must serve `eth_getProof`).
///   This is the *untrusted* RPC the app was previously reading directly — now it only
///   feeds proofs that Helios verifies.
/// * `data_dir`      — FileDB cache dir → warm starts from a cached checkpoint.
///
/// On any failure (sync timeout, server never binds, head never fresh) returns an
/// `Err` — the caller MUST then serve reads tagged `Unsynced`, NEVER fall back to a
/// raw RPC and call it `Verified`.
pub async fn launch_verified(
    consensus_rpc: &str,
    execution_rpc: &str,
    data_dir: PathBuf,
) -> Result<VerifiedReader> {
    let port = free_loopback_port()?;
    let rpc_addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let helios_url = format!("http://127.0.0.1:{port}");

    let client = build_with_server(consensus_rpc, execution_rpc, data_dir, rpc_addr)?;

    // CL checkpoint bootstrapped...
    client
        .wait_synced()
        .await
        .map_err(|e| anyhow!("helios wait_synced: {e}"))?;
    // ...then the typed client serves a fresh execution head (the honest "ready" moment;
    // wait_synced alone isn't it — the first head lands ~1 slot later).
    wait_until_serving(&client, Duration::from_secs(60)).await?;

    // Build the CONSUMER provider against the localhost server, with THE fix.
    let provider = connect_verified_provider(&helios_url).await?;

    // Prove the spawned localhost server is actually answering (and the consumer
    // provider's default-block layer works) before we declare the path live.
    wait_provider_live(&provider, Duration::from_secs(30)).await?;

    Ok(VerifiedReader {
        provider,
        localhost_url: helios_url,
        _client: client,
    })
}

/// Build a verified Helios mainnet client whose localhost JSON-RPC server is bound at
/// `rpc_addr`. FileDB → warm starts from a cached checkpoint. Sync, but must run inside
/// a tokio runtime (it spawns the server task on `.build()`).
fn build_with_server(
    cl: &str,
    el: &str,
    data_dir: PathBuf,
    rpc_addr: SocketAddr,
) -> Result<EthereumClient> {
    EthereumClientBuilder::<FileDB>::new()
        .network(Network::Mainnet)
        .consensus_rpc(cl)
        .map_err(|e| anyhow!("helios consensus_rpc {cl:?}: {e}"))?
        .execution_rpc(el)
        .map_err(|e| anyhow!("helios execution_rpc: {e}"))?
        .data_dir(data_dir)
        // strict: refuse a too-old checkpoint (hard failure, never a silent stale read).
        .strict_checkpoint_age()
        // No user-pinned checkpoint → community fallback (ethPandaOps). v1 labels reads
        // off this path Verified-by-freshness; the Degraded community-checkpoint
        // distinction is a deferred supervisor concern (see module TODO).
        .load_external_fallback()
        // THE mechanism the whole verified path rests on: spawn the localhost JSON-RPC
        // server on build() so an alloy HTTP provider can read through it.
        .rpc_address(rpc_addr)
        .with_file_db()
        .build()
        .map_err(|e| anyhow!("helios build: {e}"))
}

/// Build the **consumer** alloy provider that reads through Helios's localhost server.
///
/// THE one-line fix: `.with_default_block(BlockId::latest())`. alloy's `Provider::call`
/// defaults the block tag to `pending`; Helios (a light client) has no pending block and
/// 404s on it ("block not found: pending"). This rewrites the default to `latest` so the
/// Multicall3 `aggregate3` (portfolio) and ENS `eth_call` reads succeed. Applied
/// uniformly so every read path is identical; plain `get_balance`/`get_block_number`
/// reads are unaffected but harmless to layer.
async fn connect_verified_provider(helios_url: &str) -> Result<DynProvider> {
    let url = helios_url
        .parse()
        .map_err(|e| anyhow!("bad helios url {helios_url:?}: {e}"))?;
    Ok(ProviderBuilder::new()
        .with_default_block(BlockId::latest())
        .connect_http(url)
        .erased())
}

/// Block until the typed client serves a fresh verified head (the honest "ready to serve
/// verified reads" moment — `wait_synced` returning is NOT it).
async fn wait_until_serving(client: &EthereumClient, timeout: Duration) -> Result<U256> {
    let deadline = Instant::now() + timeout;
    loop {
        match client.get_block_number().await {
            Ok(h) => return Ok(h),
            Err(e) => {
                if Instant::now() > deadline {
                    return Err(anyhow!("helios: no fresh head within {timeout:?}: {e}"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Poll the CONSUMER alloy provider (built against the localhost server) until it answers
/// `eth_blockNumber` — proving the spawned `jsonrpc::start` task has bound AND that the
/// consumer provider talks to it. Uses the alloy provider directly so we don't pull in
/// `reqwest`/`serde_json` just for a liveness probe (the spike used reqwest).
async fn wait_provider_live(provider: &DynProvider, timeout: Duration) -> Result<u64> {
    let deadline = Instant::now() + timeout;
    loop {
        match provider.get_block_number().await {
            Ok(n) => return Ok(n),
            Err(e) => {
                if Instant::now() > deadline {
                    return Err(anyhow!(
                        "helios localhost server never answered via the consumer provider: {e}"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

/// Grab a free loopback port by binding an ephemeral socket and dropping it. Helios
/// discards the jsonrpsee `ServerHandle`, so a `:0` port can't be recovered after the
/// fact — pick a concrete one up front. (Small TOCTOU window; acceptable.)
fn free_loopback_port() -> Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    drop(l);
    Ok(port)
}

/// Collapse a multi-line error into one short line for a `reason` string.
fn one_line(e: &impl std::fmt::Display) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}
