mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{deny_reasons, Decision, Intent, IntentKind, ProposalOrigin};
use deckard_signerd::SignerClient;

use common::*;

const SEPOLIA: u64 = 11_155_111;
const DUMMY_RPC: &str = "http://127.0.0.1:1";
const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH (the default policy cap)

fn erc20_transfer_intent(token: Address, recipient: Address, amount: u64) -> Intent {
    Intent {
        chain_id: SEPOLIA,
        to: recipient,
        token: Some(token),
        value: U256::from(amount),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
}

fn approve_calldata(spender: Address, amount: U256) -> Bytes {
    let mut data = vec![0x09, 0x5e, 0xa7, 0xb3];
    let mut spender_word = [0u8; 32];
    spender_word[12..].copy_from_slice(spender.as_slice());
    data.extend_from_slice(&spender_word);
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(data)
}

fn approve_intent(token: Address, spender: Address, amount: u64) -> Intent {
    Intent {
        chain_id: SEPOLIA,
        to: token,
        token: None,
        value: U256::ZERO,
        calldata: approve_calldata(spender, U256::from(amount)),
        kind: IntentKind::ContractCall,
    }
}

#[tokio::test]
async fn erc20_transfer_send_is_admitted_as_human_review_transaction() {
    let dir = TempDir::new("erc20-transfer-admit");
    let (_wallet, recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let decision = client
        .propose(
            &erc20_transfer_intent(Address::repeat_byte(0xa0), recipient, 1_000_000),
            ProposalOrigin::App,
        )
        .await
        .unwrap();

    assert!(
        matches!(decision, Decision::NeedsApproval { .. }),
        "ERC-20 transfer must raise a human card, got {decision:?}"
    );
}

#[tokio::test]
async fn browser_origin_erc20_approve_is_admitted_as_human_review_transaction() {
    let dir = TempDir::new("erc20-approve-admit");
    let (_wallet, spender) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let decision = client
        .propose(
            &approve_intent(Address::repeat_byte(0xa0), spender, 1_000_000),
            ProposalOrigin::App,
        )
        .await
        .unwrap();

    assert!(
        matches!(decision, Decision::NeedsApproval { .. }),
        "ERC-20 approve must raise a human card, got {decision:?}"
    );
}

/// #198 parity: origin is attribution, never authorization — the same intent must get the SAME
/// decision under `App` and `Dapp` origins. Proven on two daemons with identical vaults +
/// policies (the same mnemonic is sealed in both dirs), so the second propose can never ride the
/// first's idempotent re-propose path. The suite covers every admission route origin can steer:
/// the auto-allow, the over-cap card, and BOTH halves of the exact-approve branch — the one place
/// `daemon.rs` routes on origin, where `Dapp` must ride the App always-raise-a-human-card path
/// (an unwidened `matches!(origin, App)` would silently shunt dapp approves into the agent
/// shaped-approve gate and deny them).
#[tokio::test]
async fn dapp_origin_decisions_match_app_byte_for_byte() {
    let dir_app = TempDir::new("dapp-parity-app");
    let dir_dapp = TempDir::new("dapp-parity-dapp");
    let (_w1, recipient) = seal_account0(dir_app.path());
    let (_w2, recipient_dapp) = seal_account0(dir_dapp.path());
    assert_eq!(
        recipient, recipient_dapp,
        "identical vaults → identical intents"
    );

    let d_app = spawn_daemon(dir_app.path(), DUMMY_RPC, SEPOLIA, &[]);
    let d_dapp = spawn_daemon(dir_dapp.path(), DUMMY_RPC, SEPOLIA, &[]);
    let app = SignerClient::new(d_app.socket_path.clone());
    let dapp = SignerClient::new(d_dapp.socket_path.clone());
    app.unlock(PASS).await.unwrap();
    dapp.unlock(PASS).await.unwrap();

    let send = |value: u64| Intent {
        chain_id: SEPOLIA,
        to: recipient,
        token: None,
        value: U256::from(value),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    };
    // The exact-approve branch's value guard: an approve carrying ETH is denied for BOTH.
    let approve_with_value = Intent {
        value: U256::from(1u64),
        ..approve_intent(Address::repeat_byte(0xa0), recipient, 1_000_000)
    };
    let cases: Vec<(&str, Intent)> = vec![
        ("within-cap send", send(PER_TX_CAP - 1)),
        ("over-cap send", send(PER_TX_CAP + 1)),
        (
            "exact ERC-20 approve",
            approve_intent(Address::repeat_byte(0xa0), recipient, 1_000_000),
        ),
        ("approve with value", approve_with_value),
    ];

    for (label, intent) in cases {
        let from_app = app.propose(&intent, ProposalOrigin::App).await.unwrap();
        let from_dapp = dapp
            .propose(
                &intent,
                ProposalOrigin::Dapp {
                    origin: "https://app.example.org".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            from_app, from_dapp,
            "{label}: App and Dapp origins must yield byte-identical decisions"
        );
    }

    // The parity above must not be vacuous (e.g. everything denied for one dumb shared reason):
    // pin the expected shape of each route on the Dapp daemon.
    assert_eq!(
        dapp.propose(
            &send(PER_TX_CAP - 1),
            ProposalOrigin::Dapp {
                origin: "https://app.example.org".into()
            }
        )
        .await
        .unwrap(),
        Decision::Allow,
        "a within-cap dapp send auto-allows off mainnet, exactly like App"
    );
    assert!(
        matches!(
            dapp.propose(
                &approve_intent(Address::repeat_byte(0xa0), recipient, 1_000_000),
                ProposalOrigin::Dapp {
                    origin: "https://app.example.org".into()
                }
            )
            .await
            .unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "a dapp ERC-20 approve raises a human card (the App branch), never the agent gate's deny"
    );
    match dapp
        .propose(
            &Intent {
                value: U256::from(1u64),
                ..approve_intent(Address::repeat_byte(0xa0), recipient, 1_000_000)
            },
            ProposalOrigin::Dapp {
                origin: "https://app.example.org".into(),
            },
        )
        .await
        .unwrap()
    {
        Decision::Deny { reason } => assert_eq!(reason, deny_reasons::APPROVE_WITH_VALUE),
        other => panic!("an approve carrying ETH must be denied, got {other:?}"),
    }
}
