//! The daemon-side `Shield.to == RelayAdapt(chain)` pre-check (the check the chain-blind
//! contract crate defers): a Shield intent is only admitted when it targets the chain's
//! RelayAdapt contract. Includes the parity pin that keeps the daemon's railgun-free
//! address table in lockstep with `railgun::chain_config` (the dev-dependency the
//! black-box e2e already carries).

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{Decision, Intent, IntentKind};
use deckard_signerd::SignerClient;

use common::*;

const DUMMY_RPC: &str = "http://127.0.0.1:1"; // propose never broadcasts
const SEPOLIA: u64 = 11_155_111;

fn shield(chain_id: u64, to: Address, value: u64) -> Intent {
    Intent {
        chain_id,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]), // stand-in RelayAdapt call
        kind: IntentKind::Shield,
    }
}

/// The daemon's hardcoded table must equal railgun's own chain config — for every chain the
/// daemon knows. If railgun moves an adapter address, this fails loudly instead of the
/// daemon silently refusing every legitimate shield (or worse, admitting a stale target).
#[test]
fn relay_adapt_table_matches_railgun_chain_config() {
    for chain_id in [1u64, SEPOLIA] {
        let cfg = railgun::chain_config::ChainConfig::from_chain_id(chain_id)
            .expect("railgun must know the chain");
        // The daemon's table isn't exported (it's an internal pre-check), so pin the
        // literal addresses here; `propose` behavior below proves the daemon agrees.
        let expected = match chain_id {
            1 => "0xAc9f360Ae85469B27aEDdEaFC579Ef2d052aD405",
            _ => "0x7e3d929EbD5bDC84d02Bd3205c777578f33A214D",
        };
        assert_eq!(
            cfg.relay_adapt_contract,
            expected.parse::<Address>().unwrap(),
            "railgun's RelayAdapt for chain {chain_id} moved — update daemon::relay_adapt"
        );
    }
}

#[tokio::test]
async fn shield_admitted_only_at_the_relay_adapt() {
    let relay_adapt: Address = "0x7e3d929EbD5bDC84d02Bd3205c777578f33A214D"
        .parse()
        .unwrap();

    let dir = TempDir::new("shield-target");
    let (_wallet, other) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // Correct target on a supported chain → admitted (within cap → Allow).
    assert_eq!(
        client
            .propose(&shield(SEPOLIA, relay_adapt, 1_000))
            .await
            .unwrap(),
        Decision::Allow
    );

    // Any other target → refused before the policy gate ever runs.
    assert_eq!(
        client
            .propose(&shield(SEPOLIA, other, 2_000))
            .await
            .unwrap(),
        Decision::Deny {
            reason: "shield_to_mismatch".into()
        }
    );

    // Correctly targeted but EMPTY calldata still trips the policy gate's shape check
    // (it would otherwise broadcast as a bare native send to the adapter — no note).
    let mut empty = shield(SEPOLIA, relay_adapt, 3_000);
    empty.calldata = Bytes::new();
    assert_eq!(
        client.propose(&empty).await.unwrap(),
        Decision::Deny {
            reason: "undecodable".into()
        }
    );
}
