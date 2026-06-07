//! Deckard SHIELD spike (beat 2) — de-risk the hero auto-shield action.
//!
//! Ported VERBATIM (numeric asserts) from kohaku
//! `crates/railgun/tests/integration/transact_utxo.rs` @ rev 618c53f, but run
//! against a LOCAL anvil fork of Sepolia (block 10822990) through a PLAIN alloy
//! provider (NOT Helios — that seam is proven separately in eip1193-railgun).
//!
//! Run it (anvil must already be forking Sepolia @ 10822990 on 127.0.0.1:8545):
//!   cargo test -- --ignored --nocapture                  (default features)
//!   cargo test --features parallel -- --ignored --nocapture   (parallel)
//!
//! Env:
//!   RPC_URL_SEPOLIA  — Sepolia archive RPC anvil forks from (only needed if you
//!                      let anvil read more history; the fork is already pinned).
//!   WITH_POI=1       — graft RailgunBuilder::with_poi() onto the construction to
//!                      exercise the PPOI path. NOTE: POI gates spends to notes the
//!                      POI provider marked `spendable`; on a local fork that
//!                      provider can't mark our fresh notes, so the transfer /
//!                      unshield numeric asserts only hold WITHOUT poi. Default
//!                      (no WITH_POI) keeps the 4 asserts honest; WITH_POI=1 proves
//!                      the .with_poi() construction edge compiles + builds.
//!
//! R1d (the key deliverable) — "is instant auto-shield honest?":
//!
//!   YES, but for a specific reason that the timers below make explicit.
//!   A Railgun SHIELD requires NO client-side ZK proof. `ShieldBuilder::build`
//!   (kohaku shield_builder.rs:54) only does symmetric note encryption
//!   (`encrypt_shield`) + `abi_encode` of the `shield`/`multicall` calldata —
//!   there is no `Groth16Prover`, no witness calc, no `prove_transact` on the
//!   shield path. The contract verifies the commitments on-chain. So shield IS
//!   genuinely instant (single-digit-ms build in this debug spike) because it
//!   skips proving, NOT because proving is fast.
//!
//!   The proving cost the user actually pays is on the SPEND (transfer /
//!   unshield), which routes `railgun.build(tx, rng)` through
//!   `RailgunProvider::build` -> `build_operation` -> `prove_transact`
//!   (groth16 witness + create + verify, plus a cold artifact download the
//!   first time a given circuit size is used). THOSE are the numbers that bound
//!   user-perceived proving latency.
//!
//!   Therefore:
//!     - The two SHIELD timers below are labelled "calldata/encrypt build
//!       (NO zk proof)" — they are NOT proving time, by design.
//!     - The TRANSFER and UNSHIELD timers are the REAL R1d proving wall-clock.
//!       The first measured spend is COLD (it includes the proving-key/matrices
//!       download + brotli decompress for that circuit size); subsequent spends
//!       of the SAME circuit size are warm (artifacts are LRU-cached by URL).
//!       transfer and unshield use different circuit sizes, so each pays its own
//!       cold download. Compare these numbers default-vs-parallel.

use std::{str::FromStr, sync::Arc};

use alloy::{
    network::Ethereum,
    primitives::{address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use railgun::{
    account::signer::RailgunSigner,
    builder::RailgunBuilder,
    caip::AssetId,
    chain_config::ChainConfig,
    indexer::syncer::{ChainedSyncer, RpcSyncer, SubsquidSyncer},
    transact::TransactionBuilder,
};
use rand::random;
use tracing::info;

sol! {
    #[sol(rpc)]
    // WETH interface
    contract WETH {
        function approve(address guy, uint256 wad) external returns (bool);
        function balanceOf(address input) external view returns (uint256);
        function deposit() external payable;
    }
}

const ANVIL_RPC: &str = "http://127.0.0.1:8545";
// anvil dev key #0 — already funded with ETH on the fork.
const EOA_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const FORK_BLOCK: u64 = 10_822_990;

/// Full shield -> shield_native -> transfer -> unshield scenario with the EXACT
/// upstream numeric asserts. Panics (test fails / process exits non-zero) on any
/// failed assert. Returns nothing — the asserts ARE the proof.
pub async fn run_shield_scenario() -> eyre::Result<()> {
    let with_poi = std::env::var("WITH_POI").map(|v| v == "1").unwrap_or(false);
    let parallel = cfg!(feature = "parallel");
    info!(with_poi, parallel, "shield-railgun spike start");
    println!(
        "=== shield-railgun spike (parallel_feature={parallel}, with_poi={with_poi}) ==="
    );

    let chain = ChainConfig::sepolia();
    let weth = AssetId::Erc20(chain.wrapped_base_token);

    // --- plain alloy provider over the local anvil fork ---
    let signer = PrivateKeySigner::from_str(EOA_KEY).unwrap();
    let provider = ProviderBuilder::new()
        .network::<Ethereum>()
        .wallet(signer)
        .connect(ANVIL_RPC)
        .await?
        .erased();

    let weth_contract = WETH::new(chain.wrapped_base_token, provider.clone());

    // --- Railgun construction: chained Subsquid(capped at fork) + RPC syncer ---
    let syncer = Arc::new(
        ChainedSyncer::new()
            .then(SubsquidSyncer::new(&chain.subsquid_endpoint).with_latest_block(FORK_BLOCK))
            .then(RpcSyncer::new(chain.clone(), provider.clone()).with_batch_size(1000)),
    );

    let mut builder = RailgunBuilder::new(chain.clone(), provider.clone()).with_utxo_syncer(syncer);
    if with_poi {
        // Design ask: exercise the PPOI construction path. with_poi() takes no args.
        builder = builder.with_poi();
    }
    let mut railgun = builder.build().await.map_err(|e| eyre::eyre!("build: {e}"))?;

    info!("railgun constructed (with_poi={with_poi})");

    // --- 2 railgun (0zk) accounts: random spending/viewing keys ---
    let account_1 =
        railgun::account::signer::PrivateKeySigner::new_evm(random(), random(), chain.id);
    let account_2 =
        railgun::account::signer::PrivateKeySigner::new_evm(random(), random(), chain.id);
    railgun.register(account_1.clone()).await.unwrap();
    railgun.register(account_2.clone()).await.unwrap();

    // --- fund + approve WETH (raw wei units, not 1e18-scaled) ---
    weth_contract
        .deposit()
        .value(U256::from(2_000_000))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    weth_contract
        .approve(chain.railgun_smart_wallet, U256::from(2_000_000))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // ===================== 1. SHIELD 1_000_000 =====================
    info!("step 1: shield 1_000_000");
    // NOTE: ShieldBuilder::build does NO zk proof — only encrypt_shield + abi_encode.
    // Shields are unproven on the client (contract-verified). This timer is
    // note-encryption + ABI-encode wall-clock, NOT proving time. See R1d header.
    let t0 = std::time::Instant::now();
    let shield_tx = railgun
        .shield()
        .shield(account_1.address(), weth, 1_000_000)
        .build(&mut rand::rng())
        .unwrap();
    let shield_build_ms = t0.elapsed().as_millis();
    println!(
        "R1d  SHIELD calldata/encrypt build (NO zk proof — shields are unproven): {shield_build_ms} ms  (parallel={parallel})"
    );

    for tx in shield_tx {
        provider
            .send_transaction(tx.into())
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
    }
    railgun.sync().await.unwrap();

    let balance_1 = railgun.balance(account_1.address()).await;
    let balance_2 = railgun.balance(account_2.address()).await;
    println!(
        "  after shield:        acct1[weth]={:?}  acct2[weth]={:?}  (expect Some(997_500), None)",
        balance_1.get(&weth),
        balance_2.get(&weth)
    );
    assert_eq!(balance_1.get(&weth), Some(&997_500), "shield acct1 balance");
    assert_eq!(balance_2.get(&weth), None, "shield acct2 balance");

    // ===================== 2. SHIELD NATIVE 100_000 =====================
    info!("step 2: shield_native 100_000");
    // NOTE: shield_native also does NO zk proof (same ShieldBuilder::build path,
    // via RelayAdapt wrapBase + shield multicall). This is encrypt + abi_encode
    // wall-clock, NOT proving time. See R1d header.
    let t0 = std::time::Instant::now();
    let shield_tx = railgun
        .shield()
        .shield_native(account_1.address(), 100_000)
        .build(&mut rand::rng())
        .unwrap();
    println!(
        "R1d  SHIELD_NATIVE calldata/encrypt build (NO zk proof): {} ms  (parallel={parallel})",
        t0.elapsed().as_millis()
    );

    for tx in shield_tx {
        provider
            .send_transaction(tx.into())
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
    }
    railgun.sync().await.unwrap();

    let balance_1 = railgun.balance(account_1.address()).await;
    let balance_2 = railgun.balance(account_2.address()).await;
    println!(
        "  after shield_native: acct1[weth]={:?}  acct2[weth]={:?}  (expect Some(1_097_250), None)",
        balance_1.get(&weth),
        balance_2.get(&weth)
    );
    assert_eq!(
        balance_1.get(&weth),
        Some(&1_097_250),
        "shield_native acct1 balance"
    );
    assert_eq!(balance_2.get(&weth), None, "shield_native acct2 balance");

    // ===================== 3. TRANSFER 5_000 (acct1 -> acct2) =====================
    info!("step 3: transfer 5_000");
    let tx = TransactionBuilder::new().transfer(
        account_1.clone(),
        account_2.address(),
        weth,
        5_000,
        "test transfer",
    );
    // This IS the real R1d proving wall-clock: railgun.build -> build_operation
    // -> prove_transact -> groth16 (witness + create + verify). COLD: includes
    // the first download+brotli-decompress of this circuit size's proving key +
    // matrices (LRU-cached by URL thereafter).
    let t0 = std::time::Instant::now();
    let transfer_tx = railgun.build(tx, &mut rand::rng()).await.unwrap();
    println!(
        "R1d  TRANSFER zk proof build (COLD: download+witness+prove+verify): {} ms  (parallel={parallel})",
        t0.elapsed().as_millis()
    );

    provider
        .send_transaction(transfer_tx.tx_data.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    railgun.sync().await.unwrap();

    let balance_1 = railgun.balance(account_1.address()).await;
    let balance_2 = railgun.balance(account_2.address()).await;
    println!(
        "  after transfer:      acct1[weth]={:?}  acct2[weth]={:?}  (expect Some(1_092_250), Some(5_000))",
        balance_1.get(&weth),
        balance_2.get(&weth)
    );
    assert_eq!(
        balance_1.get(&weth),
        Some(&1_092_250),
        "transfer acct1 balance"
    );
    assert_eq!(balance_2.get(&weth), Some(&5_000), "transfer acct2 balance");

    // ===================== 4. UNSHIELD 1_000 (acct1 -> EOA) =====================
    info!("step 4: unshield 1_000");
    let eoa = address!("0xe03747a83E600c3ab6C2e16dd1989C9b419D3a86");
    let tx = TransactionBuilder::new()
        .unshield(account_1.clone(), eoa, weth, 1_000)
        .unwrap();
    // Real R1d proving wall-clock (same prove_transact path as transfer). Uses a
    // DIFFERENT circuit size than transfer (different nullifier/commitment counts
    // -> different railgun/NNxMM artifact URL), so this is its OWN cold download;
    // the transfer run did not warm it.
    let t0 = std::time::Instant::now();
    let unshield_tx = railgun.build(tx, &mut rand::rng()).await.unwrap();
    println!(
        "R1d  UNSHIELD zk proof build (COLD: download+witness+prove+verify): {} ms  (parallel={parallel})",
        t0.elapsed().as_millis()
    );

    provider
        .send_transaction(unshield_tx.tx_data.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    railgun.sync().await.unwrap();

    let balance_1 = railgun.balance(account_1.address()).await;
    let balance_2 = railgun.balance(account_2.address()).await;
    let balance_eoa = weth_contract.balanceOf(eoa).call().await.unwrap();
    println!(
        "  after unshield:      acct1[weth]={:?}  acct2[weth]={:?}  EOA.WETH={}  (expect Some(1_091_250), Some(5_000), 998)",
        balance_1.get(&weth),
        balance_2.get(&weth),
        balance_eoa
    );
    assert_eq!(
        balance_1.get(&weth),
        Some(&1_091_250),
        "unshield acct1 balance"
    );
    assert_eq!(balance_2.get(&weth), Some(&5_000), "unshield acct2 balance");
    assert_eq!(balance_eoa, U256::from(998), "unshield EOA WETH balance");

    println!("=== ALL 4 STEPS PASSED (shield/shield_native/transfer/unshield) ===");
    Ok(())
}
