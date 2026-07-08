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

pub mod capabilities;
pub mod clear_signing;
pub mod decision;
pub mod deny_reasons;
pub mod intent;
pub mod message_signing;
pub mod mock;
pub mod policy;
pub mod read_status;
pub mod rpc;
pub mod shield_status;
pub mod signer;
pub mod swap_order;

pub use clear_signing::{
    clear_signing_fallback, normalize_contract_call_descriptor, ClearSigningError,
    ClearSigningFallback, ClearSigningField, ClearSigningFieldFormat, ClearSigningReview,
    Erc7730Descriptor,
};
pub use decision::{Decision, RequestId};
pub use intent::{Intent, IntentKind};
pub use message_signing::{
    MessageSigningRisk, PermitReview, SignMessage, SignMessageKind, TypedDataReview,
};
pub use mock::MockSigner;
pub use policy::{
    evaluate, evaluate_message, evaluate_order, Allowlist, ApprovalMode, Authority, Effect, Policy,
    PolicyError, Rule, POLICY_VERSION,
};
pub use read_status::ReadStatus;
pub use rpc::{
    ActivityLifecycle, ActivityRecord, ApprovalRisk, ApprovalStatus, BalanceReport, BreachedLimit,
    ExecuteResult, HelloInfo, PendingPayloadView, PendingRecord, ProposalOrigin, RailgunViewGrant,
    SignMessageResult, SignOrderResult, SignerRequest, SignerResponse, StatusView, UnlockOutcome,
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
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            // `revoked` and `spent_today_wei` are `#[serde(default)]` runtime state: they DO
            // round-trip on the wire (the `PolicyGet` RPC needs them), so any value survives —
            // kept at their defaults here only to keep the fixture's intent (a fresh policy).
            revoked: false,
            daily_cap_wei: U256::from(1_000_000_000_000_000_000_u64),
            auto_shield_min_wei: U256::from(10_000_000_000_000_000_u64),
            spent_today_wei: U256::ZERO,
            // Non-trivial allowlists (`Only(non-empty)` / `Any`) so the custom `Allowlist`
            // serde gets real coverage. (`DenyAll` round-trips too — the serializer omits the
            // field for it — exercised separately in `deny_all_allowlist_roundtrips`.)
            rules: vec![
                Rule::Send {
                    approval: ApprovalMode::OverCap,
                    per_tx_cap_wei: Some(U256::from(50_000_000_000_000_000_u64)),
                    recipients: Allowlist::Only(vec![
                        Address::repeat_byte(0xAA),
                        Address::repeat_byte(0xBB),
                    ]),
                },
                Rule::Shield {
                    approval: ApprovalMode::Never,
                    per_tx_cap_wei: None,
                },
                Rule::Swap {
                    tokens: Allowlist::Only(vec![Address::repeat_byte(0xCC)]),
                },
            ],
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

    fn sample_message() -> SignMessage {
        SignMessage {
            chain_id: 11155111,
            origin: "https://example.test".into(),
            kind: SignMessageKind::TypedDataV4(TypedDataReview {
                domain_name: Some("Permit2".into()),
                domain_version: Some("1".into()),
                domain_chain_id: Some(11155111),
                verifying_contract: Some(Address::repeat_byte(0x22)),
                primary_type: "PermitSingle".into(),
                digest: B256::repeat_byte(0x42),
                risks: vec![MessageSigningRisk::PermitLike],
                permit: Some(Box::new(PermitReview {
                    owner: Address::repeat_byte(0x11),
                    spender: Address::repeat_byte(0x33),
                    value: U256::MAX,
                    deadline: U256::from(1_950_000_000_u64),
                })),
            }),
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
        // A second fixture exercising the `Any` (⊤) allowlist on both the `Send` recipients
        // and the `Swap` tokens — `Any` round-trips byte-stably (it serializes to "any").
        // (A bare `DenyAll` must NOT appear in a round-trip fixture: it serializes to `[]` and
        // decodes back as `Only(vec![])`, so `assert_eq` would fail. The omitted-field →
        // `DenyAll` path is covered by `recipients_omitted_field_decodes_to_deny_all` below.)
        roundtrip(&Policy {
            rules: vec![
                Rule::Send {
                    approval: ApprovalMode::Always,
                    per_tx_cap_wei: None,
                    recipients: Allowlist::Any,
                },
                Rule::Shield {
                    approval: ApprovalMode::OverCap,
                    per_tx_cap_wei: None,
                },
                Rule::Swap {
                    tokens: Allowlist::Any,
                },
            ],
            ..sample_policy()
        });
    }

    #[test]
    fn recipients_omitted_field_decodes_to_deny_all() {
        // The `#[serde(default)]` path: a `send` rule that OMITS `recipients` decodes to the
        // default-deny floor (`DenyAll`) — NOT "any". This is the trust-relevant default.
        let omitted: Policy = serde_json::from_str(
            r#"{"version":1,"default":"deny","daily_cap_wei":"0x1","auto_shield_min_wei":"0x0",
                "rules":[{"action":"send","approval":"over_cap"}]}"#,
        )
        .expect("decode policy with omitted recipients");
        assert_eq!(
            omitted.recipients_for(IntentKind::Send),
            &Allowlist::DenyAll,
            "an omitted recipients field must default to DenyAll, never any"
        );

        // `"recipients":"any"` decodes to the `Any` (⊤) lattice value.
        let any: Policy = serde_json::from_str(
            r#"{"version":1,"default":"deny","daily_cap_wei":"0x1","auto_shield_min_wei":"0x0",
                "rules":[{"action":"send","approval":"over_cap","recipients":"any"}]}"#,
        )
        .expect("decode policy with recipients: any");
        assert_eq!(any.recipients_for(IntentKind::Send), &Allowlist::Any);

        // An explicit address array decodes to `Only(..)`.
        let only: Policy = serde_json::from_str(
            r#"{"version":1,"default":"deny","daily_cap_wei":"0x1","auto_shield_min_wei":"0x0",
                "rules":[{"action":"send","approval":"over_cap",
                          "recipients":["0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa"]}]}"#,
        )
        .expect("decode policy with recipients array");
        assert_eq!(
            only.recipients_for(IntentKind::Send),
            &Allowlist::Only(vec![Address::repeat_byte(0xAA)])
        );

        // A non-"any" recipients string is a hard error (not silently treated as a tag).
        let bad: Result<Policy, _> = serde_json::from_str(
            r#"{"version":1,"default":"deny","daily_cap_wei":"0x1","auto_shield_min_wei":"0x0",
                "rules":[{"action":"send","approval":"over_cap","recipients":"all"}]}"#,
        );
        assert!(
            bad.is_err(),
            "a non-\"any\" allowlist string must be rejected"
        );
    }

    #[test]
    fn deny_all_allowlist_roundtrips() {
        // The serializer SKIPS the allowlist field for `DenyAll` (omitted ⇒ `DenyAll`), so a
        // rule carrying the deny-everyone floor round-trips byte-stably in BOTH JSON and CBOR —
        // closing the old "DenyAll re-encodes to `[]` → `Only(vec![])`" gap.
        let policy = Policy {
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            revoked: false,
            daily_cap_wei: U256::from(1_000u64),
            auto_shield_min_wei: U256::ZERO,
            spent_today_wei: U256::ZERO,
            rules: vec![
                Rule::Send {
                    approval: ApprovalMode::Always,
                    per_tx_cap_wei: None,
                    recipients: Allowlist::DenyAll,
                },
                Rule::Swap {
                    tokens: Allowlist::DenyAll,
                },
                Rule::ContractCall {
                    approval: ApprovalMode::Always,
                    targets: Allowlist::DenyAll,
                },
            ],
        };
        roundtrip(&policy);
        // And the omitted field is genuinely gone from the JSON (not emitted as `[]`).
        let json = serde_json::to_string(&policy).unwrap();
        assert!(
            !json.contains("recipients") && !json.contains("tokens") && !json.contains("targets"),
            "DenyAll must omit the allowlist field, got: {json}"
        );
    }

    #[test]
    fn runtime_fields_cross_the_policy_get_wire() {
        // `revoked`/`spent_today_wei` are `#[serde(default)]`, NOT `#[serde(skip)]`: the daemon
        // returns the WHOLE `Policy` over the `PolicyGet` RPC, so the app's spend gauge and the
        // agent's spent view must read the daemon's LIVE values, not a silent zero/false. A
        // non-default round-trip proves they stay on the wire (a `skip` would reset them on
        // decode and fail this `assert_eq`). The loader's force-reset (a file can't inject a
        // spend) is a separate path, covered in `policy_store`.
        let live = Policy {
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            revoked: true,
            daily_cap_wei: U256::from(500u64),
            auto_shield_min_wei: U256::ZERO,
            spent_today_wei: U256::from(321u64),
            rules: vec![Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            }],
        };
        roundtrip(&live);
        let back: Policy = serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
        assert_eq!(
            back.spent_today_wei,
            U256::from(321u64),
            "spend lost on the wire"
        );
        assert!(back.revoked, "revoked lost on the wire");
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
            origin: ProposalOrigin::App,
        });
        roundtrip(&SignerRequest::Propose {
            intent: sample_intent(IntentKind::Send),
            origin: ProposalOrigin::Agent,
        });
        // #198: a browser dapp's transaction, attributed to the requesting site.
        roundtrip(&SignerRequest::Propose {
            intent: sample_intent(IntentKind::Send),
            origin: ProposalOrigin::Dapp {
                origin: "https://app.example.org".into(),
            },
        });
        roundtrip(&SignerRequest::Execute {
            request_id: B256::repeat_byte(0x02),
        });
        roundtrip(&SignerRequest::Status {
            request_id: B256::repeat_byte(0x03),
        });
        roundtrip(&SignerRequest::StatusView {
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
            origin: ProposalOrigin::App,
        });
        roundtrip(&SignerRequest::ProposeOrder {
            order: sample_swap_order(),
            origin: ProposalOrigin::Agent,
        });
        roundtrip(&SignerRequest::ProposeMessage {
            message: sample_message(),
            origin: ProposalOrigin::Agent,
        });
        // #198: a dapp message proposal — the wire origin and the payload's display-only
        // `SignMessage.origin` are written from the same bridge session, so they agree.
        roundtrip(&SignerRequest::ProposeMessage {
            message: sample_message(),
            origin: ProposalOrigin::Dapp {
                origin: sample_message().origin,
            },
        });
        roundtrip(&SignerRequest::SignMessage {
            request_id: B256::repeat_byte(0x09),
        });
        roundtrip(&SignerRequest::SignOrder {
            request_id: B256::repeat_byte(0x06),
        });
        roundtrip(&SignerRequest::CancelOrder {
            request_id: B256::repeat_byte(0x07),
        });
        roundtrip(&SignerRequest::PendingList);
        roundtrip(&SignerRequest::ActivityFeed);
        // The additive capability-discovery request (#31): a unit variant, so it frames as a bare
        // CBOR/JSON tag exactly like the other unit requests above.
        roundtrip(&SignerRequest::Hello);
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
        // The additive StatusView (#31): a live Pending poll (tx not yet broadcast) and the
        // terminal unknown-request shape the daemon hands back for an id it doesn't hold.
        roundtrip(&SignerResponse::StatusView(StatusView {
            request_id: B256::repeat_byte(0x03),
            status: ApprovalStatus::Pending,
            remaining_ms: 119_000,
            tx_hash: None,
            lifecycle: ActivityLifecycle::Proposed,
        }));
        roundtrip(&SignerResponse::StatusView(StatusView {
            request_id: B256::repeat_byte(0x09),
            status: ApprovalStatus::Denied {
                reason: "unknown_request".into(),
            },
            remaining_ms: 0,
            tx_hash: None,
            lifecycle: ActivityLifecycle::Expired,
        }));
        // An executed agent request: tx hash present, terminal so remaining_ms is 0.
        roundtrip(&SignerResponse::StatusView(StatusView {
            request_id: B256::repeat_byte(0x0a),
            status: ApprovalStatus::Allowed,
            remaining_ms: 0,
            tx_hash: Some(B256::repeat_byte(0x6e)),
            lifecycle: ActivityLifecycle::Executed,
        }));
        roundtrip(&SignerResponse::Ack);
        // The additive capability-discovery reply (#31): built from the single-source registry so
        // the wire shape is exactly what every implementation returns.
        roundtrip(&SignerResponse::Hello(capabilities::hello_info(
            capabilities::IMPL_SIGNERD,
        )));
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
        roundtrip(&SignerResponse::SignMessage(SignMessageResult::Signed {
            signature: Bytes::from(vec![0xEF_u8; 65]),
        }));
        roundtrip(&SignerResponse::SignMessage(SignMessageResult::Denied {
            reason: "not_approved".into(),
        }));
        roundtrip(&SignerResponse::Pending(vec![
            PendingRecord {
                request_id: B256::repeat_byte(0x01),
                status: ApprovalStatus::Pending,
                payload: PendingPayloadView::Order(sample_swap_order()),
                remaining_ms: 119_000,
                origin: ProposalOrigin::Agent,
                // Non-default reason + non-zero timestamp exercise the two new fields end-to-end.
                reason: BreachedLimit::PerTxCap,
                timestamp_ms: 1_720_000_000_000,
            },
            PendingRecord {
                request_id: B256::repeat_byte(0x02),
                status: ApprovalStatus::Allowed,
                payload: PendingPayloadView::Tx(sample_intent(IntentKind::Send)),
                remaining_ms: 0,
                origin: ProposalOrigin::App,
                reason: BreachedLimit::None,
                timestamp_ms: 0,
            },
            PendingRecord {
                request_id: B256::repeat_byte(0x03),
                status: ApprovalStatus::Pending,
                payload: PendingPayloadView::Message(sample_message()),
                remaining_ms: 60_000,
                origin: ProposalOrigin::Agent,
                reason: BreachedLimit::DailyCap,
                timestamp_ms: 1_720_000_060_000,
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
    fn activity_wire_types_roundtrip() {
        // Lifecycle: every state, both `approved` polarities of `Decided`, plus the lapsed-window
        // `Expired` (the human-absent closed state, split out from `Decided{false}`).
        roundtrip(&ActivityLifecycle::Proposed);
        roundtrip(&ActivityLifecycle::Decided { approved: true });
        roundtrip(&ActivityLifecycle::Decided { approved: false });
        roundtrip(&ActivityLifecycle::Expired);
        roundtrip(&ActivityLifecycle::Executed);

        // The breached-limit cite: every variant, and the safe default (no cap breached).
        for reason in [
            BreachedLimit::None,
            BreachedLimit::PerTxCap,
            BreachedLimit::DailyCap,
            BreachedLimit::OffAllowlist,
        ] {
            roundtrip(&reason);
        }
        assert_eq!(BreachedLimit::default(), BreachedLimit::None);

        // An executed agent shield: tx hash present, no cap breached, hands-free auto-allowed.
        let executed = ActivityRecord {
            request_id: B256::repeat_byte(0x07),
            origin: ProposalOrigin::Agent,
            payload: PendingPayloadView::Tx(sample_intent(IntentKind::Shield)),
            timestamp_ms: 1_700_000_000_123,
            tx_hash: Some(B256::repeat_byte(0x6e)),
            lifecycle: ActivityLifecycle::Executed,
            reason: BreachedLimit::None,
            auto_allowed: true,
        };
        roundtrip(&executed);

        // A pending over-daily-cap card: no tx hash, the daily-cap cite, NOT auto-allowed (a human
        // is in the loop), u64::MAX timestamp (full wire width) over the structured Approve payload.
        roundtrip(&ActivityRecord {
            request_id: B256::repeat_byte(0x08),
            origin: ProposalOrigin::App,
            payload: PendingPayloadView::Approve {
                token: Address::repeat_byte(0xA1),
                spender: Address::repeat_byte(0xC9),
                amount: U256::MAX,
                risks: vec![ApprovalRisk::UnlimitedAllowance],
            },
            timestamp_ms: u64::MAX,
            tx_hash: None,
            lifecycle: ActivityLifecycle::Proposed,
            reason: BreachedLimit::DailyCap,
            auto_allowed: false,
        });

        // The `#[serde(default)]` path: an ActivityRecord JSON without `auto_allowed` decodes to
        // the safe `false` (= a human was involved), never a phantom hands-free auto-allow.
        let without: ActivityRecord = serde_json::from_str(
            r#"{"request_id":"0x0707070707070707070707070707070707070707070707070707070707070707","origin":"Agent","payload":{"Tx":{"chain_id":1,"to":"0x2222222222222222222222222222222222222222","token":null,"value":"0x1","calldata":"0x","kind":"Send"}},"timestamp_ms":1,"tx_hash":null,"lifecycle":"Proposed","reason":"None"}"#,
        )
        .expect("decode without auto_allowed");
        assert!(
            !without.auto_allowed,
            "missing auto_allowed defaults to false"
        );

        // The full request/response round-trip for the new feed variant.
        roundtrip(&SignerRequest::ActivityFeed);
        roundtrip(&SignerResponse::Activity(vec![executed]));
        roundtrip(&SignerResponse::Activity(Vec::new()));
    }

    #[test]
    fn proposal_origin_roundtrip_and_default() {
        roundtrip(&ProposalOrigin::App);
        roundtrip(&ProposalOrigin::Agent);
        // #198: the dapp variant carries the site's origin string verbatim — a real web origin
        // and the bridge's literal `unknown-origin` fallback both round-trip unprettified.
        roundtrip(&ProposalOrigin::Dapp {
            origin: "https://app.example.org".into(),
        });
        roundtrip(&ProposalOrigin::Dapp {
            origin: "unknown-origin".into(),
        });
        // The safe default is App: an un-tagged proposal must never masquerade as an agent
        // (or a dapp).
        assert_eq!(ProposalOrigin::default(), ProposalOrigin::App);
    }

    /// #198: a `Dapp`-origin record rides the existing pending/activity wire structs unchanged —
    /// the enum variant is the only addition (no struct grew a key).
    #[test]
    fn dapp_origin_records_roundtrip() {
        roundtrip(&PendingRecord {
            request_id: B256::repeat_byte(0x03),
            status: ApprovalStatus::Pending,
            payload: PendingPayloadView::Tx(sample_intent(IntentKind::Send)),
            remaining_ms: 60_000,
            origin: ProposalOrigin::Dapp {
                origin: "https://app.example.org".into(),
            },
            reason: BreachedLimit::OffAllowlist,
            timestamp_ms: 1_720_000_123_000,
        });
        roundtrip(&ActivityRecord {
            request_id: B256::repeat_byte(0x04),
            origin: ProposalOrigin::Dapp {
                origin: "unknown-origin".into(),
            },
            payload: PendingPayloadView::Tx(sample_intent(IntentKind::Send)),
            timestamp_ms: 1,
            tx_hash: None,
            lifecycle: ActivityLifecycle::Proposed,
            reason: BreachedLimit::PerTxCap,
            auto_allowed: false,
        });
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
        roundtrip(&SignMessageResult::Signed {
            signature: Bytes::from(vec![0xEF_u8; 65]),
        });
        roundtrip(&SignMessageResult::Denied {
            reason: "revoked".into(),
        });
        roundtrip(&sample_message());

        roundtrip(&PendingPayloadView::Tx(sample_intent(IntentKind::Send)));
        roundtrip(&PendingPayloadView::Order(sample_swap_order()));
        roundtrip(&PendingPayloadView::Message(sample_message()));
        roundtrip(&PendingPayloadView::Approve {
            token: Address::repeat_byte(0xA1),
            spender: Address::repeat_byte(0xC9),
            amount: U256::from(1_000_000_000_000_000_000_u64),
            risks: Vec::new(),
        });

        roundtrip(&PendingRecord {
            request_id: B256::repeat_byte(0x01),
            status: ApprovalStatus::Pending,
            payload: PendingPayloadView::Order(sample_swap_order()),
            remaining_ms: 60_000,
            origin: ProposalOrigin::Agent,
            reason: BreachedLimit::PerTxCap,
            timestamp_ms: 1_720_000_200_000,
        });
        roundtrip(&PendingRecord {
            request_id: B256::repeat_byte(0x02),
            status: ApprovalStatus::Allowed,
            payload: PendingPayloadView::Approve {
                token: Address::repeat_byte(0xA1),
                spender: Address::repeat_byte(0xC9),
                amount: U256::MAX,
                risks: vec![ApprovalRisk::UnlimitedAllowance],
            },
            // u64::MAX exercises the wire's full width for the new field.
            remaining_ms: u64::MAX,
            origin: ProposalOrigin::App,
            // u64::MAX exercises the timestamp field's full wire width too.
            reason: BreachedLimit::None,
            timestamp_ms: u64::MAX,
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

/// The five wire-evolution rules (#31), proven at the contract layer. These are the definitive
/// proofs the rules hold; `deckard-signerd/tests/hello.rs` adds the daemon-socket half (Hello
/// answered while `Locked`, and the daemon surviving a bad frame with nothing signed).
///
/// - **E1** — the `Hello` reply shape.
/// - **E2** — existing frames encode byte-identically after adding `Hello` (freeze holds).
/// - **E3** — an unknown enum variant is rejected LOUDLY (that rejection is the compat valve).
/// - **E4** — an unknown struct key is ignored (the additive counterpart to E3).
#[cfg(test)]
mod wire_evolution {
    use super::*;
    use serde::Serialize;

    /// E1: the `Hello` answer — `spec_version` is a real `YYYY-MM-DD`, and `capabilities` is a
    /// superset of the baseline `{core, mcp.v0.1}`. Feature detection needs nothing else; no code
    /// branches on `impl_name`. Both the daemon and every mock build this from the same registry,
    /// so proving the builder's shape here proves the shape of every implementation's reply.
    #[test]
    fn e1_hello_reply_shape() {
        let info = capabilities::hello_info(capabilities::IMPL_SIGNERD);

        // spec_version matches ^\d{4}-\d{2}-\d{2}$ (validated without a regex dependency).
        let parts: Vec<&str> = info.spec_version.split('-').collect();
        assert_eq!(parts.len(), 3, "spec_version must be YYYY-MM-DD");
        assert_eq!(parts[0].len(), 4, "year must be 4 digits");
        assert_eq!(parts[1].len(), 2, "month must be 2 digits");
        assert_eq!(parts[2].len(), 2, "day must be 2 digits");
        assert!(
            parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit())),
            "spec_version must be all digits + dashes: {:?}",
            info.spec_version
        );

        // capabilities ⊇ {core, mcp.v0.1}.
        assert!(
            info.capabilities
                .iter()
                .any(|c| c == capabilities::CAP_CORE),
            "capabilities must include `core`"
        );
        assert!(
            info.capabilities
                .iter()
                .any(|c| c == capabilities::CAP_MCP_V0_1),
            "capabilities must include `mcp.v0.1`"
        );

        assert_eq!(info.impl_name, capabilities::IMPL_SIGNERD);
    }

    /// E2: adding the `Hello` variant did not perturb the encoding of any existing frame — the
    /// freeze holds, so an old-peer replay of a pre-`Hello` frame is byte-identical.
    ///
    /// The wire enums are externally tagged, so every variant is keyed by NAME; a sibling variant
    /// appended at the end cannot shift an existing variant's bytes. The real freeze proof is the
    /// hand-verifiable golden bytes below — for the unit requests (a bare CBOR text string of the
    /// variant name) and for a data-carrying frame (`Balance`, a `{tag: {field}}` map). The
    /// `signer_request_roundtrip` / `signer_response_roundtrip` fixtures complement this by
    /// asserting each value re-encodes deterministically; they were *extended* with the new `Hello`
    /// value, and every pre-existing assertion in them is unchanged.
    #[test]
    fn e2_existing_frames_are_byte_identical() {
        // A unit variant frames as one CBOR text string: 0x60|len, then the ASCII name.
        fn cbor_of(req: &SignerRequest) -> Vec<u8> {
            let mut b = Vec::new();
            ciborium::into_writer(req, &mut b).expect("cbor encode");
            b
        }

        // Golden bytes for three pre-existing unit requests, computed by hand from the CBOR spec
        // (major type 3 = text string). If `Hello` (or anything) ever shifts these, the freeze is
        // broken and this fails.
        assert_eq!(
            cbor_of(&SignerRequest::PolicyGet),
            b"\x69PolicyGet",
            "PolicyGet frame changed — freeze broken"
        );
        assert_eq!(
            cbor_of(&SignerRequest::RevokeAll),
            b"\x69RevokeAll",
            "RevokeAll frame changed — freeze broken"
        );
        assert_eq!(
            cbor_of(&SignerRequest::ActivityFeed),
            b"\x6CActivityFeed",
            "ActivityFeed frame changed — freeze broken"
        );

        // The golden bytes are the REAL encoding (decode them back), not a tautology, and the new
        // `Hello` frame slots in as just another unit-tag text string next to them.
        let policy_get: SignerRequest =
            ciborium::from_reader(&b"\x69PolicyGet"[..]).expect("golden decodes");
        assert_eq!(policy_get, SignerRequest::PolicyGet);
        assert_eq!(cbor_of(&SignerRequest::Hello), b"\x65Hello");

        // A data-carrying frame is pinned too: `Balance { shielded: true }` is the externally-tagged
        // map `{"Balance": {"shielded": true}}` → CBOR `A1 67 "Balance" A1 68 "shielded" F5` (F5 =
        // true). Golden-pinned so a field/variant reshuffle can't silently change it, and it never
        // leaks the new variant's tag into its own bytes.
        let balance = cbor_of(&SignerRequest::Balance { shielded: true });
        assert_eq!(
            balance, b"\xA1\x67Balance\xA1\x68shielded\xF5",
            "Balance frame changed — freeze broken"
        );
        assert!(
            !balance.windows(5).any(|w| w == b"Hello"),
            "the Hello tag leaked into an unrelated frame"
        );
    }

    /// E3: the backward-compat valve. A hypothetical FUTURE wire is a superset of today's
    /// requests; an old decoder (today's [`SignerRequest`]) must reject a variant it has never
    /// heard of LOUDLY — a decode `Err`, never a silent misparse and never a panic. That loud
    /// rejection is exactly how an old daemon answers a new request kind (rules #1/#3). Because
    /// the decode fails before any value materialises, nothing downstream can act on it — nothing
    /// is signed (the daemon-socket half of that is pinned in `deckard-signerd/tests/hello.rs`).
    #[test]
    fn e3_unknown_variant_is_rejected_loudly() {
        // A future superset: one new unit kind, one new data-carrying kind.
        #[derive(Serialize)]
        enum FutureRequest {
            QuantumSend,
            Teleport { to: u64 },
        }

        // CBOR (the daemon UDS wire): both an unknown unit tag and an unknown struct tag error.
        let mut unit = Vec::new();
        ciborium::into_writer(&FutureRequest::QuantumSend, &mut unit).unwrap();
        assert!(
            ciborium::from_reader::<SignerRequest, _>(&unit[..]).is_err(),
            "an unknown unit variant must be rejected, not silently accepted"
        );

        let mut data = Vec::new();
        ciborium::into_writer(&FutureRequest::Teleport { to: 7 }, &mut data).unwrap();
        assert!(
            ciborium::from_reader::<SignerRequest, _>(&data[..]).is_err(),
            "an unknown data variant must be rejected"
        );

        // JSON (the MCP wire) must reject it too.
        let json = serde_json::to_vec(&FutureRequest::QuantumSend).unwrap();
        assert!(
            serde_json::from_slice::<SignerRequest>(&json).is_err(),
            "an unknown variant must be rejected on the JSON surface too"
        );
    }

    /// E4: the additive counterpart to E3. A future producer adds a field to a wire struct; an
    /// old decoder (today's [`HelloInfo`]) must IGNORE the unknown key and still decode — the wire
    /// structs carry no `deny_unknown_fields`, on purpose (rule #3). This is what lets a newer
    /// daemon grow a `HelloInfo` field without breaking older clients.
    #[test]
    fn e4_unknown_struct_key_is_ignored() {
        // A HelloInfo a newer daemon emits, with a field this build has never heard of.
        #[derive(Serialize)]
        struct HelloInfoPlus {
            spec_version: String,
            capabilities: Vec<String>,
            impl_name: String,
            future_field: u64,
        }
        let plus = HelloInfoPlus {
            spec_version: "2099-01-01".to_string(),
            capabilities: vec![capabilities::CAP_CORE.to_string()],
            impl_name: "deckard-future".to_string(),
            future_field: 42,
        };

        // CBOR: the extra key is skipped, the known fields decode.
        let mut cbor = Vec::new();
        ciborium::into_writer(&plus, &mut cbor).unwrap();
        let back: HelloInfo = ciborium::from_reader(&cbor[..])
            .expect("an extra CBOR key must be ignored, not rejected");
        assert_eq!(back.spec_version, "2099-01-01");
        assert_eq!(back.impl_name, "deckard-future");
        assert_eq!(back.capabilities, vec![capabilities::CAP_CORE.to_string()]);

        // JSON: same rule on the MCP surface.
        let json = serde_json::to_vec(&plus).unwrap();
        let back2: HelloInfo =
            serde_json::from_slice(&json).expect("an extra JSON key must be ignored, not rejected");
        assert_eq!(back2.impl_name, "deckard-future");
    }

    /// #198 under rule E2: adding `ProposalOrigin::Dapp` did not perturb the existing origin
    /// frames — `App`/`Agent` stay bare CBOR text strings, and the new variant's own bytes are
    /// hand-verifiable (an externally-tagged `{"Dapp":{"origin":…}}` map).
    #[test]
    fn e2_proposal_origin_frames_are_byte_identical() {
        fn cbor_of(origin: &ProposalOrigin) -> Vec<u8> {
            let mut b = Vec::new();
            ciborium::into_writer(origin, &mut b).expect("cbor encode");
            b
        }

        // The pre-#198 frames, computed by hand from the CBOR spec (major type 3 = text string):
        // if the new variant ever shifts these, the freeze is broken and this fails.
        assert_eq!(
            cbor_of(&ProposalOrigin::App),
            b"\x63App",
            "App frame changed — freeze broken"
        );
        assert_eq!(
            cbor_of(&ProposalOrigin::Agent),
            b"\x65Agent",
            "Agent frame changed — freeze broken"
        );

        // The new frame, hand-computed: map(1) { text(4) "Dapp": map(1) { text(6) "origin":
        // text(23) "https://app.example.org" } }.
        let dapp = ProposalOrigin::Dapp {
            origin: "https://app.example.org".into(),
        };
        assert_eq!(
            cbor_of(&dapp),
            b"\xA1\x64Dapp\xA1\x66origin\x77https://app.example.org",
            "Dapp frame differs from the hand-computed golden bytes"
        );

        // The golden bytes are the REAL encoding, not a tautology: they decode back, verbatim.
        let back: ProposalOrigin =
            ciborium::from_reader(&b"\xA1\x64Dapp\xA1\x66origin\x77https://app.example.org"[..])
                .expect("golden decodes");
        assert_eq!(back, dapp);
    }

    /// #198 under rule E3: the valve, pointed at the new origin variant. An OLD decoder — one
    /// built before `Dapp` existed, modelled here as the pre-#198 two-variant enum — must reject
    /// a `Dapp`-tagged frame LOUDLY on both wire surfaces: a decode `Err`, never a silent
    /// misparse into `App`/`Agent` and never a panic. Because `origin` is a required field of
    /// every pending/activity record, an old peer rejects the whole frame — nothing downstream
    /// can act on a mis-attributed record.
    #[test]
    fn e3_old_decoder_rejects_a_dapp_origin() {
        // The pre-#198 wire enum, byte-identical to what an old daemon/app/sidecar decodes.
        #[derive(Debug, serde::Deserialize)]
        enum OldProposalOrigin {
            #[allow(dead_code)] // reason: decode-only stand-in for the pre-#198 peer.
            App,
            #[allow(dead_code)] // reason: decode-only stand-in for the pre-#198 peer.
            Agent,
        }

        let dapp = ProposalOrigin::Dapp {
            origin: "https://app.example.org".into(),
        };

        // CBOR (the daemon UDS wire).
        let mut cbor = Vec::new();
        ciborium::into_writer(&dapp, &mut cbor).unwrap();
        assert!(
            ciborium::from_reader::<OldProposalOrigin, _>(&cbor[..]).is_err(),
            "an old decoder must reject the Dapp tag, not silently accept it"
        );

        // JSON (the MCP wire).
        let json = serde_json::to_vec(&dapp).unwrap();
        assert!(
            serde_json::from_slice::<OldProposalOrigin>(&json).is_err(),
            "an old decoder must reject the Dapp tag on the JSON surface too"
        );

        // And the old frames still decode on the NEW enum — compatibility is one-way additive.
        let app: ProposalOrigin = ciborium::from_reader(&b"\x63App"[..]).expect("App decodes");
        assert_eq!(app, ProposalOrigin::App);
        let agent: ProposalOrigin =
            ciborium::from_reader(&b"\x65Agent"[..]).expect("Agent decodes");
        assert_eq!(agent, ProposalOrigin::Agent);
    }
}
