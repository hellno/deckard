//! Message-signing daemon tests for issue #46.
//!
//! These drive the real daemon over the socket but do not touch a chain: message signing is
//! off-chain, human-approved, and never broadcast.

mod common;

use alloy_primitives::{Address, Bytes, B256};
use deckard_contract::{
    ApprovalStatus, Decision, MessageSigningRisk, PendingPayloadView, ProposalOrigin, SignMessage,
    SignMessageKind, SignMessageResult, SignerRequest, SignerResponse, TypedDataReview,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
const DUMMY_RPC: &str = "http://127.0.0.1:1";

fn personal_message() -> SignMessage {
    SignMessage {
        chain_id: CHAIN,
        origin: "https://example.test".into(),
        kind: SignMessageKind::PersonalSign {
            message: Bytes::from_static(b"Sign in to Deckard"),
        },
    }
}

fn typed_message(domain_chain_id: u64) -> SignMessage {
    SignMessage {
        chain_id: CHAIN,
        origin: "https://example.test".into(),
        kind: SignMessageKind::TypedDataV4(TypedDataReview {
            domain_name: Some("Permit2".into()),
            domain_version: Some("1".into()),
            domain_chain_id: Some(domain_chain_id),
            verifying_contract: Some(Address::repeat_byte(0x22)),
            primary_type: "PermitSingle".into(),
            digest: B256::repeat_byte(0x42),
            risks: vec![MessageSigningRisk::PermitLike],
            permit: None,
        }),
    }
}

fn eth_sign_message() -> SignMessage {
    SignMessage {
        chain_id: CHAIN,
        origin: "https://example.test".into(),
        kind: SignMessageKind::EthSign {
            digest: B256::repeat_byte(0x33),
        },
    }
}

fn delegation_message() -> SignMessage {
    SignMessage {
        chain_id: CHAIN,
        origin: "https://example.test".into(),
        kind: SignMessageKind::Authorization7702 {
            delegate: Address::repeat_byte(0x44),
            nonce: 7,
        },
    }
}

fn needs_id(decision: Decision) -> deckard_contract::RequestId {
    match decision {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

#[tokio::test]
async fn personal_sign_requires_approval_then_signs_once() {
    let dir = TempDir::new("message-personal");
    let (_wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    assert_eq!(
        client
            .propose_message(&personal_message(), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "locked".into()
        }
    );

    client.unlock(PASS).await.unwrap();
    let id = needs_id(
        client
            .propose_message(&personal_message(), ProposalOrigin::App)
            .await
            .unwrap(),
    );
    assert_eq!(
        id,
        SignerClient::request_id_for_message(&personal_message())
    );

    let pending = client.pending_list().await.unwrap();
    let rec = pending
        .iter()
        .find(|r| r.request_id == id)
        .expect("message proposal appears in pending list");
    assert_eq!(rec.status, ApprovalStatus::Pending);
    assert!(matches!(rec.payload, PendingPayloadView::Message(_)));

    assert_eq!(
        client.sign_message(id).await.unwrap(),
        SignMessageResult::Denied {
            reason: "not_approved".into()
        }
    );

    d.resolve(id, true);
    match client.sign_message(id).await.unwrap() {
        SignMessageResult::Signed { signature } => assert_eq!(signature.len(), 65),
        other => panic!("expected signed message, got {other:?}"),
    }
    assert_eq!(
        client.sign_message(id).await.unwrap(),
        SignMessageResult::Denied {
            reason: "already_signed".into()
        }
    );
}

#[tokio::test]
async fn typed_data_signs_digest_after_approval() {
    let dir = TempDir::new("message-typed");
    let (_wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = needs_id(
        client
            .propose_message(&typed_message(CHAIN), ProposalOrigin::App)
            .await
            .unwrap(),
    );
    d.resolve(id, true);
    match client.sign_message(id).await.unwrap() {
        SignMessageResult::Signed { signature } => assert_eq!(signature.len(), 65),
        other => panic!("expected signed typed data, got {other:?}"),
    }
}

#[tokio::test]
async fn unsafe_message_shapes_are_refused_before_pending() {
    let dir = TempDir::new("message-refuse");
    let (_wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    assert_eq!(
        client
            .propose_message(&typed_message(1), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "chainid_mismatch".into()
        }
    );
    assert_eq!(
        client
            .propose_message(&eth_sign_message(), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "eth_sign_refused".into()
        }
    );
    assert_eq!(
        client
            .propose_message(&delegation_message(), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "delegation_refused".into()
        }
    );
    assert!(client.pending_list().await.unwrap().is_empty());
}

#[tokio::test]
async fn stop_revokes_approved_but_unsigned_message() {
    let dir = TempDir::new("message-stop");
    let (_wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let id = needs_id(
        client
            .propose_message(&personal_message(), ProposalOrigin::Agent)
            .await
            .unwrap(),
    );
    d.resolve(id, true);
    match client
        .request(&SignerRequest::RevokeAll)
        .await
        .expect("revoke all")
    {
        SignerResponse::Ack => {}
        other => panic!("expected Ack, got {other:?}"),
    }
    assert_eq!(
        client.sign_message(id).await.unwrap(),
        SignMessageResult::Denied {
            reason: "revoked".into()
        }
    );
}
