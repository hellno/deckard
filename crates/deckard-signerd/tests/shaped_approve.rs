//! #24 acceptance #7 — the shaped-approve admission gate. The daemon relaxes its v1
//! `ContractCall` deny ONLY for an exact `approve(spender, amount)` to a stored order's sell
//! token, where the spender is the GPv2 vault relayer and the amount is the order's EXACT sell
//! amount. Everything else stays denied. These vectors drive the real binary over the socket
//! (no chain — `propose` never broadcasts).
//!
//! It also pins the security property codex flagged: an admitted approve is `NeedsApproval`
//! (raises a human card), NEVER an auto-allow — even off mainnet, where a value-0 ContractCall
//! would otherwise sail through the Send caps path hands-free.

mod common;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{Decision, Intent, IntentKind, SwapOrder};
use deckard_signerd::SignerClient;

use common::*;

const SEPOLIA: u64 = 11_155_111;
const DUMMY_RPC: &str = "http://127.0.0.1:1"; // propose never broadcasts
const SELL_AMOUNT: u64 = 1_000_000;

/// A sell token the proposed order uses; the admissible approve must target exactly this.
fn sell_token() -> Address {
    Address::repeat_byte(0x77)
}

/// Build `approve(address spender, uint256 amount)` calldata (selector + two 32-byte words).
fn approve_calldata(spender: Address, amount: U256) -> Bytes {
    let mut data = vec![0x09, 0x5e, 0xa7, 0xb3];
    let mut spender_word = [0u8; 32];
    spender_word[12..].copy_from_slice(spender.as_slice());
    data.extend_from_slice(&spender_word);
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(data)
}

/// A `ContractCall` intent carrying `calldata` to `to` (token None, value 0) — the wire shape
/// a key-less client uses for a vault-relayer approval.
fn contract_call(to: Address, calldata: Bytes) -> Intent {
    Intent {
        chain_id: SEPOLIA,
        to,
        token: None,
        value: U256::ZERO,
        calldata,
        kind: IntentKind::ContractCall,
    }
}

/// A well-formed order whose sell token + amount the approve cases match against.
fn order(wallet: Address) -> SwapOrder {
    SwapOrder {
        chain_id: SEPOLIA,
        owner: wallet,
        sell_token: sell_token(),
        buy_token: Address::repeat_byte(0x88),
        sell_amount: U256::from(SELL_AMOUNT),
        buy_amount_min: U256::from(900_000u64),
        receiver: wallet,
        valid_to: 0, // placeholder; the caller sets it inside the 24h horizon from `now`
        app_data: deckard_core::APP_DATA_HASH,
    }
}

#[tokio::test]
async fn shaped_approve_admission_matrix() {
    let dir = TempDir::new("shaped-approve");
    let (wallet, _other) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // Store a matching order first (valid_to inside the 24h horizon from now).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut ord = order(wallet);
    ord.valid_to = (now + 3_600) as u32;
    assert!(
        matches!(
            client.propose_order(&ord).await.unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "a well-formed order should be NeedsApproval"
    );

    let relayer = deckard_core::GPV2_VAULT_RELAYER;

    // (a) ADMITTED: exact approve to the relayer for the order's sell token + exact amount.
    //     The security property: it is NeedsApproval (a card), NOT an auto-allow — even off
    //     mainnet. (Before the fix this auto-allowed on Sepolia.)
    assert!(
        matches!(
            client
                .propose(&contract_call(
                    sell_token(),
                    approve_calldata(relayer, U256::from(SELL_AMOUNT))
                ))
                .await
                .unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "an exact shaped approve must be admitted AS A CARD, never auto-allowed"
    );

    // (b) wrong spender (not the vault relayer) → Deny.
    assert_eq!(
        client
            .propose(&contract_call(
                sell_token(),
                approve_calldata(Address::repeat_byte(0x99), U256::from(SELL_AMOUNT))
            ))
            .await
            .unwrap(),
        Decision::Deny {
            reason: "approve_wrong_spender".into()
        }
    );

    // (c) no matching stored order (approve targets a DIFFERENT token) → Deny.
    assert_eq!(
        client
            .propose(&contract_call(
                Address::repeat_byte(0x66), // no order has this sell token
                approve_calldata(relayer, U256::from(SELL_AMOUNT))
            ))
            .await
            .unwrap(),
        Decision::Deny {
            reason: "approve_no_matching_order".into()
        }
    );

    // (d) UNLIMITED approval (amount = U256::MAX) for the right token → Deny: the amount must be
    //     the order's EXACT sell amount, so an infinite allowance can never be admitted.
    assert_eq!(
        client
            .propose(&contract_call(
                sell_token(),
                approve_calldata(relayer, U256::MAX)
            ))
            .await
            .unwrap(),
        Decision::Deny {
            reason: "approve_no_matching_order".into()
        }
    );

    // (e) a generic (non-approve) ContractCall is still denied outright.
    assert_eq!(
        client
            .propose(&contract_call(
                sell_token(),
                Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef])
            ))
            .await
            .unwrap(),
        Decision::Deny {
            reason: "unsupported_v1".into()
        }
    );

    // (f) an otherwise-exact approve that ALSO carries ETH `value` → Deny: a real ERC-20 approve
    //     never sends ETH, and the value would ride the card invisibly. (Exact spender + amount,
    //     so only the value gate can reject it.)
    let mut approve_with_value = contract_call(
        sell_token(),
        approve_calldata(relayer, U256::from(SELL_AMOUNT)),
    );
    approve_with_value.value = U256::from(1u64);
    assert_eq!(
        client.propose(&approve_with_value).await.unwrap(),
        Decision::Deny {
            reason: "approve_with_value".into()
        }
    );
}

/// A shaped approve is admitted ONLY by a LIVE (still `Pending`) order. Once the matching order
/// is resolved (no longer `Pending`), the identical approve must be refused — a stale / resolved /
/// expired order can't admit a fresh approve card; a new swap brings its own pending order.
#[tokio::test]
async fn shaped_approve_requires_a_live_pending_order() {
    let dir = TempDir::new("shaped-approve-live");
    let (wallet, _other) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, SEPOLIA, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut ord = order(wallet);
    ord.valid_to = (now + 3_600) as u32;
    let order_id = match client.propose_order(&ord).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval for a well-formed order, got {other:?}"),
    };

    let relayer = deckard_core::GPV2_VAULT_RELAYER;
    let approve = contract_call(
        sell_token(),
        approve_calldata(relayer, U256::from(SELL_AMOUNT)),
    );

    // While the order is Pending, the exact approve is admitted (a card).
    assert!(
        matches!(
            client.propose(&approve).await.unwrap(),
            Decision::NeedsApproval { .. }
        ),
        "a Pending order must admit its exact shaped approve"
    );

    // Resolve the order → it is no longer Pending. The SAME approve must now be refused.
    d.resolve(order_id, true);
    assert_eq!(
        client.propose(&approve).await.unwrap(),
        Decision::Deny {
            reason: "approve_no_matching_order".into()
        },
        "a non-Pending (resolved/stale) order must NOT admit a shaped approve"
    );
}
