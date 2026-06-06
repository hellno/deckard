//! The Ethereum provider — a single `alloy` JSON-RPC client living on a dedicated
//! background thread that owns a current-thread tokio runtime. The GUI sends typed
//! requests in and gets a `flume::Receiver` back; it awaits that receiver on its own
//! executor (`cx.spawn`), so a slow RPC never stalls a frame.
//!
//! v0 points at a public mainnet RPC by default (overridable in settings). The
//! trustless default — a bundled Helios light client serving localhost — is the next
//! increment per the spec; swapping it in is just a different URL passed to `spawn`.

use alloy::ens::ProviderEnsExt;
use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};

use crate::balances::{fetch_portfolio, Portfolio};

/// A reliable public mainnet RPC, used until the bundled Helios light client lands.
/// Overridable via settings (bring-your-own-RPC).
pub const DEFAULT_RPC: &str = "https://ethereum-rpc.publicnode.com";

/// The reply half of a request: the worker sends the result here; the caller awaits it.
type Reply<T> = flume::Sender<anyhow::Result<T>>;

/// Typed requests the GUI sends to the network worker. Each carries its own reply
/// channel so call sites stay ergonomic and unrelated requests never head-of-line block.
enum EthReq {
    Balance {
        addr: Address,
        reply: Reply<U256>,
    },
    BlockNumber {
        reply: Reply<u64>,
    },
    Portfolio {
        addr: Address,
        reply: Reply<Portfolio>,
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
    /// Spawn the network worker pointed at `rpc_url`. Never blocks; the runtime and
    /// the alloy provider are built on the worker thread.
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

    /// Fetch the native ETH balance (wei) of `addr`. Returns immediately; the caller
    /// awaits the receiver on its own executor. A dead worker resolves to an error
    /// rather than hanging.
    pub fn balance(&self, addr: Address) -> flume::Receiver<anyhow::Result<U256>> {
        self.request(|reply| EthReq::Balance { addr, reply })
    }

    /// Fetch the latest block number — a cheap liveness/sync probe for the status line.
    pub fn block_number(&self) -> flume::Receiver<anyhow::Result<u64>> {
        self.request(|reply| EthReq::BlockNumber { reply })
    }

    /// Fetch the full portfolio (native + listed ERC-20 balances) in one Multicall3
    /// round-trip. Non-blocking; await the receiver on the UI executor.
    pub fn portfolio(&self, addr: Address) -> flume::Receiver<anyhow::Result<Portfolio>> {
        self.request(|reply| EthReq::Portfolio { addr, reply })
    }

    /// Forward-resolve an ENS name (e.g. `vitalik.eth`) to an address.
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

/// The worker entry point: build the runtime + provider, then service requests until
/// every `EthProvider` handle has dropped (which closes `rx`).
fn run_worker(rpc_url: String, rx: flume::Receiver<EthReq>) {
    // Fatal-at-startup boundary: a current-thread runtime we cannot build leaves the worker unable
    // to do anything; panicking with a clear message beats silently servicing nothing.
    #[allow(clippy::expect_used)]
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio current-thread runtime");

    rt.block_on(async move {
        // A bad URL yields `None`; we still drain the queue and answer every request
        // with an error so the UI never hangs waiting on a reply that never comes.
        let provider: Option<DynProvider> = rpc_url
            .parse()
            .ok()
            .map(|url| ProviderBuilder::new().connect_http(url).erased());

        while let Ok(req) = rx.recv_async().await {
            match req {
                EthReq::Balance { addr, reply } => {
                    let _ = reply.send(fetch_balance(provider.as_ref(), addr).await);
                }
                EthReq::BlockNumber { reply } => {
                    let _ = reply.send(fetch_block_number(provider.as_ref()).await);
                }
                EthReq::Portfolio { addr, reply } => {
                    let res = match provider.as_ref() {
                        Some(p) => fetch_portfolio(p, addr).await,
                        None => Err(anyhow::anyhow!("invalid RPC URL")),
                    };
                    let _ = reply.send(res);
                }
                EthReq::ResolveName { name, reply } => {
                    let res = match provider.as_ref() {
                        Some(p) => p.resolve_name(&name).await.map_err(anyhow::Error::from),
                        None => Err(anyhow::anyhow!("invalid RPC URL")),
                    };
                    let _ = reply.send(res);
                }
            }
        }
    });
}

async fn fetch_balance(provider: Option<&DynProvider>, addr: Address) -> anyhow::Result<U256> {
    let provider = provider.ok_or_else(|| anyhow::anyhow!("invalid RPC URL"))?;
    Ok(provider.get_balance(addr).await?)
}

async fn fetch_block_number(provider: Option<&DynProvider>) -> anyhow::Result<u64> {
    let provider = provider.ok_or_else(|| anyhow::anyhow!("invalid RPC URL"))?;
    Ok(provider.get_block_number().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::providers::mock::Asserter;

    /// The provider abstraction reads a balance off a mocked transport — no network,
    /// deterministic, fast. Proves the decode path without hitting a real RPC.
    #[tokio::test]
    async fn balance_reads_from_mocked_transport() {
        let asserter = Asserter::new();
        asserter.push_success(&U256::from(31_415u64));
        let provider = ProviderBuilder::new()
            .connect_mocked_client(asserter)
            .erased();

        let bal = fetch_balance(Some(&provider), Address::ZERO).await.unwrap();
        assert_eq!(bal, U256::from(31_415u64));
    }

    /// A bad RPC URL fails closed with an error rather than panicking or hanging.
    #[tokio::test]
    async fn invalid_url_errors_cleanly() {
        assert!(fetch_balance(None, Address::ZERO).await.is_err());
    }
}
