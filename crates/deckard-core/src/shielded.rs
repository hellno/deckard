//! Shielded-balance sync — the read-only Railgun account actor (Wave-2 T9).
//!
//! Mirrors [`EthProvider`](crate::eth::EthProvider): one OS thread with a current-thread tokio
//! runtime owns the Railgun provider and syncs in the background, updating a cached snapshot the
//! GUI reads instantly (so a read never blocks behind a full sync). The app holds only a
//! read-only VIEW grant — the viewing key + 0zk address, never the spending key — so it can SEE
//! private balances, not spend them. Sync rides the raw RPC + Subsquid (Railgun's `getLogs`
//! path is NOT Helios-verified), so a synced private balance is honestly `Unsynced`, never
//! `Verified`; and while the first sync runs the balance is UNKNOWN, never silently zero.
//!
//! The `ViewOnlySigner` and the underlying `RailgunProvider` are private to this module: a real
//! spend must go back through the daemon, never "reuse" this read-only signer.

use std::sync::{Arc, Mutex};

use alloy::providers::{Provider, ProviderBuilder};
use deckard_contract::{RailgunViewGrant, ReadStatus};
use railgun::{
    account::{
        address::RailgunAddress,
        chain::ChainId,
        signer::{RailgunSigner, RailgunSignerError},
    },
    builder::RailgunBuilder,
    caip::AssetId,
    chain_config::ChainConfig,
    crypto::keys::{HexKey, SpendingKey, SpendingSignature, ViewingKey},
    indexer::syncer::{ChainedSyncer, RpcSyncer, SubsquidSyncer},
};

use crate::U256;

/// The cached shielded-balance state the GUI renders. `shielded_wei` is `None` until the first
/// successful sync — UNKNOWN, never silently zero. `syncing` marks an in-flight sync; `error`
/// holds the last failure reason.
#[derive(Clone, Default)]
pub struct ShieldedSnapshot {
    pub shielded_wei: Option<U256>,
    pub syncing: bool,
    pub error: Option<String>,
}

impl ShieldedSnapshot {
    /// The honest trust label: a private balance is NEVER `Verified` in v1 — the sync rides the
    /// raw RPC / Subsquid, not the Helios-verified read path.
    pub fn read_status(&self) -> ReadStatus {
        ReadStatus::unsynced("private sync uses unverified RPC/subsquid")
    }
}

/// A handle to the background Railgun sync. The GUI reads [`snapshot`](Self::snapshot) and
/// triggers [`resync`](Self::resync) (e.g. after a shield). Dropping it stops the worker.
pub struct ShieldedHandle {
    cached: Arc<Mutex<ShieldedSnapshot>>,
    resync: flume::Sender<()>,
}

impl ShieldedHandle {
    /// Spawn the sync worker for `chain_id` using the read-only `grant`. The worker builds its
    /// own raw provider; any build/parse failure surfaces as the snapshot's `error`, never a
    /// panic (fail-closed — the UI never hangs and never shows a fabricated balance).
    pub fn spawn(rpc_url: String, chain_id: u64, grant: RailgunViewGrant) -> Self {
        let cached = Arc::new(Mutex::new(ShieldedSnapshot {
            syncing: true,
            ..Default::default()
        }));
        let (resync, resync_rx) = flume::bounded(1);
        let worker_cached = cached.clone();
        std::thread::spawn(move || run_worker(rpc_url, chain_id, grant, worker_cached, resync_rx));
        Self { cached, resync }
    }

    /// The current cached snapshot — instant, never blocks on a sync. Returns the default
    /// (unknown, not syncing) if the lock was poisoned.
    pub fn snapshot(&self) -> ShieldedSnapshot {
        self.cached.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Request a re-sync (e.g. after a shield broadcast). Coalesces: a pending request is kept,
    /// extra requests are dropped, so syncs never overlap.
    pub fn resync(&self) {
        let _ = self.resync.try_send(());
    }
}

/// The worker: build the provider + Railgun account once, then sync on spawn and on every
/// `resync` signal, folding each result into the shared snapshot.
fn run_worker(
    rpc_url: String,
    chain_id: u64,
    grant: RailgunViewGrant,
    cached: Arc<Mutex<ShieldedSnapshot>>,
    resync_rx: flume::Receiver<()>,
) {
    // Startup-fatal boundary (mirrors `eth::run_worker`): a runtime we can't build leaves the
    // worker unable to do anything; a clear panic beats silently servicing nothing.
    #[allow(clippy::expect_used)]
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio current-thread runtime");

    rt.block_on(async move {
        let Some(chain) = ChainConfig::from_chain_id(chain_id) else {
            set_error(&cached, format!("unsupported chain {chain_id}"));
            return;
        };
        let signer = match ViewOnlySigner::from_grant(&grant, chain_id) {
            Ok(s) => Arc::new(s),
            Err(e) => return set_error(&cached, e),
        };
        let address = signer.address();
        let weth = AssetId::Erc20(chain.wrapped_base_token);

        let Ok(url) = rpc_url.parse() else {
            set_error(&cached, format!("bad rpc url: {rpc_url}"));
            return;
        };
        let provider = ProviderBuilder::new().connect_http(url).erased();
        let syncer = Arc::new(
            ChainedSyncer::new()
                .then(SubsquidSyncer::new(&chain.subsquid_endpoint))
                .then(RpcSyncer::new(chain.clone(), provider.clone()).with_batch_size(1000)),
        );
        let mut railgun = match RailgunBuilder::new(chain, provider)
            .with_utxo_syncer(syncer)
            .build()
            .await
        {
            Ok(r) => r,
            Err(e) => return set_error(&cached, format!("railgun build: {e}")),
        };
        if let Err(e) = railgun.register(signer).await {
            return set_error(&cached, format!("register: {e}"));
        }

        // Initial sync, then re-sync on each trigger. Single-threaded → no overlap.
        loop {
            mark_syncing(&cached);
            match railgun.sync().await {
                Ok(()) => {
                    let wei = railgun
                        .balance(address)
                        .await
                        .get(&weth)
                        .copied()
                        .unwrap_or(0);
                    set_synced(&cached, U256::from(wei));
                }
                Err(e) => set_error(&cached, format!("sync: {e}")),
            }
            // Wait for the next resync; the channel closing means the handle was dropped.
            if resync_rx.recv_async().await.is_err() {
                break;
            }
        }
    });
}

fn mark_syncing(cached: &Mutex<ShieldedSnapshot>) {
    if let Ok(mut s) = cached.lock() {
        s.syncing = true;
        s.error = None;
    }
}

fn set_synced(cached: &Mutex<ShieldedSnapshot>, wei: U256) {
    if let Ok(mut s) = cached.lock() {
        s.shielded_wei = Some(wei);
        s.syncing = false;
        s.error = None;
    }
}

fn set_error(cached: &Mutex<ShieldedSnapshot>, reason: impl Into<String>) {
    if let Ok(mut s) = cached.lock() {
        s.syncing = false;
        s.error = Some(reason.into());
    }
}

/// A read-only Railgun signer: the real viewing key + a precomputed address, with a
/// NON-functional dummy spending key. The sync/balance path uses only `address()` +
/// `viewing_key()`, so the dummy is never exercised; this type is module-private and the
/// `RailgunProvider` is never exposed, so it can never be wired to a real spend.
struct ViewOnlySigner {
    address: RailgunAddress,
    viewing: ViewingKey,
    dummy_spending: SpendingKey,
    chain: ChainId,
}

impl ViewOnlySigner {
    fn from_grant(grant: &RailgunViewGrant, chain_id: u64) -> Result<Self, String> {
        let address = grant
            .address
            .parse::<RailgunAddress>()
            .map_err(|e| format!("bad 0zk address: {e}"))?;
        let viewing = ViewingKey::from_hex(&grant.viewing_key)
            .map_err(|e| format!("bad viewing key: {e}"))?;
        // A throwaway spending key — never used (address() is overridden, sign() unreachable).
        let dummy_spending =
            SpendingKey::from_hex(&"0".repeat(64)).map_err(|e| format!("dummy key: {e}"))?;
        Ok(Self {
            address,
            viewing,
            dummy_spending,
            chain: ChainId::evm(chain_id),
        })
    }
}

impl RailgunSigner for ViewOnlySigner {
    fn chain_id(&self) -> ChainId {
        self.chain
    }
    fn viewing_key(&self) -> ViewingKey {
        self.viewing
    }
    fn spending_key(&self) -> SpendingKey {
        self.dummy_spending
    }
    // Override so the address is the granted one, never re-derived from the dummy spending key.
    fn address(&self) -> RailgunAddress {
        self.address
    }
    fn sign(&self, inputs: U256) -> Result<SpendingSignature, RailgunSignerError> {
        // Unreachable on the read-only balance path; signs with the inert dummy key. This signer
        // is module-private and never wired to a spend — real spends go through the daemon.
        Ok(self.dummy_spending.sign(inputs))
    }
}
