//! End-to-end daemon tests that drive the real `deckard-signerd` binary over the socket, but
//! do NOT need a chain (no transaction is broadcast). Covers acceptance #2 (socket perms),
//! #3 (unlock outcomes), the propose-decision half of #4 (chain mismatch / unsupported kind /
//! allowlist / cap classification), #5 (resolve-false + TTL deny execute), #6 (STOP zeroize +
//! re-arm), and #7 (TOCTOU). The successful broadcasts live in `anvil_e2e.rs`.

mod common;

use std::os::unix::fs::PermissionsExt;

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    ApprovalMode, ApprovalStatus, Decision, ExecuteResult, Intent, IntentKind, Policy,
    ProposalOrigin, SignerRequest, SignerResponse, UnlockOutcome,
};
use deckard_signerd::SignerClient;

use common::*;

const CHAIN: u64 = 31337;
/// We never broadcast in this file, so the RPC is never contacted — a dead address is fine.
const DUMMY_RPC: &str = "http://127.0.0.1:1";
const PER_TX_CAP: u64 = 50_000_000_000_000_000; // 0.05 ETH (the default policy cap)

fn send(to: Address, value: u64) -> Intent {
    Intent {
        chain_id: CHAIN,
        to,
        token: None,
        value: U256::from(value),
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
}

#[tokio::test]
async fn unlock_outcomes() {
    // NoVault: empty config dir.
    let dir = TempDir::new("novault");
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    assert_eq!(
        client.unlock("anything").await.unwrap(),
        UnlockOutcome::NoVault
    );
    drop(d);

    // Sealed vault: wrong → BadPassphrase; correct → Unlocked{account0}.
    let dir = TempDir::new("unlock");
    let (wallet, _recipient) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    assert_eq!(
        client.unlock("wrong-pass").await.unwrap(),
        UnlockOutcome::BadPassphrase
    );
    assert_eq!(
        client.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
}

#[tokio::test]
async fn propose_decision_matrix() {
    let dir = TempDir::new("propose");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    // Locked → Deny{locked}.
    assert_eq!(
        client
            .propose(&send(to, 1_000), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "locked".into()
        }
    );
    client.unlock(PASS).await.unwrap();

    // Within cap → Allow.
    assert_eq!(
        client
            .propose(&send(to, 1_000), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Allow
    );

    // Over per-tx cap → NeedsApproval.
    assert!(matches!(
        client
            .propose(&send(to, PER_TX_CAP + 1), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::NeedsApproval { .. }
    ));

    // chain mismatch.
    let mut wrong_chain = send(to, 1_000);
    wrong_chain.chain_id = 1;
    assert_eq!(
        client
            .propose(&wrong_chain, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "chain_mismatch".into()
        }
    );

    // Shield on a chain with NO known RelayAdapt (31337) is refused outright — the daemon
    // can't pin the target, so it never signs. The positive path (correct RelayAdapt on a
    // supported chain → Allow) lives in `shield_target.rs`.
    let mut shield = send(to, 1_000);
    shield.kind = IntentKind::Shield;
    shield.calldata = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]); // stand-in RelayAdapt call
    assert_eq!(
        client.propose(&shield, ProposalOrigin::App).await.unwrap(),
        Decision::Deny {
            reason: "shield_to_mismatch".into()
        }
    );

    // A Shield with EMPTY calldata is rejected too. On 31337 the RelayAdapt pre-check fires
    // first (no adapter known); the calldata-shape `undecodable` deny for a correctly-
    // targeted shield is asserted on a supported chain in `shield_target.rs`.
    let mut empty_shield = send(to, 1_000);
    empty_shield.kind = IntentKind::Shield; // calldata stays empty (from send())
    assert_eq!(
        client
            .propose(&empty_shield, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "shield_to_mismatch".into()
        }
    );

    // Unshield stays a fast-follow → Deny{unsupported_v1}.
    let mut unshield = send(to, 1_000);
    unshield.kind = IntentKind::Unshield;
    assert_eq!(
        client
            .propose(&unshield, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "unsupported_v1".into()
        }
    );

    // ERC-20 sends (token = Some) are admitted as reviewed browser transactions.
    let mut erc20 = send(to, 1_000);
    erc20.token = Some(Address::repeat_byte(0xEE));
    assert!(matches!(
        client.propose(&erc20, ProposalOrigin::App).await.unwrap(),
        Decision::NeedsApproval { .. }
    ));
}

#[tokio::test]
async fn locked_wrong_chain_denies_chain_mismatch_not_locked() {
    // The chain check runs before the lock gate (it needs no key), so a LOCKED daemon
    // configured for a different chain than the intent answers `chain_mismatch`, not `locked`.
    // This is what makes the MCP sidecar's connect-time chain probe conclusive even while the
    // daemon is locked (a `locked` deny then implies the chain matched).
    let dir = TempDir::new("locked-wrong-chain");
    let (_wallet, to) = seal_account0(dir.path());
    // Daemon on CHAIN (31337), never unlocked → Locked.
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());

    // Intent targets a DIFFERENT chain (1) while the daemon is still locked.
    let mut wrong_chain = send(to, 1_000);
    wrong_chain.chain_id = 1;
    assert_eq!(
        client
            .propose(&wrong_chain, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "chain_mismatch".into()
        },
        "a locked, wrong-chain daemon must deny chain_mismatch (not locked)"
    );

    // Sanity: a SAME-chain intent on the still-locked daemon does report `locked`.
    assert_eq!(
        client
            .propose(&send(to, 1_000), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "locked".into()
        }
    );
}

#[tokio::test]
async fn off_allowlist_denies() {
    let dir = TempDir::new("allowlist");
    let (_wallet, to) = seal_account0(dir.path());
    // A policy whose allowlist excludes `to`.
    let policy = Policy {
        per_tx_cap_wei: U256::from(PER_TX_CAP),
        daily_cap_wei: U256::from(200_000_000_000_000_000u64),
        spent_today_wei: U256::ZERO,
        allow_to: vec![Address::repeat_byte(0x99)],
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u64),
        require_approval: ApprovalMode::OverCap,
        revoked: false,
        allow_swap_tokens: vec![],
    };
    std::fs::write(
        dir.path().join("policy.json"),
        serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();

    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();
    assert_eq!(
        client
            .propose(&send(to, 1_000), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Deny {
            reason: "off_allowlist".into()
        }
    );
}

#[tokio::test]
async fn resolve_false_and_ttl_deny_execute() {
    let dir = TempDir::new("ttl");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // resolve(false) → execute Denied{user_denied}.
    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    d.resolve(id, false);
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "user_denied".into()
        }
    );

    // TTL expiry → status Expired, execute Denied{expired}. (Different value → different id.)
    let over2 = send(to, PER_TX_CAP + 2);
    let id2 = needs_approval_id(&client, &over2).await;
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    assert_eq!(status(&client, id2).await, ApprovalStatus::Expired);
    assert_eq!(
        client.execute(id2).await.unwrap(),
        ExecuteResult::Denied {
            reason: "expired".into()
        }
    );
}

#[tokio::test]
async fn stop_zeroizes_and_re_arms() {
    let dir = TempDir::new("stop");
    let (wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // An Allowed within-cap request, then STOP.
    let intent = send(to, 1_000);
    assert_eq!(
        client.propose(&intent, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
    let id = SignerClient::request_id_for_intent(&intent);
    ack(&client, SignerRequest::RevokeAll).await;

    // Address now reports locked (Deny-style, per the schema).
    match client.request(&SignerRequest::Address).await.unwrap() {
        SignerResponse::Decision(Decision::Deny { reason }) => assert_eq!(reason, "locked"),
        other => panic!("expected locked Deny for Address, got {other:?}"),
    }
    // execute on the pre-STOP Allow → Denied{revoked}.
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );

    // A fresh unlock re-arms (and starts a clean session).
    assert_eq!(
        client.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
    assert_eq!(
        client
            .propose(&send(to, 1_000), ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Allow
    );
}

#[tokio::test]
async fn toctou_resolve_then_revoke_then_execute_denied() {
    let dir = TempDir::new("toctou");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    d.resolve(id, true);
    assert_eq!(status(&client, id).await, ApprovalStatus::Allowed);
    // STOP after approval but before execute.
    ack(&client, SignerRequest::RevokeAll).await;
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );
}

#[tokio::test]
async fn socket_is_0600_in_0700_dir() {
    let dir = TempDir::new("perms");
    let _ = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);

    let sock_mode = std::fs::metadata(&d.socket_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(sock_mode, 0o600, "socket must be 0600");
    let parent = d.socket_path.parent().unwrap();
    let dir_mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "socket dir must be 0700");
}

#[tokio::test]
async fn allowed_request_expires() {
    // #2 regression: an APPROVED (Allowed) request goes stale after the TTL and can't execute.
    let dir = TempDir::new("allowed-ttl");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(
        dir.path(),
        DUMMY_RPC,
        CHAIN,
        &[("DECKARD_APPROVAL_TTL_SECS", "1")],
    );
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    let over = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &over).await;
    d.resolve(id, true);
    assert_eq!(status(&client, id).await, ApprovalStatus::Allowed);

    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
    assert_eq!(status(&client, id).await, ApprovalStatus::Expired);
    assert_eq!(
        client.execute(id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "expired".into()
        }
    );
}

#[tokio::test]
async fn re_propose_is_idempotent() {
    // #3 regression: a re-propose never resets a live card, re-raises a Deny, or downgrades
    // an approval.
    let dir = TempDir::new("idempotent");
    let (_wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let client = SignerClient::new(d.socket_path.clone());
    client.unlock(PASS).await.unwrap();

    // (a) within-cap: re-propose returns the same Allow.
    let within = send(to, 1_000);
    assert_eq!(
        client.propose(&within, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );
    assert_eq!(
        client.propose(&within, ProposalOrigin::App).await.unwrap(),
        Decision::Allow
    );

    // (b) a user Deny is sticky — re-propose does NOT re-raise the card.
    let denied = send(to, PER_TX_CAP + 1);
    let id = needs_approval_id(&client, &denied).await;
    d.resolve(id, false);
    assert_eq!(
        client.propose(&denied, ProposalOrigin::App).await.unwrap(),
        Decision::Deny {
            reason: "user_denied".into()
        }
    );

    // (c) an approval is not downgraded back to Pending by a re-propose.
    let approved = send(to, PER_TX_CAP + 2);
    let id2 = needs_approval_id(&client, &approved).await;
    d.resolve(id2, true);
    assert_eq!(
        client
            .propose(&approved, ProposalOrigin::App)
            .await
            .unwrap(),
        Decision::Allow
    );
}

#[tokio::test]
async fn two_clients_interleave_app_and_mcp_sessions() {
    // The two-client reality on ONE daemon: the GUI app and the MCP sidecar are both
    // same-uid key-less clients of the same socket. Walks the lifecycle the launch demo
    // exercises: app unlock → MCP propose/approve flow → MCP STOP locks the app out too →
    // app re-unlock starts a CLEAN session that invalidates every in-flight MCP request id
    // (with an explanatory error, not a silent failure).
    let dir = TempDir::new("interleave");
    let (wallet, to) = seal_account0(dir.path());
    let d = spawn_daemon(dir.path(), DUMMY_RPC, CHAIN, &[]);
    let app = SignerClient::new(d.socket_path.clone());
    let mcp = SignerClient::new(d.socket_path.clone());

    // 1. The app unlocks; the MCP client immediately sees an unlocked daemon.
    assert_eq!(
        app.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
    let within = send(to, 1_000);
    assert_eq!(
        mcp.propose(&within, ProposalOrigin::Agent).await.unwrap(),
        Decision::Allow
    );
    let within_id = SignerClient::request_id_for_intent(&within);

    // 2. An over-cap MCP request raises a card; the APP resolves it over its private capability
    //    channel (it is the designated human-facing resolver — the MCP client, like any public
    //    caller, is refused `Resolve`; see resolver_auth.rs) and the MCP client observes the
    //    approval.
    let over = send(to, PER_TX_CAP + 1);
    let over_id = needs_approval_id(&mcp, &over).await;
    d.resolve(over_id, true);
    assert_eq!(status(&mcp, over_id).await, ApprovalStatus::Allowed);

    // 3. MCP STOP (revoke_all) — the panic brake cuts BOTH clients: the app's next call
    //    surfaces locked, and every pre-STOP approval is dead.
    ack(&mcp, SignerRequest::RevokeAll).await;
    match app.request(&SignerRequest::Address).await.unwrap() {
        SignerResponse::Decision(Decision::Deny { reason }) => assert_eq!(reason, "locked"),
        other => panic!("app must see locked after MCP STOP, got {other:?}"),
    }
    assert_eq!(
        mcp.execute(over_id).await.unwrap(),
        ExecuteResult::Denied {
            reason: "revoked".into()
        }
    );

    // 4. The app re-unlocks → a FRESH session: the request table is cleared, so the MCP
    //    client's stale ids come back `unknown_request` (the explanatory cue to re-run the
    //    flow), never a stale approval that silently executes.
    assert_eq!(
        app.unlock(PASS).await.unwrap(),
        UnlockOutcome::Unlocked { address: wallet }
    );
    for stale in [within_id, over_id] {
        assert_eq!(
            mcp.execute(stale).await.unwrap(),
            ExecuteResult::Denied {
                reason: "unknown_request".into()
            }
        );
        assert_eq!(
            status(&mcp, stale).await,
            ApprovalStatus::Denied {
                reason: "unknown_request".into()
            }
        );
    }
    // And the fresh session is live again for new work.
    assert_eq!(
        mcp.propose(&send(to, 2_000), ProposalOrigin::Agent)
            .await
            .unwrap(),
        Decision::Allow
    );
}

// --- small request helpers -----------------------------------------------------------------

async fn needs_approval_id(client: &SignerClient, intent: &Intent) -> deckard_contract::RequestId {
    // Origin is inbox-display only and never changes the verdict or the derived id, so this
    // shared helper tags App; the scenarios that assert on the agent origin propose directly.
    match client.propose(intent, ProposalOrigin::App).await.unwrap() {
        Decision::NeedsApproval { request_id } => request_id,
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}

async fn ack(client: &SignerClient, req: SignerRequest) {
    assert_eq!(client.request(&req).await.unwrap(), SignerResponse::Ack);
}

async fn status(client: &SignerClient, id: deckard_contract::RequestId) -> ApprovalStatus {
    match client
        .request(&SignerRequest::Status { request_id: id })
        .await
        .unwrap()
    {
        SignerResponse::Status(s) => s,
        other => panic!("expected Status, got {other:?}"),
    }
}
