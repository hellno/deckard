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
pub mod intent;
pub mod mock;
pub mod policy;
pub mod read_status;
pub mod rpc;
pub mod signer;

pub use decision::{Decision, RequestId};
pub use intent::{Intent, IntentKind};
pub use mock::MockSigner;
pub use policy::{evaluate, ApprovalMode, Policy};
pub use read_status::ReadStatus;
pub use rpc::{
    ApprovalStatus, BalanceReport, ExecuteResult, SignerRequest, SignerResponse, UnlockOutcome,
};
pub use signer::Signer;

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
        // empty allowlist + revoked variant
        roundtrip(&Policy {
            allow_to: vec![],
            revoked: true,
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
}
