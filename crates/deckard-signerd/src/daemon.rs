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
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256, U256};
use zeroize::Zeroizing;

use deckard_contract::{
    evaluate, ApprovalStatus, BalanceReport, Decision, ExecuteResult, Intent, IntentKind, Policy,
    RequestId, SignerRequest, SignerResponse, UnlockOutcome,
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
        }
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
        if intent.kind != IntentKind::Send {
            return Decision::Deny {
                reason: "unsupported_v1".into(),
            };
        }
        // v1 spine is native ETH only; an ERC-20 (`token = Some`) Send is a fast-follow.
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
        let (to, value, scalar) = {
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
            (req.intent.to, req.intent.value, scalar)
        };

        // Phase 2: sign + broadcast (lock held — serialized; acceptable for v1). A bounded
        // timeout keeps a hung RPC from wedging the daemon (and STOP) behind the held lock.
        let broadcast = signing::broadcast_native_send(
            scalar.as_slice(),
            &self.cfg.rpc_url,
            self.cfg.chain_id,
            to,
            value,
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

    /// Public balance via the RPC (key-less). `shielded_wei` is 0 until T-Privacy. A locked
    /// daemon reports zeros (it doesn't know which address to read).
    async fn balance(&mut self, _shielded: bool) -> BalanceReport {
        self.rollover();
        let addr = match &self.state {
            VaultState::Unlocked { address, .. } => *address,
            VaultState::Locked => {
                return BalanceReport {
                    public_wei: U256::ZERO,
                    shielded_wei: U256::ZERO,
                }
            }
        };
        let public_wei = signing::read_balance(&self.cfg.rpc_url, addr)
            .await
            .unwrap_or(U256::ZERO);
        BalanceReport {
            public_wei,
            shielded_wei: U256::ZERO,
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
