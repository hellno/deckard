//! commit_flow — the shared, GPUI-light machinery behind every "compose → review → hold-to-confirm"
//! surface (Shield, Send, and later Swap). It collapses the two field-identical proposal structs
//! into one [`Proposal`] and lifts the epoch/hold/reset state-machine out of `shell.rs` into a
//! testable [`CommitFlow`] (with its pure, entity-free [`CommitState`] core).
//!
//! Step 0 is additive: this module is `pub` but not yet wired into `Shell` — the flat `shield_*` /
//! `send_*` fields and their handlers stay exactly as they are. A later step migrates them onto
//! `CommitFlow`. The state methods below reproduce the EXACT semantics of the shell handlers they
//! mirror (cited per-method); behavior must be identical once migration happens.

use alloy_primitives::B256;
use deckard_contract::{Intent, RequestId};
use gpui::Entity;
use gpui_component::input::InputState;

/// A reviewed-and-allowed action, ready to sign. Carries a **recipient snapshot** taken at review
/// time so the clear-signing card always shows the recipient that is actually inside `intent` —
/// never a value the user edited in the input after `propose` landed.
///
/// This is the single type behind the (formerly duplicated) `ShieldProposal` / `SendProposal` — see
/// the `pub type` aliases in `shell.rs`. Every existing construction/use site is unchanged: the
/// fields here are byte-identical to both old structs (`{ intent, request_id, recipient, needs_resolve }`).
#[derive(Clone)]
pub struct Proposal {
    pub intent: Intent,
    pub request_id: RequestId,
    pub recipient: String,
    /// True when the daemon answered `NeedsApproval` (over-cap, or a mainnet-guardrail downgrade
    /// of an auto-allow). The completed hold-to-confirm IS the human approval — the app is the wire
    /// contract's designated resolver — so confirm sends `Resolve{approved: true}` before `Execute`.
    pub needs_resolve: bool,
    // TODO(swap): extras — Swap needs to carry its quote/min_out alongside the proposal. Adding a
    // `pub extra: ProposalExtra` field now would force every existing `ShieldProposal { .. }` /
    // `SendProposal { .. }` construction site in shell.rs to name the new field, which Step 0
    // forbids (behavior + call sites must stay identical). Deferred to the Swap migration step.
}

/// The entity-free core of a commit flow: the proposal, the in-flight/hold flags, the surfaced
/// error + broadcast result, and the two monotonic epochs that fence out stale background replies
/// and stale hold timers. Split out from [`CommitFlow`] so the state machine is unit-testable
/// without a GPUI context (the `Entity<InputState>` handles need a `Window`/`cx` to construct).
///
/// `CommitFlow` derefs to this, so callers still write `flow.proposal`, `flow.busy`,
/// `flow.begin_review()`, etc. — the requested flat surface, with a testable core underneath.
pub struct CommitState {
    /// Set once `propose` returns `Allow`/`NeedsApproval`. `Some` means the review card +
    /// hold-to-confirm are live; it carries the recipient snapshot.
    pub proposal: Option<Proposal>,
    /// True while a `propose`/`resolve`/`execute` round-trip runs on a background thread.
    pub busy: bool,
    /// One-line, user-facing error (parse / resolve / deny / broadcast).
    pub error: Option<String>,
    /// Set on a successful `execute` broadcast — the "on its way" confirmation state.
    pub tx: Option<B256>,
    /// True while the confirm button is being held; drives the amber fill-sweep.
    pub holding: bool,
    /// Bumped on each review (and on reset) so a slow propose/resolve reply for a
    /// since-cancelled/re-issued review can't install a stale proposal.
    review_epoch: u64,
    /// Bumped on each hold-start (and on cancel/reset) so a stale hold timer can't fire a later
    /// confirm.
    hold_epoch: u64,
}

impl CommitState {
    fn new() -> Self {
        Self {
            proposal: None,
            busy: false,
            error: None,
            tx: None,
            holding: false,
            review_epoch: 0,
            hold_epoch: 0,
        }
    }

    /// Clear all transient state (proposal, error, broadcast, busy, hold). Bumps BOTH the hold +
    /// review epochs so any in-flight hold timer or propose/resolve reply lands as a no-op.
    /// Mirrors `Shell::reset_shield` (shell.rs:1360-1368) / `reset_send` (shell.rs:1578-…).
    pub fn reset(&mut self) {
        self.proposal = None;
        self.error = None;
        self.tx = None;
        self.busy = false;
        self.holding = false;
        self.hold_epoch = self.hold_epoch.wrapping_add(1);
        self.review_epoch = self.review_epoch.wrapping_add(1);
    }

    /// Begin a review: bump the review epoch (each review supersedes the last), set `busy`, and
    /// return the new epoch for the caller to capture and re-check before installing the reply.
    /// Mirrors the epoch bump + `busy = true` in `Shell::review_shield` (shell.rs:1399-1403).
    ///
    /// Note: the shell handler also clears `error`/`proposal` *before* this bump, after its
    /// parse/validation early-returns; those concerns stay in the (impure, `cx`-bound) handler.
    pub fn begin_review(&mut self) -> u64 {
        self.review_epoch = self.review_epoch.wrapping_add(1);
        self.busy = true;
        self.review_epoch
    }

    /// True when `epoch` is the current review epoch — i.e. this background reply is not stale.
    /// Mirrors the `this.shield_review_epoch != epoch` guard in `review_shield` (shell.rs:1418).
    pub fn review_is_current(&self, epoch: u64) -> bool {
        epoch == self.review_epoch
    }

    /// Release an in-progress hold before it completed. Returns true when a hold was actually
    /// cancelled (so the caller can `cx.notify()`), bumping the hold epoch to cancel the pending
    /// timer. Mirrors `Shell::shield_hold_cancel` (shell.rs:1557-1563).
    pub fn cancel_hold(&mut self) -> bool {
        if self.holding {
            self.holding = false;
            self.hold_epoch = self.hold_epoch.wrapping_add(1);
            true
        } else {
            false
        }
    }
}

/// A commit flow's full state: the two text inputs plus the entity-free [`CommitState`] core. Holds
/// no key — it only carries what a "compose → review → hold-to-confirm" surface renders and the
/// epochs that fence its background work. Derefs to [`CommitState`] for the flat field/method
/// surface (`flow.proposal`, `flow.begin_review()`, …).
pub struct CommitFlow {
    /// Amount (ETH, free text).
    pub amount: Entity<InputState>,
    /// Recipient input (a `0x…`/ENS address for Send, a `0zk…` address for Shield).
    pub recipient: Entity<InputState>,
    state: CommitState,
}

impl CommitFlow {
    /// A fresh flow: no proposal, not busy, no error, no broadcast, not holding, epochs at 0.
    pub fn new(amount: Entity<InputState>, recipient: Entity<InputState>) -> Self {
        Self {
            amount,
            recipient,
            state: CommitState::new(),
        }
    }
}

impl std::ops::Deref for CommitFlow {
    type Target = CommitState;
    fn deref(&self) -> &CommitState {
        &self.state
    }
}

impl std::ops::DerefMut for CommitFlow {
    fn deref_mut(&mut self) -> &mut CommitState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, U256};
    use deckard_contract::IntentKind;

    /// A `CommitState` carrying a proposal, for the hold-guard tests. The `Intent`/`Proposal`
    /// contents are irrelevant to the epoch/guard logic — only `proposal.is_some()` matters.
    fn state_with_proposal() -> CommitState {
        let mut s = CommitState::new();
        s.proposal = Some(Proposal {
            intent: Intent {
                chain_id: 31337,
                to: Address::repeat_byte(0x11),
                token: None,
                value: U256::from(1u64),
                calldata: Bytes::new(),
                kind: IntentKind::Shield,
            },
            request_id: RequestId::ZERO,
            recipient: "0zk…".into(),
            needs_resolve: false,
        });
        s
    }

    #[test]
    fn review_epoch_supersedes_the_previous_review() {
        let mut s = CommitState::new();
        let epoch1 = s.begin_review();
        let epoch2 = s.begin_review();
        // A reply for the first, superseded review must be rejected; the latest passes.
        assert!(!s.review_is_current(epoch1));
        assert!(s.review_is_current(epoch2));
        // begin_review sets busy (mirrors review_shield).
        assert!(s.busy);
    }

    #[test]
    fn cancel_hold_is_a_noop_when_nothing_is_held() {
        // The hold gesture is retired (confirm is now ⌘↵ / a click); `cancel_hold` survives only
        // as the leave-a-surface guard and must be a no-op when nothing is held.
        let mut s = state_with_proposal();
        assert!(!s.holding);
        assert!(!s.cancel_hold());
    }

    #[test]
    fn reset_invalidates_the_review_epoch_and_clears_state() {
        let mut s = state_with_proposal();
        let review = s.begin_review();
        s.busy = false;
        s.holding = true;
        s.error = Some("boom".into());
        s.tx = Some(B256::repeat_byte(0xab));

        s.reset();

        // The review epoch moved past its pre-reset value.
        assert!(!s.review_is_current(review));
        // Everything transient is cleared.
        assert!(s.proposal.is_none());
        assert!(s.tx.is_none());
        assert!(!s.busy);
        assert!(!s.holding);
        assert!(s.error.is_none());
    }
}
