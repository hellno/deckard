//! The daemon state machine: `Locked` ⇄ `Unlocked { vault }`, the in-flight request table,
//! and the handlers for every [`SignerRequest`]. The verdict for a `propose` comes from the
//! ONE shared [`deckard_contract::evaluate`] — the daemon adds only the process-level
//! pre-checks the policy can't express (`Locked`, `chain_mismatch`, unsupported kind).
//!
//! All requests are serialized behind a single [`Daemon`] (the server holds it in a
//! `tokio::sync::Mutex`), so `propose`/`execute` can never race. `execute` holds that lock
//! across the broadcast — acceptable for v1 (anvil is instant); a STOP arriving *during* an
//! in-progress broadcast can't unsend a tx already on the wire, but the TOCTOU guard refuses
//! any execute whose STOP landed first.

use std::collections::HashMap;
#[cfg(feature = "verified-reads")]
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256, U256};
#[cfg(feature = "verified-reads")]
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

use deckard_contract::{
    evaluate, ApprovalStatus, BalanceReport, Decision, ExecuteResult, Intent, IntentKind, Policy,
    ReadStatus, RequestId, SignerRequest, SignerResponse, UnlockOutcome,
};
use deckard_core::{UnlockedVault, Vault};

use crate::config::Config;
use crate::policy_store::{self, current_utc_day};
use crate::request_id::request_id_for;
use crate::signing;

/// Default lifetime of a `NeedsApproval` before `status`/`execute` report `Expired`.
/// Overridable via `DECKARD_APPROVAL_TTL_SECS` (used by tests to exercise expiry quickly).
const APPROVAL_TTL: Duration = Duration::from_secs(120);

/// Resolve the approval TTL: `DECKARD_APPROVAL_TTL_SECS` if set + parseable, else the default.
fn approval_ttl() -> Duration {
    std::env::var("DECKARD_APPROVAL_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(APPROVAL_TTL)
}

/// `Locked` holds no key; `Unlocked` owns the decrypted vault (dropped — and zeroized — on
/// lock/STOP) plus its cached primary address.
enum VaultState {
    Locked,
    Unlocked {
        vault: UnlockedVault,
        address: Address,
    },
}

/// One tracked proposal. `status` is the wire-visible approval state; `broadcast` is `Some`
/// once `execute` has signed it (so a second `execute` is idempotently refused). `approved`
/// is `true` only once a human `Resolve`d it — an *auto*-allow (within-cap) is re-checked
/// against the caps at execute time, while a human-approved overage is not.
struct PendingReq {
    intent: Intent,
    status: ApprovalStatus,
    expires_at: Instant,
    broadcast: Option<B256>,
    approved: bool,
}

/// Upper bound on a single broadcast round-trip. A hung/blackholed RPC fails after this
/// rather than wedging the daemon (and STOP) forever behind the held state lock.
const BROADCAST_TIMEOUT: Duration = Duration::from_secs(30);

/// The daemon's embedded Helios verified-read path, kept in a **separately-locked** cell so
/// the multi-second-to-90s `launch_verified` bootstrap can run WITHOUT holding the daemon's
/// own `Mutex`. The server clones this `Arc` and primes it (see [`HeliosCell::ensure`]) off
/// the daemon lock before dispatching a `Balance`, so a slow first read can never serialize
/// behind it — the STOP/Lock brake stays responsive.
///
/// Its own `Drop` (via the inner `VerifiedReader`) tears the spawned localhost server down.
/// `None` inside the option means "not built / failed to come up" — reads then fall back,
/// tagged Unsynced, never silently Verified.
///
/// TODO(post-v1): v1 runs an INDEPENDENT Helios instance here (separate from the app's
/// deckard-core::EthProvider one). The "consolidate all reads into the daemon" refactor is
/// deferred.
#[cfg(feature = "verified-reads")]
#[derive(Clone, Default)]
pub struct HeliosCell {
    inner: Arc<AsyncMutex<Option<deckard_core::VerifiedReader>>>,
}

#[cfg(feature = "verified-reads")]
impl HeliosCell {
    fn new() -> Self {
        Self::default()
    }

    /// Bootstrap the embedded Helios client if it isn't up yet. Runs the long
    /// `launch_verified` while holding ONLY this cell's lock — never the daemon's — so the
    /// security brake (STOP/Lock) and every other request stay live during the bootstrap.
    /// Idempotent: a second caller that finds the client already built returns immediately.
    /// On failure leaves the cell empty and logs (so the read falls back to a raw,
    /// Unsynced-tagged read — never a silent Verified).
    pub async fn ensure(
        &self,
        consensus_rpc: &str,
        execution_rpc: &str,
        data_dir: std::path::PathBuf,
    ) {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return;
        }
        match deckard_core::launch_verified(consensus_rpc, execution_rpc, data_dir).await {
            Ok(reader) => *guard = Some(reader),
            Err(e) => {
                // Stays None → the read path falls back to a raw, Unsynced read.
                eprintln!("signerd: helios bootstrap failed (reads tagged unsynced): {}", one_line(&e));
            }
        }
    }
}

/// The whole daemon: config, the lock state, the live policy (with in-memory daily spend),
/// and the request table.
pub struct Daemon {
    cfg: Config,
    state: VaultState,
    policy: Policy,
    /// UTC day of the current `spent_today_wei` window (for the midnight rollover).
    spent_day: u64,
    /// Lifetime of a `NeedsApproval` record.
    approval_ttl: Duration,
    requests: HashMap<RequestId, PendingReq>,
    /// The daemon's embedded Helios verified-read path, held in a SEPARATELY-locked
    /// [`HeliosCell`] so its slow bootstrap never blocks the daemon mutex (see the cell's
    /// docs). The server primes it off the daemon lock before a `Balance` dispatch; the
    /// `balance` handler then borrows the already-built reader for the quick read. Cloning
    /// the `Arc` is cheap and lets the server hold a handle without the daemon lock.
    #[cfg(feature = "verified-reads")]
    helios: HeliosCell,
}

impl Daemon {
    /// Build a `Locked` daemon, loading the policy (or its safe default) up front.
    pub fn new(cfg: Config) -> Self {
        let policy = policy_store::load_policy(&cfg.policy_path());
        Self {
            cfg,
            state: VaultState::Locked,
            policy,
            spent_day: current_utc_day(),
            approval_ttl: approval_ttl(),
            requests: HashMap::new(),
            #[cfg(feature = "verified-reads")]
            helios: HeliosCell::new(),
        }
    }

    /// A clone of the daemon's [`HeliosCell`] handle, so the server can prime the Helios
    /// bootstrap OFF the daemon lock before dispatching a `Balance` (keeping the STOP/Lock
    /// brake responsive — the long bootstrap never holds the daemon mutex).
    #[cfg(feature = "verified-reads")]
    pub fn helios_cell(&self) -> HeliosCell {
        self.helios.clone()
    }

    /// The (consensus_rpc, execution_rpc, data_dir) the embedded Helios client bootstraps
    /// with. Exposed so the server can prime the cell off the daemon lock.
    #[cfg(feature = "verified-reads")]
    pub fn helios_bootstrap_args(&self) -> (&'static str, String, std::path::PathBuf) {
        (
            deckard_core::DEFAULT_CONSENSUS_RPC,
            self.cfg.rpc_url.clone(),
            self.cfg.config_dir.join("helios-signerd"),
        )
    }

    /// Dispatch one request to one response. `async` because `execute`/`balance` do network
    /// I/O and `unlock` runs Argon2 on the blocking pool.
    pub async fn handle(&mut self, req: SignerRequest) -> SignerResponse {
        match req {
            SignerRequest::Unlock { passphrase } => {
                SignerResponse::Unlock(self.unlock(passphrase).await)
            }
            // Lock and RevokeAll are the same act in v1: zeroize the key → Locked, deny
            // everything in flight. Only a fresh Unlock re-arms.
            SignerRequest::Lock | SignerRequest::RevokeAll => {
                self.lock();
                SignerResponse::Ack
            }
            SignerRequest::Resolve {
                request_id,
                approved,
            } => {
                self.resolve(request_id, approved);
                SignerResponse::Ack
            }
            SignerRequest::Propose { intent } => SignerResponse::Decision(self.propose(&intent)),
            SignerRequest::Execute { request_id } => {
                SignerResponse::Execute(self.execute(request_id).await)
            }
            SignerRequest::Status { request_id } => SignerResponse::Status(self.status(request_id)),
            SignerRequest::PolicyGet => {
                self.rollover();
                SignerResponse::Policy(self.policy.clone())
            }
            SignerRequest::Address => match &self.state {
                VaultState::Unlocked { address, .. } => SignerResponse::Address(*address),
                // No Address-specific error variant exists; signal locked Deny-style.
                VaultState::Locked => SignerResponse::Decision(Decision::Deny {
                    reason: "locked".into(),
                }),
            },
            SignerRequest::Balance { shielded } => {
                SignerResponse::Balance(self.balance(shielded).await)
            }
        }
    }

    /// Read the keystore, decrypt under `passphrase`, and hold the key. The raw passphrase is
    /// moved into `Zeroizing` immediately and never echoed or logged.
    async fn unlock(&mut self, passphrase: String) -> UnlockOutcome {
        let pass = Zeroizing::new(passphrase);
        let vault_path = self.cfg.vault_path();
        if !vault_path.exists() {
            return UnlockOutcome::NoVault;
        }
        // Argon2id is CPU-heavy: read + unlock on the blocking pool so the reactor stays free.
        let pass_for_blocking = pass.clone();
        let result = tokio::task::spawn_blocking(move || {
            let vault = Vault::read(&vault_path)?;
            vault.unlock(pass_for_blocking.as_str())
        })
        .await;

        match result {
            Ok(Ok(unlocked)) => match unlocked.primary_address() {
                Ok(address) => {
                    self.state = VaultState::Unlocked {
                        vault: unlocked,
                        address,
                    };
                    self.policy.revoked = false; // a fresh unlock re-arms
                    self.requests.clear(); // fresh session: no stale approvals survive a re-unlock
                    UnlockOutcome::Unlocked { address }
                }
                // A successfully decrypted vault that can't derive an address is corrupt;
                // treat as a failed unlock rather than holding an unusable key.
                Err(_) => UnlockOutcome::BadPassphrase,
            },
            // Wrong passphrase, a tampered vault, or a read error: one generic outcome, no
            // oracle, no key held.
            Ok(Err(_)) | Err(_) => UnlockOutcome::BadPassphrase,
        }
    }

    /// Zeroize + drop the key → `Locked`, deny EVERY non-broadcast approval (both `Pending`
    /// and already-`Allowed`, so an approval granted before STOP can never execute — even via
    /// `status` polling), and trip the policy brake (so `PolicyGet` honestly reports
    /// `revoked`). Shared by `Lock` and `RevokeAll`.
    fn lock(&mut self) {
        self.state = VaultState::Locked; // dropping UnlockedVault zeroizes the secret
        self.policy.revoked = true;
        for req in self.requests.values_mut() {
            if req.broadcast.is_none()
                && matches!(
                    req.status,
                    ApprovalStatus::Pending | ApprovalStatus::Allowed
                )
            {
                req.status = ApprovalStatus::Denied {
                    reason: "revoked".into(),
                };
            }
        }
    }

    /// Close an approval loop: a human (or a test) flips a `Pending` record to
    /// `Allowed`/`Denied`. No-op for any other state (already resolved/expired).
    fn resolve(&mut self, request_id: RequestId, approved: bool) {
        self.expire_stale();
        if let Some(req) = self.requests.get_mut(&request_id) {
            if req.status == ApprovalStatus::Pending {
                if approved {
                    req.status = ApprovalStatus::Allowed;
                    req.approved = true; // explicit human consent: not re-capped at execute
                } else {
                    req.status = ApprovalStatus::Denied {
                        reason: "user_denied".into(),
                    };
                }
            }
        }
    }

    /// Policy check only — NEVER signs. Process-level pre-checks first, then the shared
    /// `evaluate`. On `NeedsApproval`/`Allow` a pending record is stored under the intent's
    /// deterministic id; on `Deny` nothing is stored.
    fn propose(&mut self, intent: &Intent) -> Decision {
        self.rollover();
        self.expire_stale();

        // Pre-checks the Policy can't express (the mock has none of these states, which is
        // why feeding both the same (Intent, Policy) yields identical decisions — the parity
        // contract). These run before `evaluate`.
        if matches!(self.state, VaultState::Locked) {
            return Decision::Deny {
                reason: "locked".into(),
            };
        }
        if intent.chain_id != self.cfg.chain_id {
            return Decision::Deny {
                reason: "chain_mismatch".into(),
            };
        }
        // v1 admits a native Send and a Shield (the privacy hero). The Shield's RelayAdapt
        // calldata is built key-less in deckard-core and rides in `intent.calldata`; the
        // daemon never sees the ZK crate, it only signs+broadcasts the handed bytes. Unshield
        // / ContractCall stay a fast-follow.
        if !matches!(intent.kind, IntentKind::Send | IntentKind::Shield) {
            return Decision::Deny {
                reason: "unsupported_v1".into(),
            };
        }
        // v1 spine is native ETH only; an ERC-20 (`token = Some`) Send is a fast-follow.
        // A native shield is `token: None` (the value rides as msg.value via RelayAdapt
        // wrapBase), so it passes this guard.
        if intent.token.is_some() {
            return Decision::Deny {
                reason: "erc20_unsupported_v1".into(),
            };
        }

        let id = request_id_for(intent);

        // Idempotent re-propose: an identical intent maps to the same id, so an existing record
        // is returned AS-IS — a re-propose can't reset a `Pending` card's TTL, downgrade a
        // human approval, or re-raise a `Denied`/`Expired` request. Retrying a terminal intent
        // needs a fresh session (`Unlock` clears the table).
        if let Some(existing) = self.requests.get(&id) {
            return match &existing.status {
                _ if existing.broadcast.is_some() => Decision::Deny {
                    reason: "already_executed".into(),
                },
                ApprovalStatus::Pending => Decision::NeedsApproval { request_id: id },
                ApprovalStatus::Allowed => Decision::Allow,
                ApprovalStatus::Denied { reason } => Decision::Deny {
                    reason: reason.clone(),
                },
                ApprovalStatus::Expired => Decision::Deny {
                    reason: "expired".into(),
                },
            };
        }

        // No record yet: the ONE shared decision function decides.
        let status = match evaluate(intent, &self.policy) {
            deny @ Decision::Deny { .. } => return deny,
            Decision::Allow => ApprovalStatus::Allowed,
            Decision::NeedsApproval { .. } => ApprovalStatus::Pending,
        };
        self.requests.insert(
            id,
            PendingReq {
                intent: intent.clone(),
                status: status.clone(),
                expires_at: Instant::now() + self.approval_ttl,
                broadcast: None,
                approved: false,
            },
        );

        match status {
            ApprovalStatus::Allowed => Decision::Allow,
            _ => Decision::NeedsApproval { request_id: id },
        }
    }

    /// Sign + broadcast, only for an `Allowed` request that survives the sign-time re-check.
    async fn execute(&mut self, request_id: RequestId) -> ExecuteResult {
        self.rollover();
        self.expire_stale();

        // Phase 1 (lock held): TOCTOU re-check + eligibility, then extract tx params and the
        // raw scalar (transiently, into `Zeroizing`). Borrows end before the await.
        let (to, value, calldata, scalar) = {
            let vault = match &self.state {
                // STOP landed first — refuse even a previously-approved request.
                VaultState::Locked => {
                    return ExecuteResult::Denied {
                        reason: "revoked".into(),
                    }
                }
                VaultState::Unlocked { vault, .. } => vault,
            };
            let req = match self.requests.get(&request_id) {
                None => {
                    return ExecuteResult::Denied {
                        reason: "unknown_request".into(),
                    }
                }
                Some(req) => req,
            };
            if req.broadcast.is_some() {
                return ExecuteResult::Denied {
                    reason: "already_executed".into(),
                };
            }
            match &req.status {
                // The only state that signs (covers within-cap Allow + human-approved over-cap).
                ApprovalStatus::Allowed => {}
                ApprovalStatus::Pending => {
                    return ExecuteResult::Denied {
                        reason: "not_approved".into(),
                    }
                }
                ApprovalStatus::Denied { reason } => {
                    return ExecuteResult::Denied {
                        reason: reason.clone(),
                    }
                }
                ApprovalStatus::Expired => {
                    return ExecuteResult::Denied {
                        reason: "expired".into(),
                    }
                }
            }
            // Spend TOCTOU: an *auto*-allow must still be within policy at sign time, so two
            // within-cap proposals can't both execute past the daily cap (`spent_today` only
            // grows on prior executes). A human-APPROVED request carries explicit consent for
            // its overage and is not re-capped.
            if !req.approved && evaluate(&req.intent, &self.policy) != Decision::Allow {
                return ExecuteResult::Denied {
                    reason: "cap_exceeded".into(),
                };
            }
            let signer = match vault.account_signer(0) {
                Ok(s) => s,
                Err(e) => {
                    return ExecuteResult::Denied {
                        reason: format!("signer_error: {e}"),
                    }
                }
            };
            // Only the version-stable raw scalar crosses into our alloy stack; zeroized on drop.
            let scalar = Zeroizing::new(signer.to_bytes().0);
            // Calldata is empty for a native Send (→ broadcast is byte-identical to before) and
            // carries the RelayAdapt call for a Shield. The empty-vs-non-empty input IS the
            // native/contract-call discriminator, so no IntentKind branch is needed here.
            (
                req.intent.to,
                req.intent.value,
                req.intent.calldata.clone(),
                scalar,
            )
        };

        // Phase 2: sign + broadcast (lock held — serialized; acceptable for v1). A bounded
        // timeout keeps a hung RPC from wedging the daemon (and STOP) behind the held lock.
        let broadcast = signing::broadcast_intent(
            scalar.as_slice(),
            &self.cfg.rpc_url,
            self.cfg.chain_id,
            to,
            value,
            &calldata,
        );
        let tx_hash = match tokio::time::timeout(BROADCAST_TIMEOUT, broadcast).await {
            Ok(Ok(hash)) => hash,
            Ok(Err(e)) => {
                return ExecuteResult::Denied {
                    reason: format!("broadcast_failed: {}", one_line(&e)),
                }
            }
            Err(_elapsed) => {
                return ExecuteResult::Denied {
                    reason: "broadcast_timeout".into(),
                }
            }
        };

        // Phase 3: record the broadcast + bump the daily spend.
        if let Some(req) = self.requests.get_mut(&request_id) {
            req.broadcast = Some(tx_hash);
        }
        self.policy.spent_today_wei = self.policy.spent_today_wei.saturating_add(value);
        ExecuteResult::Broadcast { tx_hash }
    }

    /// Poll an approval handle. Unknown ids report `Denied{unknown_request}` (matching the
    /// mock); a `Pending` past its TTL reports `Expired`.
    fn status(&mut self, request_id: RequestId) -> ApprovalStatus {
        self.expire_stale();
        match self.requests.get(&request_id) {
            Some(req) => req.status.clone(),
            None => ApprovalStatus::Denied {
                reason: "unknown_request".into(),
            },
        }
    }

    /// Public balance, key-less. `shielded_wei` is 0 until T-Privacy.
    ///
    /// With `verified-reads` on (the default), the read goes through the daemon's own
    /// embedded Helios light client (built lazily here) and is tagged
    /// [`ReadStatus::Verified`] only when a fresh Helios head backs it. If Helios isn't
    /// up / the head is stale / the read fails, the value is tagged `Unsynced` — we
    /// **stop the old silent `.unwrap_or(ZERO)`-as-truth**: a 0 is no longer reported as
    /// a trusted balance. With the feature off, the read goes through the raw RPC and is
    /// always tagged `Unsynced("verification disabled")` — never `Verified`.
    ///
    /// A locked daemon doesn't know which address to read, so it reports zeros tagged
    /// `Unsynced("locked")` — honest non-verification, not a trusted zero.
    async fn balance(&mut self, _shielded: bool) -> BalanceReport {
        self.rollover();
        let addr = match &self.state {
            VaultState::Unlocked { address, .. } => *address,
            VaultState::Locked => {
                return BalanceReport {
                    public_wei: U256::ZERO,
                    shielded_wei: U256::ZERO,
                    read_status: ReadStatus::unsynced("locked"),
                }
            }
        };

        let (public_wei, read_status) = self.read_public_balance(addr).await;
        BalanceReport {
            public_wei,
            shielded_wei: U256::ZERO,
            read_status,
        }
    }

    /// Resolve the read endpoint + trust label, then read the native balance. Verified
    /// path: reuse the embedded Helios client (already primed off the daemon lock by the
    /// server — see [`HeliosCell`]), read through its localhost server, and label by head
    /// freshness. Feature-off / Helios-down: read the raw RPC, label `Unsynced`. NEVER
    /// returns `Verified` without a fresh Helios-verified read.
    ///
    /// The configured rpc_url is the EXECUTION-layer endpoint Helios proves against (must
    /// serve eth_getProof); Nimbus drives the CL sync (deckard-core's default).
    #[cfg(feature = "verified-reads")]
    async fn read_public_balance(&mut self, addr: Address) -> (U256, ReadStatus) {
        // The server primes the cell off-lock before dispatch, so this is normally a fast
        // already-built borrow. `ensure` here is a defensive no-op fallback for callers
        // (e.g. unit tests) that drive `handle` directly without the server priming first.
        let (cl, el, data_dir) = self.helios_bootstrap_args();
        self.helios.ensure(cl, &el, data_dir).await;

        let guard = self.helios.inner.lock().await;
        let reader = match guard.as_ref() {
            Some(reader) => reader,
            None => {
                // Helios isn't up. Read the raw RPC so a value can be shown, but tag it
                // Unsynced — we do NOT claim a raw read is Verified.
                drop(guard);
                let wei = signing::read_balance(&self.cfg.rpc_url, addr)
                    .await
                    .unwrap_or(U256::ZERO);
                return (wei, ReadStatus::unsynced("helios unavailable"));
            }
        };
        // Read the value FIRST, then derive its freshness label, so a `Verified` tag is
        // bound to a head observed *after* the value came back (consistent with the
        // app-side path in deckard-core::eth). A small TOCTOU window remains between the
        // two round-trips, but it always fails toward "fresh head backed the value".
        let read_url = reader.localhost_url().to_string();
        match signing::read_balance(&read_url, addr).await {
            Ok(wei) => {
                // head_status() re-probes Helios freshness; a head gone stale → Unsynced.
                let status = reader.head_status().await;
                (wei, status)
            }
            Err(e) => (
                U256::ZERO,
                ReadStatus::unsynced(format!("verified read failed: {}", one_line(&e))),
            ),
        }
    }

    /// Feature-off path: read the raw RPC directly, always tagged Unsynced — never claim
    /// a raw read is Verified.
    #[cfg(not(feature = "verified-reads"))]
    async fn read_public_balance(&mut self, addr: Address) -> (U256, ReadStatus) {
        match signing::read_balance(&self.cfg.rpc_url, addr).await {
            Ok(wei) => (wei, ReadStatus::unsynced("verification disabled")),
            Err(e) => (
                U256::ZERO,
                ReadStatus::unsynced(format!("read failed: {}", one_line(&e))),
            ),
        }
    }

    /// Expire any non-broadcast request past its TTL — both `Pending` (the card was never
    /// answered) and `Allowed` (an approval/auto-allow that went stale). So a stale id can
    /// never be executed later, matching the frozen `ApprovalStatus::Expired` guarantee.
    fn expire_stale(&mut self) {
        let now = Instant::now();
        for req in self.requests.values_mut() {
            if req.broadcast.is_none()
                && matches!(
                    req.status,
                    ApprovalStatus::Pending | ApprovalStatus::Allowed
                )
                && now >= req.expires_at
            {
                req.status = ApprovalStatus::Expired;
            }
        }
    }

    /// Reset the daily spend window when the UTC day ticks over.
    fn rollover(&mut self) {
        let today = current_utc_day();
        if today != self.spent_day {
            self.spent_day = today;
            self.policy.spent_today_wei = U256::ZERO;
        }
    }
}

/// Collapse a multi-line error into one short line for a `reason` string (never includes a
/// secret — broadcast/signing errors carry only addresses/amounts/RPC text).
fn one_line(e: &anyhow::Error) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}
