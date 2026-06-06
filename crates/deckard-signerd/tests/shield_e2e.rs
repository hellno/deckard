//! Repeatable, black-box SHIELD integration test — the privacy hero, driven end-to-end
//! through DECKARD'S OWN path against a fresh anvil fork of Sepolia.
//!
//! What it proves (the privacy property, shield-only — fast, NO ZK proving / artifacts):
//!   1. deckard-core's KEY-LESS builder turns `(chain, 0zk recipient, value)` into an
//!      `Intent{kind:Shield, to=RelayAdapt, value, calldata}` — no spending key, no sync.
//!   2. The daemon admits + signs + broadcasts that Intent (generalized broadcast carries the
//!      calldata; the daemon never touches the ZK crate — it just signs the handed bytes).
//!   3. After `railgun.sync()`, the recipient's PRIVATE 0zk balance is up by exactly
//!      `value - value*25/10000` (the on-chain 25-bps shield fee; the calldata carried the
//!      gross value), and the EOA's PUBLIC balance is down by ~value + gas.
//!
//! Shield does NO client ZK proof (the de-risked finding): `ShieldBuilder::build` only
//! encrypts the note + ABI-encodes — the contract verifies the commitment. So this is fast.
//! Transfer/unshield (slow; download ZK artifacts) stay in `spikes/shield-railgun`, NOT here.
//!
//! `#[ignore]` (needs network + a fresh anvil). Run:
//!   RUSTUP_TOOLCHAIN=1.95.0-aarch64-apple-darwin \
//!   RPC_URL_SEPOLIA=<archive-rpc> \
//!   cargo test -p deckard-signerd --test shield_e2e -- --ignored --nocapture
//!
//! It spawns its OWN fresh anvil fork each run (deterministic — a re-used non-reset fork
//! accumulates the EOA balance and drifts the asserts) and kills it on drop.

#![cfg(feature = "shield")]

mod common;

use std::sync::Arc;

use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::U256;

use deckard_contract::{Decision, ExecuteResult};
use deckard_core::build_shield_native_intent;
use deckard_signerd::SignerClient;

use railgun::{
    account::signer::{PrivateKeySigner as RailgunKeySigner, RailgunSigner},
    builder::RailgunBuilder,
    caip::AssetId,
    chain_config::ChainConfig,
    indexer::syncer::{ChainedSyncer, RpcSyncer, SubsquidSyncer},
};
use rand_09::random;

use common::*;

/// Sepolia archive RPC the spec pre-verified to serve the pinned fork block. Overridable via
/// `RPC_URL_SEPOLIA` (CI secret) so the literal isn't the only path.
const DEFAULT_SEPOLIA_RPC: &str =
    "https://eth-sepolia.g.alchemy.com/v2/xqR9JXkWao0ETLYaaZt9fye8yeE4Cxyd";
/// Pinned fork block (pre-verified). A fixed block keeps Subsquid + the asserts deterministic.
const FORK_BLOCK: u64 = 10_822_990;
/// Sepolia chain id — the fork preserves it; the daemon's chain_id + the Intent must match it.
const SEPOLIA_CHAIN_ID: u64 = 11_155_111;
/// anvil dev key #0 — the EOA the daemon's account-0 maps to (sealed from [`MNEMONIC`]); the
/// fork prefunds it with ETH. Used here only to read its public balance for the down-assert.
const EOA_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// On-chain shield fee in basis points (== `ChainConfig::sepolia().unshield_fee_bps`); the
/// contract deducts it, so the synced private note reads `value - value*25/10000`.
const SHIELD_FEE_BPS: u128 = 25;

fn sepolia_rpc() -> String {
    std::env::var("RPC_URL_SEPOLIA").unwrap_or_else(|_| DEFAULT_SEPOLIA_RPC.to_string())
}

#[tokio::test]
#[ignore = "network: spawns a fresh anvil Sepolia fork + drives the full shield path"]
async fn shield_e2e_privacy_property() {
    if !anvil_available() {
        eprintln!("SKIP shield_e2e_privacy_property: anvil not on PATH");
        return;
    }

    // --- fresh anvil fork of Sepolia @ the pinned block (chain id preserved) ---
    let anvil = start_anvil_fork(&sepolia_rpc(), FORK_BLOCK, SEPOLIA_CHAIN_ID);
    wait_anvil_ready(&anvil.url()).await;
    assert_eq!(anvil.chain_id(), SEPOLIA_CHAIN_ID);

    // --- daemon, sealed for anvil account-0 (the funded EOA), pointed at the fork ---
    let dir = TempDir::new("shield-e2e");
    let (wallet, _recipient_eoa) = seal_account0(dir.path());
    // verified-reads is irrelevant here (we never call Balance on the daemon), but the daemon
    // would try to bootstrap Helios against the fork URL on a Balance request; we avoid that
    // path entirely by reading public balance directly from the fork below.
    let d = spawn_daemon(dir.path(), &anvil.url(), SEPOLIA_CHAIN_ID, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    let unlocked = client.unlock(PASS).await.unwrap();
    assert_eq!(
        unlocked,
        deckard_contract::UnlockOutcome::Unlocked { address: wallet },
        "daemon's account-0 must be the funded EOA"
    );

    // --- test-side railgun provider: register an EPHEMERAL 0zk recipient, then sync+assert ---
    let chain = ChainConfig::sepolia();
    let weth = AssetId::Erc20(chain.wrapped_base_token);

    // A plain alloy erased provider over the fork (for railgun's RPC syncer). The EOA wallet
    // here is only used by the syncer's read path; the daemon owns the BROADCAST wallet.
    let read_signer = PrivateKeySigner::from_str_eoa(EOA_KEY);
    let provider = ProviderBuilder::new()
        .network::<Ethereum>()
        .wallet(read_signer)
        .connect(&anvil.url())
        .await
        .expect("connect provider")
        .erased();

    let syncer = Arc::new(
        ChainedSyncer::new()
            .then(SubsquidSyncer::new(&chain.subsquid_endpoint).with_latest_block(FORK_BLOCK))
            .then(RpcSyncer::new(chain.clone(), provider.clone()).with_batch_size(1000)),
    );
    let mut railgun = RailgunBuilder::new(chain.clone(), provider.clone())
        .with_utxo_syncer(syncer)
        .build()
        .await
        .expect("build railgun");

    // Ephemeral 0zk recipient (random spending/viewing keys). KEY-LESS shield: the builder
    // takes only this account's RailgunAddress — never its keys.
    let recipient_acct = RailgunKeySigner::new_evm(random(), random(), chain.id);
    railgun
        .register(recipient_acct.clone())
        .await
        .expect("register recipient");
    let recipient_0zk = recipient_acct.address();

    // Sanity: recipient has no private balance before the shield.
    railgun.sync().await.expect("pre-sync");
    let before_private = railgun.balance(recipient_0zk).await;
    assert_eq!(
        before_private.get(&weth),
        None,
        "recipient must have NO 0zk note before the shield"
    );

    // ============================ DECKARD'S OWN PATH ============================
    // 1. deckard-core KEY-LESS builder → Intent{kind:Shield, ...}. The shield value (raw wei)
    //    is well within the default 0.05 ETH per-tx cap, so propose → Allow directly.
    let shield_value: u128 = 1_000_000;
    let intent = build_shield_native_intent(
        SEPOLIA_CHAIN_ID,
        recipient_0zk,
        U256::from(shield_value),
    )
    .expect("build shield intent");
    assert_eq!(
        intent.kind,
        deckard_contract::IntentKind::Shield,
        "builder must produce a Shield intent"
    );
    assert_eq!(intent.to, chain.relay_adapt_contract);
    assert!(!intent.calldata.is_empty());

    // 2. propose → the daemon admits the Shield (within cap → Allow).
    let decision = client.propose(&intent).await.expect("propose");
    assert_eq!(
        decision,
        Decision::Allow,
        "within-cap native shield must be admitted by the daemon"
    );
    let id = SignerClient::request_id_for_intent(&intent);

    // Public balance of the EOA before broadcast (for the down-assert).
    let eoa = wallet;
    let public_before = balance(&anvil.url(), eoa).await;

    // 3. execute → the daemon signs + broadcasts the Intent's calldata + value.
    let tx_hash = match client.execute(id).await.expect("execute") {
        ExecuteResult::Broadcast { tx_hash } => tx_hash,
        other => panic!("expected Broadcast, got {other:?}"),
    };
    let receipt = wait_receipt(&anvil.url(), tx_hash)
        .await
        .expect("a mined shield receipt");
    assert!(receipt.status(), "the shield tx must succeed on-chain");
    // ============================================================================

    // --- ASSERT THE PRIVACY PROPERTY ---
    railgun.sync().await.expect("post-shield sync");

    // Private balance UP by exactly value - 25bps fee (the recipient 0zk note now exists).
    let expected_net = shield_value - shield_value * SHIELD_FEE_BPS / 10_000;
    let after_private = railgun.balance(recipient_0zk).await;
    println!(
        "shield_e2e: recipient 0zk[weth] = {:?} (expect Some({expected_net}))",
        after_private.get(&weth)
    );
    assert_eq!(
        after_private.get(&weth),
        Some(&expected_net),
        "recipient private balance must be value minus the 25-bps on-chain shield fee \
         ({shield_value} -> {expected_net})"
    );

    // Public balance DOWN by ~value + gas (strictly more than the gross value).
    let public_after = balance(&anvil.url(), eoa).await;
    let public_spent = public_before - public_after;
    println!(
        "shield_e2e: EOA public spent = {public_spent} wei (>= gross value {shield_value} + gas)"
    );
    assert!(
        public_spent >= U256::from(shield_value),
        "EOA public balance must drop by at least the gross shield value (+ gas)"
    );

    // Sanity that the drop is value + gas (not wildly more): bound it loosely (< value + 0.01 ETH gas).
    assert!(
        public_spent < U256::from(shield_value) + U256::from(10_000_000_000_000_000u128),
        "EOA spend should be value + reasonable gas, got {public_spent}"
    );

    println!("=== shield_e2e PASSED: private +{expected_net}, public -{public_spent} ===");
}

/// Small helper so the alloy `PrivateKeySigner::from_str` import doesn't clash names with
/// railgun's signer in scope.
trait FromStrEoa {
    fn from_str_eoa(s: &str) -> Self;
}
impl FromStrEoa for PrivateKeySigner {
    fn from_str_eoa(s: &str) -> Self {
        use std::str::FromStr;
        PrivateKeySigner::from_str(s).expect("parse EOA key")
    }
}
