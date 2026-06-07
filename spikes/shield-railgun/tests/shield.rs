//! Runnable, self-asserting integration test for the SHIELD spike.
//!
//! Requires a local anvil forking Sepolia @ 10822990 on 127.0.0.1:8545:
//!   anvil --fork-url $RPC_URL_SEPOLIA --fork-block-number 10822990
//!
//! Run:
//!   cargo test -- --ignored --nocapture                       (default)
//!   cargo test --features parallel -- --ignored --nocapture   (parallel, R1d)

use tracing_subscriber::EnvFilter;

#[tokio::test]
#[ignore = "network: needs anvil forking Sepolia @ 10822990 on 127.0.0.1:8545"]
async fn test_shield_transact_utxo() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_test_writer()
        .try_init()
        .ok();

    shield_railgun::run_shield_scenario()
        .await
        .expect("shield scenario failed");
}
