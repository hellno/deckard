//! Deckard R2 spike — the Helios walkaway beat, end to end on mainnet.
//!
//! What it proves, headless:
//!   1. Embed Helios as a library, sync a verified mainnet client, serve a
//!      verified `get_balance` (the deposit contract by default).
//!   2. WALKAWAY: cut the primary (centralized) EL RPC on camera and keep serving
//!      a *verified* balance by failing over to an independent second EL — the
//!      head never freezes (it's consensus-driven), only state reads fail and
//!      recover. ReadStatus transitions Verified → Degraded{failover}.
//!   3. Measure: cold vs warm sync time, and cut→failover latency.
//!
//! Run (env-configurable):
//!   EL1=<primary EL, the one we "cut">  EL2=<independent failover EL>
//!   CL=<beacon light-client API>        [CHECKPOINT=<0x.. B256>]  [WIPE=1 for cold]
//!   cargo run --release
//!
//! Defaults wire EL2/CL to verified-live public endpoints; pass EL1 (e.g. your
//! Alchemy key) via env. See README.md.

mod proxy;
mod read_status;
mod upstreams;

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use alloy::eips::BlockNumberOrTag;
use alloy::primitives::{utils::format_ether, Address, B256, U256};
use eyre::{eyre, Result};
use helios_ethereum::config::networks::Network;
use helios_ethereum::database::FileDB;
use helios_ethereum::{EthereumClient, EthereumClientBuilder};
use tracing::info;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};

use read_status::ReadStatus;
use upstreams::{Upstream, Upstreams};

struct Cfg {
    el1: String,
    el2: String,
    cl: String,
    checkpoint: Option<B256>,
    data_dir: PathBuf,
    addr: Address,
    wipe: bool,
    proxy_bind: String,
}

fn cfg() -> Result<Cfg> {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    Ok(Cfg {
        // EL1 is proxied + cuttable. Pass your own (e.g. Alchemy) for the hero.
        el1: env("EL1").unwrap_or_else(|| "https://ethereum-rpc.publicnode.com".into()),
        // EL2 is the independent failover EL, wired straight through.
        el2: env("EL2").unwrap_or_else(|| "https://eth.drpc.org".into()),
        // Beacon light-client API. Verified to actually drive a Helios sync: Nimbus-testing
        // (default) + dRPC (eth-beacon-chain.drpc.org). Lodestar/PublicNode return 200 but
        // fail Helios sync — don't use them as CL.
        cl: env("CL").unwrap_or_else(|| "http://testing.mainnet.beacon-api.nimbus.team".into()),
        checkpoint: match env("CHECKPOINT") {
            Some(s) => Some(B256::from_str(s.trim_start_matches("0x").trim_start_matches("0X"))
                .or_else(|_| B256::from_str(&s))
                .map_err(|e| eyre!("bad CHECKPOINT: {e}"))?),
            None => None,
        },
        data_dir: env("DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("deckard-helios-spike")),
        // Deposit contract — a large, stable, well-known balance.
        addr: Address::from_str(
            &env("ADDR").unwrap_or_else(|| "0x00000000219ab540356cBB839Cbe05303d7705Fa".into()),
        )?,
        wipe: env("WIPE").is_some(),
        proxy_bind: env("PROXY_BIND").unwrap_or_else(|| "127.0.0.1:18545".into()),
    })
}

/// `wait_synced()` only blocks until the **consensus** checkpoint is bootstrapped;
/// the latest **execution** head isn't pushed into the cache until the next
/// optimistic update (~one slot, ≤12s). Until then `get_block_number` fails the 60s
/// head-age gate. Poll until a fresh head is actually servable — this is the honest
/// "ready to serve verified reads" moment.
async fn wait_until_serving(client: &EthereumClient, label: &str, timeout: Duration) -> Result<U256> {
    let deadline = Instant::now() + timeout;
    loop {
        match client.get_block_number().await {
            Ok(h) => return Ok(h),
            Err(e) => {
                if Instant::now() > deadline {
                    return Err(eyre!("{label}: no fresh head within {timeout:?}: {e}"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

/// Build a verified Helios mainnet client (FileDB → warm starts via cached checkpoint).
fn build_client(cl: &str, el: &str, checkpoint: Option<B256>, data_dir: PathBuf) -> Result<EthereumClient> {
    let b = EthereumClientBuilder::<FileDB>::new()
        .network(Network::Mainnet)
        .consensus_rpc(cl)?
        .execution_rpc(el)?
        .data_dir(data_dir)
        // strict: refuse a too-old checkpoint instead of warning → surfaces as a
        // hard failure rather than a silent stale read (demo-honest).
        .strict_checkpoint_age();
    let b = match checkpoint {
        Some(cp) => b.checkpoint(cp),
        // No user-pinned checkpoint → community fallback (ethPandaOps). Honest spike
        // default; in Deckard this read path is labeled Degraded (untrusted source).
        None => b.load_external_fallback(),
    };
    b.with_file_db().build()
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

    // SCENARIO=lie → the integrity demo (a malicious RPC, Deckard refuses the lie).
    // default → the walkaway/availability demo (cut the RPC, keep serving verified).
    if std::env::var("SCENARIO").ok().as_deref() == Some("lie") {
        if cfg.wipe {
            let _ = std::fs::remove_dir_all(&cfg.data_dir);
        }
        return scenario_lie(&cfg).await;
    }

    let primary_dir = cfg.data_dir.join("primary");
    let secondary_dir = cfg.data_dir.join("secondary");

    if cfg.wipe {
        let _ = std::fs::remove_dir_all(&cfg.data_dir);
        info!("WIPE=1 → cleared {} (forcing COLD start)", cfg.data_dir.display());
    }
    let warm = primary_dir.join("checkpoint").exists();
    info!(start = if warm { "WARM (cached checkpoint present)" } else { "COLD (no cached checkpoint)" }, "");

    // ── Step 0: stand up the killable proxy in front of EL1 ───────────────────
    let proxied_el1 = format!("http://{}", cfg.proxy_bind);
    let kill = proxy::spawn(&cfg.proxy_bind, cfg.el1.clone(), false).await?;
    info!(primary_el = %redact(&cfg.el1), via = %proxied_el1, secondary_el = %redact(&cfg.el2), cl = %cfg.cl, "upstreams");

    // ── Step 1: build + sync the primary (proxied EL1), measure sync time ─────
    let t = Instant::now();
    let primary = build_client(&cfg.cl, &proxied_el1, cfg.checkpoint, primary_dir.clone())?;
    primary.wait_synced().await?;
    let head0 = wait_until_serving(&primary, "primary", Duration::from_secs(45)).await?;
    let sync_secs = t.elapsed().as_secs_f64(); // build → first verified head servable
    info!(
        sync_secs = format!("{sync_secs:.1}"),
        kind = if warm { "warm" } else { "cold" },
        head = %head0,
        "primary ready (verified head servable)"
    );

    // Secondary on the independent EL2 (wired straight through, not proxied).
    let secondary = build_client(&cfg.cl, &cfg.el2, cfg.checkpoint, secondary_dir.clone())?;
    secondary.wait_synced().await?;
    wait_until_serving(&secondary, "secondary", Duration::from_secs(45)).await?;
    info!("secondary ready");

    let sup = Upstreams::new(
        vec![
            Upstream { label: "EL1(primary)".into(), client: primary },
            Upstream { label: "EL2(failover)".into(), client: secondary },
        ],
        Duration::from_secs(8),
    );

    // ── Step 2: verified read from the primary ────────────────────────────────
    let r = sup.get_balance(cfg.addr).await;
    let pre = r.value.ok_or_else(|| eyre!("no verified balance before cut"))?;
    assert_eq!(r.status, ReadStatus::Verified, "expected Verified before the cut");
    info!(
        addr = %cfg.addr,
        balance_eth = %format_ether(pre),
        status = %r.status,
        served_by = %r.served_by.clone().unwrap_or_default(),
        "STEP 2 — verified balance (primary)"
    );

    // ── Step 3: WALKAWAY — cut EL1, keep serving verified reads via EL2 ───────
    info!("STEP 3 — ✂️  CUTTING primary EL RPC (on camera)…");
    let cut_at = Instant::now();
    kill.cut();

    // Head keeps advancing (consensus-driven, EL-independent): prove liveness.
    // State reads fail on EL1 and recover on EL2.
    let deadline = Instant::now() + Duration::from_secs(30);
    let (failover_latency, post, final_status) = loop {
        let r = sup.get_balance(cfg.addr).await;
        match (&r.status, r.value) {
            (ReadStatus::Degraded { .. }, Some(v)) => {
                let latency = cut_at.elapsed();
                info!(
                    failover_ms = latency.as_millis() as u64,
                    served_by = %r.served_by.clone().unwrap_or_default(),
                    status = %r.status,
                    balance_eth = %format_ether(v),
                    "STEP 3 — failover read is VERIFIED (recovered after the cut)"
                );
                break (latency, v, r.status);
            }
            (status, _) => {
                info!(status = %status, "…primary down, failing over");
            }
        }
        if Instant::now() > deadline {
            return Err(eyre!("failover did not produce a verified read within 30s"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };

    // Head still live after the cut — proves it's consensus-driven, not EL-derived.
    // Read it from the PRIMARY (whose EL we just cut): get_block_number returns from
    // the CL-pushed cache, so the dead-EL client still serves a fresh head.
    let head1 = sup.head_of_primary().await.unwrap_or(head0);

    // ── Summary ───────────────────────────────────────────────────────────────
    println!("\n──────────────── Deckard R2 walkaway — RESULT ────────────────");
    println!("  start mode          : {}", if warm { "WARM (cached checkpoint)" } else { "COLD (fresh checkpoint)" });
    println!("  primary sync time   : {sync_secs:.1}s");
    println!("  pre-cut  balance    : {} ETH  [{}]", format_ether(pre), ReadStatus::Verified);
    println!("  cut→failover latency: {} ms", failover_latency.as_millis());
    println!("  post-cut balance    : {} ETH  [{}]", format_ether(post), final_status);
    println!("  head at sync        : {head0}");
    println!("  primary head post-cut: {head1}  ({} — the EL1-cut client still serves the head from CL cache)",
        if head1 >= head0 { "EL-independent ✓" } else { "?" });
    let drift = if post >= pre { post - pre } else { pre - post };
    let ok = matches!(final_status, ReadStatus::Degraded { .. }) && post > U256::ZERO;
    println!("  balance drift       : {} wei (deposits between blocks; verified either way)", drift);
    println!("  VERDICT             : {}", if ok { "PASS ✅  cut the RPC, still serving verified reads" } else { "FAIL ❌" });
    println!("──────────────────────────────────────────────────────────────\n");

    if ok { Ok(()) } else { Err(eyre!("walkaway scenario failed")) }
}

/// The integrity demo: point Helios at a MALICIOUS RPC (a proxy that rewrites the
/// balance in every `eth_getProof`) and show Deckard **refuses the lie** — because
/// it verifies the proof against the CL-signed state root, it never returns the fake
/// number. A centralized wallet would just display it. This is the real moat;
/// "cut the cable" (the default scenario) is the availability sibling of this.
async fn scenario_lie(cfg: &Cfg) -> Result<()> {
    let dir = cfg.data_dir.join("liar");
    let bind = "127.0.0.1:18547";
    let lying_el = format!("http://{bind}");

    // The proxy forwards to a REAL EL but tampers eth_getProof balances (lie = true).
    let _kill = proxy::spawn(bind, cfg.el1.clone(), true).await?;
    info!(real_el = %redact(&cfg.el1), via_lying_proxy = %lying_el, cl = %cfg.cl, "MALICIOUS-RPC scenario");

    let client = build_client(&cfg.cl, &lying_el, cfg.checkpoint, dir)?;
    client.wait_synced().await?;
    let head = wait_until_serving(&client, "client", Duration::from_secs(45)).await?;
    info!(head = %head, "synced + head servable THROUGH the lying RPC (head is CL-verified, not from the EL)");

    // What the malicious RPC claims (ask it directly).
    let claimed = raw_get_proof_balance(&lying_el, cfg.addr)
        .await
        .unwrap_or_else(|| "<unknown>".into());

    // What Deckard does: verify the proof → reject the tampered account.
    // Tighten the assertion: only a PROOF rejection counts as a pass. An unrelated
    // transport/sync error must NOT be mistaken for "caught the lie."
    let (verdict, rejected) = match client.get_balance(cfg.addr, BlockNumberOrTag::Latest.into()).await {
        Ok(v) => (format!("LEAKED {} ETH — verification FAILED to catch the lie!", format_ether(v)), false),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            let is_proof_rejection = msg.contains("proof");
            if is_proof_rejection {
                (format!("REJECTED — {e}"), true)
            } else {
                (format!("errored for an UNRELATED reason (not a proof rejection): {e}"), false)
            }
        }
    };

    println!("\n──────────── Deckard: malicious-RPC detection (integrity) ────────────");
    println!("  address             : {}", cfg.addr);
    println!("  malicious RPC claims : {claimed} ETH   (balance rewritten in eth_getProof; proof left intact)");
    println!("  Deckard get_balance  : {verdict}");
    println!("  a centralized wallet : would display {claimed} ETH (no proof to check — trusts the RPC)");
    println!("  VERDICT              : {}", if rejected { "PASS ✅ — Deckard refuses to be lied to" } else { "FAIL ❌" });
    println!("──────────────────────────────────────────────────────────────────────\n");

    if rejected { Ok(()) } else { Err(eyre!("Deckard did not reject the lie")) }
}

/// Ask an RPC directly for an address's balance via `eth_getProof` (used to show
/// what the malicious proxy claims, for contrast with what Deckard accepts).
async fn raw_get_proof_balance(url: &str, addr: Address) -> Option<String> {
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_getProof",
        "params": [addr.to_string(), [], "latest"],
    });
    let resp = reqwest::Client::new().post(url).json(&req).send().await.ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let hex = v.get("result")?.get("balance")?.as_str()?;
    let wei = U256::from_str_radix(hex.trim_start_matches("0x"), 16).ok()?;
    Some(format_ether(wei).to_string())
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
