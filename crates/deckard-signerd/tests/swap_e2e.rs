//! Repeatable, black-box SWAP integration test — the full on-fork swap TRUST PATH, driven
//! end-to-end through DECKARD'S OWN daemon against a fresh anvil fork of Sepolia.
//!
//! ## Why this drives the trust path, NOT a live CoW submission
//! A real CoW order can NOT be accepted+open from a local fork: the live orderbook
//! (`api.cow.fi/sepolia`) validates balances against the REAL Sepolia chain, not our fork, so a
//! fork order is always rejected. That is the exact wall the swap feature hit. The honest seam is:
//!   - this test proves everything the DAEMON owns on a fork — propose → human-approval (control
//!     channel) → the real exact-gross relayer approve broadcast → sign_order → signature recovers
//!     to the wallet — plus a balance-set demonstration of the simulated fill;
//!   - the actual order completion (quote/put_app_data/submit) is what the in-fork STUB in
//!     deckard-core's `CowOrderbook` does (gated by `DECKARD_DEMO_SWAP_STUB`), and that stub is
//!     what the app and the MCP sidecar use to close the loop on the demo fork. This test does
//!     NOT touch `CowOrderbook` — the daemon must never compile the CoW HTTP client (see
//!     `feature_gate.rs`), and cargo would unify a dev `cow-client` feature back onto the
//!     signerd→core edge and trip that gate. So the fill here is demonstrated with a direct
//!     `anvil_setStorageAt` (the same cheatcode the stub uses), not by submitting an order.
//!
//! `swap_lifecycle.rs` already covers the pure sign lifecycle (no chain). This test adds what only
//! a fork makes possible: the real on-chain exact-gross approve broadcast + allowance assertion,
//! and the simulated buy-token fill.
//!
//! `#[ignore]` (needs network + a fresh anvil). Run:
//!   RPC_URL_SEPOLIA=<archive-rpc> \
//!   cargo test -p deckard-signerd --test swap_e2e -- --ignored --nocapture
//! or: RPC_URL_SEPOLIA=<archive-rpc> just swap-e2e
//!
//! It spawns its OWN fresh anvil fork each run (deterministic — a re-used non-reset fork drifts
//! the seeded balances and allowance) and kills it on drop.

mod common;

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::{Address, Bytes, Signature, B256, U256};
use deckard_contract::{
    Decision, ExecuteResult, Intent, IntentKind, SignOrderResult, SignerRequest, SignerResponse,
    SwapOrder,
};
use deckard_core::{order_digest, APPROVE_SELECTOR, APP_DATA_HASH, GPV2_VAULT_RELAYER};
use deckard_signerd::SignerClient;

use common::*;

/// Pinned fork block (pre-verified, same as `shield_e2e` / `just demo`). A fixed block keeps the
/// asserts deterministic.
const FORK_BLOCK: u64 = 10_822_990;
/// Sepolia chain id — the fork preserves it; the daemon's chain_id + the order must match it.
const SEPOLIA_CHAIN_ID: u64 = 11_155_111;

/// Sepolia WETH (WETH9 layout): the SELL token. Its `balanceOf` mapping is the 4th storage var
/// (name, symbol, decimals, balanceOf) → slot 3. Seeded so the wallet holds the sell token and the
/// real exact-gross approve broadcast succeeds on the fork.
fn sepolia_weth() -> Address {
    Address::from_str("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14").expect("WETH addr")
}
const WETH_BALANCES_SLOT: u64 = 3;

/// Sepolia COW (OpenZeppelin ERC-20): the BUY token. Its `_balances` mapping is the first storage
/// var → slot 0. Seeded after sign to DEMONSTRATE the simulated fill the stub performs (the daemon
/// never moves the buy token itself; the stub / a real fill does).
fn sepolia_cow() -> Address {
    Address::from_str("0x0625aFB445C3B6B7B929342a04A22599fd5dBB59").expect("COW addr")
}
const COW_BALANCES_SLOT: u64 = 0;

/// Sepolia archive RPC serving the pinned fork block — supplied ONLY via `RPC_URL_SEPOLIA`. No RPC
/// key is baked into the source (W0 publish-blocker; see SECURITY.md). Use any free Sepolia
/// *archive* endpoint (Alchemy / Infura / dRPC).
fn sepolia_rpc() -> String {
    std::env::var("RPC_URL_SEPOLIA").unwrap_or_else(|_| {
        panic!(
            "RPC_URL_SEPOLIA is not set — this test needs a Sepolia *archive* RPC that \
             serves the pinned fork block {FORK_BLOCK}. Set a free archive endpoint and \
             re-run, e.g.:\n  \
             RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/<your-key> \\\n  \
             cargo test -p deckard-signerd --test swap_e2e -- --ignored --nocapture"
        )
    })
}

/// Unix seconds now (the daemon uses the same wall clock at propose time).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_secs()
}

/// A well-formed sell order for `receiver`/`owner = wallet`, `valid_to` one hour out (well inside
/// the 24h horizon). `owner` is set to `wallet` to match the daemon's rebind, so the locally-
/// derived digest is identical to the stored one (mirrors `swap_lifecycle.rs::order_for`).
fn order_for(wallet: Address, sell: Address, buy: Address, sell_amount: u64) -> SwapOrder {
    SwapOrder {
        chain_id: SEPOLIA_CHAIN_ID,
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

/// The exact-gross ERC-20 `approve(GPV2_VAULT_RELAYER, gross)` intent against the sell token —
/// byte-identical to the app's `build_exact_approve_intent` (#25). The daemon admits it ONLY
/// because a matching pending order exists (same sell token + same gross), then it broadcasts the
/// calldata as-is.
fn exact_approve_intent(sell_token: Address, gross: U256) -> Intent {
    let mut calldata = Vec::with_capacity(4 + 32 + 32);
    calldata.extend_from_slice(&APPROVE_SELECTOR);
    // address arg: left-pad the 20-byte spender to a 32-byte word.
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(GPV2_VAULT_RELAYER.as_slice());
    // uint256 arg: the big-endian 32-byte gross amount.
    calldata.extend_from_slice(&gross.to_be_bytes::<32>());
    Intent {
        chain_id: SEPOLIA_CHAIN_ID,
        to: sell_token,
        token: None,
        value: U256::ZERO,
        calldata: Bytes::from(calldata),
        kind: IntentKind::ContractCall,
    }
}

// --- request helpers (raw async path, mirrors swap_lifecycle.rs / daemon_e2e.rs) ------------

async fn propose_order(client: &SignerClient, order: &SwapOrder) -> Decision {
    match client
        .request(&SignerRequest::ProposeOrder {
            order: order.clone(),
        })
        .await
        .unwrap()
    {
        SignerResponse::Decision(d) => d,
        other => panic!("expected Decision for ProposeOrder, got {other:?}"),
    }
}

async fn needs_approval_order_id(
    client: &SignerClient,
    order: &SwapOrder,
) -> deckard_contract::RequestId {
    match propose_order(client, order).await {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval for the order, got {other:?}"),
    }
}

async fn sign_order(client: &SignerClient, id: deckard_contract::RequestId) -> SignOrderResult {
    match client
        .request(&SignerRequest::SignOrder { request_id: id })
        .await
        .unwrap()
    {
        SignerResponse::SignOrder(r) => r,
        other => panic!("expected SignOrder for SignOrder, got {other:?}"),
    }
}

/// Recover the signer address from a 65-byte r||s||v signature over `digest` (mirrors
/// `swap_lifecycle.rs::recover`).
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
#[ignore = "network: spawns a fresh anvil Sepolia fork + drives the full swap trust path"]
async fn swap_e2e_trust_path() {
    if !anvil_available() {
        eprintln!("SKIP swap_e2e_trust_path: anvil not on PATH");
        return;
    }

    // --- fresh anvil fork of Sepolia @ the pinned block (chain id preserved) ---
    let anvil = start_anvil_fork(&sepolia_rpc(), FORK_BLOCK, SEPOLIA_CHAIN_ID);
    wait_anvil_ready(&anvil.url()).await;
    assert_eq!(anvil.chain_id(), SEPOLIA_CHAIN_ID);

    // --- daemon, sealed for anvil account-0 (the funded EOA), pointed at the fork ---
    let dir = TempDir::new("swap-e2e");
    let (wallet, _recipient) = seal_account0(dir.path());
    // The daemon never talks to a CoW backend (it has no cow-client; see feature_gate.rs), and the
    // fill here is demonstrated directly via anvil_setStorageAt below — exactly what the stub does.
    // So no DECKARD_DEMO_SWAP_STUB is needed: the stub lives in the app/MCP's CowOrderbook, not the
    // daemon.
    let d = spawn_daemon(dir.path(), &anvil.url(), SEPOLIA_CHAIN_ID, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    let unlocked = client.unlock(PASS).await.unwrap();
    assert_eq!(
        unlocked,
        deckard_contract::UnlockOutcome::Unlocked { address: wallet },
        "daemon's account-0 must be the funded EOA"
    );

    // --- seed the SELL token (WETH) so the real exact-gross approve broadcast succeeds ---
    // The order sells `gross`; give the wallet 10x headroom so the approve + a real pull would fit.
    let gross: u64 = 1_000_000;
    let weth = sepolia_weth();
    set_erc20_balance(
        &anvil.url(),
        weth,
        wallet,
        U256::from(WETH_BALANCES_SLOT),
        U256::from(gross * 10),
    )
    .await;
    assert_eq!(
        erc20_balance(&anvil.url(), weth, wallet).await,
        U256::from(gross * 10),
        "WETH seed must land in the wallet's balanceOf slot"
    );

    // ============================ DECKARD'S OWN PATH ============================
    // 1. Propose the order → ALWAYS NeedsApproval (swaps never auto-allow in v1).
    let cow = sepolia_cow();
    let order = order_for(wallet, weth, cow, gross);
    let order_id = needs_approval_order_id(&client, &order).await;

    // 2. A human approves the order over the control channel (the MCP sidecar can NOT do this —
    //    that is the no-self-approve property).
    d.resolve(order_id, true);

    // 3. The exact-gross relayer approve — admitted ONLY because the order is now pending — is
    //    proposed, approved, and broadcast ON the fork (this is the real on-chain step). The
    //    relayer starts with no allowance for a fresh wallet.
    assert_eq!(
        relayer_allowance(&anvil.url(), weth, wallet).await,
        U256::ZERO,
        "the relayer must start with no allowance"
    );
    let approve = exact_approve_intent(weth, U256::from(gross));
    let approve_id = SignerClient::request_id_for_intent(&approve);
    match client.propose(&approve).await.unwrap() {
        // The shaped approve is stored Pending (a card), so it comes back NeedsApproval.
        Decision::NeedsApproval { .. } | Decision::Allow => {}
        Decision::Deny { reason } => {
            panic!("the shaped approve must be admitted, got Deny({reason})")
        }
    }
    d.resolve(approve_id, true);
    let approve_tx = match client.execute(approve_id).await.unwrap() {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected the approve to Broadcast, got {other:?}"),
    };
    let receipt = wait_receipt(&anvil.url(), approve_tx)
        .await
        .expect("a mined approve receipt");
    assert!(receipt.status(), "the approve tx must succeed on-chain");

    // The relayer is now approved for EXACTLY the gross sell amount.
    assert_eq!(
        relayer_allowance(&anvil.url(), weth, wallet).await,
        U256::from(gross),
        "the relayer allowance must equal the gross sell amount after the approve"
    );

    // 4. Sign the approved order → a 65-byte signature that recovers to the wallet (the daemon
    //    signed the order it stored, with owner bound to the wallet). NO HTTP.
    let signature = match sign_order(&client, order_id).await {
        SignOrderResult::Signed { signature } => signature,
        other => panic!("expected Signed, got {other:?}"),
    };
    assert_eq!(signature.len(), 65, "signature must be 65 bytes (r||s||v)");
    let bound = SwapOrder {
        owner: wallet,
        ..order.clone()
    };
    let recovered = recover(order_digest(&bound), &signature);
    assert_eq!(
        recovered, wallet,
        "the recovered signer must be the unlocked wallet"
    );
    // ============================================================================

    // 5. Demonstrate the SIMULATED fill the stub performs: credit the buy token (COW) to the
    //    wallet on the fork. The live orderbook can't accept this fork order, so the stub (used by
    //    the app/MCP, gated by DECKARD_DEMO_SWAP_STUB) credits the buy token directly. Here we do
    //    the same cheatcode to prove the buy-side balance moves as a real fill would.
    let before_cow = erc20_balance(&anvil.url(), cow, wallet).await;
    set_erc20_balance(
        &anvil.url(),
        cow,
        wallet,
        U256::from(COW_BALANCES_SLOT),
        before_cow + order.buy_amount_min,
    )
    .await;
    let after_cow = erc20_balance(&anvil.url(), cow, wallet).await;
    assert_eq!(
        after_cow - before_cow,
        order.buy_amount_min,
        "the simulated fill must credit the buy token by at least the min-receive amount"
    );

    println!(
        "=== swap_e2e PASSED: approve broadcast (allowance {gross}), order signed (recovers \
         to {wallet}), simulated fill credited +{} COW ===",
        order.buy_amount_min
    );
}

/// The ERC-20 `allowance(wallet, GPV2_VAULT_RELAYER)` of `token` via `url` — `eth_call` of the
/// 0xdd62ed3e selector, decoded as a big-endian U256. Mirrors `common::erc20_balance` but for the
/// two-arg allowance.
async fn relayer_allowance(url: &str, token: Address, owner: Address) -> U256 {
    use alloy::network::{Ethereum, TransactionBuilder};
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy::rpc::types::TransactionRequest;

    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    // allowance(address,address) selector ‖ owner ‖ spender, each left-padded to 32 bytes.
    let mut calldata = Vec::with_capacity(4 + 32 + 32);
    calldata.extend_from_slice(&[0xdd, 0x62, 0xed, 0x3e]);
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(owner.as_slice());
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(GPV2_VAULT_RELAYER.as_slice());

    let mut tx = TransactionRequest::default();
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_to(&mut tx, token);
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_input(&mut tx, Bytes::from(calldata));
    let raw = provider.call(tx).await.expect("allowance eth_call");
    U256::from_be_slice(raw.as_ref())
}
