//! The Ethereum provider — a single `alloy` JSON-RPC client living on a dedicated
//! background thread that owns a current-thread tokio runtime. The GUI sends typed
//! requests in and gets a `flume::Receiver` back; it awaits that receiver on its own
//! executor (`cx.spawn`), so a slow RPC never stalls a frame.
//!
//! ## Verified reads (the `verified-reads` feature, ON by default)
//!
//! When `verified-reads` is on, the worker stands up an **embedded Helios light
//! client** (see [`crate::helios`]) whose localhost JSON-RPC server is what the alloy
//! provider reads through — every read is proof-checked. The `rpc_url` passed to
//! [`EthProvider::spawn`] becomes the *execution-layer* endpoint Helios proves against
//! (it must serve `eth_getProof`); it is no longer read directly. Each read is tagged
//! with a [`ReadStatus`]: `Verified` when a fresh Helios head backs it, `Unsynced`
//! otherwise.
//!
//! When `verified-reads` is OFF, the worker keeps the original raw-RPC path but tags
//! every read `ReadStatus::Unsynced("verification disabled")` — it never claims a raw
//! read is Verified.
//!
//! Threading model (eng-review decision, preserved): a *single* background tokio
//! current-thread runtime owns every network call — including Helios's spawned
//! localhost server task, which runs cooperatively on the same runtime. The GUI never
//! blocks and never touches tokio.
//!
//! TODO(post-v1): v1 runs an INDEPENDENT Helios instance per reader (this one + the
//! daemon's). The "consolidate all reads into the daemon" refactor is deferred.
//! TODO(post-v1): if Helios's server task starves under load on the shared
//! current-thread runtime, consider `new_multi_thread`. Do NOT switch preemptively —
//! it would break the "single current-thread runtime" decision without proven need.

use alloy::ens::ProviderEnsExt;
use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};

use deckard_contract::ReadStatus;

use crate::balances::{fetch_portfolio, Portfolio};

/// A reliable public mainnet RPC, used as the execution-layer endpoint Helios proves
/// against (or, with `verified-reads` off, read directly). Overridable via settings.
pub const DEFAULT_RPC: &str = "https://ethereum-rpc.publicnode.com";

/// The reply half of a request: the worker sends the result here; the caller awaits it.
type Reply<T> = flume::Sender<anyhow::Result<T>>;

/// A value read off-chain, with the trust label that read carries. Returned to the UI
/// so it can render the verified/unsynced state alongside the value.
#[derive(Clone, Debug)]
pub struct Read<T> {
    pub value: T,
    pub status: ReadStatus,
}

impl<T> Read<T> {
    fn new(value: T, status: ReadStatus) -> Self {
        Self { value, status }
    }
}

/// Typed requests the GUI sends to the network worker. Each carries its own reply
/// channel so call sites stay ergonomic and unrelated requests never head-of-line block.
enum EthReq {
    Balance {
        addr: Address,
        reply: Reply<Read<U256>>,
    },
    BlockNumber {
        reply: Reply<Read<u64>>,
    },
    Portfolio {
        addr: Address,
        reply: Reply<Read<Portfolio>>,
    },
    ResolveName {
        name: String,
        reply: Reply<Address>,
    },
}

/// A cloneable handle to the network worker thread. Clone it freely into UI views;
/// when the last clone drops, the request channel closes and the worker thread exits.
#[derive(Clone)]
pub struct EthProvider {
    tx: flume::Sender<EthReq>,
}

impl EthProvider {
    /// Spawn the network worker pointed at `rpc_url`. Never blocks; the runtime, the
    /// embedded Helios client (when `verified-reads` is on), and the alloy provider are
    /// all built on the worker thread.
    pub fn spawn(rpc_url: impl Into<String>) -> Self {
        let rpc_url = rpc_url.into();
        let (tx, rx) = flume::unbounded::<EthReq>();
        // Fatal-at-startup boundary: if the OS refuses to spawn the network thread the app cannot
        // function, so a clear panic is correct here — this is not fallible user input.
        #[allow(clippy::expect_used)]
        std::thread::Builder::new()
            .name("deckard-eth".into())
            .spawn(move || run_worker(rpc_url, rx))
            .expect("spawn deckard-eth worker thread");
        Self { tx }
    }

    /// Fetch the native ETH balance (wei) of `addr`, with its trust label. Returns
    /// immediately; the caller awaits the receiver on its own executor. A dead worker
    /// resolves to an error rather than hanging.
    pub fn balance(&self, addr: Address) -> flume::Receiver<anyhow::Result<Read<U256>>> {
        self.request(|reply| EthReq::Balance { addr, reply })
    }

    /// Fetch the latest block number (a cheap liveness/sync probe) with its trust label.
    pub fn block_number(&self) -> flume::Receiver<anyhow::Result<Read<u64>>> {
        self.request(|reply| EthReq::BlockNumber { reply })
    }

    /// Fetch the full portfolio (native + listed ERC-20 balances) in one Multicall3
    /// round-trip, with its trust label. Non-blocking; await on the UI executor.
    pub fn portfolio(&self, addr: Address) -> flume::Receiver<anyhow::Result<Read<Portfolio>>> {
        self.request(|reply| EthReq::Portfolio { addr, reply })
    }

    /// Forward-resolve an ENS name (e.g. `vitalik.eth`) to an address. Not value-bearing,
    /// so no trust label — the resulting address is then read with one.
    pub fn resolve_name(
        &self,
        name: impl Into<String>,
    ) -> flume::Receiver<anyhow::Result<Address>> {
        let name = name.into();
        self.request(|reply| EthReq::ResolveName { name, reply })
    }

    /// Shared plumbing: build a one-shot reply channel, enqueue the request, and hand
    /// the receiver back. If the worker is gone, pre-load the error so the await still
    /// resolves.
    fn request<T: Send + 'static>(
        &self,
        make: impl FnOnce(Reply<T>) -> EthReq,
    ) -> flume::Receiver<anyhow::Result<T>> {
        let (reply, rx) = flume::bounded(1);
        if self.tx.send(make(reply.clone())).is_err() {
            let _ = reply.send(Err(anyhow::anyhow!("eth worker thread is gone")));
        }
        rx
    }
}

/// The worker entry point: build the runtime + the read provider (verified or raw),
/// then service requests until every `EthProvider` handle has dropped (closing `rx`).
fn run_worker(rpc_url: String, rx: flume::Receiver<EthReq>) {
    // Fatal-at-startup boundary: a current-thread runtime we cannot build leaves the worker unable
    // to do anything; panicking with a clear message beats silently servicing nothing.
    #[allow(clippy::expect_used)]
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio current-thread runtime");

    rt.block_on(async move {
        let read_path = ReadPath::build(&rpc_url).await;

        while let Ok(req) = rx.recv_async().await {
            match req {
                EthReq::Balance { addr, reply } => {
                    let _ = reply.send(read_path.balance(addr).await);
                }
                EthReq::BlockNumber { reply } => {
                    let _ = reply.send(read_path.block_number().await);
                }
                EthReq::Portfolio { addr, reply } => {
                    let _ = reply.send(read_path.portfolio(addr).await);
                }
                EthReq::ResolveName { name, reply } => {
                    let _ = reply.send(read_path.resolve_name(&name).await);
                }
            }
        }
    });
}

/// The worker's resolved read path. Holds the alloy provider it reads through and, when
/// `verified-reads` is on, the embedded Helios client that owns the localhost server
/// (kept alive for the worker's lifetime — its Drop tears the server down).
struct ReadPath {
    /// `None` when the URL was unparseable / Helios failed to come up. Every read then
    /// answers with an error or an `Unsynced` status (fail-closed; the UI never hangs).
    provider: Option<DynProvider>,
    /// `None` → this is the verified Helios path: the trust label is re-derived per read
    /// from Helios head freshness. `Some(reason)` → a non-verified path (Helios down or
    /// the feature disabled): every read is tagged `Unsynced(reason)`, NEVER `Verified`.
    unverified_reason: Option<String>,
    /// Keeps the embedded Helios localhost server alive. `None` for the raw path.
    #[cfg(feature = "verified-reads")]
    _helios: Option<crate::helios::VerifiedReader>,
}

impl ReadPath {
    /// Build the read path on the worker thread, inside the worker's tokio runtime.
    #[cfg(feature = "verified-reads")]
    async fn build(rpc_url: &str) -> Self {
        // The configured RPC is now the EXECUTION-layer endpoint Helios proves against
        // (it must serve eth_getProof) — never read directly. CL drives the sync.
        let data_dir = crate::config::config_dir()
            .map(|d| d.join("helios"))
            .unwrap_or_else(|| std::path::PathBuf::from(".deckard-helios"));

        match crate::helios::launch_verified(
            crate::helios::DEFAULT_CONSENSUS_RPC,
            rpc_url,
            data_dir,
        )
        .await
        {
            Ok(reader) => {
                // Clone the verified localhost provider out for the read handlers; the
                // VerifiedReader is retained so the server task stays alive.
                let provider = reader.provider().clone();
                Self {
                    provider: Some(provider),
                    unverified_reason: None, // verified path: label by head freshness
                    _helios: Some(reader),
                }
            }
            Err(e) => {
                // Helios never came up: serve reads as Unsynced. We do NOT fall back to a
                // raw read of the (untrusted) RPC and call it Verified. We still build a
                // raw provider so values can be shown, but always tagged Unsynced with an
                // honest reason.
                let reason = format!("helios unavailable: {}", one_line(&e));
                let provider = rpc_url
                    .parse()
                    .ok()
                    .map(|url| ProviderBuilder::new().connect_http(url).erased());
                Self {
                    provider,
                    unverified_reason: Some(reason),
                    _helios: None,
                }
            }
        }
    }

    /// Feature-off build: the original raw-RPC path, always tagged Unsynced.
    #[cfg(not(feature = "verified-reads"))]
    async fn build(rpc_url: &str) -> Self {
        let provider = rpc_url
            .parse()
            .ok()
            .map(|url| ProviderBuilder::new().connect_http(url).erased());
        Self {
            provider,
            unverified_reason: Some("verification disabled".to_string()),
        }
    }

    /// The trust label for a read taken now. On the verified path (`unverified_reason ==
    /// None`), re-derive from the Helios head (a head gone stale mid-session downgrades to
    /// Unsynced). Otherwise the fixed honest reason. NEVER returns Verified without a
    /// fresh Helios head behind it.
    ///
    /// Each value-bearing read (`balance`/`block_number`/`portfolio`) reads the value FIRST
    /// and then calls `status()` — so a `Verified` tag is bound to a head observed *after*
    /// the value came back (the daemon's read path uses the same ordering). A small
    /// time-of-check/time-of-use window remains between the two round-trips: a head could go
    /// stale in the gap. This is an accepted v1 limitation; it always fails toward "a fresh
    /// verified head backed the value", never toward a false `Verified`.
    async fn status(&self) -> ReadStatus {
        match &self.unverified_reason {
            None => {
                #[cfg(feature = "verified-reads")]
                if let Some(reader) = &self._helios {
                    return reader.head_status().await;
                }
                // Defensive: a verified path with no client shouldn't happen.
                ReadStatus::unsynced("verification unavailable")
            }
            Some(reason) => ReadStatus::unsynced(reason.clone()),
        }
    }

    async fn balance(&self, addr: Address) -> anyhow::Result<Read<U256>> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no RPC/Helios read path"))?;
        let value = provider.get_balance(addr).await?;
        Ok(Read::new(value, self.status().await))
    }

    async fn block_number(&self) -> anyhow::Result<Read<u64>> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no RPC/Helios read path"))?;
        let value = provider.get_block_number().await?;
        Ok(Read::new(value, self.status().await))
    }

    async fn portfolio(&self, addr: Address) -> anyhow::Result<Read<Portfolio>> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no RPC/Helios read path"))?;
        let value = fetch_portfolio(provider, addr).await?;
        Ok(Read::new(value, self.status().await))
    }

    async fn resolve_name(&self, name: &str) -> anyhow::Result<Address> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no RPC/Helios read path"))?;
        provider
            .resolve_name(name)
            .await
            .map_err(anyhow::Error::from)
    }
}

/// Collapse a multi-line error into one short line for a `reason` string. Only the
/// verified-reads build constructs a reason from an error.
#[cfg(feature = "verified-reads")]
fn one_line(e: &impl std::fmt::Display) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::providers::mock::Asserter;

    /// Build a ReadPath over a mocked transport (no network, deterministic). Reads are
    /// tagged Unsynced because there is no Helios behind a mock — the hard rule holds.
    fn mocked_path(asserter: Asserter) -> ReadPath {
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter)
            .erased();
        ReadPath {
            provider: Some(provider),
            unverified_reason: Some("test (no helios)".to_string()),
            #[cfg(feature = "verified-reads")]
            _helios: None,
        }
    }

    /// The read path decodes a balance off a mocked transport and attaches a status.
    #[tokio::test]
    async fn balance_reads_from_mocked_transport_with_status() {
        let asserter = Asserter::new();
        asserter.push_success(&U256::from(31_415u64));
        let path = mocked_path(asserter);

        let read = path.balance(Address::ZERO).await.unwrap();
        assert_eq!(read.value, U256::from(31_415u64));
        // No Helios behind a mock → never Verified.
        assert!(!read.status.is_trustworthy());
    }

    /// A missing provider fails closed with an error rather than panicking or hanging.
    #[tokio::test]
    async fn no_provider_errors_cleanly() {
        let path = ReadPath {
            provider: None,
            unverified_reason: Some("test (no provider)".to_string()),
            #[cfg(feature = "verified-reads")]
            _helios: None,
        };
        assert!(path.balance(Address::ZERO).await.is_err());
    }
}
