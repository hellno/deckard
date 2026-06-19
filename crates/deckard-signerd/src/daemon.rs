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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alloy_primitives::{Address, Bytes, B256, U256};
#[cfg(feature = "verified-reads")]
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroizing;

use deckard_contract::{
    deny_reasons, evaluate, evaluate_order, ActivityLifecycle, ActivityRecord, ApprovalStatus,
    BalanceReport, BreachedLimit, Decision, ExecuteResult, Intent, IntentKind, PendingPayloadView,
    PendingRecord, Policy, ProposalOrigin, ReadStatus, RequestId, SignOrderResult, SignerRequest,
    SignerResponse, SwapOrder, UnlockOutcome,
};
// Only the `shield`-gated view-grant handler constructs this; an unconditional import would
// warn in the no-default-features build (e.g. deckard-mcp's dependency edge).
#[cfg(feature = "shield")]
use deckard_contract::RailgunViewGrant;
use deckard_core::{UnlockedVault, Vault};

use crate::config::Config;
use crate::policy_store::{self, current_utc_day};
use crate::request_id::{request_id_for, request_id_for_order};
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

/// The Railgun **RelayAdapt** contract a native `Shield` intent MUST target on `chain_id`.
///
/// The policy gate defers this check by charter (the contract crate is chain-blind and has
/// no railgun dependency), and without it a within-cap `Intent{kind: Shield}` carrying junk
/// calldata to ANY address would be signed — calldata-non-empty alone doesn't pin the
/// target. The daemon owns the pre-check; this table is the daemon's (railgun-free) source
/// of truth, pinned against `railgun::chain_config` by a parity test in
/// `tests/shield_target.rs` so the two can never drift.
///
/// Addresses sourced from Railgun's published network config (the same source
/// `railgun::chain_config` cites):
/// <https://github.com/Railgun-Community/shared-models/blob/main/src/models/network-config.ts>
fn relay_adapt(chain_id: u64) -> Option<Address> {
    use alloy_primitives::address;
    match chain_id {
        // Ethereum mainnet.
        1 => Some(address!("0xAc9f360Ae85469B27aEDdEaFC579Ef2d052aD405")),
        // Sepolia (the demo fork preserves this chain id).
        11155111 => Some(address!("0x7e3d929EbD5bDC84d02Bd3205c777578f33A214D")),
        // Unknown chain → no known adapter → a Shield there is always refused.
        _ => None,
    }
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

/// What a tracked request carries: a plain transaction [`Intent`] (Send / Shield / admitted
/// shaped-approve), or a swap [`SwapOrder`] proposed via `propose_order`. ONE request table
/// serves both so `lock`/`resolve`/`status`/`expire_stale` stay payload-agnostic (they read
/// only `status`/`broadcast`/`expires_at`). An `Order` is signed via `sign_order` (EIP-712,
/// no broadcast) and cancelled via `cancel_order` (an on-chain `invalidateOrder`); a `Tx` is
/// broadcast via `execute`.
enum PendingPayload {
    Tx(Intent),
    Order(SwapOrder),
}

/// One tracked proposal. `status` is the wire-visible approval state; `broadcast` is `Some`
/// once `execute` (or, for an order, `cancel_order`) has put a tx on the wire (so a second
/// `execute` is idempotently refused). `signature` is `Some` once an `Order` has been signed
/// by `sign_order` (so a second `sign_order` is refused — an order signs exactly once).
/// `approved` is `true` only once a human `Resolve`d it — an *auto*-allow (within-cap) is
/// re-checked against the caps at execute time, while a human-approved overage is not.
struct PendingReq {
    payload: PendingPayload,
    status: ApprovalStatus,
    expires_at: Instant,
    broadcast: Option<B256>,
    signature: Option<Bytes>,
    approved: bool,
    /// Who proposed this record (a foreground human app action vs an autonomous agent).
    /// Inbox-display only — it never affects the policy verdict, the TTL, or signing.
    origin: ProposalOrigin,
    /// Unix epoch **millis** stamped when the record was first proposed — surfaced as
    /// `ActivityRecord::timestamp_ms`. Display-only; `expires_at` (an `Instant`) still drives
    /// the TTL. The wall clock is read once, here, never trusted for security.
    created_ms: u64,
    /// A monotonic insertion sequence — the activity feed sorts by it descending to give a
    /// stable newest-first order (`requests` is an unordered `HashMap`, so it can't).
    seq: u64,
    /// Which fence this proposal breached, recomputed off the verdict path at propose time so
    /// the feed cites the actual cap hit. `None` for a within-cap auto-allow or guardrail hold.
    breached: BreachedLimit,
    /// True once this record's only on-chain broadcast was an `invalidateOrder` CANCEL (a STOP
    /// pass or an explicit `cancel_order`), not a successful execute. Keeps the feed from reading
    /// a cancelled swap as `Executed` — a cancel is the opposite of the order going through.
    cancelled: bool,
    /// True ONLY when this record was auto-allowed hands-free at propose time (within cap, off
    /// mainnet). A mainnet-guardrail hold and an over-cap card are both `false` (a human is in the
    /// loop) even though neither breached a cap — so the feed can say "auto-approved within cap"
    /// vs "you approved" HONESTLY, instead of inferring it from the (absent) breach reason.
    auto_allowed: bool,
}

/// Upper bound on a single broadcast round-trip. A hung/blackholed RPC fails after this
/// rather than wedging the daemon (and STOP) forever behind the held state lock.
const BROADCAST_TIMEOUT: Duration = Duration::from_secs(30);

/// Cap on the activity feed the daemon returns — the newest this many records. Session-scoped
/// in memory, so this just bounds the response size; the underlying `requests` table is itself
/// cleared on every `Unlock`.
const ACTIVITY_FEED_CAP: usize = 200;

/// Which channel a request arrived on — the load-bearing distinction for resolver
/// authentication (PRD-01). Same-uid peer-cred proves *who* connected, never *which role*;
/// the authority to approve is carried by possession of the private capability channel the
/// daemon inherits from the supervising app, NOT by the wire frame. So only [`Channel::Control`]
/// may `Resolve`; the public proposer socket can propose/read/execute/STOP but never approve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    /// The public, same-uid proposer socket (`socket.rs`). Anything but `Resolve`.
    Public,
    /// The private capability channel handed only to the supervised app (`supervise.rs` →
    /// inherited `socketpair` end). The sole channel that may `Resolve`.
    Control,
}

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
                eprintln!(
                    "signerd: helios bootstrap failed (reads tagged unsynced): {}",
                    one_line(&e)
                );
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
    /// Monotonic counter handed to each new request's `seq`, so the activity feed can recover
    /// insertion order from the unordered `requests` map. Never reset (a re-unlock clears
    /// `requests`, but a fresh `seq` window only ever moves forward — order stays correct).
    seq_counter: u64,
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
            seq_counter: 0,
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
    ///
    /// `channel` is the resolver-authentication gate (PRD-01): a `Resolve` is honoured only on
    /// [`Channel::Control`]. Every other request behaves identically on both channels — STOP
    /// (`Lock`/`RevokeAll`) deliberately stays reachable everywhere, since it only *reduces*
    /// authority.
    pub async fn handle(&mut self, req: SignerRequest, channel: Channel) -> SignerResponse {
        match req {
            SignerRequest::Unlock { passphrase } => {
                SignerResponse::Unlock(self.unlock(passphrase).await)
            }
            // Lock and RevokeAll are unified in v1: zeroize the key → Locked, deny everything
            // in flight, and — the STOP guarantee — best-effort cancel any already-signed swap
            // order ON-CHAIN before the key is gone. Only a fresh Unlock re-arms. Accepted on
            // EITHER channel: the panic brake must never depend on the capability handshake.
            SignerRequest::Lock | SignerRequest::RevokeAll => {
                self.stop().await;
                SignerResponse::Ack
            }
            SignerRequest::Resolve {
                request_id,
                approved,
            } => {
                // Resolver authentication: approval authority lives on the private capability
                // channel ONLY. A `Resolve` on the public proposer socket — the textbook
                // same-uid self-approval (THREAT-MODEL residual #1) — is refused with a typed,
                // payload-free denial; the pending record is left untouched.
                if channel != Channel::Control {
                    return SignerResponse::Decision(Decision::Deny {
                        reason: deny_reasons::RESOLVE_NOT_AUTHORIZED.into(),
                    });
                }
                self.resolve(request_id, approved);
                SignerResponse::Ack
            }
            SignerRequest::Propose { intent, origin } => {
                SignerResponse::Decision(self.propose(&intent, origin))
            }
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
                    reason: deny_reasons::LOCKED.into(),
                }),
            },
            SignerRequest::Balance { shielded } => {
                SignerResponse::Balance(self.balance(shielded).await)
            }
            SignerRequest::RailgunViewGrant { chain_id, index } => {
                self.railgun_view_grant(chain_id, index)
            }
            SignerRequest::ProposeOrder { order, origin } => {
                SignerResponse::Decision(self.propose_order(&order, origin))
            }
            SignerRequest::SignOrder { request_id } => {
                SignerResponse::SignOrder(self.sign_order(request_id).await)
            }
            SignerRequest::CancelOrder { request_id } => {
                SignerResponse::Execute(self.cancel_order(request_id).await)
            }
            SignerRequest::PendingList => SignerResponse::Pending(self.pending_list()),
            SignerRequest::ActivityFeed => SignerResponse::Activity(self.activity_feed()),
        }
    }

    /// Export the read-only Railgun view grant (0zk address + viewing key) for the unlocked
    /// vault. Refuses if locked, and — crucially — if the derivation known-answer test fails:
    /// a grant from an unverified derivation would let the app show a wrong/silent-$0 private
    /// balance. The spending key never leaves the daemon.
    #[cfg(feature = "shield")]
    fn railgun_view_grant(&self, chain_id: u64, index: u32) -> SignerResponse {
        let vault = match &self.state {
            VaultState::Unlocked { vault, .. } => vault,
            VaultState::Locked => {
                return SignerResponse::Decision(Decision::Deny {
                    reason: deny_reasons::LOCKED.into(),
                })
            }
        };
        if !deckard_core::known_answer_ok() {
            return SignerResponse::Decision(Decision::Deny {
                reason: deny_reasons::DERIVATION_UNVERIFIED.into(),
            });
        }
        match vault.railgun_view_grant(chain_id, index) {
            Ok((address, viewing_key)) => SignerResponse::RailgunView(RailgunViewGrant {
                address,
                viewing_key,
            }),
            Err(e) => SignerResponse::Decision(Decision::Deny {
                reason: deny_reasons::railgun_keys(one_line(&e)),
            }),
        }
    }

    /// Without the `shield` feature there is no Railgun derivation to grant.
    #[cfg(not(feature = "shield"))]
    fn railgun_view_grant(&self, _chain_id: u64, _index: u32) -> SignerResponse {
        SignerResponse::Decision(Decision::Deny {
            reason: deny_reasons::SHIELD_UNAVAILABLE.into(),
        })
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
                    reason: deny_reasons::REVOKED.into(),
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
                        reason: deny_reasons::USER_DENIED.into(),
                    };
                }
            }
        }
    }

    /// Policy check only — NEVER signs. Process-level pre-checks first, then the shared
    /// `evaluate`. On `NeedsApproval`/`Allow` a pending record is stored under the intent's
    /// deterministic id; on `Deny` nothing is stored. `origin` is threaded into the stored
    /// record for the inbox display only; it never changes the verdict.
    fn propose(&mut self, intent: &Intent, origin: ProposalOrigin) -> Decision {
        self.rollover();
        self.expire_stale();

        // Pre-checks the Policy can't express (the mock has none of these states, which is
        // why feeding both the same (Intent, Policy) yields identical decisions — the parity
        // contract). These run before `evaluate`.
        // Chain check FIRST, before the lock gate: it needs no key, so a wrong-chain daemon
        // answers `chain_mismatch` even while locked. This makes the MCP sidecar's connect-time
        // chain probe conclusive (instead of inconclusive-when-locked) — otherwise a sidecar
        // attached to a locked wrong-chain daemon could pass its probe and e.g. read that
        // daemon's policy via PolicyGet (which deliberately succeeds while locked). It also
        // means a `locked` deny now implies the chain matched.
        if intent.chain_id != self.cfg.chain_id {
            return Decision::Deny {
                reason: deny_reasons::CHAIN_MISMATCH.into(),
            };
        }
        if matches!(self.state, VaultState::Locked) {
            return Decision::Deny {
                reason: deny_reasons::LOCKED.into(),
            };
        }
        // SHAPED APPROVE (swap v1): a `ContractCall` carrying an exact `approve(spender,amount)`
        // to the order's sell token is admitted — but ONLY when it's the GPv2 vault relayer for
        // the EXACT sell amount of a stored, matching order. This is the only ContractCall v1
        // signs; everything else still falls through to `unsupported_v1`. We intercept BEFORE
        // the kind guard so a non-approve ContractCall keeps its existing `unsupported_v1`
        // reason and Send/Shield behaviour is byte-identical. On admission we fall through to
        // the normal Send caps path below (the broadcast carries `intent.calldata` as-is).
        if intent.kind == IntentKind::ContractCall && intent.token.is_none() {
            if let Some((spender, amount)) = deckard_core::decode_approve(&intent.calldata) {
                if let Some(deny) = self.shaped_approve_admission(intent, spender, amount) {
                    return deny;
                }
                // Admitted: an allowance tx ALWAYS raises a human card (it is part of the swap,
                // and "every swap raises an approval card"). Pass `always_needs_card = true` so it
                // is stored `Pending` and NEVER auto-broadcast — even off mainnet, where the Send
                // caps path would otherwise auto-allow a value-0 ContractCall hands-free.
                return self.finish_propose(intent, true, origin);
            }
        }
        // v1 admits a native Send and a Shield (the privacy hero). The Shield's RelayAdapt
        // calldata is built key-less in deckard-core and rides in `intent.calldata`; the
        // daemon never sees the ZK crate, it only signs+broadcasts the handed bytes. Unshield
        // / ContractCall stay a fast-follow.
        if !matches!(intent.kind, IntentKind::Send | IntentKind::Shield) {
            return Decision::Deny {
                reason: deny_reasons::UNSUPPORTED_V1.into(),
            };
        }
        // v1 spine is native ETH only; an ERC-20 (`token = Some`) Send is a fast-follow.
        // A native shield is `token: None` (the value rides as msg.value via RelayAdapt
        // wrapBase), so it passes this guard.
        if intent.token.is_some() {
            return Decision::Deny {
                reason: deny_reasons::ERC20_UNSUPPORTED_V1.into(),
            };
        }
        // A Shield must target the chain's RelayAdapt contract. The contract crate's policy
        // gate deliberately can't express this (it is chain-blind); without the pre-check a
        // within-cap "Shield" to an arbitrary address would be signed (see [`relay_adapt`]).
        if intent.kind == IntentKind::Shield && relay_adapt(intent.chain_id) != Some(intent.to) {
            return Decision::Deny {
                reason: deny_reasons::SHIELD_TO_MISMATCH.into(),
            };
        }

        self.finish_propose(intent, false, origin)
    }

    /// The shared tail of `propose`: derive the id, honour an idempotent re-propose, decide the
    /// stored status, and store a `Tx` record. Reached from the Send/Shield path AND the admitted
    /// shaped-approve path (both broadcast `intent.calldata` as-is via `execute`).
    ///
    /// `always_needs_card`: when `true` (the shaped-approve path) the record is stored `Pending`
    /// unconditionally — the allowance tx is part of the swap and must raise a human card, never
    /// auto-broadcast (off mainnet the Send caps path would otherwise auto-allow a value-0
    /// ContractCall hands-free). The shaped-approve prechecks already are its policy gate, so we
    /// skip `evaluate` (allowlist/caps gate value transfers, not the relayer approval) and only
    /// honour the STOP brake. When `false` (Send/Shield) the ONE shared `evaluate` (+ mainnet
    /// guardrail) decides as before.
    ///
    /// `origin` is stored on the new record for the inbox display only — it never changes the
    /// stored status or the verdict.
    fn finish_propose(
        &mut self,
        intent: &Intent,
        always_needs_card: bool,
        origin: ProposalOrigin,
    ) -> Decision {
        let id = request_id_for(intent);

        // Idempotent re-propose: an identical intent maps to the same id, so an existing record
        // is returned AS-IS — a re-propose can't reset a `Pending` card's TTL, downgrade a
        // human approval, or re-raise a `Denied`/`Expired` request. Retrying a terminal
        // (already-broadcast) Send/Shield needs a fresh session (`Unlock` clears the table) — that
        // strict replay guard is what stops a double-spend of a fund-moving tx.
        if let Some(existing) = self.requests.get(&id) {
            // A shaped relayer-approve is the ONE exception. It moves no funds (an ERC-20
            // `approve` is idempotent on-chain) and is re-issued for EVERY swap, so a prior
            // already-broadcast approve must NOT permanently block an identical later swap — a
            // user must be able to swap the same amount any number of times. Once the previous
            // approve is on the wire, start a FRESH approval cycle by falling through to overwrite
            // the record below. It stays gated by a matching pending order
            // (`shaped_approve_admission`) and the human hold, so this is never a hands-free
            // re-broadcast. Fund-moving Send/Shield intents keep the strict replay guard.
            let fresh_approve_cycle = always_needs_card && existing.broadcast.is_some();
            if !fresh_approve_cycle {
                return match &existing.status {
                    _ if existing.broadcast.is_some() => Decision::Deny {
                        reason: deny_reasons::ALREADY_EXECUTED.into(),
                    },
                    ApprovalStatus::Pending => Decision::NeedsApproval { request_id: id },
                    ApprovalStatus::Allowed => Decision::Allow,
                    ApprovalStatus::Denied { reason } => Decision::Deny {
                        reason: reason.clone(),
                    },
                    ApprovalStatus::Expired => Decision::Deny {
                        reason: deny_reasons::EXPIRED.into(),
                    },
                };
            }
        }

        // No record yet: decide the stored status.
        //
        // The shaped-approve path forces a card (see the fn doc): store `Pending` after only the
        // STOP brake check — never auto-allow an allowance tx.
        //
        // Otherwise the ONE shared decision function decides, with the mainnet guardrail
        // (post-`evaluate`, mock/daemon parity carve-out): on chain 1, unless the operator set
        // the override (see `Config::mainnet_override` — its env var is documented only in
        // THREAT-MODEL.md and must never appear in a reason), EVERY auto-Allow is downgraded to
        // `NeedsApproval`. The default policy is `OverCap` with an empty (= any-recipient)
        // allowlist, so without this a prompt-injected client could move real funds hands-free
        // within the caps. A human resolver (the app's hold-to-confirm) flips it to `Allowed` via
        // `Resolve`. Like `locked`/`chain_mismatch`, this is a process-level check the pure policy
        // can't express — the parity contract with `MockSigner` covers `evaluate` only.
        let status = if always_needs_card {
            if self.policy.revoked {
                return Decision::Deny {
                    reason: deny_reasons::REVOKED.into(),
                };
            }
            ApprovalStatus::Pending
        } else {
            match evaluate(intent, &self.policy) {
                deny @ Decision::Deny { .. } => return deny,
                Decision::Allow if self.mainnet_guardrail_active() => {
                    eprintln!(
                        "signerd: mainnet guardrail — auto-allow downgraded to NeedsApproval \
                         (approve in the Deckard app)"
                    );
                    ApprovalStatus::Pending
                }
                Decision::Allow => ApprovalStatus::Allowed,
                Decision::NeedsApproval { .. } => ApprovalStatus::Pending,
            }
        };
        // Cite the breached fence for the feed (display-only; recomputed off the verdict path).
        // `None` for a within-cap auto-allow or a guardrail-downgraded hold. The shaped-approve
        // card (`always_needs_card`) cites NO cap: its `intent.to` is the ERC-20 token, which the
        // value-transfer `allow_to` does not gate, so running `breach_for` on it would mis-cite
        // OffAllowlist — match the swap order record, which also stores `None`.
        let breached = if always_needs_card {
            BreachedLimit::None
        } else {
            breach_for(intent, &self.policy)
        };
        // Hands-free ONLY when the genuine within-cap auto-allow stored `Allowed` (the mainnet
        // guardrail and over-cap both store `Pending`, the shaped-approve card forces `Pending`),
        // so a later human `Resolve` to `Allowed` never flips this true.
        let auto_allowed = status == ApprovalStatus::Allowed;
        let seq = self.next_seq();
        self.requests.insert(
            id,
            PendingReq {
                payload: PendingPayload::Tx(intent.clone()),
                status: status.clone(),
                expires_at: Instant::now() + self.approval_ttl,
                broadcast: None,
                signature: None,
                approved: false,
                origin,
                created_ms: now_ms(),
                seq,
                breached,
                cancelled: false,
                auto_allowed,
            },
        );

        match status {
            ApprovalStatus::Allowed => Decision::Allow,
            _ => Decision::NeedsApproval { request_id: id },
        }
    }

    /// Shaped-approve admission prechecks. Returns `Some(Deny)` if the (already-decoded)
    /// `approve(spender, amount)` to `intent.to` is NOT an admissible swap approval, or `None`
    /// when it should be admitted (the caller then runs the normal caps path). Pure over the
    /// daemon's stored orders + the GPv2 relayer constant — no key, no I/O.
    ///
    /// Admission requires: the approve carries NO ETH value, the spender is the GPv2 vault
    /// relayer, AND there is a stored swap `Order` whose `sell_token == intent.to` and
    /// `sell_amount == amount` (EXACT amount only — no unbounded/infinite approval). Distinct
    /// reasons per failure so a client can tell which invariant it tripped.
    fn shaped_approve_admission(
        &self,
        intent: &Intent,
        spender: Address,
        amount: U256,
    ) -> Option<Decision> {
        // A legitimate ERC-20 `approve` never carries ETH. Reject a value-bearing approve: the
        // admitted-approve path skips the Send caps check (`finish_propose(.., true)`), and the
        // human card renders only `{token, spender, amount}` (`PendingPayloadView::Approve`), so a
        // non-zero `value` would move ETH to the token contract on Resolve while staying invisible
        // on the card. Bounding it here is the only place the value is gated.
        if intent.value != U256::ZERO {
            return Some(Decision::Deny {
                reason: deny_reasons::APPROVE_WITH_VALUE.into(),
            });
        }
        if spender != deckard_core::GPV2_VAULT_RELAYER {
            return Some(Decision::Deny {
                reason: deny_reasons::APPROVE_WRONG_SPENDER.into(),
            });
        }
        // The order must be LIVE — still `Pending`, awaiting its human hold. A Denied / Expired /
        // already-signed order with a matching sell token + amount must NOT admit a fresh approve
        // card: a new swap brings its OWN pending order. (Tightens the "matching pending order"
        // invariant; matters more now that `finish_propose` lets a repeated approve start a fresh
        // cycle — without this, a stale completed order could keep admitting approves.)
        let has_matching_order = self.requests.values().any(|req| match &req.payload {
            PendingPayload::Order(order) => {
                matches!(req.status, ApprovalStatus::Pending)
                    && order.sell_token == intent.to
                    && order.sell_amount == amount
            }
            PendingPayload::Tx(_) => false,
        });
        if !has_matching_order {
            return Some(Decision::Deny {
                reason: deny_reasons::APPROVE_NO_MATCHING_ORDER.into(),
            });
        }
        None
    }

    /// Propose a swap [`SwapOrder`] — policy check only, NEVER signs. Mirrors `propose`'s
    /// process-level pre-checks (chain, locked) then defers to the pure `evaluate_order`. The
    /// `owner` and `receiver` are BOUND to the unlocked wallet here; the client's `owner` is
    /// never trusted (a client-supplied owner could otherwise make the daemon sign an order
    /// that pays out to someone else). A valid order is ALWAYS `NeedsApproval` (swaps never
    /// auto-allow in v1). The record is stored `Pending` under [`request_id_for_order`].
    ///
    /// `origin` records WHO proposed (a foreground human GUI swap vs an agent) for the inbox/feed
    /// display only — it never affects the verdict. A user-driven GUI swap passes `App` so the
    /// order row reads "You", not "Atlas".
    fn propose_order(&mut self, order: &SwapOrder, origin: ProposalOrigin) -> Decision {
        self.rollover();
        self.expire_stale();

        // Chain check first (key-less, like `propose`), so a wrong-chain daemon answers
        // `chain_mismatch` even while locked.
        if order.chain_id != self.cfg.chain_id {
            return Decision::Deny {
                reason: deny_reasons::CHAIN_MISMATCH.into(),
            };
        }
        // We need the unlocked wallet to bind owner/receiver, so a locked daemon can't propose.
        let wallet = match &self.state {
            VaultState::Unlocked { address, .. } => *address,
            VaultState::Locked => {
                return Decision::Deny {
                    reason: deny_reasons::LOCKED.into(),
                }
            }
        };

        // BIND owner to the unlocked wallet — never trust the client's `owner`. (The
        // receiver/owner equality is enforced by `evaluate_order`, which denies
        // `receiver_not_wallet`; binding the owner keeps the signed order's owner == signer.)
        let mut bound = order.clone();
        bound.owner = wallet;
        let id = request_id_for_order(&bound);

        // Idempotent re-propose: the same bound order maps to the same id, so an existing record
        // is returned AS-IS (payload-agnostic — same contract as `propose`).
        if let Some(existing) = self.requests.get(&id) {
            return match &existing.status {
                _ if existing.broadcast.is_some() => Decision::Deny {
                    reason: deny_reasons::ALREADY_EXECUTED.into(),
                },
                ApprovalStatus::Pending => Decision::NeedsApproval { request_id: id },
                ApprovalStatus::Allowed => Decision::Allow,
                ApprovalStatus::Denied { reason } => Decision::Deny {
                    reason: reason.clone(),
                },
                ApprovalStatus::Expired => Decision::Deny {
                    reason: deny_reasons::EXPIRED.into(),
                },
            };
        }

        // The pure swap decision (never returns Allow → always Pending on success).
        match evaluate_order(&bound, &self.policy, wallet, now_secs()) {
            deny @ Decision::Deny { .. } => deny,
            Decision::NeedsApproval { .. } | Decision::Allow => {
                let seq = self.next_seq();
                self.requests.insert(
                    id,
                    PendingReq {
                        payload: PendingPayload::Order(bound),
                        status: ApprovalStatus::Pending,
                        expires_at: Instant::now() + self.approval_ttl,
                        broadcast: None,
                        signature: None,
                        approved: false,
                        // WHO proposed (display-only). A GUI swap is App-origin → the feed reads
                        // "You", not "Atlas"; an agent-driven order (future sidecar path) passes Agent.
                        origin,
                        created_ms: now_ms(),
                        seq,
                        // A swap card cites no spend cap (orders gate on token/receiver/validity,
                        // not the ETH per-tx/daily caps), so the feed shows no breached-limit line.
                        breached: BreachedLimit::None,
                        cancelled: false,
                        // Swaps never auto-allow in v1 — always a human card.
                        auto_allowed: false,
                    },
                );
                Decision::NeedsApproval { request_id: id }
            }
        }
    }

    /// Sign a stored, approved swap order's EIP-712 digest → a 65-byte signature. NO HTTP, NO
    /// broadcast (the app/MCP posts the signed order to the CoW orderbook). Re-checks `revoked`
    /// at sign time (TOCTOU) so an order approved before a STOP is still refused. An order
    /// signs at most once (`signature.is_some()` → `already_signed`).
    async fn sign_order(&mut self, request_id: RequestId) -> SignOrderResult {
        self.rollover();
        self.expire_stale();

        // Phase 1 (borrows end before the mutation below): eligibility + TOCTOU re-check, then
        // compute the digest over the STORED (bound) order and extract the raw scalar.
        let (digest, scalar) = {
            // STOP landed first — refuse even a previously-approved order.
            let vault = match &self.state {
                VaultState::Locked => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::REVOKED.into(),
                    }
                }
                VaultState::Unlocked { vault, .. } => vault,
            };
            let req = match self.requests.get(&request_id) {
                None => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::UNKNOWN_REQUEST.into(),
                    }
                }
                Some(req) => req,
            };
            let order = match &req.payload {
                PendingPayload::Order(order) => order,
                PendingPayload::Tx(_) => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::NOT_AN_ORDER.into(),
                    }
                }
            };
            if req.signature.is_some() {
                return SignOrderResult::Denied {
                    reason: deny_reasons::ALREADY_SIGNED.into(),
                };
            }
            match &req.status {
                ApprovalStatus::Allowed => {}
                ApprovalStatus::Pending => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::NOT_APPROVED.into(),
                    }
                }
                ApprovalStatus::Denied { reason } => {
                    return SignOrderResult::Denied {
                        reason: reason.clone(),
                    }
                }
                ApprovalStatus::Expired => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::EXPIRED.into(),
                    }
                }
            }
            // TOCTOU: a STOP can trip `revoked` between approval and sign without changing this
            // record's status if the brake landed via a policy path; refuse on the live flag.
            if self.policy.revoked {
                return SignOrderResult::Denied {
                    reason: deny_reasons::REVOKED.into(),
                };
            }
            let digest = deckard_core::order_digest(order);
            let signer = match vault.account_signer(0) {
                Ok(s) => s,
                Err(e) => {
                    return SignOrderResult::Denied {
                        reason: deny_reasons::signer_error(one_line(&e)),
                    }
                }
            };
            // Only the version-stable raw scalar crosses into our alloy stack; zeroized on drop.
            (digest, Zeroizing::new(signer.to_bytes().0))
        };

        // Phase 2: sign the digest offline (no network), then pin the signature on the record.
        let sig = match signing::sign_order_digest(scalar.as_slice(), digest) {
            Ok(sig) => sig,
            Err(e) => {
                return SignOrderResult::Denied {
                    reason: deny_reasons::sign_failed(one_line(&e)),
                }
            }
        };
        let signature = Bytes::from(sig.to_vec());
        if let Some(req) = self.requests.get_mut(&request_id) {
            req.signature = Some(signature.clone());
        }
        SignOrderResult::Signed { signature }
    }

    /// Broadcast an `invalidateOrder` cancel for a stored swap order (an on-chain cancellation
    /// at the GPv2 settlement contract). Requires an unlocked wallet (the owner must sign the
    /// cancel). Used both by an explicit `CancelOrder` and by the STOP pass (see [`stop`]).
    async fn cancel_order(&mut self, request_id: RequestId) -> ExecuteResult {
        self.rollover();
        // Look up the order and confirm it IS an order before touching the key. (The shared
        // `cancel_one_order` re-checks this, but answering `unknown_request`/`not_an_order`
        // here keeps the CancelOrder reasons precise for a non-order id.)
        match self.requests.get(&request_id) {
            None => {
                return ExecuteResult::Denied {
                    reason: deny_reasons::UNKNOWN_REQUEST.into(),
                }
            }
            Some(req) if !matches!(req.payload, PendingPayload::Order(_)) => {
                return ExecuteResult::Denied {
                    reason: deny_reasons::NOT_AN_ORDER.into(),
                }
            }
            Some(_) => {}
        }
        self.cancel_one_order(request_id).await
    }

    /// The on-chain cancel for ONE already-resolved swap order, shared by `cancel_order` and
    /// the STOP pass. The caller has confirmed the request exists and is an `Order`. Extracts
    /// the scalar, builds `invalidateOrder(uid)` calldata key-lessly via deckard-core, and
    /// broadcasts to the settlement contract (value 0) under [`BROADCAST_TIMEOUT`] so a dead
    /// RPC can never wedge the daemon (and STOP) behind the held lock.
    async fn cancel_one_order(&mut self, request_id: RequestId) -> ExecuteResult {
        // Phase 1 (lock held): require unlocked, extract the order params + the scalar.
        let (calldata, scalar) = {
            let vault = match &self.state {
                VaultState::Locked => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::REVOKED.into(),
                    }
                }
                VaultState::Unlocked { vault, .. } => vault,
            };
            let order = match self.requests.get(&request_id) {
                Some(req) => match &req.payload {
                    PendingPayload::Order(order) => order,
                    PendingPayload::Tx(_) => {
                        return ExecuteResult::Denied {
                            reason: deny_reasons::NOT_AN_ORDER.into(),
                        }
                    }
                },
                None => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::UNKNOWN_REQUEST.into(),
                    }
                }
            };
            let digest = deckard_core::order_digest(order);
            let uid = deckard_core::order_uid(digest, order.owner, order.valid_to);
            let calldata = deckard_core::build_invalidate_order_calldata(&uid);
            let signer = match vault.account_signer(0) {
                Ok(s) => s,
                Err(e) => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::signer_error(one_line(&e)),
                    }
                }
            };
            let scalar = Zeroizing::new(signer.to_bytes().0);
            (calldata, scalar)
        };

        // Phase 2: broadcast the cancel (lock held — serialized), bounded by the timeout.
        let broadcast = signing::broadcast_intent(
            scalar.as_slice(),
            &self.cfg.rpc_url,
            self.cfg.chain_id,
            deckard_core::GPV2_SETTLEMENT,
            U256::ZERO,
            &calldata,
        );
        let tx_hash = match tokio::time::timeout(BROADCAST_TIMEOUT, broadcast).await {
            Ok(Ok(hash)) => hash,
            Ok(Err(e)) => {
                return ExecuteResult::Denied {
                    reason: deny_reasons::broadcast_failed(one_line(&e)),
                }
            }
            Err(_elapsed) => {
                return ExecuteResult::Denied {
                    reason: deny_reasons::BROADCAST_TIMEOUT.into(),
                }
            }
        };

        // Record the cancel tx hash (pins the record; a second cancel/execute is refused). Mark
        // it `cancelled` so the activity feed reads this as a STOP/cancel, NOT a successful
        // execute — the cancel hash is an `invalidateOrder`, the opposite of the order landing.
        if let Some(req) = self.requests.get_mut(&request_id) {
            req.broadcast = Some(tx_hash);
            req.cancelled = true;
        }
        ExecuteResult::Broadcast { tx_hash }
    }

    /// STOP (`Lock` / `RevokeAll`, unified in v1): the security brake. BEFORE the key is
    /// zeroized, best-effort cancel every SIGNED, not-yet-cancelled, non-expired swap order
    /// ON-CHAIN — a signed order is already loose on the orderbook, so local-only revocation
    /// can't unsubmit it; only an `invalidateOrder` can. Each cancel is bounded by
    /// [`BROADCAST_TIMEOUT`], so a dead RPC fails fast and STOP stays responsive; errors are
    /// swallowed (logged one redacted line). THEN `lock()` zeroizes the key and flips every
    /// in-flight record to `Denied{revoked}` — the local-authority kill happens regardless of
    /// the on-chain cancel outcome, so `sign_order`/`execute` refuse from here on.
    async fn stop(&mut self) {
        // Collect the order ids to cancel up front (an immutable borrow that ends before the
        // mutable cancel calls). Only SIGNED, un-cancelled orders are loose on the CoW orderbook
        // and worth an on-chain invalidate (an unsigned order was never submitted).
        //
        // The selection (which signed orders are still settleable) is extracted to
        // `select_orders_to_cancel` so it can be unit-tested without a chain — see its doc for why
        // it gates on the order's real `valid_to`, NOT the daemon's local approval-TTL `Expired`.
        let to_cancel = select_orders_to_cancel(&self.requests, now_secs());

        for id in to_cancel {
            // The key is still live here (we lock AFTER this pass). Swallow + log errors so one
            // failed cancel can't block the rest or the lock; the timeout keeps STOP responsive.
            if let ExecuteResult::Denied { reason } = self.cancel_one_order(id).await {
                eprintln!("signerd: STOP could not cancel a signed order on-chain: {reason}");
            }
        }

        // Local authority kill — always, regardless of the on-chain cancel outcomes above.
        self.lock();
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
                        reason: deny_reasons::REVOKED.into(),
                    }
                }
                VaultState::Unlocked { vault, .. } => vault,
            };
            let req = match self.requests.get(&request_id) {
                None => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::UNKNOWN_REQUEST.into(),
                    }
                }
                Some(req) => req,
            };
            if req.broadcast.is_some() {
                return ExecuteResult::Denied {
                    reason: deny_reasons::ALREADY_EXECUTED.into(),
                };
            }
            match &req.status {
                // The only state that signs (covers within-cap Allow + human-approved over-cap).
                ApprovalStatus::Allowed => {}
                ApprovalStatus::Pending => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::NOT_APPROVED.into(),
                    }
                }
                ApprovalStatus::Denied { reason } => {
                    return ExecuteResult::Denied {
                        reason: reason.clone(),
                    }
                }
                ApprovalStatus::Expired => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::EXPIRED.into(),
                    }
                }
            }
            // `execute` only broadcasts a plain transaction; a swap Order is signed via
            // `sign_order` and cancelled via `cancel_order`, never broadcast here. An Order in
            // this table can't reach `execute` through any normal flow, but guard defensively.
            let intent = match &req.payload {
                PendingPayload::Tx(intent) => intent,
                PendingPayload::Order(_) => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::NOT_AN_ORDER.into(),
                    }
                }
            };
            // Spend TOCTOU: an *auto*-allow must still be within policy at sign time, so two
            // within-cap proposals can't both execute past the daily cap (`spent_today` only
            // grows on prior executes). A human-APPROVED request carries explicit consent for
            // its overage and is not re-capped.
            if !req.approved && evaluate(intent, &self.policy) != Decision::Allow {
                return ExecuteResult::Denied {
                    reason: deny_reasons::CAP_EXCEEDED.into(),
                };
            }
            let signer = match vault.account_signer(0) {
                Ok(s) => s,
                Err(e) => {
                    return ExecuteResult::Denied {
                        reason: deny_reasons::signer_error(one_line(&e)),
                    }
                }
            };
            // Only the version-stable raw scalar crosses into our alloy stack; zeroized on drop.
            let scalar = Zeroizing::new(signer.to_bytes().0);
            // Calldata is empty for a native Send (→ broadcast is byte-identical to before) and
            // carries the RelayAdapt call for a Shield (or the shaped approve). The empty-vs-
            // non-empty input IS the native/contract-call discriminator, so no IntentKind branch
            // is needed here.
            (intent.to, intent.value, intent.calldata.clone(), scalar)
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
                    reason: deny_reasons::broadcast_failed(one_line(&e)),
                }
            }
            Err(_elapsed) => {
                return ExecuteResult::Denied {
                    reason: deny_reasons::BROADCAST_TIMEOUT.into(),
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
                reason: deny_reasons::UNKNOWN_REQUEST.into(),
            },
        }
    }

    /// The approval inbox: every in-flight record with its wire-visible payload, for the GUI
    /// to render (child #25). A `Tx` whose calldata decodes as an `approve(spender, amount)`
    /// is surfaced as the structured `Approve { token, spender, amount }` view (so the GUI can
    /// show "approve X to the CoW relayer"); any other `Tx` rides as the raw intent. Returns
    /// regardless of lock state — statuses already reflect `revoked`, and order/intent fields
    /// are not secret (no key, passphrase, or viewing key crosses here).
    fn pending_list(&mut self) -> Vec<PendingRecord> {
        // Expire FIRST so a Pending row past its 120s TTL is never surfaced as still-pending —
        // the inbox sees `Expired` (with `remaining_ms == 0`), matching `status`/`execute`.
        self.expire_stale();
        let now = Instant::now();
        self.requests
            .iter()
            .map(|(id, req)| {
                // Panic-free: `checked_duration_since` can't underflow (a past `expires_at`
                // yields `None` → 0); the `.min(u64::MAX)` clamp makes the `as u64` cast lossless.
                let remaining_ms = match req.status {
                    ApprovalStatus::Pending | ApprovalStatus::Allowed => req
                        .expires_at
                        .checked_duration_since(now)
                        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
                        .unwrap_or(0),
                    ApprovalStatus::Denied { .. } | ApprovalStatus::Expired => 0,
                };
                PendingRecord {
                    request_id: *id,
                    status: req.status.clone(),
                    payload: payload_view(&req.payload),
                    remaining_ms,
                    origin: req.origin,
                }
            })
            .collect()
    }

    /// Hand out the next monotonic insertion sequence for a new request.
    fn next_seq(&mut self) -> u64 {
        let seq = self.seq_counter;
        self.seq_counter = self.seq_counter.wrapping_add(1);
        seq
    }

    /// The **activity feed**: every tracked action as an [`ActivityRecord`], newest-first,
    /// capped at [`ACTIVITY_FEED_CAP`]. Unlike `pending_list`, this is the surface the GUI feed
    /// reads — it retains auto-allowed and executed rows (with their `tx_hash` + `timestamp_ms`)
    /// so the human can see what the agent *did*, not only what is pending.
    ///
    /// Expires stale rows FIRST (matching `pending_list`/`status`), so a lapsed `Pending` card is
    /// never shown as still proposed. The lifecycle is derived from the live record state, so a
    /// resolve / execute / STOP is reflected the next time the feed is read. Like `pending_list`,
    /// it returns regardless of lock state (the statuses already reflect `revoked`; no key,
    /// passphrase, or viewing key crosses here).
    fn activity_feed(&mut self) -> Vec<ActivityRecord> {
        self.expire_stale();
        let mut rows: Vec<(&RequestId, &PendingReq)> = self.requests.iter().collect();
        // Newest-first by insertion sequence (the unordered map can't give this on its own).
        rows.sort_by_key(|(_, req)| std::cmp::Reverse(req.seq));
        rows.into_iter()
            .take(ACTIVITY_FEED_CAP)
            .map(|(id, req)| ActivityRecord {
                request_id: *id,
                origin: req.origin,
                payload: payload_view(&req.payload),
                timestamp_ms: req.created_ms,
                tx_hash: if req.cancelled { None } else { req.broadcast },
                lifecycle: activity_lifecycle(req),
                reason: req.breached,
                auto_allowed: req.auto_allowed,
            })
            .collect()
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
        // Demo / local-fork mode: when verified reads are disabled at runtime
        // (`DECKARD_VERIFIED_READS=0`), skip Helios entirely. Embedded Helios is mainnet-only
        // and re-bootstraps on every failed Balance — against a Sepolia fork it never verifies
        // and stalls the read. Fall back to a raw fork RPC read, honestly tagged Unsynced
        // (matches the feature-off path below; the server skips priming for the same reason).
        if !deckard_core::verified_reads_enabled() {
            return match signing::read_balance(&self.cfg.rpc_url, addr).await {
                Ok(wei) => (
                    wei,
                    ReadStatus::unsynced("verification disabled (demo mode)"),
                ),
                Err(e) => (
                    U256::ZERO,
                    ReadStatus::unsynced(format!("read failed: {}", one_line(&e))),
                ),
            };
        }

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

    /// Whether the chain-1 guardrail is armed: the daemon signs for mainnet and the
    /// operator has NOT set the override. While armed, no auto-Allow exists — every
    /// within-policy write still requires a human `Resolve` (the app's hold-to-confirm).
    fn mainnet_guardrail_active(&self) -> bool {
        self.cfg.chain_id == 1 && !self.cfg.mainnet_override
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

/// STOP's on-chain-cancel selection, extracted from `stop()` so it is unit-testable without a
/// chain. Returns the SIGNED, un-cancelled orders that are STILL settleable at `now` (unix secs)
/// — the ones loose on the CoW orderbook that an `invalidateOrder` must kill before the key is
/// zeroized.
///
/// It gates on the order's REAL `valid_to`, NEVER the daemon's local approval-TTL
/// `ApprovalStatus::Expired`. Those are different clocks: a signed order whose 120s approval
/// record has lapsed is still settleable by a solver until `valid_to` (up to 24h), so STOP must
/// still cancel it. (Skipping locally-`Expired` orders was a STOP-integrity hole — they could
/// settle after the kill switch.) Unsigned orders (never submitted), already-cancelled/broadcast
/// orders, plain `Tx` records, and orders whose `valid_to` is already in the past (genuinely
/// unsettleable — a cancel would be wasted gas) are all skipped.
fn select_orders_to_cancel(requests: &HashMap<RequestId, PendingReq>, now: u64) -> Vec<RequestId> {
    requests
        .iter()
        .filter_map(|(id, req)| {
            let order = match &req.payload {
                PendingPayload::Order(o) => o,
                PendingPayload::Tx(_) => return None,
            };
            let signed = req.signature.is_some();
            let not_cancelled = req.broadcast.is_none();
            let still_settleable = u64::from(order.valid_to) > now;
            (signed && not_cancelled && still_settleable).then_some(*id)
        })
        .collect()
}

/// Map a stored [`PendingPayload`] to its wire [`PendingPayloadView`] for the inbox. A `Tx`
/// whose calldata decodes as an exact `approve(spender, amount)` is surfaced as the structured
/// `Approve` view; any other `Tx` rides as the raw intent; an `Order` rides as-is.
fn payload_view(payload: &PendingPayload) -> PendingPayloadView {
    match payload {
        PendingPayload::Order(order) => PendingPayloadView::Order(order.clone()),
        PendingPayload::Tx(intent) => match deckard_core::decode_approve(&intent.calldata) {
            Some((spender, amount)) => PendingPayloadView::Approve {
                token: intent.to,
                spender,
                amount,
            },
            None => PendingPayloadView::Tx(intent.clone()),
        },
    }
}

/// Wall-clock unix seconds — injected into the pure `evaluate_order` so it stays a pure
/// function of its inputs. Propagates (never `expect`s): a clock before the epoch yields `0`,
/// which `evaluate_order` treats as "now", at worst tightening the `valid_to` window.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Wall-clock unix **millis** — stamped onto a new record as `created_ms` for the activity
/// feed's timestamp. Display-only (the TTL runs off a monotonic `Instant`), so a clock before
/// the epoch harmlessly yields `0`.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Recompute, OFF the frozen verdict path, which fence a proposal breached — so the feed can
/// cite the actual cap hit instead of a hardcoded "over per-tx cap". Mirrors the order of
/// [`deckard_contract::evaluate`]'s own checks (allowlist, then the `spent_today + value`
/// projection against the per-tx then daily cap) WITHOUT changing its single-`over`-bool
/// verdict. Returns [`BreachedLimit::None`] for a within-cap intent (an auto-allow, or a
/// guardrail-downgraded hold) and for the value-0 shaped-approve card.
fn breach_for(intent: &Intent, policy: &Policy) -> BreachedLimit {
    if !policy.allow_to.is_empty() && !policy.allow_to.contains(&intent.to) {
        return BreachedLimit::OffAllowlist;
    }
    let projected = policy.spent_today_wei.saturating_add(intent.value);
    if projected > policy.per_tx_cap_wei {
        BreachedLimit::PerTxCap
    } else if projected > policy.daily_cap_wei {
        BreachedLimit::DailyCap
    } else {
        BreachedLimit::None
    }
}

/// Derive a record's feed lifecycle from its live state. A broadcast tx is `Executed` regardless
/// of status; otherwise the `ApprovalStatus` maps onto the lifecycle. A lapsed `Expired` card maps
/// to its own `ActivityLifecycle::Expired` (NOT `Decided{approved:false}`) so the feed can render
/// it neutrally — no human acted on a window that simply timed out, so it must not carry the amber
/// "you acted" tint that a human denial / STOP revoke does.
fn activity_lifecycle(req: &PendingReq) -> ActivityLifecycle {
    // A cancelled order's only broadcast was an `invalidateOrder` — the order did NOT go through,
    // so read it as a non-approval (stopped), never `Executed`. Check this BEFORE the broadcast
    // branch (a cancel sets `broadcast`). A STOP revoke IS a human action → `Decided{false}`.
    if req.cancelled {
        return ActivityLifecycle::Decided { approved: false };
    }
    if req.broadcast.is_some() {
        return ActivityLifecycle::Executed;
    }
    match req.status {
        ApprovalStatus::Pending => ActivityLifecycle::Proposed,
        ApprovalStatus::Allowed => ActivityLifecycle::Decided { approved: true },
        // A human denial or STOP revoke (both `Denied`) → a human acted.
        ApprovalStatus::Denied { .. } => ActivityLifecycle::Decided { approved: false },
        // The window lapsed AFTER a human approved it (resolve set `approved`) but before execute
        // fired — a human DID act, so keep it `Decided{approved:true}` (amber "you approved"), never
        // the neutral Expired that would erase the human-action signal. `expire_stale` flips both
        // Pending and Allowed past-TTL records to `Expired`, so a human-approved-but-not-executed
        // over-cap card reaches here as Expired with `approved == true`.
        ApprovalStatus::Expired if req.approved => ActivityLifecycle::Decided { approved: true },
        // The window lapsed with nobody acting → neutral, human-absent close.
        ApprovalStatus::Expired => ActivityLifecycle::Expired,
    }
}

/// Collapse a multi-line error into one short, **redacted** line for a `reason` string or a
/// log. Transport errors can echo the full RPC URL — which carries the API key in its
/// path/query — so every embedded URL is scrubbed to `scheme://host[:port]`
/// ([`crate::config::sanitize_reason`]) BEFORE truncation. Reasons cross into agent
/// transcripts; they must never carry a credential.
fn one_line(e: &anyhow::Error) -> String {
    let first = e.to_string();
    let first = first.lines().next().unwrap_or("");
    crate::config::sanitize_reason(first)
        .chars()
        .take(160)
        .collect()
}

#[cfg(test)]
mod stop_selection_tests {
    //! Regression guard for the STOP-integrity fix: STOP must cancel a SIGNED order on-chain even
    //! when its local approval record has lapsed to `Expired`, because the order is still
    //! settleable on the CoW orderbook until its real `valid_to`. Drives the extracted
    //! `select_orders_to_cancel` directly — no socket, no chain.
    use super::*;

    fn order(valid_to: u32) -> SwapOrder {
        SwapOrder {
            chain_id: 11_155_111,
            owner: Address::ZERO,
            sell_token: Address::repeat_byte(0x11),
            buy_token: Address::repeat_byte(0x22),
            sell_amount: U256::from(1u64),
            buy_amount_min: U256::from(1u64),
            receiver: Address::ZERO,
            valid_to,
            app_data: deckard_core::APP_DATA_HASH,
        }
    }

    /// A record carrying `payload`, optionally signed / already-broadcast. Its local `status` is
    /// `Expired` on purpose: the selection must IGNORE it and consult `valid_to` instead.
    fn req(payload: PendingPayload, signed: bool, broadcast: bool) -> PendingReq {
        PendingReq {
            payload,
            status: ApprovalStatus::Expired,
            expires_at: Instant::now(),
            broadcast: broadcast.then(|| B256::repeat_byte(0xbb)),
            signature: signed.then(|| Bytes::from_static(b"sig")),
            approved: true,
            origin: ProposalOrigin::App,
            created_ms: 0,
            seq: 0,
            breached: BreachedLimit::None,
            cancelled: false,
            auto_allowed: false,
        }
    }

    #[test]
    fn cancels_a_signed_order_even_when_locally_expired() {
        // now = 1000s; the order is valid_to = 2000s (still settleable) but its local record is
        // `Expired`. Before the fix the `not_expired` filter dropped it — STOP must select it.
        let id = request_id_for_order(&order(2000));
        let mut reqs = HashMap::new();
        reqs.insert(id, req(PendingPayload::Order(order(2000)), true, false));
        assert_eq!(select_orders_to_cancel(&reqs, 1000), vec![id]);
    }

    #[test]
    fn skips_unsettleable_unsigned_cancelled_and_tx_records() {
        let now = 1000u64;
        let mut reqs = HashMap::new();
        // (a) valid_to already in the past → unsettleable → skip (a cancel would be wasted gas).
        reqs.insert(
            B256::repeat_byte(0x01),
            req(PendingPayload::Order(order(500)), true, false),
        );
        // (b) unsigned → never posted to the orderbook → skip.
        reqs.insert(
            B256::repeat_byte(0x02),
            req(PendingPayload::Order(order(2000)), false, false),
        );
        // (c) already cancelled/broadcast → skip (its only broadcast was the invalidate).
        reqs.insert(
            B256::repeat_byte(0x03),
            req(PendingPayload::Order(order(2000)), true, true),
        );
        // (d) a plain Tx (Send/Shield/shaped-approve) is not an order → skip.
        let intent = Intent {
            chain_id: 11_155_111,
            to: Address::ZERO,
            token: None,
            value: U256::ZERO,
            calldata: Bytes::new(),
            kind: IntentKind::ContractCall,
        };
        reqs.insert(
            B256::repeat_byte(0x04),
            req(PendingPayload::Tx(intent), true, false),
        );
        assert!(select_orders_to_cancel(&reqs, now).is_empty());
    }
}
