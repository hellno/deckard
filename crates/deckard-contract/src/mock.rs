//! An in-memory, deterministic [`Signer`] so the agent surface, the desktop app, and the
//! test harness can run the acceptance scenario before the real `deckard-signerd` exists.
//!
//! Pinned for byte-stable tests: address `0x1111…11`, broadcast tx hash `0xABAB…AB`, and
//! `request_id`s assigned `0x0101…01`, `0x0202…02`, … in propose order. Holds a `Mutex<Policy>`
//! and a `Mutex` of in-flight requests; **carries no key material** and never signs anything —
//! `execute` just returns the pinned hash.

use std::collections::HashMap;
use std::sync::Mutex;

use alloy_primitives::{Address, B256, U256};

use crate::decision::{Decision, RequestId};
use crate::intent::Intent;
use crate::policy::{self, Policy};
use crate::rpc::{ApprovalStatus, BalanceReport, ExecuteResult, UnlockOutcome};
use crate::signer::Signer;

/// One tracked proposal. `status` is the wire-visible approval state; `broadcast` is `Some`
/// once `execute` has signed it (so a second `execute` is idempotently refused).
#[derive(Clone, Debug)]
struct Request {
    intent: Intent,
    status: ApprovalStatus,
    broadcast: Option<B256>,
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
            }),
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
                intent: intent.clone(),
                status,
                broadcast: None,
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

        match req.status.clone() {
            // The only state that signs (covers allow-equivalent and human-approved).
            ApprovalStatus::Allowed => {
                let tx = Self::broadcast_tx_hash();
                let value = req.intent.value;
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

    /// A policy with an empty allowlist and `auto_shield_min_wei = 10`.
    fn policy(per_tx: u64, daily: u64, spent: u64, mode: ApprovalMode) -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(per_tx),
            daily_cap_wei: U256::from(daily),
            spent_today_wei: U256::from(spent),
            allow_to: vec![],
            auto_shield_min_wei: U256::from(10u64),
            require_approval: mode,
            revoked: false,
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
            calldata: Bytes::new(),
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
}
