//! # deckard-contract
//!
//! The **freeze-first wire** every Deckard process speaks: the `Intent` / `Decision` /
//! `Policy` types, the signer-daemon RPC enums, a sync [`Signer`] trait, and an in-memory
//! [`MockSigner`]. Published as a standalone crate so the agent surface (`deckard-mcp`),
//! the desktop app, and the test harness can build and run the acceptance scenario
//! **before** the real signer daemon (`deckard-signerd`) exists.
//!
//! Frozen contract owned by `docs/build/30-mcp-shape.md` — do not redefine these types
//! elsewhere. This crate carries **zero key material**: types + a trait + a mock. It never
//! signs and never holds a key; the key boundary is the daemon's process.
//!
//! ## Encodings
//!
//! The types are `serde`-derived so the same definitions serialize to **JSON** (the MCP
//! surface) and **CBOR** (the daemon's Unix-domain-socket framing, via `ciborium`). Both
//! encodings round-trip byte-stably; see the crate tests.
//!
//! **Wei on the JSON wire are 0x-hex strings, not bare numbers.** `alloy-primitives`
//! encodes every `U256` (e.g. [`Intent::value`], the [`Policy`] caps, [`BalanceReport`])
//! as a `"0x…"` string in JSON. A JSON producer (a JS/Python MCP client) MUST emit wei that
//! way: a bare number literal above `u64::MAX` — routine for wei (> ~18.4 ETH) — is parsed
//! as a float and rejected on decode. CBOR has no such limit.

pub mod decision;
pub mod deny_reasons;
pub mod intent;
pub mod mock;
pub mod policy;
pub mod read_status;
pub mod rpc;
pub mod shield_status;
pub mod signer;
pub mod swap_order;

pub use decision::{Decision, RequestId};
pub use intent::{Intent, IntentKind};
pub use mock::MockSigner;
pub use policy::{evaluate, evaluate_order, ApprovalMode, Policy};
pub use read_status::ReadStatus;
pub use rpc::{
    ApprovalStatus, BalanceReport, ExecuteResult, PendingPayloadView, PendingRecord,
    RailgunViewGrant, SignOrderResult, SignerRequest, SignerResponse, UnlockOutcome,
};
pub use shield_status::ShieldStatus;
pub use signer::Signer;
pub use swap_order::SwapOrder;

#[cfg(test)]
mod roundtrip_tests {
    //! Every wire type must survive both encodings unchanged: JSON (the MCP surface) and
    //! CBOR (the daemon UDS, via ciborium). Both are also asserted byte-stable (re-encoding
    //! the same value yields identical bytes) — the wire types contain no maps/sets, so
    //! encoding is deterministic.

    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U256};
    use core::fmt::Debug;
    use serde::de::DeserializeOwned;
    use serde::Serialize;

    fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
        // JSON (human-readable): encode → decode → assert_eq, and assert byte-stability.
        let json = serde_json::to_vec(value).expect("json encode");
        let from_json: T = serde_json::from_slice(&json).expect("json decode");
        assert_eq!(&from_json, value, "json round-trip changed the value");
        assert_eq!(
            json,
            serde_json::to_vec(value).unwrap(),
            "json not byte-stable"
        );

        // CBOR (binary): encode → decode → assert_eq, and assert byte-stability.
        let mut cbor = Vec::new();
        ciborium::into_writer(value, &mut cbor).expect("cbor encode");
        let from_cbor: T = ciborium::from_reader(&cbor[..]).expect("cbor decode");
        assert_eq!(&from_cbor, value, "cbor round-trip changed the value");
        let mut cbor2 = Vec::new();
        ciborium::into_writer(value, &mut cbor2).unwrap();
        assert_eq!(cbor, cbor2, "cbor not byte-stable");
    }

    fn sample_intent(kind: IntentKind) -> Intent {
        Intent {
            chain_id: 8453,
            to: Address::repeat_byte(0x22),
            token: Some(Address::repeat_byte(0x33)),
            value: U256::from(123_456_789_u64),
            calldata: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
            kind,
        }
    }

    fn sample_policy() -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(50_000_000_000_000_000_u64),
            daily_cap_wei: U256::from(1_000_000_000_000_000_000_u64),
            spent_today_wei: U256::from(7_u64),
            allow_to: vec![Address::repeat_byte(0xAA), Address::repeat_byte(0xBB)],
            auto_shield_min_wei: U256::from(10_000_000_000_000_000_u64),
            require_approval: ApprovalMode::OverCap,
            revoked: false,
            // Non-empty so the new field's serde coverage isn't all defaults.
            allow_swap_tokens: vec![Address::repeat_byte(0xCC)],
        }
    }

    fn sample_swap_order() -> SwapOrder {
        SwapOrder {
            chain_id: 11155111,
            owner: Address::repeat_byte(0x11),
            sell_token: Address::repeat_byte(0xA1),
            buy_token: Address::repeat_byte(0xB2),
            sell_amount: U256::from(1_000_000_000_000_000_000_u64),
            buy_amount_min: U256::from(950_000_000_000_000_000_u64),
            receiver: Address::repeat_byte(0x11),
            valid_to: 1_700_003_600,
            app_data: B256::repeat_byte(0xCD),
        }
    }

    #[test]
    fn intent_and_kind_roundtrip() {
        for kind in [
            IntentKind::Send,
            IntentKind::Shield,
            IntentKind::Unshield,
            IntentKind::ContractCall,
        ] {
            roundtrip(&kind);
            roundtrip(&sample_intent(kind));
        }
        // native ETH (token = None) and empty calldata
        roundtrip(&Intent {
            token: None,
            calldata: Bytes::new(),
            ..sample_intent(IntentKind::Send)
        });
    }

    #[test]
    fn decision_roundtrip() {
        roundtrip(&Decision::Allow);
        roundtrip(&Decision::Deny {
            reason: "off_allowlist".into(),
        });
        roundtrip(&Decision::NeedsApproval {
            request_id: B256::repeat_byte(0x01),
        });
    }

    #[test]
    fn policy_and_mode_roundtrip() {
        for mode in [
            ApprovalMode::Never,
            ApprovalMode::OverCap,
            ApprovalMode::Always,
        ] {
            roundtrip(&mode);
        }
        roundtrip(&sample_policy());
        // empty allowlist + revoked variant + empty swap allowlist (the #[serde(default)]
        // path: an existing policy.json with no allow_swap_tokens decodes to vec![]).
        roundtrip(&Policy {
            allow_to: vec![],
            revoked: true,
            allow_swap_tokens: vec![],
            ..sample_policy()
        });
    }

    #[test]
    fn signer_request_roundtrip() {
        roundtrip(&SignerRequest::Unlock {
            passphrase: "correct horse battery staple".into(),
        });
        roundtrip(&SignerRequest::Lock);
        roundtrip(&SignerRequest::Resolve {
            request_id: B256::repeat_byte(0x04),
            approved: true,
        });
        roundtrip(&SignerRequest::Resolve {
            request_id: B256::repeat_byte(0x05),
            approved: false,
        });
        roundtrip(&SignerRequest::Propose {
            intent: sample_intent(IntentKind::Shield),
        });
        roundtrip(&SignerRequest::Execute {
            request_id: B256::repeat_byte(0x02),
        });
        roundtrip(&SignerRequest::Status {
            request_id: B256::repeat_byte(0x03),
        });
        roundtrip(&SignerRequest::RevokeAll);
        roundtrip(&SignerRequest::PolicyGet);
        roundtrip(&SignerRequest::Address);
        roundtrip(&SignerRequest::Balance { shielded: true });
        roundtrip(&SignerRequest::Balance { shielded: false });
        roundtrip(&SignerRequest::RailgunViewGrant {
            chain_id: 1,
            index: 0,
        });
        roundtrip(&SignerRequest::ProposeOrder {
            order: sample_swap_order(),
        });
        roundtrip(&SignerRequest::SignOrder {
            request_id: B256::repeat_byte(0x06),
        });
        roundtrip(&SignerRequest::CancelOrder {
            request_id: B256::repeat_byte(0x07),
        });
        roundtrip(&SignerRequest::PendingList);
    }

    #[test]
    fn railgun_view_grant_roundtrips_and_redacts_debug() {
        let grant = RailgunViewGrant {
            address: "0zk1example".into(),
            viewing_key: "deadbeefdeadbeef".into(),
        };
        roundtrip(&SignerResponse::RailgunView(grant.clone()));
        // The viewing key is a secret: it must never appear in Debug output.
        let dbg = format!("{grant:?}");
        assert!(
            dbg.contains("<redacted>"),
            "viewing key not redacted: {dbg}"
        );
        assert!(
            !dbg.contains("deadbeef"),
            "viewing key leaked in Debug: {dbg}"
        );
    }

    #[test]
    fn signer_response_roundtrip() {
        roundtrip(&SignerResponse::Unlock(UnlockOutcome::Unlocked {
            address: Address::repeat_byte(0x11),
        }));
        roundtrip(&SignerResponse::Unlock(UnlockOutcome::BadPassphrase));
        roundtrip(&SignerResponse::Unlock(UnlockOutcome::NoVault));
        roundtrip(&SignerResponse::Decision(Decision::Allow));
        roundtrip(&SignerResponse::Execute(ExecuteResult::Broadcast {
            tx_hash: B256::repeat_byte(0xAB),
        }));
        roundtrip(&SignerResponse::Status(ApprovalStatus::Pending));
        roundtrip(&SignerResponse::Ack);
        roundtrip(&SignerResponse::Policy(sample_policy()));
        roundtrip(&SignerResponse::Address(Address::repeat_byte(0x11)));
        roundtrip(&SignerResponse::Balance(BalanceReport {
            public_wei: U256::from(1_u64),
            shielded_wei: U256::from(2_u64),
            read_status: ReadStatus::Verified,
        }));
        roundtrip(&SignerResponse::SignOrder(SignOrderResult::Signed {
            signature: Bytes::from(vec![0xCD_u8; 65]),
        }));
        roundtrip(&SignerResponse::SignOrder(SignOrderResult::Denied {
            reason: "not_approved".into(),
        }));
        roundtrip(&SignerResponse::Pending(vec![
            PendingRecord {
                request_id: B256::repeat_byte(0x01),
                status: ApprovalStatus::Pending,
                payload: PendingPayloadView::Order(sample_swap_order()),
            },
            PendingRecord {
                request_id: B256::repeat_byte(0x02),
                status: ApprovalStatus::Allowed,
                payload: PendingPayloadView::Tx(sample_intent(IntentKind::Send)),
            },
        ]));
    }

    #[test]
    fn execute_result_and_status_roundtrip() {
        roundtrip(&ExecuteResult::Broadcast {
            tx_hash: B256::repeat_byte(0xAB),
        });
        roundtrip(&ExecuteResult::Denied {
            reason: "already_executed".into(),
        });
        roundtrip(&ApprovalStatus::Pending);
        roundtrip(&ApprovalStatus::Allowed);
        roundtrip(&ApprovalStatus::Denied {
            reason: "revoked".into(),
        });
        roundtrip(&ApprovalStatus::Expired);
    }

    #[test]
    fn swap_wire_types_roundtrip() {
        // The order itself, both sign outcomes, every payload view, and a full record.
        roundtrip(&sample_swap_order());

        roundtrip(&SignOrderResult::Signed {
            signature: Bytes::from(vec![0xCD_u8; 65]),
        });
        roundtrip(&SignOrderResult::Denied {
            reason: "revoked".into(),
        });

        roundtrip(&PendingPayloadView::Tx(sample_intent(IntentKind::Send)));
        roundtrip(&PendingPayloadView::Order(sample_swap_order()));
        roundtrip(&PendingPayloadView::Approve {
            token: Address::repeat_byte(0xA1),
            spender: Address::repeat_byte(0xC9),
            amount: U256::from(1_000_000_000_000_000_000_u64),
        });

        roundtrip(&PendingRecord {
            request_id: B256::repeat_byte(0x01),
            status: ApprovalStatus::Pending,
            payload: PendingPayloadView::Order(sample_swap_order()),
        });
        roundtrip(&PendingRecord {
            request_id: B256::repeat_byte(0x02),
            status: ApprovalStatus::Allowed,
            payload: PendingPayloadView::Approve {
                token: Address::repeat_byte(0xA1),
                spender: Address::repeat_byte(0xC9),
                amount: U256::MAX,
            },
        });
    }

    #[test]
    fn balance_report_roundtrip() {
        // Exercise every ReadStatus variant (incl. the owned-String reasons) so both
        // CBOR and JSON coverage of the new field stays complete + byte-stable.
        roundtrip(&BalanceReport {
            public_wei: U256::from(0_u64),
            shielded_wei: U256::from(0_u64),
            read_status: ReadStatus::Verified,
        });
        roundtrip(&BalanceReport {
            public_wei: U256::MAX,
            shielded_wei: U256::from(42_u64),
            read_status: ReadStatus::Unsynced {
                reason: "head stale".into(),
            },
        });
        roundtrip(&BalanceReport {
            public_wei: U256::from(7_u64),
            shielded_wei: U256::from(0_u64),
            read_status: ReadStatus::Degraded {
                reason: "failover→nimbus".into(),
            },
        });
    }

    #[test]
    fn read_status_roundtrip() {
        roundtrip(&ReadStatus::Verified);
        roundtrip(&ReadStatus::Degraded {
            reason: "failover→drpc".into(),
        });
        roundtrip(&ReadStatus::Unsynced {
            reason: "verification disabled".into(),
        });
    }

    #[test]
    fn shield_status_roundtrip() {
        // Every variant of the shield lifecycle must survive both wire encodings,
        // including the owned-String failure reason and the U256 spendable amount.
        roundtrip(&ShieldStatus::Sending);
        roundtrip(&ShieldStatus::ConfirmingOnChain {
            tx_hash: B256::repeat_byte(0xCD),
            confirmed: 2,
            target: 6,
        });
        roundtrip(&ShieldStatus::SyncingPrivate {
            tx_hash: B256::repeat_byte(0xEF),
        });
        roundtrip(&ShieldStatus::PrivateSpendable {
            shielded_wei: U256::from(997_500_u64),
        });
        roundtrip(&ShieldStatus::Failed {
            reason: "reverted".into(),
        });
    }

    #[test]
    fn shield_status_glyph_and_terminality() {
        // Glyph + lifecycle predicates: in-flight states share the pending glyph and
        // are non-terminal; the two terminal states report themselves as such.
        assert_eq!(ShieldStatus::Sending.glyph(), "clock-ring");
        assert_eq!(
            ShieldStatus::PrivateSpendable {
                shielded_wei: U256::from(1_u64),
            }
            .glyph(),
            "check-filled"
        );
        assert_eq!(
            ShieldStatus::Failed { reason: "x".into() }.glyph(),
            "x-ring"
        );

        assert!(!ShieldStatus::Sending.is_terminal());
        assert!(!ShieldStatus::SyncingPrivate {
            tx_hash: B256::ZERO,
        }
        .is_terminal());

        let spendable = ShieldStatus::PrivateSpendable {
            shielded_wei: U256::from(5_u64),
        };
        assert!(spendable.is_spendable());
        assert!(spendable.is_terminal());

        let failed = ShieldStatus::Failed {
            reason: "sync_failed".into(),
        };
        assert!(!failed.is_spendable());
        assert!(failed.is_terminal());
    }
}
