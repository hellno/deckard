//! #6 — the swap signing lifecycle, driven against the REAL `deckard-signerd` binary over the
//! socket. NO chain is needed: `sign_order` is a pure EIP-712 ECDSA sign (no broadcast), so a
//! dead RPC is fine. The daemon runs on Sepolia (11155111) because `propose_order` chain-checks
//! the order against the configured chain id.
//!
//! Covered:
//!   - happy path: propose_order → Resolve(approve) → sign_order → 65-byte signature, and the
//!     recovered signer == the unlocked wallet (proving the daemon signed the order it stored,
//!     with owner rebound to the wallet);
//!   - reject path: a different order, Resolve(false) → sign_order Denied (no signature);
//!   - locked daemon: propose_order on a never-unlocked daemon → Deny{locked};
//!   - STOP kills a pending order: propose_order (pending) → RevokeAll → sign_order Denied;
//!   - TOCTOU: propose_order → Resolve(true) → RevokeAll → sign_order Denied{revoked};
//!   - idempotent re-propose: the same order proposed twice mints the SAME request id.
//!
//! We drive the daemon via the raw async `client.request(&SignerRequest::…)` path (the same
//! pattern `daemon_e2e.rs` uses for Resolve / RevokeAll / Status) so everything runs cleanly
//! inside one `#[tokio::test]` runtime — no nested `block_on`.

mod common;

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{Address, Signature, B256, U256};
use deckard_contract::{
    ApprovalStatus, Decision, ProposalOrigin, RequestId, SignOrderResult, SignerRequest,
    SignerResponse, SwapOrder,
};
use deckard_core::{order_digest, APP_DATA_HASH};
use deckard_signerd::SignerClient;

use common::*;

/// Sepolia — `propose_order` requires a supported chain id (mainnet 1 or Sepolia 11155111),
/// and the daemon is configured for this chain.
const CHAIN: u64 = 11155111;
/// `sign_order` never broadcasts, so the RPC is never contacted — a dead address is fine.
const DUMMY_RPC: &str = "http://127.0.0.1:1";

// Sepolia token addresses (from the frozen interface). The exact tokens don't matter for the
// signing path (no allowlist is configured → any token is admitted); these are just realistic.
fn sepolia_weth() -> Address {
    Address::from_str("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14").expect("WETH addr")
}
fn sepolia_cow() -> Address {
    Address::from_str("0x0625aFB445C3B6B7B929342a04A22599fd5dBB59").expect("COW addr")
}

/// Unix seconds now (the daemon uses the same wall clock at propose time).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_secs()
}

/// A well-formed order for `receiver`/`owner = wallet`, `valid_to` one hour out (well inside the
/// 24h horizon). `owner` is set to `wallet` to match the daemon's rebind (the daemon overwrites
/// it anyway, but matching keeps the local digest identical to the stored one without recompute).
fn order_for(wallet: Address, sell: Address, buy: Address, sell_amount: u64) -> SwapOrder {
    SwapOrder {
        chain_id: CHAIN,
        owner: wallet,
        sell_token: sell,
        buy_token: buy,
        sell_amount: U256::from(sell_amount),
        buy_amount_min: U256::from(sell_amount / 2),
        receiver: wallet,
        valid_to: (now_secs() + 3600) as u32,
        app_data: APP_DATA_HASH,
    }
}

// --- request helpers (raw async path, mirrors daemon_e2e.rs) --------------------------------

async fn propose_order(client: &SignerClient, order: &SwapOrder) -> Decision {
    match client
        .request(&SignerRequest::ProposeOrder {
            order: order.clone(),
            origin: ProposalOrigin::App,
        })
        .await
        .unwrap()
    {
        SignerResponse::Decision(d) => d,
        other => panic!("expected Decision for ProposeOrder, got {other:?}"),
    }
}

async fn sign_order(client: &SignerClient, id: RequestId) -> SignOrderResult {
    match client
        .request(&SignerRequest::SignOrder { request_id: id })
        .await
        .unwrap()
    {
        SignerResponse::SignOrder(r) => r,
        other => panic!("expected SignOrder for SignOrder, got {other:?}"),
    }
}

/// Approve/deny over the authenticated control channel — a `Resolve` on the public socket is
/// refused (PRD-01). Sync (a quick socketpair round-trip); callers drop the `.await`.
fn resolve(d: &DaemonProc, id: RequestId, approved: bool) {
    d.resolve(id, approved);
}

async fn revoke_all(client: &SignerClient) {
    assert_eq!(
        client.request(&SignerRequest::RevokeAll).await.unwrap(),
        SignerResponse::Ack
    );
}

async fn needs_approval_id(client: &SignerClient, order: &SwapOrder) -> RequestId {
    match propose_order(client, order).await {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

/// Recover the signer address from a 65-byte r||s||v signature over `digest` (the same decode
/// `signing.rs`'s own round-trip test performs). The daemon rebinds the order's `owner` to the
/// unlocked wallet, so the digest passed here must be recomputed over `owner = wallet`.
fn recover(digest: B256, signature: &[u8]) -> Address {
    assert_eq!(signature.len(), 65, "EIP-712 signature must be 65 bytes");
    let (r_bytes, rest) = signature.split_at(32);
    let (s_bytes, v_byte) = rest.split_at(32);
    let r = U256::from_be_slice(r_bytes);
    let s = U256::from_be_slice(s_bytes);
    let v = v_byte.first().copied().expect("v byte");
    assert!(v == 27 || v == 28, "v must be legacy 27/28, got {v}");
    let sig = Signature::new(r, s, v == 28);
    sig.recover_address_from_prehash(&digest)
        .expect("recover address from prehash")
}

#[tokio::test]
async fn happy_path_propose_approve_sign_recovers_wallet() {
    let dir = TempDir::new("swap-happy");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 1_000_000);
    let id = needs_approval_id(&client, &order).await;

    // Approve, then sign.
    resolve(&d, id, true);
    let sig = match sign_order(&client, id).await {
        SignOrderResult::Signed { signature } => signature,
        other => panic!("expected Signed, got {other:?}"),
    };
    assert_eq!(sig.len(), 65, "signature must be 65 bytes (r||s||v)");

    // The daemon rebinds owner = wallet; recompute the digest over the BOUND order for recovery.
    let bound = SwapOrder {
        owner: wallet,
        ..order.clone()
    };
    let digest = order_digest(&bound);
    let recovered = recover(digest, &sig);
    assert_eq!(
        recovered, wallet,
        "the recovered signer must be the unlocked wallet"
    );
}

#[tokio::test]
async fn reject_path_resolve_false_then_sign_denied() {
    let dir = TempDir::new("swap-reject");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // A DIFFERENT order (distinct amount → distinct id from the happy path).
    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 2_222_222);
    let id = needs_approval_id(&client, &order).await;

    resolve(&d, id, false);
    match sign_order(&client, id).await {
        SignOrderResult::Denied { .. } => {}
        SignOrderResult::Signed { .. } => panic!("a user-denied order must NOT sign"),
    }
}

#[tokio::test]
async fn locked_daemon_propose_order_denies_locked() {
    let dir = TempDir::new("swap-locked");
    let (wallet, _recipient) = seal_account0(dir.path());
    // Never unlocked → the daemon has no key to bind the owner/receiver.
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 1_000_000);
    assert_eq!(
        propose_order(&client, &order).await,
        Decision::Deny {
            reason: "locked".into()
        },
        "a locked daemon must refuse to propose an order"
    );
}

#[tokio::test]
async fn revoke_all_kills_pending_order() {
    let dir = TempDir::new("swap-revoke-pending");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 3_333_333);
    let id = needs_approval_id(&client, &order).await;

    // STOP fires while the order is still Pending. RevokeAll may attempt an on-chain cancel of
    // SIGNED orders against the dead RPC and fail fast — that's fine; STOP must still complete
    // (the Ack) and the pending order must be dead.
    revoke_all(&client).await;
    match sign_order(&client, id).await {
        SignOrderResult::Denied { .. } => {}
        SignOrderResult::Signed { .. } => panic!("a revoked pending order must NOT sign"),
    }
}

#[tokio::test]
async fn toctou_approve_then_revoke_then_sign_denied_revoked() {
    let dir = TempDir::new("swap-toctou");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 4_444_444);
    let id = needs_approval_id(&client, &order).await;

    // Approve BEFORE the STOP, then STOP, then attempt to sign — the sign-time revoked re-check
    // is the only thing standing between an approval and a signature.
    resolve(&d, id, true);
    revoke_all(&client).await;
    match sign_order(&client, id).await {
        SignOrderResult::Denied { reason } => assert_eq!(
            reason, "revoked",
            "a STOP after approval must deny with `revoked`"
        ),
        SignOrderResult::Signed { .. } => panic!("a revoked order must NOT sign after a STOP"),
    }
}

#[tokio::test]
async fn stop_attempts_cancel_of_a_signed_order_and_stays_responsive() {
    // The STOP guarantee for SIGNED orders: a signed order is loose on the orderbook, so STOP
    // best-effort cancels it ON-CHAIN before zeroizing. Here the cancel broadcast hits the dead
    // DUMMY_RPC and fails fast — STOP must still complete (return Ack within the per-cancel
    // timeout, never hang) and the order must be dead afterward. This exercises `stop()`'s
    // signed-order selection branch (a pending order would skip the on-chain cancel entirely).
    let dir = TempDir::new("swap-stop-signed");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 6_666_666);
    let id = needs_approval_id(&client, &order).await;
    resolve(&d, id, true);
    // Sign it — now `signature.is_some()`, so STOP will SELECT it for an on-chain cancel.
    match sign_order(&client, id).await {
        SignOrderResult::Signed { .. } => {}
        other => panic!("expected Signed, got {other:?}"),
    }

    // STOP: must complete (Ack) despite the cancel broadcast failing fast against the dead RPC.
    revoke_all(&client).await;

    // The order is dead: a re-sign on the now-locked daemon is refused.
    match sign_order(&client, id).await {
        SignOrderResult::Denied { .. } => {}
        SignOrderResult::Signed { .. } => panic!("a signed order must not re-sign after STOP"),
    }
}

#[tokio::test]
async fn re_propose_order_is_idempotent_same_id() {
    let dir = TempDir::new("swap-idempotent");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let order = order_for(wallet, sepolia_weth(), sepolia_cow(), 5_555_555);

    // The daemon rebinds owner = wallet before computing the id, so the locally-derived id is
    // over the BOUND order.
    let bound = SwapOrder {
        owner: wallet,
        ..order.clone()
    };
    let expected = SignerClient::request_id_for_swap_order(&bound);

    let id1 = needs_approval_id(&client, &order).await;
    // A second propose of the identical order must NOT mint a new card — same id, still pending.
    let id2 = needs_approval_id(&client, &order).await;
    assert_eq!(
        id1, id2,
        "re-proposing the same order must yield the same id"
    );
    assert_eq!(
        id1, expected,
        "the daemon's id must equal request_id_for_order over the bound order"
    );

    // And the record is still a live pending order under that id.
    let status = match client
        .request(&SignerRequest::Status { request_id: id1 })
        .await
        .unwrap()
    {
        SignerResponse::Status(s) => s,
        other => panic!("expected Status, got {other:?}"),
    };
    assert_eq!(status, ApprovalStatus::Pending);
}
