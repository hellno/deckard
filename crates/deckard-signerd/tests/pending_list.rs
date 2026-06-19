//! `PendingList` snapshot tests for the agent-approval inbox (Lane A). These pin the two NEW
//! `PendingRecord` fields the GUI inbox reads — `origin` (who proposed) and `remaining_ms` (the
//! TTL countdown, a daemon-computed snapshot) — and the expire-BEFORE-list guarantee: the list
//! never surfaces a `Pending` row past its 120s TTL.
//!
//! No chain is needed (nothing is broadcast — every assertion is about the inbox snapshot, not
//! signing), so these run under plain `cargo test`.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ActivityLifecycle, ApprovalStatus, BreachedLimit, Decision, Intent, IntentKind, ProposalOrigin,
    SignerRequest, SignerResponse,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
/// Nothing is broadcast here, so the RPC is never contacted.
const DUMMY_RPC: &str = "http://127.0.0.1:1";
const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH (the default policy cap)
/// The frozen 120s approval TTL ceiling — a fresh `remaining_ms` can never exceed it.
const APPROVAL_TTL_MS: u64 = 120_000;

fn send(to: Address, value: u64) -> Intent {
    Intent {
        chain_id: CHAIN,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
}

/// Propose an over-cap Send and return its `NeedsApproval` request id (a `Pending` card).
async fn pending_id(
    client: &SignerClient,
    to: Address,
    value: u64,
    origin: ProposalOrigin,
) -> deckard_contract::RequestId {
    match client.propose(&send(to, value), origin).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

/// A1: an over-cap App send is surfaced in the inbox as a `Pending` record tagged `App`, with a
/// live `remaining_ms` inside the frozen TTL window (`0 < remaining_ms <= 120_000`).
#[tokio::test]
async fn pending_list_reports_app_origin_and_live_remaining() {
    let dir = TempDir::new("pending-list-app");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::App).await;

    let records = client.pending_list().await.unwrap();
    let rec = records
        .iter()
        .find(|r| r.request_id == id)
        .expect("the proposed record must appear in the inbox");

    assert_eq!(
        rec.origin,
        ProposalOrigin::App,
        "an app-proposed record must be tagged App"
    );
    assert_eq!(
        rec.status,
        ApprovalStatus::Pending,
        "an un-resolved over-cap send is Pending"
    );
    assert!(
        rec.remaining_ms > 0 && rec.remaining_ms <= APPROVAL_TTL_MS,
        "a live Pending row must carry 0 < remaining_ms <= 120_000, got {}",
        rec.remaining_ms
    );
}

/// A1 (agent half): an order is agent-proposed, so a record the agent raises is tagged `Agent`.
/// Proposing the over-cap send over a second client tagged `Agent` proves the origin is carried
/// from the proposal through to the inbox (not hard-coded).
#[tokio::test]
async fn pending_list_reports_agent_origin() {
    let dir = TempDir::new("pending-list-agent");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::Agent).await;

    let records = client.pending_list().await.unwrap();
    let rec = records
        .iter()
        .find(|r| r.request_id == id)
        .expect("the proposed record must appear in the inbox");
    assert_eq!(
        rec.origin,
        ProposalOrigin::Agent,
        "an agent-proposed record must be tagged Agent"
    );
}

/// A2: expire-BEFORE-list. With a 1s TTL, a `Pending` record left to go stale is surfaced as
/// `Expired` (NOT Pending) with `remaining_ms == 0` — proving `pending_list` runs `expire_stale`
/// first, so the inbox never shows a row past its TTL as still pending.
#[tokio::test]
async fn pending_list_expires_before_listing() {
    let dir = TempDir::new("pending-list-expire");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::App).await;

    // Wait past the 1s TTL, then list — the row must be expired, not pending.
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    let records = client.pending_list().await.unwrap();
    let rec = records
        .iter()
        .find(|r| r.request_id == id)
        .expect("the record must still appear (terminal, not dropped)");

    assert_eq!(
        rec.status,
        ApprovalStatus::Expired,
        "pending_list must expire a stale row BEFORE listing it"
    );
    assert_eq!(
        rec.remaining_ms, 0,
        "an expired row carries no remaining time"
    );
}

/// A3: a terminal (Denied) record carries `remaining_ms == 0`. A user-denied record (via the
/// authenticated control channel) is the canonical terminal-but-not-expired case.
#[tokio::test]
async fn pending_list_terminal_record_has_zero_remaining() {
    let dir = TempDir::new("pending-list-terminal");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::App).await;
    // Deny it over the control channel → terminal `Denied`, while well within the TTL window.
    d.resolve(id, false);

    let records = client.pending_list().await.unwrap();
    let rec = records
        .iter()
        .find(|r| r.request_id == id)
        .expect("the denied record must still appear in the inbox");

    assert!(
        matches!(rec.status, ApprovalStatus::Denied { .. }),
        "the record must be terminal Denied, got {:?}",
        rec.status
    );
    assert_eq!(
        rec.remaining_ms, 0,
        "a terminal Denied row carries 0 remaining_ms even before the TTL elapses"
    );
}

// ─────────────────────────── activity feed (#60) ───────────────────────────
// The feed is the see-and-stop ledger: unlike `pending_list` it also retains auto-allowed and
// executed rows, and it cites the ACTUAL breached cap. These pin the no-chain half (pending,
// auto-allowed, expire-on-read, lock-shows-revoke); the executed-with-tx-hash half needs a real
// broadcast and lives in `anvil_e2e.rs`.

/// #60: an over-cap proposal appears in the feed as `Proposed`, tagged with the actor, a
/// daemon-stamped `timestamp_ms`, no `tx_hash` yet, and the ACTUAL breached cap (per-tx here,
/// since the value alone clears the per-tx ceiling) — never a hardcoded cite.
#[tokio::test]
async fn activity_feed_pending_carries_reason_and_timestamp() {
    let dir = TempDir::new("activity-pending");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::Agent).await;

    let feed = client.activity_feed().await.unwrap();
    let rec = feed
        .iter()
        .find(|r| r.request_id == id)
        .expect("the over-cap proposal must appear in the feed");

    assert_eq!(
        rec.origin,
        ProposalOrigin::Agent,
        "actor carried into the feed"
    );
    assert_eq!(
        rec.lifecycle,
        ActivityLifecycle::Proposed,
        "still awaiting a human"
    );
    assert!(
        !rec.auto_allowed,
        "an over-cap card was NOT auto-allowed hands-free — a human is in the loop"
    );
    assert_eq!(rec.tx_hash, None, "nothing broadcast yet");
    assert_eq!(
        rec.reason,
        BreachedLimit::PerTxCap,
        "the value alone clears the per-tx cap, so the feed cites per-tx (not a hardcoded string)"
    );
    assert!(
        rec.timestamp_ms > 0,
        "the daemon stamps a wall-clock millis timestamp"
    );
}

/// #60: a WITHIN-cap action auto-allows off mainnet and lands in the feed as `Decided{approved}`
/// with no breached cap — the "auto-approved within cap" row the issue calls out. It never
/// enters `PendingList` (it is not pending), so the feed is the only surface that shows it.
#[tokio::test]
async fn activity_feed_within_cap_auto_allow_is_decided() {
    let dir = TempDir::new("activity-autoallow");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // Under the per-tx cap (CHAIN 31337 ≠ mainnet, so the guardrail is inactive → auto-allow).
    let intent = send(to, PER_TX_CAP - 1);
    assert_eq!(
        client
            .propose(&intent, ProposalOrigin::Agent)
            .await
            .unwrap(),
        Decision::Allow,
        "a within-cap send auto-allows off mainnet"
    );
    let id = SignerClient::request_id_for_intent(&intent);

    // It is NOT in the pending queue (auto-allows don't wait)…
    let pending = client.pending_list().await.unwrap();
    let prec = pending.iter().find(|r| r.request_id == id).unwrap();
    assert_eq!(prec.status, ApprovalStatus::Allowed);

    // …but it IS in the feed, decided-approved with no cap breached and no tx hash (not yet sent).
    let feed = client.activity_feed().await.unwrap();
    let rec = feed.iter().find(|r| r.request_id == id).unwrap();
    assert_eq!(rec.lifecycle, ActivityLifecycle::Decided { approved: true });
    assert!(
        rec.auto_allowed,
        "a within-cap auto-allow off mainnet is hands-free (auto_allowed) — the feed must say so"
    );
    assert_eq!(
        rec.reason,
        BreachedLimit::None,
        "within cap → no breached fence"
    );
    assert_eq!(rec.tx_hash, None, "decided but not yet executed");
}

/// #60: the feed expires BEFORE reading, so a lapsed `Pending` card never shows as still
/// proposed. A never-decided lapse maps to its own `ActivityLifecycle::Expired` (NOBODY acted),
/// kept distinct from a human denial / STOP revoke so the feed renders it neutral.
#[tokio::test]
async fn activity_feed_expires_before_read() {
    let dir = TempDir::new("activity-expire");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::Agent).await;
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;

    let feed = client.activity_feed().await.unwrap();
    let rec = feed.iter().find(|r| r.request_id == id).unwrap();
    assert_eq!(
        rec.lifecycle,
        ActivityLifecycle::Expired,
        "a lapsed card reads as its own `Expired` lifecycle (NOBODY acted), distinct from a human \
         denial / STOP revoke — so the feed renders it neutral, never the amber 'you acted' tint"
    );
}

/// #60 amber-honesty: a card a human APPROVED (resolve → Allowed) that then lapses before execute
/// fires must still read `Decided{approved:true}` — a human acted, so the row stays amber. Only a
/// NEVER-approved lapse is the neutral `Expired` (see `activity_feed_expires_before_read`).
/// `expire_stale` flips both Pending AND Allowed past-TTL records to `Expired`, so this guards that
/// `activity_lifecycle` consults `req.approved` and does not erase the human-action signal.
#[tokio::test]
async fn activity_feed_human_approved_then_lapsed_stays_decided() {
    let dir = TempDir::new("activity-approved-lapse");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::Agent).await;
    d.resolve(id, true); // human approves: status → Allowed, approved = true
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await; // lapse before execute

    let feed = client.activity_feed().await.unwrap();
    let rec = feed.iter().find(|r| r.request_id == id).unwrap();
    assert_eq!(
        rec.lifecycle,
        ActivityLifecycle::Decided { approved: true },
        "a human-approved card that lapsed before execute must stay Decided{{approved:true}} (a \
         human acted → amber), never the neutral, human-absent Expired"
    );
}

/// #60 acceptance 3: STOP (`RevokeAll`) flips an in-flight proposal to revoked, and the feed
/// shows it — `Decided{approved:false}`, no tx hash. The feed is the surface that proves the kill.
#[tokio::test]
async fn activity_feed_lock_shows_revoke() {
    let dir = TempDir::new("activity-revoke");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = pending_id(&client, to, PER_TX_CAP + 1, ProposalOrigin::Agent).await;

    // STOP: the panic brake denies every in-flight record.
    assert_eq!(
        client.request(&SignerRequest::RevokeAll).await.unwrap(),
        SignerResponse::Ack
    );

    let feed = client.activity_feed().await.unwrap();
    let rec = feed.iter().find(|r| r.request_id == id).unwrap();
    assert_eq!(
        rec.lifecycle,
        ActivityLifecycle::Decided { approved: false },
        "STOP revokes the in-flight card; the feed shows it as a non-approval"
    );
    assert_eq!(rec.tx_hash, None, "a revoked card was never broadcast");
}
