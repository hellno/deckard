//! An in-memory, deterministic [`Signer`] so the agent surface, the desktop app, and the
//! test harness can run the acceptance scenario before the real `deckard-signerd` exists.
//!
//! Pinned for byte-stable tests: address `0x1111…11`, broadcast tx hash `0xABAB…AB`, and
//! `request_id`s assigned `0x0101…01`, `0x0202…02`, … in propose order. Holds a `Mutex<Policy>`
//! and a `Mutex` of in-flight requests; **carries no key material** and never signs anything —
//! `execute` just returns the pinned hash.

use std::collections::HashMap;
use std::sync::Mutex;

use alloy_primitives::{Address, Bytes, B256, U256};

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::policy::{self, Policy};
use crate::read_status::ReadStatus;
use crate::rpc::{ApprovalStatus, BalanceReport, ExecuteResult, SignOrderResult, UnlockOutcome};
use crate::signer::Signer;
use crate::swap_order::SwapOrder;

/// What a tracked request carries. One request table serves BOTH plain intents (`Tx`) and
/// swap orders (`Order`) — mirroring the real daemon's payload enum so the mock and the
/// daemon share request-lifecycle shape (faithful parity).
#[derive(Clone, Debug)]
enum ReqPayload {
    Tx(Intent),
    Order(SwapOrder),
}

/// One tracked proposal. `status` is the wire-visible approval state; `broadcast` is `Some`
/// once `execute`/`cancel_order` has broadcast it (so a second broadcast is idempotently
/// refused); `signature` is `Some` once an order has been signed by `sign_order`.
#[derive(Clone, Debug)]
struct Request {
    payload: ReqPayload,
    status: ApprovalStatus,
    broadcast: Option<B256>,
    signature: Option<Bytes>,
}

/// The request table, the deterministic id counter (`1, 2, …`), and the most recently
/// minted id. The pinned single-byte `repeat_byte(n)` scheme tops out at 255 ids.
#[derive(Debug)]
struct Requests {
    by_id: HashMap<RequestId, Request>,
    next_id: u8,
    last_id: Option<RequestId>,
}

/// An in-memory signer. The `policy` and `requests` locks are always acquired **policy
/// before requests**, so the pair can never deadlock; `balance` is only ever taken alone.
///
/// The mock holds no real key, so its `Locked`/`Unlocked` state is modelled by the
/// `Policy::revoked` brake: `lock`/`revoke_all` trip it (deny everything), `unlock` clears
/// it (re-arm). This mirrors the daemon, where `Lock` and `RevokeAll` both reach `Locked`
/// and only a fresh `Unlock` re-arms.
#[derive(Debug)]
pub struct MockSigner {
    policy: Mutex<Policy>,
    requests: Mutex<Requests>,
    balance: Mutex<BalanceReport>,
    /// The injected unix-secs clock `propose_order` feeds to `evaluate_order` (the pure
    /// decision fn takes `now` as a parameter so the mock stays deterministic + offline).
    mock_now: Mutex<u64>,
}

impl MockSigner {
    /// Build a mock from a starting policy. Balances default to zero; set them with
    /// [`MockSigner::set_balance`].
    pub fn new(policy: Policy) -> Self {
        Self {
            policy: Mutex::new(policy),
            requests: Mutex::new(Requests {
                by_id: HashMap::new(),
                next_id: 1,
                last_id: None,
            }),
            balance: Mutex::new(BalanceReport {
                public_wei: U256::ZERO,
                shielded_wei: U256::ZERO,
                // The mock is deterministic + offline; it never touches a chain, so
                // it reports its canned balances as Verified (no untrusted RPC behind it).
                read_status: ReadStatus::Verified,
            }),
            // A fixed, far-future-of-genesis default so an order's `valid_to` can sit
            // inside the 24h horizon without a real wall clock. Override with `set_now`.
            mock_now: Mutex::new(1_700_000_000),
        }
    }

    /// The pinned deterministic address (`0x1111…11`).
    pub fn mock_address() -> Address {
        Address::repeat_byte(0x11)
    }

    /// The pinned broadcast tx hash every successful `execute` returns (`0xABAB…AB`).
    pub fn broadcast_tx_hash() -> B256 {
        B256::repeat_byte(0xAB)
    }

    /// The pinned 65-byte order signature `sign_order` returns (`0xCD` × 65). The mock holds
    /// no key — it returns a stable stand-in shaped like a real r||s||v EIP-712 signature.
    pub fn pinned_order_signature() -> Bytes {
        Bytes::from(vec![0xCD_u8; 65])
    }

    /// Set the injected unix-secs clock used by `propose_order` (setup helper for the
    /// `valid_to` horizon check). Mirrors the daemon's `now_unix()` being made injectable.
    pub fn set_now(&self, now: u64) {
        *self.mock_now.lock().expect("mock now mutex poisoned") = now;
    }

    /// Overwrite the reported balances (setup helper).
    pub fn set_balance(&self, report: BalanceReport) {
        *self.balance.lock().expect("mock balance mutex poisoned") = report;
    }

    /// Test helper: flip a `Pending` request to `Allowed`, simulating the human tapping
    /// Approve on the native card. Thin wrapper over [`Signer::resolve`].
    pub fn approve(&self, request_id: RequestId) {
        self.resolve(request_id, true);
    }

    /// Test helper: the id of the most recently minted request, or `None` if none yet.
    /// Useful for executing an `Allow` decision, which does not carry the id on the wire.
    pub fn last_request_id(&self) -> Option<RequestId> {
        self.requests
            .lock()
            .expect("mock requests mutex poisoned")
            .last_id
    }

    /// Test helper: the [`SwapOrder`] stored under `request_id`, or `None` if the id is
    /// unknown or carries a plain `Tx` intent. Lets a test assert the daemon-parity property
    /// that `sign_order` operates on the order captured at propose time.
    pub fn stored_order(&self, request_id: RequestId) -> Option<SwapOrder> {
        let reqs = self.requests.lock().expect("mock requests mutex poisoned");
        match reqs.by_id.get(&request_id).map(|r| &r.payload) {
            Some(ReqPayload::Order(order)) => Some(order.clone()),
            _ => None,
        }
    }

    /// Mint the next deterministic id (`repeat_byte(1)`, `repeat_byte(2)`, …). Caller holds
    /// the requests lock. The pinned single-byte scheme yields at most 255 distinct ids;
    /// minting a 256th would collide with a live entry, so we panic loudly rather than
    /// silently wrap (which would clobber an in-flight request and defeat idempotency).
    fn mint_id(reqs: &mut Requests) -> RequestId {
        assert!(
            reqs.next_id != 0,
            "MockSigner request-id space (u8) exhausted: this mock supports at most 255 proposals"
        );
        let id = B256::repeat_byte(reqs.next_id);
        reqs.next_id = reqs.next_id.wrapping_add(1);
        reqs.last_id = Some(id);
        id
    }
}

impl Signer for MockSigner {
    fn unlock(&self, _passphrase: &str) -> UnlockOutcome {
        // The mock holds no real keystore, so any passphrase "unlocks" it. A fresh unlock
        // re-arms the session by clearing the `revoked` brake (mirrors the daemon's
        // "re-unlock to re-arm").
        self.policy
            .lock()
            .expect("mock policy mutex poisoned")
            .revoked = false;
        UnlockOutcome::Unlocked {
            address: Self::mock_address(),
        }
    }

    fn lock(&self) {
        // Lock the session (trip the brake) and deny everything in flight — same as the
        // daemon's `Lock`, which reaches `Locked` exactly like `RevokeAll`.
        let mut policy = self.policy.lock().expect("mock policy mutex poisoned");
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");
        policy.revoked = true;
        deny_pending(&mut reqs);
    }

    fn resolve(&self, request_id: RequestId, approved: bool) {
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");
        if let Some(req) = reqs.by_id.get_mut(&request_id) {
            if req.status == ApprovalStatus::Pending {
                req.status = if approved {
                    ApprovalStatus::Allowed
                } else {
                    ApprovalStatus::Denied {
                        reason: "user_denied".into(),
                    }
                };
            }
        }
    }

    fn address(&self) -> Address {
        Self::mock_address()
    }

    fn balance(&self, _shielded: bool) -> BalanceReport {
        self.balance
            .lock()
            .expect("mock balance mutex poisoned")
            .clone()
    }

    fn policy(&self) -> Policy {
        self.policy
            .lock()
            .expect("mock policy mutex poisoned")
            .clone()
    }

    fn propose(&self, intent: &Intent) -> Decision {
        // The verdict comes from the ONE shared decision function — no logic lives here.
        // (`revoked`, the mock's lock state, is one of the checks `evaluate` makes.)
        let needs_card = {
            let policy = self.policy.lock().expect("mock policy mutex poisoned");
            match policy::evaluate(intent, &policy) {
                // Terminal verdicts return straight through.
                deny @ Decision::Deny { .. } => return deny,
                Decision::Allow => false,
                Decision::NeedsApproval { .. } => true,
            }
        }; // policy lock released before taking the requests lock (preserves lock order)

        // Mint the real, trackable id (replacing `evaluate`'s placeholder) and store the
        // pending record under it.
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");
        let id = Self::mint_id(&mut reqs);
        let status = if needs_card {
            ApprovalStatus::Pending
        } else {
            ApprovalStatus::Allowed
        };
        reqs.by_id.insert(
            id,
            Request {
                payload: ReqPayload::Tx(intent.clone()),
                status,
                broadcast: None,
                signature: None,
            },
        );

        if needs_card {
            Decision::NeedsApproval { request_id: id }
        } else {
            Decision::Allow
        }
    }

    fn execute(&self, request_id: RequestId) -> ExecuteResult {
        // Always policy-before-requests so execute and revoke_all can't deadlock.
        let mut policy = self.policy.lock().expect("mock policy mutex poisoned");
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");

        let req = match reqs.by_id.get_mut(&request_id) {
            None => {
                return ExecuteResult::Denied {
                    reason: "unknown_request".into(),
                }
            }
            Some(req) => req,
        };

        // Idempotency: a broadcast id never signs twice.
        if req.broadcast.is_some() {
            return ExecuteResult::Denied {
                reason: "already_executed".into(),
            };
        }

        // TOCTOU guard: re-check `revoked` at sign time. An approval granted before
        // revoke_all must still be denied here.
        if policy.revoked {
            return ExecuteResult::Denied {
                reason: "revoked".into(),
            };
        }

        // `execute` is the intent broadcast path; swap orders are signed via `sign_order`
        // and never reach here through `propose`. Refuse an Order payload defensively.
        let value = match &req.payload {
            ReqPayload::Tx(intent) => intent.value,
            ReqPayload::Order(_) => {
                return ExecuteResult::Denied {
                    reason: "not_an_order".into(),
                }
            }
        };

        match req.status.clone() {
            // The only state that signs (covers allow-equivalent and human-approved).
            ApprovalStatus::Allowed => {
                let tx = Self::broadcast_tx_hash();
                req.broadcast = Some(tx);
                policy.spent_today_wei = policy.spent_today_wei.saturating_add(value);
                ExecuteResult::Broadcast { tx_hash: tx }
            }
            ApprovalStatus::Pending => ExecuteResult::Denied {
                reason: "not_approved".into(),
            },
            ApprovalStatus::Denied { reason } => ExecuteResult::Denied { reason },
            ApprovalStatus::Expired => ExecuteResult::Denied {
                reason: "expired".into(),
            },
        }
    }

    fn status(&self, request_id: RequestId) -> ApprovalStatus {
        let reqs = self.requests.lock().expect("mock requests mutex poisoned");
        match reqs.by_id.get(&request_id) {
            Some(req) => req.status.clone(),
            None => ApprovalStatus::Denied {
                reason: "unknown_request".into(),
            },
        }
    }

    fn revoke_all(&self) {
        // STOP: trip the policy brake, then deny everything in flight.
        // Same lock order as execute(): policy before requests.
        let mut policy = self.policy.lock().expect("mock policy mutex poisoned");
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");
        policy.revoked = true;
        deny_pending(&mut reqs);
    }

    fn propose_order(&self, order: &SwapOrder) -> Decision {
        // The verdict comes from the ONE shared swap-order decision fn — no logic lives here.
        // `evaluate_order` never returns Allow: a valid order is ALWAYS NeedsApproval, so a
        // non-Deny verdict always mints a Pending record (no auto-allow branch like `propose`).
        // BIND owner to the mock wallet — mirrors the daemon, which never trusts the client's
        // `owner` (so `stored_order` returns the same bound order the daemon would sign).
        let mut bound = order.clone();
        bound.owner = Self::mock_address();
        {
            let policy = self.policy.lock().expect("mock policy mutex poisoned");
            let now = *self.mock_now.lock().expect("mock now mutex poisoned");
            if let deny @ Decision::Deny { .. } =
                policy::evaluate_order(&bound, &policy, Self::mock_address(), now)
            {
                return deny;
            }
        } // policy lock released before taking the requests lock (preserves lock order)

        // Mint the real, trackable id (replacing `evaluate_order`'s placeholder) and store
        // the pending (bound) order record under it.
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");
        let id = Self::mint_id(&mut reqs);
        reqs.by_id.insert(
            id,
            Request {
                payload: ReqPayload::Order(bound),
                status: ApprovalStatus::Pending,
                broadcast: None,
                signature: None,
            },
        );
        Decision::NeedsApproval { request_id: id }
    }

    fn sign_order(&self, request_id: RequestId) -> SignOrderResult {
        // Always policy-before-requests so sign_order and revoke_all can't deadlock.
        let policy = self.policy.lock().expect("mock policy mutex poisoned");
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");

        let req = match reqs.by_id.get_mut(&request_id) {
            None => {
                return SignOrderResult::Denied {
                    reason: "unknown_request".into(),
                }
            }
            Some(req) => req,
        };

        // Only an Order payload can be signed.
        match &req.payload {
            ReqPayload::Order(_) => {}
            ReqPayload::Tx(_) => {
                return SignOrderResult::Denied {
                    reason: "not_an_order".into(),
                }
            }
        }

        // TOCTOU guard: re-check `revoked` at sign time. An approval granted before
        // revoke_all must still be refused here (and `deny_pending` has already flipped
        // a still-Pending order to Denied{revoked}).
        if policy.revoked {
            return SignOrderResult::Denied {
                reason: "revoked".into(),
            };
        }

        match req.status.clone() {
            // The only state that signs (a human-approved order).
            ApprovalStatus::Allowed => {
                let sig = Self::pinned_order_signature();
                req.signature = Some(sig.clone());
                SignOrderResult::Signed { signature: sig }
            }
            ApprovalStatus::Pending => SignOrderResult::Denied {
                reason: "not_approved".into(),
            },
            ApprovalStatus::Denied { reason } => SignOrderResult::Denied { reason },
            ApprovalStatus::Expired => SignOrderResult::Denied {
                reason: "expired".into(),
            },
        }
    }

    fn cancel_order(&self, request_id: RequestId) -> ExecuteResult {
        // Always policy-before-requests so cancel_order and revoke_all can't deadlock.
        let _policy = self.policy.lock().expect("mock policy mutex poisoned");
        let mut reqs = self.requests.lock().expect("mock requests mutex poisoned");

        let req = match reqs.by_id.get_mut(&request_id) {
            None => {
                return ExecuteResult::Denied {
                    reason: "unknown_request".into(),
                }
            }
            Some(req) => req,
        };

        // Only an Order payload can be cancelled (intents broadcast/finalise via `execute`).
        match &req.payload {
            ReqPayload::Order(_) => {}
            ReqPayload::Tx(_) => {
                return ExecuteResult::Denied {
                    reason: "not_an_order".into(),
                }
            }
        }

        // Idempotency: a cancelled order never broadcasts a second cancel.
        if req.broadcast.is_some() {
            return ExecuteResult::Denied {
                reason: "already_executed".into(),
            };
        }

        // An order is cancellable once it has been approved (Allowed) or signed — the on-chain
        // `invalidateOrder` is what stops a signed order from settling. A signed order stays
        // cancellable even if a later STOP flipped its status, so the broadcast hash is pinned
        // whenever a signature exists. A still-Pending / Denied / Expired order (never signed)
        // has nothing to invalidate.
        if req.signature.is_some() || req.status == ApprovalStatus::Allowed {
            let tx = Self::broadcast_tx_hash();
            req.broadcast = Some(tx);
            return ExecuteResult::Broadcast { tx_hash: tx };
        }
        match req.status.clone() {
            ApprovalStatus::Denied { reason } => ExecuteResult::Denied { reason },
            ApprovalStatus::Expired => ExecuteResult::Denied {
                reason: "expired".into(),
            },
            // Pending (and the already-handled Allowed) fall here as "nothing to cancel yet".
            ApprovalStatus::Pending | ApprovalStatus::Allowed => ExecuteResult::Denied {
                reason: "not_approved".into(),
            },
        }
    }
}

/// Flip every still-`Pending` request to `Denied{revoked}` — shared by `lock`/`revoke_all`.
fn deny_pending(reqs: &mut Requests) {
    for req in reqs.by_id.values_mut() {
        if req.status == ApprovalStatus::Pending {
            req.status = ApprovalStatus::Denied {
                reason: "revoked".into(),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::IntentKind;
    use crate::policy::ApprovalMode;
    use alloy_primitives::Bytes;

    // --- builders -------------------------------------------------------------------

    /// A policy with an empty allowlist and `auto_shield_min_wei = 10`. The swap allowlist is
    /// empty too (any token allowed) — swap tests that need a non-empty list set it directly.
    fn policy(per_tx: u64, daily: u64, spent: u64, mode: ApprovalMode) -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(per_tx),
            daily_cap_wei: U256::from(daily),
            spent_today_wei: U256::from(spent),
            allow_to: vec![],
            auto_shield_min_wei: U256::from(10u64),
            require_approval: mode,
            revoked: false,
            allow_swap_tokens: vec![],
        }
    }

    fn send(value: u64) -> Intent {
        Intent {
            chain_id: 1,
            to: Address::repeat_byte(0x22),
            token: None,
            value: U256::from(value),
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        }
    }

    fn shield(value: u64) -> Intent {
        Intent {
            chain_id: 1,
            to: Address::repeat_byte(0x44),
            token: None,
            value: U256::from(value),
            // A real Shield always carries the RelayAdapt call; the policy gate now requires
            // it (an empty payload would degrade into a bare native send). Stand-in bytes here.
            calldata: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            kind: IntentKind::Shield,
        }
    }

    fn unwrap_needs(d: Decision) -> RequestId {
        match d {
            Decision::NeedsApproval { request_id } => request_id,
            other => panic!("expected NeedsApproval, got {other:?}"),
        }
    }

    // --- decision matrix ------------------------------------------------------------

    #[test]
    fn within_cap_allows() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(s.propose(&send(20)), Decision::Allow);
    }

    #[test]
    fn over_per_tx_cap_needs_approval() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert!(matches!(
            s.propose(&send(60)),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn over_daily_cap_needs_approval() {
        // per-tx effectively unbounded so only the daily cap can bind.
        let s = MockSigner::new(policy(u64::MAX, 100, 90, ApprovalMode::OverCap));
        assert!(matches!(
            s.propose(&send(20)),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn off_allowlist_denies() {
        let mut p = policy(50, 1000, 0, ApprovalMode::OverCap);
        p.allow_to = vec![Address::repeat_byte(0x33)]; // send() targets 0x22
        let s = MockSigner::new(p);
        assert_eq!(
            s.propose(&send(20)),
            Decision::Deny {
                reason: "off_allowlist".into()
            }
        );
    }

    #[test]
    fn on_allowlist_allows() {
        let mut p = policy(50, 1000, 0, ApprovalMode::OverCap);
        p.allow_to = vec![Address::repeat_byte(0x22)]; // matches send()'s target
        let s = MockSigner::new(p);
        assert_eq!(s.propose(&send(20)), Decision::Allow);
    }

    #[test]
    fn revoked_policy_denies_propose() {
        let mut p = policy(50, 1000, 0, ApprovalMode::OverCap);
        p.revoked = true;
        let s = MockSigner::new(p);
        assert_eq!(
            s.propose(&send(20)),
            Decision::Deny {
                reason: "revoked".into()
            }
        );
    }

    #[test]
    fn undecodable_calldata_denies() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        // A Send must have empty calldata.
        let mut bad_send = send(20);
        bad_send.calldata = Bytes::from_static(&[0x01, 0x02]);
        assert_eq!(
            s.propose(&bad_send),
            Decision::Deny {
                reason: "undecodable".into()
            }
        );
        // A ContractCall must have non-empty calldata.
        let empty_call = Intent {
            kind: IntentKind::ContractCall,
            calldata: Bytes::new(),
            ..send(20)
        };
        assert_eq!(
            s.propose(&empty_call),
            Decision::Deny {
                reason: "undecodable".into()
            }
        );
        // A Shield with EMPTY calldata is rejected: without the RelayAdapt call it would
        // degrade into a bare native send to `to` (no private note) while labelled "Shield".
        let empty_shield = Intent {
            calldata: Bytes::new(),
            ..shield(20)
        };
        assert_eq!(
            s.propose(&empty_shield),
            Decision::Deny {
                reason: "undecodable".into()
            }
        );
    }

    #[test]
    fn never_over_cap_denies() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::Never));
        assert_eq!(
            s.propose(&send(60)),
            Decision::Deny {
                reason: "over_cap".into()
            }
        );
    }

    #[test]
    fn always_within_cap_needs_approval() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::Always));
        assert!(matches!(
            s.propose(&send(20)),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn execute_on_pending_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose(&send(60)));
        assert_eq!(
            s.execute(id),
            ExecuteResult::Denied {
                reason: "not_approved".into()
            }
        );
    }

    #[test]
    fn approve_then_execute_broadcasts_and_increments_spent() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose(&send(60)));
        s.approve(id);
        assert_eq!(
            s.execute(id),
            ExecuteResult::Broadcast {
                tx_hash: MockSigner::broadcast_tx_hash()
            }
        );
        assert_eq!(s.policy().spent_today_wei, U256::from(60u64));
    }

    #[test]
    fn toctou_revoke_then_execute_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose(&send(60)));
        s.approve(id); // human approved BEFORE the STOP
        s.revoke_all();
        assert_eq!(
            s.execute(id),
            ExecuteResult::Denied {
                reason: "revoked".into()
            }
        );
        // and nothing was spent
        assert_eq!(s.policy().spent_today_wei, U256::ZERO);
    }

    #[test]
    fn unknown_id_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(
            s.execute(B256::repeat_byte(0xFF)),
            ExecuteResult::Denied {
                reason: "unknown_request".into()
            }
        );
    }

    #[test]
    fn double_execute_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        // within-cap → Allow, stored as allow-equivalent and executable by its minted id.
        assert_eq!(s.propose(&send(20)), Decision::Allow);
        let id = s.last_request_id().expect("an id was minted");
        assert!(matches!(s.execute(id), ExecuteResult::Broadcast { .. }));
        assert_eq!(
            s.execute(id),
            ExecuteResult::Denied {
                reason: "already_executed".into()
            }
        );
        // spent incremented exactly once
        assert_eq!(s.policy().spent_today_wei, U256::from(20u64));
    }

    #[test]
    fn auto_shield_within_cap_never_allows() {
        // The demo beat: an inbound shield within cap, hands-free (Never).
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::Never));
        assert_eq!(s.propose(&shield(20)), Decision::Allow);
    }

    #[test]
    fn revoke_all_flips_pending_to_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose(&send(60))); // Pending
        s.revoke_all();
        assert_eq!(
            s.status(id),
            ApprovalStatus::Denied {
                reason: "revoked".into()
            }
        );
        assert!(s.policy().revoked);
    }

    #[test]
    fn pinned_constants_and_first_id() {
        assert_eq!(MockSigner::mock_address(), Address::repeat_byte(0x11));
        assert_eq!(MockSigner::broadcast_tx_hash(), B256::repeat_byte(0xAB));
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(s.address(), Address::repeat_byte(0x11));
        // The first minted request_id is 0x0101…01.
        let id = unwrap_needs(s.propose(&send(60)));
        assert_eq!(id, B256::repeat_byte(0x01));
    }

    #[test]
    fn box_dyn_signer_is_usable() {
        let s: Box<dyn Signer> =
            Box::new(MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap)));
        assert_eq!(s.address(), MockSigner::mock_address());
        assert_eq!(s.propose(&send(20)), Decision::Allow);
        assert!(!s.policy().revoked);
        s.revoke_all();
        assert!(s.policy().revoked);
    }

    #[test]
    fn balance_reads_what_was_set() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        s.set_balance(BalanceReport {
            public_wei: U256::from(7u64),
            shielded_wei: U256::from(3u64),
            read_status: ReadStatus::Verified,
        });
        let b = s.balance(false);
        assert_eq!(b.public_wei, U256::from(7u64));
        assert_eq!(b.shielded_wei, U256::from(3u64));
    }

    // --- boundary + guard coverage (added after review) -----------------------------

    #[test]
    fn exact_per_tx_cap_is_within() {
        // projected == per_tx_cap is "within" (strict `>`): pins against a `>=` regression.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(s.propose(&send(50)), Decision::Allow);
    }

    #[test]
    fn exact_daily_cap_is_within_one_over_needs_approval() {
        // per-tx unbounded so only the daily cap binds. projected == daily → Allow;
        // projected == daily + 1 → NeedsApproval.
        let s = MockSigner::new(policy(u64::MAX, 100, 90, ApprovalMode::OverCap));
        assert_eq!(s.propose(&send(10)), Decision::Allow); // 90 + 10 == 100
        let s2 = MockSigner::new(policy(u64::MAX, 100, 90, ApprovalMode::OverCap));
        assert!(matches!(
            s2.propose(&send(11)), // 90 + 11 == 101 > 100
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn toctou_revoke_then_execute_within_cap_allow_denied() {
        // STOP must also block an unexecuted within-cap Allow (status=Allowed), not just the
        // human-approved over-cap path: the execute-time revoked guard is the only thing
        // standing between an Allow and a broadcast after revoke_all.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(s.propose(&send(20)), Decision::Allow);
        let id = s.last_request_id().expect("Allow minted an id");
        s.revoke_all();
        assert_eq!(
            s.execute(id),
            ExecuteResult::Denied {
                reason: "revoked".into()
            }
        );
        assert_eq!(s.policy().spent_today_wei, U256::ZERO);
    }

    #[test]
    fn last_request_id_tracks_latest_mint() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(s.last_request_id(), None);
        s.propose(&send(20)); // mints 0x01
        assert_eq!(s.last_request_id(), Some(B256::repeat_byte(0x01)));
        s.propose(&send(20)); // mints 0x02
        assert_eq!(s.last_request_id(), Some(B256::repeat_byte(0x02)));
    }

    #[test]
    #[should_panic(expected = "exhausted")]
    fn request_id_space_exhaustion_panics_instead_of_wrapping() {
        // The pinned single-byte id scheme supports 255 ids; the 256th proposal must panic
        // loudly rather than silently wrap and clobber a live request.
        let s = MockSigner::new(policy(u64::MAX, u64::MAX, 0, ApprovalMode::OverCap));
        for _ in 0..256 {
            let _ = s.propose(&send(1)); // within cap → Allow → mints an id each time
        }
    }

    // --- swap-order lifecycle -------------------------------------------------------

    /// The mock's default clock; the order builder sits its `valid_to` inside the horizon.
    const MOCK_NOW: u64 = 1_700_000_000;

    /// A well-formed order: owner/receiver bound to the mock's pinned address, `valid_to`
    /// one hour out (inside the 24h horizon), tokens left off the (empty by default) list.
    fn order() -> SwapOrder {
        SwapOrder {
            chain_id: 11155111,
            owner: MockSigner::mock_address(),
            sell_token: Address::repeat_byte(0xA1),
            buy_token: Address::repeat_byte(0xB2),
            sell_amount: U256::from(1_000_000u64),
            buy_amount_min: U256::from(900_000u64),
            receiver: MockSigner::mock_address(),
            valid_to: (MOCK_NOW + 3600) as u32,
            app_data: B256::repeat_byte(0xCD),
        }
    }

    #[test]
    fn propose_order_rebinds_owner_to_the_wallet() {
        // The mock mirrors the daemon: a client-supplied `owner` is never trusted — the stored
        // order has `owner == mock_address()`, so `stored_order` returns the bound order the
        // signer would actually sign.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let spoofed = SwapOrder {
            owner: Address::repeat_byte(0xEE), // a wrong/attacker owner
            ..order()
        };
        let id = unwrap_needs(s.propose_order(&spoofed));
        let stored = s.stored_order(id).expect("an order is stored under the id");
        assert_eq!(
            stored.owner,
            MockSigner::mock_address(),
            "the stored order's owner must be rebound to the wallet, not the client's value"
        );
    }

    #[test]
    fn propose_order_then_approve_then_sign_returns_pinned_signature() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        // A valid order is ALWAYS NeedsApproval (swaps never auto-allow in v1).
        let id = unwrap_needs(s.propose_order(&order()));
        // Until approved, signing is refused.
        assert_eq!(
            s.sign_order(id),
            SignOrderResult::Denied {
                reason: "not_approved".into()
            }
        );
        s.approve(id);
        assert_eq!(
            s.sign_order(id),
            SignOrderResult::Signed {
                signature: MockSigner::pinned_order_signature()
            }
        );
        // The pinned signature is 65 bytes of 0xCD.
        assert_eq!(MockSigner::pinned_order_signature().len(), 65);
    }

    #[test]
    fn sign_order_on_unknown_id_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert_eq!(
            s.sign_order(B256::repeat_byte(0xFF)),
            SignOrderResult::Denied {
                reason: "unknown_request".into()
            }
        );
    }

    #[test]
    fn sign_order_on_a_tx_payload_denied() {
        // An intent stored via `propose` is not an order and can't be signed as one.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        s.propose(&send(20)); // within cap → Allow, stored as a Tx payload
        let id = s.last_request_id().expect("an id was minted");
        assert_eq!(
            s.sign_order(id),
            SignOrderResult::Denied {
                reason: "not_an_order".into()
            }
        );
    }

    #[test]
    fn revoke_all_then_sign_order_denied_revoked() {
        // TOCTOU: a human approves, STOP fires, then `sign_order` must still refuse.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose_order(&order()));
        s.approve(id); // approved BEFORE the STOP
        s.revoke_all();
        assert_eq!(
            s.sign_order(id),
            SignOrderResult::Denied {
                reason: "revoked".into()
            }
        );
    }

    #[test]
    fn propose_order_denied_passes_through() {
        // A receiver that isn't the bound wallet is a terminal deny — no record minted.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let bad = SwapOrder {
            receiver: Address::repeat_byte(0x22),
            ..order()
        };
        assert_eq!(
            s.propose_order(&bad),
            Decision::Deny {
                reason: "receiver_not_wallet".into()
            }
        );
        assert_eq!(s.last_request_id(), None);
    }

    #[test]
    fn propose_order_off_swap_list_denied() {
        let mut p = policy(50, 1000, 0, ApprovalMode::OverCap);
        // Only the buy token is listed; the sell token is off-list.
        p.allow_swap_tokens = vec![Address::repeat_byte(0xB2)];
        let s = MockSigner::new(p);
        assert_eq!(
            s.propose_order(&order()),
            Decision::Deny {
                reason: "off_swap_list".into()
            }
        );
    }

    #[test]
    fn propose_order_respects_injected_clock() {
        // Default clock: an order one hour out is inside the horizon.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        assert!(matches!(
            s.propose_order(&order()),
            Decision::NeedsApproval { .. }
        ));
        // Rewind the clock far enough that the same order is now > 24h out → too far.
        let s2 = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        s2.set_now(MOCK_NOW - 86_401);
        assert_eq!(
            s2.propose_order(&order()),
            Decision::Deny {
                reason: "valid_to_too_far".into()
            }
        );
    }

    #[test]
    fn cancel_order_after_sign_broadcasts_pinned_hash() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose_order(&order()));
        s.approve(id);
        assert!(matches!(s.sign_order(id), SignOrderResult::Signed { .. }));
        assert_eq!(
            s.cancel_order(id),
            ExecuteResult::Broadcast {
                tx_hash: MockSigner::broadcast_tx_hash()
            }
        );
        // Idempotent: a second cancel is refused.
        assert_eq!(
            s.cancel_order(id),
            ExecuteResult::Denied {
                reason: "already_executed".into()
            }
        );
    }

    #[test]
    fn cancel_order_on_pending_denied() {
        // Nothing to invalidate until the order is approved/signed.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose_order(&order()));
        assert_eq!(
            s.cancel_order(id),
            ExecuteResult::Denied {
                reason: "not_approved".into()
            }
        );
    }

    #[test]
    fn cancel_order_on_tx_payload_denied() {
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        s.propose(&send(20));
        let id = s.last_request_id().expect("an id was minted");
        assert_eq!(
            s.cancel_order(id),
            ExecuteResult::Denied {
                reason: "not_an_order".into()
            }
        );
    }

    #[test]
    fn revoke_all_flips_pending_order_to_denied() {
        // The shared `deny_pending` path works for order payloads too: a Pending order
        // becomes Denied{revoked}, and `sign_order` then reports it as such.
        let s = MockSigner::new(policy(50, 1000, 0, ApprovalMode::OverCap));
        let id = unwrap_needs(s.propose_order(&order()));
        s.revoke_all();
        assert_eq!(
            s.status(id),
            ApprovalStatus::Denied {
                reason: "revoked".into()
            }
        );
    }
}
