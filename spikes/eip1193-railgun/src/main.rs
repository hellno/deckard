//! Deckard T-Trustless #3 spike — **Helios as Railgun's EIP-1193 provider.**
//!
//! Proves the v1 seam end to end, headless:
//!   1. Boot a verified Helios mainnet light client whose **localhost JSON-RPC
//!      server** (`EthereumClientBuilder::rpc_address`) serves proof-checked reads.
//!   2. Point an alloy HTTP provider at it, `.erased()` → `DynProvider`, and wrap
//!      it through Kohaku's **real** `IntoEip1193Provider` adapter
//!      (`eip-1193-provider` crate) — the exact trait `RailgunBuilder::new` takes.
//!   3. Drive every method Railgun's read/sync path calls THROUGH Helios:
//!        get_block_number → eth_blockNumber   (RpcSyncer.latest_block)
//!        logs             → eth_getLogs       (RpcSyncer.events, tail range)
//!        eth_call         → eth_call          (SmartWalletUtxoVerifier.verify_root)
//!      + get_chain_id (eth_chainId) for completeness.
//!   4. A logging proxy in front of Helios records the actual JSON-RPC `method`s,
//!      cross-checked against Helios's served set.
//!   5. Measure the loopback-hop overhead vs a direct typed `HeliosApi` call.
//!
//! With `--features railgun` it ALSO links Kohaku's full `railgun` crate and
//! drives the real `RailgunBuilder` + `RpcSyncer` + `SmartWalletUtxoVerifier`
//! against the live mainnet RAILGUN smart wallet through Helios (Tier-1).
//!
//! Read-only: no signing, no broadcasting, no funds.

mod helios;
mod proxy;
#[cfg(feature = "railgun")]
mod railgun_tier1;

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use alloy::eips::BlockId;
use alloy::primitives::{Address, B256, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use eip_1193_provider::provider::{Eip1193Caller, Eip1193Provider, IntoEip1193Provider};
use eyre::{eyre, Result};
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};

/// RAILGUN smart wallet on Ethereum mainnet (from Kohaku `ChainConfig::mainnet()`).
const RAILGUN_SMART_WALLET_MAINNET: &str = "0xFA7093CDD9EE6932B4eb2c9e1cde7CE00B1FA4b9";

// The one read Railgun's verifier makes through the provider: the `rootHistory`
// getter on the smart wallet (`SmartWalletUtxoVerifier::verify_root` → sol_call →
// eth_call). Same signature as Kohaku's ABI.
alloy::sol! {
    function rootHistory(uint256 treeNumber, bytes32 root) external view returns (bool);
}

struct Cfg {
    el: String,
    cl: String,
    checkpoint: Option<B256>,
    data_dir: PathBuf,
    wipe: bool,
    /// eth_getLogs window (#blocks back from head) for the demo read.
    window: u64,
    railgun_wallet: Address,
}

fn cfg() -> Result<Cfg> {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    Ok(Cfg {
        el: env("EL").unwrap_or_else(|| "https://ethereum-rpc.publicnode.com".into()),
        cl: env("CL").unwrap_or_else(|| "http://testing.mainnet.beacon-api.nimbus.team".into()),
        checkpoint: match env("CHECKPOINT") {
            Some(s) => Some(
                B256::from_str(s.trim_start_matches("0x"))
                    .or_else(|_| B256::from_str(&s))
                    .map_err(|e| eyre!("bad CHECKPOINT: {e}"))?,
            ),
            None => None,
        },
        data_dir: env("DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("deckard-eip1193-railgun-spike")),
        wipe: env("WIPE").is_some(),
        window: env("WINDOW").and_then(|s| s.parse().ok()).unwrap_or(2000),
        railgun_wallet: Address::from_str(RAILGUN_SMART_WALLET_MAINNET)?,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let cfg = cfg()?;
    if cfg.wipe {
        let _ = std::fs::remove_dir_all(&cfg.data_dir);
        info!("WIPE=1 → cleared {}", cfg.data_dir.display());
    }

    // ── Step 1: boot a verified Helios mainnet client + its localhost server ──
    let helios_port = helios::free_loopback_port()?;
    let helios_addr = format!("127.0.0.1:{helios_port}");
    let helios_url = format!("http://{helios_addr}");
    info!(server = %helios_url, el = %redact(&cfg.el), cl = %cfg.cl, "building Helios with localhost JSON-RPC server");

    let t = Instant::now();
    let client = helios::build_with_server(
        &cfg.cl,
        &cfg.el,
        cfg.checkpoint,
        cfg.data_dir.clone(),
        helios_addr.parse()?,
    )?;
    client.wait_synced().await?;
    let head = helios::wait_until_serving(&client, Duration::from_secs(60)).await?;
    let sync_secs = t.elapsed().as_secs_f64();
    let chain_id = helios::wait_server_live(&helios_url, Duration::from_secs(30)).await?;
    info!(sync_secs = format!("{sync_secs:.1}"), head = %head, chain_id, "Helios verified + localhost server live");

    // ── Step 2: logging proxy in front of Helios's server (Task 4 tap) ───────
    let proxy_port = helios::free_loopback_port()?;
    let proxy_addr = format!("127.0.0.1:{proxy_port}");
    let proxy_url = format!("http://{proxy_addr}");
    let method_log = proxy::spawn(&proxy_addr, helios_url.clone()).await?;
    info!(proxy = %proxy_url, "→ Helios; logging every JSON-RPC method");

    // ── Step 3: alloy provider → Kohaku's REAL IntoEip1193Provider adapter ───
    // This is the exact v1 wiring `RailgunBuilder::new(chain, provider)` accepts:
    // `ProviderBuilder::new().connect(url).erased()` is a `DynProvider`, and
    // `impl IntoEip1193Provider for DynProvider` ships in the eip-1193-provider crate.
    //
    // ⚠ THE ONE REQUIRED v1 FIX (spike finding): alloy's `Provider::call` defaults
    // to the `pending` block tag (alloy-provider 1.8.3 trait.rs:198), and Kohaku's
    // `Alloy::eth_call` adapter calls `inner.call(req)` with no block override. Helios
    // is a light client with NO pending block → `eth_call(pending)` fails with
    // "block not found: pending". Since the read/sync path's `verify_root` → eth_call
    // rides this, v1 needs eth_call pinned to `latest`. `.with_default_block(latest)`
    // installs alloy's `BlockIdLayer`, which rewrites the default block for
    // eth_call/estimateGas/etc to `latest` — making Kohaku's UNMODIFIED adapter work
    // against Helios. One line, Deckard-side, no Kohaku/Helios patch.
    let provider_proxied: DynProvider = ProviderBuilder::new()
        .with_default_block(BlockId::latest())
        .connect(&proxy_url)
        .await?
        .erased();
    let eip: Arc<dyn Eip1193Provider> = provider_proxied.into_eip1193();
    info!("wrapped alloy DynProvider via Kohaku IntoEip1193Provider::into_eip1193()");

    // ── Step 4: drive the 3 read-path methods THROUGH Helios via the adapter ─
    // (a) eth_chainId
    let got_chain = eip.get_chain_id().await.map_err(|e| eyre!("get_chain_id: {e}"))?;
    // (b) eth_blockNumber — Railgun's RpcSyncer.latest_block
    let got_head = eip.get_block_number().await.map_err(|e| eyre!("get_block_number: {e}"))?;
    // (c) eth_getLogs — Railgun's RpcSyncer.events (tail range; Subsquid carries history)
    let from = got_head.saturating_sub(cfg.window);
    let logs = eip
        .logs(cfg.railgun_wallet, None, Some(from), Some(got_head))
        .await
        .map_err(|e| eyre!("logs: {e}"))?;
    // (d) eth_call — Railgun's SmartWalletUtxoVerifier.verify_root (rootHistory getter).
    //     ZERO root has never been "seen" → returns false. The point is the eth_call
    //     resolves THROUGH Helios and decodes to sane data.
    let seen: bool = eip
        .sol_call(cfg.railgun_wallet, rootHistoryCall { treeNumber: U256::ZERO, root: B256::ZERO })
        .await
        .map_err(|e| eyre!("eth_call rootHistory: {e}"))?;

    info!(
        chain_id = got_chain,
        head = %got_head,
        logs_in_window = logs.len(),
        window = cfg.window,
        root_history_zero = seen,
        "STEP 4 — all read-path methods resolved through Helios"
    );

    // ── Step 5: loopback-hop overhead (Task 5) ───────────────────────────────
    // direct  = typed in-process HeliosApi call (0 hops, served from CL cache)
    // loopback= alloy provider → Helios localhost server (1 hop), no proxy in path
    let provider_direct: DynProvider = ProviderBuilder::new()
        .with_default_block(BlockId::latest())
        .connect(&helios_url)
        .await?
        .erased();
    let eip_direct: Arc<dyn Eip1193Provider> = provider_direct.into_eip1193();
    let (direct_us, loop_us) = measure_overhead(&client, &eip_direct, 25).await?;

    // ── Step 6 (Tier-1, --features railgun): real RailgunBuilder path ────────
    #[cfg(feature = "railgun")]
    let tier1 = railgun_tier1::run(&proxy_url, got_head, cfg.window, &method_log).await?;

    // ── Report ────────────────────────────────────────────────────────────────
    let served = method_log.snapshot();
    println!("\n──────────── Deckard T-Trustless #3 — Helios⇄Railgun EIP-1193 seam ────────────");
    println!("  Helios sync (build→servable) : {sync_secs:.1}s   chain_id={chain_id}   head={head}");
    println!("  v1 wiring                    : alloy DynProvider → Kohaku IntoEip1193Provider::into_eip1193()  ✅ (no custom adapter)");
    println!("  required v1 fix              : ProviderBuilder::with_default_block(latest) — alloy's call() defaults to `pending`,");
    println!("                                 which Helios (light client) can't serve; the BlockIdLayer pins eth_call→latest. 1 line, Deckard-side.");
    println!("  reads through Helios         : eth_chainId={got_chain}  eth_blockNumber={got_head}");
    println!("                                 eth_getLogs([head-{}, head] on RAILGUN wallet) → {} logs", cfg.window, logs.len());
    println!("                                 eth_call rootHistory(0,0x00..) → {seen}");
    println!("  loopback-hop overhead        : direct(typed)≈{direct_us}µs/call  loopback(alloy→Helios)≈{loop_us}µs/call  Δ≈{}µs", loop_us.saturating_sub(direct_us));
    println!("  JSON-RPC methods seen (proxy):");
    for (m, c) in &served {
        let served_by_helios = HELIOS_SERVED.contains(&m.as_str());
        println!("       {:<26} ×{:<4} {}", m, c, if served_by_helios { "served-by-Helios ✅" } else { "⚠ NOT in Helios served set" });
    }
    #[cfg(feature = "railgun")]
    {
        println!("  Tier-1 (real railgun crate)  : {}", tier1.summary);
    }
    #[cfg(not(feature = "railgun"))]
    {
        println!("  Tier-1 (real railgun crate)  : not linked (run with --features railgun)");
    }
    let all_served = served.iter().all(|(m, _)| HELIOS_SERVED.contains(&m.as_str()));
    println!("  VERDICT                      : {}", if all_served {
        "PASS ✅ — v1 localhost path WORKS: Helios serves every method the read path called"
    } else {
        "⚠ a called method is NOT in Helios's served set — see flags above"
    });
    println!("────────────────────────────────────────────────────────────────────────────────\n");

    if all_served { Ok(()) } else { Err(eyre!("a called method is not served by Helios")) }
}

/// Methods Helios 0.11.1's localhost server serves (eth namespace, from
/// `core/src/jsonrpc/mod.rs`). Used to flag any method the read path calls that
/// Helios does NOT serve.
const HELIOS_SERVED: &[&str] = &[
    "eth_chainId", "eth_blockNumber", "eth_getLogs", "eth_call", "eth_estimateGas",
    "eth_gasPrice", "eth_getTransactionCount", "eth_getBalance", "eth_getCode",
    "eth_getProof", "eth_getStorageAt", "eth_getBlockByNumber", "eth_getBlockByHash",
    "eth_getTransactionReceipt", "eth_getBlockReceipts", "eth_getTransactionByHash",
    "eth_sendRawTransaction", "eth_maxPriorityFeePerGas", "eth_syncing",
    "eth_createAccessList", "eth_newFilter", "eth_getFilterChanges", "eth_getFilterLogs",
    "net_version", "web3_clientVersion",
];

/// Median per-call latency (µs) of a direct typed `HeliosApi` head read vs the
/// same read over the alloy provider → Helios localhost loopback. Rough is fine.
async fn measure_overhead(
    client: &helios_ethereum::EthereumClient,
    eip_direct: &Arc<dyn Eip1193Provider>,
    n: usize,
) -> Result<(u128, u128)> {
    let median = |mut v: Vec<u128>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let mut direct = Vec::with_capacity(n);
    let mut loopback = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let _ = client.get_block_number().await?;
        direct.push(t.elapsed().as_micros());

        let t = Instant::now();
        let _ = eip_direct.get_block_number().await.map_err(|e| eyre!("loopback head: {e}"))?;
        loopback.push(t.elapsed().as_micros());
    }
    Ok((median(direct), median(loopback)))
}

/// Hide API keys in logs (path segments after the host).
fn redact(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}/…")
        }
        None => url.to_string(),
    }
}
