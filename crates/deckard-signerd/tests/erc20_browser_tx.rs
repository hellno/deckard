mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{Decision, Intent, IntentKind, ProposalOrigin};
use deckard_signerd::SignerClient;

use common::*;

const SEPOLIA: u64 = 11_155_111;
const DUMMY_RPC: &str = "http://127.0.0.1:1";

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
