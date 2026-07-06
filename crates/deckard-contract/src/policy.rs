//! The spending fence the agent is allowed to READ (so it can stay inside the fence) but
//! never write. The daemon enforces it; `MockSigner` enforces the same rules in memory.
//!
//! [`evaluate`] is **the one decision function** — both `MockSigner` and the real
//! `deckard-signerd` call it, so there is no mock⇄daemon drift in the verdict logic.
//!
//! # Policy v2 — a versioned, default-deny, per-action typed rule list (ADR 0005, issue #135)
//!
//! The flat god-struct (one global `require_approval`, an `allow_to: Vec` whose **empty =
//! any** sentinel default-*allowed* the most dangerous axis) is gone. In its place is a
//! [`Vec`] of typed [`Rule`]s, one per action, each carrying only the constraints that make
//! sense for it. Three trust-relevant properties drive the shape:
//!
//! * **Default-deny.** No [`Rule`] matches an action ⇒ `Deny{NO_RULE}`. A present-but-empty
//!   `rules: []` denies everything. There is no representable "allow by default": [`Effect`]
//!   has only `Deny`, so a wallet can never be allow-by-default by construction.
//! * **A real allowlist lattice, not a sentinel.** [`Allowlist`] is `DenyAll` (⊥) / `Any`
//!   (⊤) / `Only(set)` — so "deny everyone" and "allow everyone" are *distinct, explicit*
//!   values. The old "empty `Vec` accidentally means any" foot-gun is unrepresentable, and a
//!   future grant intersection (#33/#48) is well-defined against this lattice.
//! * **Wei stays `U256`.** Every cap comparison is native `U256` (no general policy engine
//!   can enforce a `uint256` ceiling — see ADR 0005 §3), and the daily cap is **one global
//!   wall** mirroring the single `SpendStore` (#108); per-tx caps live per action.
//!
//! Adding a wallet action is therefore a localized `Rule` variant + one match arm — never a
//! new top-level field threaded through `evaluate`, the loader, the demo policy, and every
//! test (the sprawl v1 exists to kill).

use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::decision::{Decision, RequestId};
use crate::deny_reasons;
use crate::intent::{Intent, IntentKind};
use crate::message_signing::{SignMessage, SignMessageKind};
use crate::swap_order::SwapOrder;

/// The on-the-wire schema version of [`Policy`]. A file without a `version` key is a legacy
/// v0 flat policy; the loader rejects it loudly rather than reinterpret its `allow_to: [] =
/// any` semantics (which would silently flip every recipient axis to deny-all). Bumping this
/// is an intentional, versioned breaking change to a freeze-first crate (ADR 0005 §6).
pub const POLICY_VERSION: u32 = 1;

/// Sentinel returned by [`Policy::recipients_for`] / [`Policy::swap_tokens`] when an action
/// carries no allowlist (Shield/Unshield) — `Any`, since those actions never gate on a
/// recipient set. `'static` so the accessors hand back a borrow with no heap allocation.
static ANY: Allowlist = Allowlist::Any;

/// Sentinel returned by [`Policy::recipients_for`] / [`Policy::swap_tokens`] when **no rule**
/// matches the action — `DenyAll`, the default-deny floor (an action with no rule grants no
/// authority). `'static`, like [`ANY`], so the accessors allocate nothing.
static DENY_ALL: Allowlist = Allowlist::DenyAll;

/// The agent-readable policy. All caps are in wei.
///
/// `version`, `default`, the two caps, and `rules` are the authored fields. `revoked` and
/// `spent_today_wei` are runtime state owned by the daemon (the STOP brake and the rolling
/// spend counter): they are `#[serde(default)]`, so a hand-authored `policy.json` may omit
/// them (the loader force-resets both on load — a file can never inject a spend), but they
/// **stay on the wire** for the `PolicyGet` RPC so a client (the app gauge, the agent's
/// `spent_today` view) reads the daemon's live values instead of a silent zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Schema version. The loader rejects anything but [`POLICY_VERSION`] (loud, never a
    /// silent verdict) via [`Policy::validate`].
    pub version: u32,
    /// The default effect when no rule matches. v1 can only be [`Effect::Deny`] — `"allow"`
    /// is intentionally not representable (a wallet must not be allow-by-default).
    #[serde(rename = "default")]
    pub default_effect: Effect,
    /// Set true by `revoke_all` / STOP. Re-checked at execute time (TOCTOU guard). Runtime
    /// state the loader force-resets to `false` on load (a daemon boots armed); `#[serde(default)]`
    /// so the file may omit it, but it stays on the `PolicyGet` wire so a client sees a live STOP.
    #[serde(default)]
    pub revoked: bool,
    /// The **one global** daily ceiling, applied to every value-bearing action. A single wall
    /// mirrors the single `SpendStore` counter (#108) — a per-action daily cap would let a
    /// `Send` consume an `Unshield`'s budget, so it is deliberately not v1.
    pub daily_cap_wei: U256,
    /// Demo rule: auto-shield inbound ETH ≥ this. **Advisory** — read by the agent (via
    /// `policy_get`) to decide *whether to propose a shield*; the policy gate itself does not
    /// switch on it (preserves today's semantics).
    pub auto_shield_min_wei: U256,
    /// Spent so far today; the cap check compares `spent_today_wei + value`. Runtime state the
    /// loader force-resets to `0` on load (the durable `SpendStore`/#108 counter is the source
    /// of truth, never the file); `#[serde(default)]` so the file may omit it, but it stays on
    /// the `PolicyGet` wire so the app's spend gauge reads the daemon's live counter.
    #[serde(default)]
    pub spent_today_wei: U256,
    /// The typed, per-action rule list. The loader rejects a duplicate action (see
    /// [`Policy::validate`]), so "find the rule for this action" is unambiguous. An empty list
    /// denies everything (default-deny).
    pub rules: Vec<Rule>,
}

/// One per-action rule. On the JSON wire it is the RFC 9396 "array of typed objects" shape —
/// internally tagged by `"action"` (`{ "action": "send", … }`). Each action's constraints live
/// **inside its own variant**: adding a capability is one variant + one match arm, never a new
/// top-level [`Policy`] field.
///
/// Reachability honesty (ADR 0005 §2): the v1 daemon denies anything but `Send`/`Shield`
/// before `evaluate`, and the one signed `ContractCall` (the shaped relayer-approve) skips
/// `evaluate` entirely. So `Unshield` and `ContractCall` rules are **forward-compat but not
/// yet reachable** — a permissive one grants no live authority today.
///
/// ## Why hand-rolled serde instead of `#[serde(tag = "action")]`
///
/// `Policy` must round-trip byte-stably in **both** JSON (the MCP surface) and CBOR (the
/// daemon's `PolicyGet` over a Unix socket, via `ciborium`). Serde's internally-tagged enum
/// derive buffers each variant into a self-describing `Content` to find the tag first — and
/// `ciborium` is **not** self-describing: it writes `Address`/`U256` as raw byte strings (its
/// `is_human_readable()` is `false`), but the buffered `Content` re-deserializer reports
/// `is_human_readable() == true`, so `alloy`'s `Address`/`U256` then demand a hex string and
/// fail with `"expected a 32 byte hex string"`. So the derive can't be used here. The
/// [`Serialize`]/[`Deserialize`] impls below produce the **exact same** `{ "action": … }`
/// map shape while decoding each field through the live format (no `Content` buffering), so
/// CBOR and JSON both round-trip. Unknown fields inside a rule are rejected, a missing
/// required field errors, and an unknown action value errors — the strictness the loader
/// relies on is preserved by hand.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// Plain transfer. Carries a per-tx cap (optional) and a recipient allowlist.
    Send {
        approval: ApprovalMode,
        per_tx_cap_wei: Option<U256>,
        recipients: Allowlist,
    },
    /// Railgun deposit to one's own 0zk balance — value moves to self, so no recipient set. It
    /// DOES carry an optional per-tx cap: a shield still moves value off the public balance, and
    /// the daily wall alone let a large deposit (0.15 ETH under a stated 0.1 per-move cap)
    /// auto-broadcast (#185). `evaluate` enforces this cap on the shield path exactly as it does
    /// for `Send`/`Unshield`.
    Shield {
        approval: ApprovalMode,
        per_tx_cap_wei: Option<U256>,
    },
    /// Railgun withdraw back to a public balance. Forward-compat; not yet reachable (see the
    /// type-level note above).
    Unshield {
        approval: ApprovalMode,
        per_tx_cap_wei: Option<U256>,
    },
    /// Read by [`evaluate_order`] only — carries the sell+buy token allowlist and nothing
    /// else (no `approval`/`per_tx_cap`: a well-formed swap is ALWAYS `NeedsApproval`, so those
    /// would be dead fields that lie to the policy author).
    Swap { tokens: Allowlist },
    /// Generic contract write (forward-compat for plugins); not yet reachable. Carries a
    /// target allowlist.
    ContractCall {
        approval: ApprovalMode,
        targets: Allowlist,
    },
}

impl Serialize for Rule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        // Each arm emits an `{"action": <tag>, …}` map — the same shape `#[serde(tag =
        // "action")]` would, but written field-by-field through the live serializer so
        // `Address`/`U256` use the format's native encoding (no `Content` buffering). The
        // optional `per_tx_cap_wei` is skipped when `None` (matching the old
        // `skip_serializing_if = "Option::is_none"`).
        match self {
            Rule::Send {
                approval,
                per_tx_cap_wei,
                recipients,
            } => {
                // `DenyAll` is the omitted-field default, so we SKIP the field for it (rather
                // than emit `[]`): that makes every `Allowlist` variant round-trip byte-stably
                // (`DenyAll` ⇒ omitted ⇒ `DenyAll`), not just `Any`/`Only`.
                let has_recipients = !recipients.is_deny_all();
                let len = 2 + usize::from(per_tx_cap_wei.is_some()) + usize::from(has_recipients);
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("action", "send")?;
                map.serialize_entry("approval", approval)?;
                if let Some(cap) = per_tx_cap_wei {
                    map.serialize_entry("per_tx_cap_wei", cap)?;
                }
                if has_recipients {
                    map.serialize_entry("recipients", recipients)?;
                }
                map.end()
            }
            Rule::Shield {
                approval,
                per_tx_cap_wei,
            } => {
                let len = 2 + usize::from(per_tx_cap_wei.is_some());
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("action", "shield")?;
                map.serialize_entry("approval", approval)?;
                if let Some(cap) = per_tx_cap_wei {
                    map.serialize_entry("per_tx_cap_wei", cap)?;
                }
                map.end()
            }
            Rule::Unshield {
                approval,
                per_tx_cap_wei,
            } => {
                let len = 2 + usize::from(per_tx_cap_wei.is_some());
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("action", "unshield")?;
                map.serialize_entry("approval", approval)?;
                if let Some(cap) = per_tx_cap_wei {
                    map.serialize_entry("per_tx_cap_wei", cap)?;
                }
                map.end()
            }
            Rule::Swap { tokens } => {
                let has_tokens = !tokens.is_deny_all();
                let len = 1 + usize::from(has_tokens);
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("action", "swap")?;
                if has_tokens {
                    map.serialize_entry("tokens", tokens)?;
                }
                map.end()
            }
            Rule::ContractCall { approval, targets } => {
                let has_targets = !targets.is_deny_all();
                let len = 2 + usize::from(has_targets);
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("action", "contract_call")?;
                map.serialize_entry("approval", approval)?;
                if has_targets {
                    map.serialize_entry("targets", targets)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// The field keys a rule map may carry. Unknown keys are rejected (the by-hand
        /// equivalent of `deny_unknown_fields`).
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Action,
            Approval,
            PerTxCapWei,
            Recipients,
            Tokens,
            Targets,
        }

        struct RuleVisitor;

        impl<'de> serde::de::Visitor<'de> for RuleVisitor {
            type Value = Rule;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a rule object with an \"action\" key")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Rule, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                use serde::de::Error as _;
                let mut action: Option<String> = None;
                let mut approval: Option<ApprovalMode> = None;
                let mut per_tx_cap_wei: Option<U256> = None;
                let mut recipients: Option<Allowlist> = None;
                let mut tokens: Option<Allowlist> = None;
                let mut targets: Option<Allowlist> = None;

                // Read every key through the live deserializer — no `Content` buffering, so
                // `Address`/`U256` decode natively in both JSON and CBOR.
                while let Some(field) = map.next_key::<Field>()? {
                    match field {
                        Field::Action => {
                            if action.is_some() {
                                return Err(M::Error::duplicate_field("action"));
                            }
                            action = Some(map.next_value()?);
                        }
                        Field::Approval => {
                            if approval.is_some() {
                                return Err(M::Error::duplicate_field("approval"));
                            }
                            approval = Some(map.next_value()?);
                        }
                        Field::PerTxCapWei => {
                            if per_tx_cap_wei.is_some() {
                                return Err(M::Error::duplicate_field("per_tx_cap_wei"));
                            }
                            per_tx_cap_wei = Some(map.next_value()?);
                        }
                        Field::Recipients => {
                            if recipients.is_some() {
                                return Err(M::Error::duplicate_field("recipients"));
                            }
                            recipients = Some(map.next_value()?);
                        }
                        Field::Tokens => {
                            if tokens.is_some() {
                                return Err(M::Error::duplicate_field("tokens"));
                            }
                            tokens = Some(map.next_value()?);
                        }
                        Field::Targets => {
                            if targets.is_some() {
                                return Err(M::Error::duplicate_field("targets"));
                            }
                            targets = Some(map.next_value()?);
                        }
                    }
                }

                let action = action.ok_or_else(|| M::Error::missing_field("action"))?;
                // Reject a field that belongs to a *different* action, so a `send` carrying
                // `tokens` (a no-op that would lie to the author) is an error — matching the
                // strictness of a per-variant `deny_unknown_fields`.
                let reject = |present: bool, name: &'static str| -> Result<(), M::Error> {
                    if present {
                        Err(M::Error::custom(format!(
                            "field {name:?} is not valid for action {action:?}"
                        )))
                    } else {
                        Ok(())
                    }
                };
                match action.as_str() {
                    "send" => {
                        reject(tokens.is_some(), "tokens")?;
                        reject(targets.is_some(), "targets")?;
                        Ok(Rule::Send {
                            approval: approval
                                .ok_or_else(|| M::Error::missing_field("approval"))?,
                            per_tx_cap_wei,
                            recipients: recipients.unwrap_or_default(),
                        })
                    }
                    "shield" => {
                        reject(recipients.is_some(), "recipients")?;
                        reject(tokens.is_some(), "tokens")?;
                        reject(targets.is_some(), "targets")?;
                        Ok(Rule::Shield {
                            approval: approval
                                .ok_or_else(|| M::Error::missing_field("approval"))?,
                            per_tx_cap_wei,
                        })
                    }
                    "unshield" => {
                        reject(recipients.is_some(), "recipients")?;
                        reject(tokens.is_some(), "tokens")?;
                        reject(targets.is_some(), "targets")?;
                        Ok(Rule::Unshield {
                            approval: approval
                                .ok_or_else(|| M::Error::missing_field("approval"))?,
                            per_tx_cap_wei,
                        })
                    }
                    "swap" => {
                        reject(approval.is_some(), "approval")?;
                        reject(per_tx_cap_wei.is_some(), "per_tx_cap_wei")?;
                        reject(recipients.is_some(), "recipients")?;
                        reject(targets.is_some(), "targets")?;
                        Ok(Rule::Swap {
                            tokens: tokens.unwrap_or_default(),
                        })
                    }
                    "contract_call" => {
                        reject(per_tx_cap_wei.is_some(), "per_tx_cap_wei")?;
                        reject(recipients.is_some(), "recipients")?;
                        reject(tokens.is_some(), "tokens")?;
                        Ok(Rule::ContractCall {
                            approval: approval
                                .ok_or_else(|| M::Error::missing_field("approval"))?,
                            targets: targets.unwrap_or_default(),
                        })
                    }
                    other => Err(M::Error::custom(format!("unknown rule action {other:?}"))),
                }
            }
        }

        deserializer.deserialize_map(RuleVisitor)
    }
}

/// A recipient / token / target allowlist as a real lattice — **not** a "empty `Vec` = any"
/// sentinel. `DenyAll` (⊥) denies everyone; `Any` (⊤) allows everyone; `Only(set)` allows
/// exactly the listed addresses. The distinction is what makes default-deny ([`Default`] is
/// `DenyAll`) and a future monotonic grant-narrowing (#33/#48) well-defined.
///
/// Custom serde (see [`Serialize`]/[`Deserialize`] impls below): `Any` ↔ the string `"any"`,
/// `Only(v)` ↔ a JSON array, and `DenyAll` ↔ an **omitted** field (the [`Rule`] serializer skips
/// the field for it; the decoder's `unwrap_or_default` restores it). So every variant — `DenyAll`
/// included — round-trips byte-stably in both JSON and CBOR.
#[derive(Clone, Debug, PartialEq)]
pub enum Allowlist {
    /// Deny everyone (⊥). The default — an action whose allowlist is omitted grants nothing.
    DenyAll,
    /// Allow everyone (⊤).
    Any,
    /// Allow exactly the listed addresses.
    Only(Vec<Address>),
}

impl Allowlist {
    /// `true` for the deny-everyone floor (⊥). The `Rule` serializer skips the allowlist field
    /// for this variant (omitted ⇒ `DenyAll`), so the wire byte-stably round-trips it.
    pub fn is_deny_all(&self) -> bool {
        matches!(self, Allowlist::DenyAll)
    }
}

impl Default for Allowlist {
    /// Default-deny: an omitted allowlist field denies everyone. The rule decoder relies on
    /// this (ADR 0005 §5).
    fn default() -> Self {
        Allowlist::DenyAll
    }
}

impl<'de> Deserialize<'de> for Allowlist {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // A hand-rolled visitor (NOT `#[serde(untagged)]`) because the wire carries either a
        // string (`"any"`) or a sequence of `Address`. An untagged helper buffers the input
        // into serde's self-describing `Content` and re-deserializes it — but `alloy`'s
        // `Address` `Deserialize` switches on `is_human_readable()` (hex string in JSON, raw
        // bytes in CBOR), and the buffered re-deserialization loses the CBOR format flag, so
        // the byte array fails to decode as a "20 byte hex string". `deserialize_any` lets the
        // format itself dispatch to `visit_str` / `visit_seq`, decoding `Address` natively in
        // both JSON and CBOR.
        struct AllowlistVisitor;

        impl<'de> serde::de::Visitor<'de> for AllowlistVisitor {
            type Value = Allowlist;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(r#"the string "any" or an array of addresses"#)
            }

            fn visit_str<E>(self, s: &str) -> Result<Allowlist, E>
            where
                E: serde::de::Error,
            {
                if s == "any" {
                    Ok(Allowlist::Any)
                } else {
                    Err(E::custom(format!(
                        "allowlist string must be \"any\", got {s:?}"
                    )))
                }
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Allowlist, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut v = Vec::new();
                while let Some(addr) = seq.next_element::<Address>()? {
                    v.push(addr);
                }
                Ok(Allowlist::Only(v))
            }
        }

        deserializer.deserialize_any(AllowlistVisitor)
    }
}

impl Serialize for Allowlist {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            // ⊤ renders as the tag string the deserializer round-trips.
            Allowlist::Any => serializer.serialize_str("any"),
            Allowlist::Only(v) => v.serialize(serializer),
            // ⊥ has no string form. The `Rule` serializer never reaches here — it SKIPS the
            // allowlist field for `DenyAll` (omitted ⇒ `DenyAll`), which is what makes the wire
            // round-trip byte-stable. This arm only fires if an `Allowlist` is serialized
            // standalone (off the `Rule` path); it renders the same deny-everyone `[]`.
            Allowlist::DenyAll => Vec::<Address>::new().serialize(serializer),
        }
    }
}

/// The default effect when no rule matches. v1 has exactly one variant: a wallet must not be
/// allow-by-default, so `"allow"` is intentionally not representable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    Deny,
}

/// When a rule's action raises a native approval card.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Never raise a card. Within cap → allow; over cap → deny (no card to override it).
    Never,
    /// Raise a card only when over a cap; within cap → allow.
    OverCap,
    /// Always raise a card, even within cap.
    Always,
}

/// A policy that fails [`Policy::validate`]. Surfaced by the loader, which converts it to a
/// loud most-restrictive deny-all fallback rather than ever returning a silent verdict.
#[derive(Clone, Debug, PartialEq)]
pub enum PolicyError {
    /// The `version` field is not [`POLICY_VERSION`] (e.g. a legacy v0 file or a future v2).
    UnsupportedVersion(u32),
    /// Two rules share the same action — "find the rule for this action" would be ambiguous.
    /// Carries the action's wire tag (`"send"`, `"shield"`, …).
    DuplicateAction(&'static str),
}

impl core::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PolicyError::UnsupportedVersion(v) => write!(
                f,
                "unsupported policy version {v} (this build speaks version {POLICY_VERSION})"
            ),
            PolicyError::DuplicateAction(action) => {
                write!(f, "duplicate rule for action {action:?}")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// The inputs for the shared Review's **Allowed by** authority line (DESIGN §Clear-signing) —
/// produced by [`Policy::authority_for`] so the UI renders the *same* rule + cap the engine
/// enforces, never a recomputed figure that could drift. All wei; the UI formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Authority {
    /// The label of the rule that governs the action ([`Rule::label`]) — the line's subject.
    pub rule_label: &'static str,
    /// The one global daily ceiling (the "of $Y" total).
    pub daily_cap_wei: U256,
    /// What remains of the daily ceiling AFTER this move (the "$X"): `daily_cap − (spent + value)`,
    /// saturating at zero.
    pub daily_remaining_after_wei: U256,
    /// `true` when the move trips a cap (per-tx OR daily) — re-derives [`evaluate`]'s cap test so
    /// the UI knows the "daily left after this" clause is a breach the danger line owns, not calm
    /// headroom. Pinned to `evaluate`'s verdict by a unit test so the two can't drift.
    pub over_cap: bool,
}

impl Rule {
    /// This rule's action as its wire tag — used for duplicate-detection error messages and
    /// to match a rule against an [`IntentKind`].
    fn action_tag(&self) -> &'static str {
        match self {
            Rule::Send { .. } => "send",
            Rule::Shield { .. } => "shield",
            Rule::Unshield { .. } => "unshield",
            Rule::Swap { .. } => "swap",
            Rule::ContractCall { .. } => "contract_call",
        }
    }

    /// A human label for the rule — the subject of the shared Review's **Allowed by** authority
    /// line (`Send rule`, `Shield rule`, …). One source so the UI never invents a rule name, and
    /// the label it shows names the exact rule `evaluate` matched.
    pub fn label(&self) -> &'static str {
        match self {
            Rule::Send { .. } => "Send rule",
            Rule::Shield { .. } => "Shield rule",
            Rule::Unshield { .. } => "Unshield rule",
            Rule::Swap { .. } => "Swap rule",
            Rule::ContractCall { .. } => "Contract-call rule",
        }
    }

    /// Does this rule govern `kind`? (There is no `Swap` `IntentKind`; the `Swap` rule is
    /// reached only via [`evaluate_order`]/[`Policy::swap_tokens`].)
    fn matches_kind(&self, kind: &IntentKind) -> bool {
        matches!(
            (self, kind),
            (Rule::Send { .. }, IntentKind::Send)
                | (Rule::Shield { .. }, IntentKind::Shield)
                | (Rule::Unshield { .. }, IntentKind::Unshield)
                | (Rule::ContractCall { .. }, IntentKind::ContractCall)
        )
    }
}

impl Policy {
    /// Validate the loaded policy: the `version` must be [`POLICY_VERSION`] and no two rules
    /// may share an action. The loader calls this; a failure becomes a loud deny-all fallback,
    /// never a verdict.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.version != POLICY_VERSION {
            return Err(PolicyError::UnsupportedVersion(self.version));
        }
        // Reject duplicate actions so `rule_for` is unambiguous. The rule count is tiny (one
        // per action), so the quadratic scan is irrelevant and needs no allocation.
        for (i, rule) in self.rules.iter().enumerate() {
            if self.rules[..i]
                .iter()
                .any(|earlier| earlier.action_tag() == rule.action_tag())
            {
                return Err(PolicyError::DuplicateAction(rule.action_tag()));
            }
        }
        Ok(())
    }

    /// The first rule governing `kind`, or `None` (⇒ default-deny). With duplicate actions
    /// rejected by [`Policy::validate`], "first" is the only matching rule.
    pub fn rule_for(&self, kind: IntentKind) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.matches_kind(&kind))
    }

    /// The per-tx cap carried by the rule for `kind`, if any. `Send`/`Shield`/`Unshield` rules can
    /// carry one; every other action (and a no-rule) yields `None`. Shield joined this set in #185
    /// so a large deposit can't slip past the per-move cap on the daily wall alone.
    pub fn per_tx_cap_for(&self, kind: IntentKind) -> Option<U256> {
        match self.rule_for(kind)? {
            Rule::Send { per_tx_cap_wei, .. }
            | Rule::Shield { per_tx_cap_wei, .. }
            | Rule::Unshield { per_tx_cap_wei, .. } => *per_tx_cap_wei,
            _ => None,
        }
    }

    /// The recipient/target allowlist for `kind`: the `Send` rule's `recipients`, the
    /// `ContractCall` rule's `targets`, [`ANY`] for `Shield`/`Unshield` (they never gate on a
    /// recipient set), and the [`DENY_ALL`] floor when no rule matches (default-deny).
    pub fn recipients_for(&self, kind: IntentKind) -> &Allowlist {
        match self.rule_for(kind) {
            Some(Rule::Send { recipients, .. }) => recipients,
            Some(Rule::ContractCall { targets, .. }) => targets,
            // Shield/Unshield carry no recipient allowlist — they don't gate on one.
            Some(Rule::Shield { .. }) | Some(Rule::Unshield { .. }) => &ANY,
            // A Swap rule (no Swap IntentKind reaches here) or no rule at all → deny-all floor.
            Some(Rule::Swap { .. }) | None => &DENY_ALL,
        }
    }

    /// The approval mode for `kind`, or `None` for `Swap` (a well-formed swap is always
    /// `NeedsApproval`, so it has no mode) and for a no-rule.
    pub fn approval_for(&self, kind: IntentKind) -> Option<ApprovalMode> {
        match self.rule_for(kind)? {
            Rule::Send { approval, .. }
            | Rule::Shield { approval, .. }
            | Rule::Unshield { approval, .. }
            | Rule::ContractCall { approval, .. } => Some(*approval),
            Rule::Swap { .. } => None,
        }
    }

    /// The `Swap` rule's token allowlist, or the [`DENY_ALL`] floor when there is no `Swap`
    /// rule (default-deny: no rule ⇒ no swap is allowed, surfaced as `OFF_SWAP_LIST` by
    /// [`evaluate_order`] to preserve the existing tag).
    pub fn swap_tokens(&self) -> &Allowlist {
        self.rules
            .iter()
            .find_map(|rule| match rule {
                Rule::Swap { tokens } => Some(tokens),
                _ => None,
            })
            .unwrap_or(&DENY_ALL)
    }

    /// The **Allowed by** authority-line inputs for a proposed `(kind, value)` — the rule that
    /// governs the action plus the daily budget remaining AFTER the move (see [`Authority`]). The
    /// UI renders these verbatim, so its cap figure is the *same* one [`evaluate`] enforces and can
    /// never drift. Returns `None` when no rule governs `kind` (default-deny — there is no authority
    /// to cite; the review shows the deny instead). Used only for the native-value paths
    /// (`Send`/`Shield`/`Unshield`); a swap always asks and cites its `Swap rule` via [`Rule::label`]
    /// directly, since `evaluate_order` enforces no numeric cap the daily line could truthfully claim.
    pub fn authority_for(&self, kind: IntentKind, value: U256) -> Option<Authority> {
        let rule = self.rule_for(kind.clone())?;
        let projected = self.spent_today_wei.saturating_add(value);
        // Mirror `evaluate`'s cap test (per-tx OR daily) — a unit test pins the two together.
        let over_daily = projected > self.daily_cap_wei;
        let over_pertx = self.per_tx_cap_for(kind).is_some_and(|cap| projected > cap);
        Some(Authority {
            rule_label: rule.label(),
            daily_cap_wei: self.daily_cap_wei,
            daily_remaining_after_wei: self.daily_cap_wei.saturating_sub(projected),
            over_cap: over_pertx || over_daily,
        })
    }
}

/// **The** decision function. A *pure* `(Intent, Policy) -> Decision` with no I/O, no
/// signing, no state — both [`MockSigner`](crate::MockSigner) and `deckard-signerd` call
/// it so the verdict can never drift between the mock and the real daemon.
///
/// It owns the policy-level checks (`revoked`, the matching rule, the allowlist, calldata
/// shape, the caps × mode matrix). Process-level pre-checks that the policy can't express —
/// the daemon being `Locked`, a `chain_id` mismatch, an unsupported `IntentKind` — are the
/// daemon's job and run *before* this function (the mock has none of those states, so feeding
/// both the same `(Intent, Policy)` yields identical `Decision`s; this is the parity contract).
///
/// For [`Decision::NeedsApproval`] the returned `request_id` is the **placeholder**
/// [`RequestId::ZERO`](alloy_primitives::B256::ZERO): minting a real, trackable id is the
/// stateful caller's job (it stores the pending record under that id). Callers must replace
/// it before returning the decision on the wire.
pub fn evaluate(intent: &Intent, policy: &Policy) -> Decision {
    // 1. STOP / revoked overrides everything.
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    // 2. Default-deny: no rule governs this action ⇒ deny. This is the whole point of v2 —
    //    an action a policy never mentions grants no authority.
    let kind = intent.kind.clone();
    let Some(rule) = policy.rule_for(kind.clone()) else {
        return Decision::Deny {
            reason: deny_reasons::NO_RULE.into(),
        };
    };
    // 3. Calldata must be decodable for the kind (Send empty; Shield/Unshield/ContractCall
    //    non-empty — closes the "empty Shield degrades into a bare native send" hole).
    if !calldata_ok(intent) {
        return Decision::Deny {
            reason: deny_reasons::UNDECODABLE.into(),
        };
    }
    // 4. Allowlist (the lattice — DenyAll or off-`Only` denies). `Send` gates on the rule's
    //    `recipients`, `ContractCall` on its `targets`; `Shield`/`Unshield` carry none, so
    //    `recipients_for` returns `Any` for them and this never denies.
    match policy.recipients_for(kind.clone()) {
        Allowlist::DenyAll => {
            return Decision::Deny {
                reason: deny_reasons::OFF_ALLOWLIST.into(),
            };
        }
        Allowlist::Only(v) if !v.contains(&intent.to) => {
            return Decision::Deny {
                reason: deny_reasons::OFF_ALLOWLIST.into(),
            };
        }
        _ => {}
    }
    // Cap check: spent_today + value vs the per-tx cap (rule-carried) and the global daily cap.
    let projected = policy.spent_today_wei.saturating_add(intent.value);
    let over_daily = projected > policy.daily_cap_wei;
    let over_pertx = policy
        .per_tx_cap_for(kind)
        .is_some_and(|cap| projected > cap);
    let over = over_pertx || over_daily;

    // 5. The approval mode × over-cap matrix.
    match rule_approval(rule) {
        // Never raises no card, so an over-cap write has nothing to authorise it → deny.
        ApprovalMode::Never => {
            if over {
                Decision::Deny {
                    reason: deny_reasons::OVER_CAP.into(),
                }
            } else {
                Decision::Allow
            }
        }
        ApprovalMode::OverCap => {
            if over {
                Decision::NeedsApproval {
                    request_id: RequestId::ZERO,
                }
            } else {
                Decision::Allow
            }
        }
        ApprovalMode::Always => Decision::NeedsApproval {
            request_id: RequestId::ZERO,
        },
    }
}

/// The matched rule's approval mode. `evaluate` only ever holds a rule that matched an
/// `IntentKind`, and the four intent-bearing variants all carry an `approval`; `Swap` is
/// unreachable here (no `Swap` `IntentKind`) — default it to the most cautious `Always` so a
/// mis-wiring fails closed (raises a card) rather than auto-allowing.
fn rule_approval(rule: &Rule) -> ApprovalMode {
    match rule {
        Rule::Send { approval, .. }
        | Rule::Shield { approval, .. }
        | Rule::Unshield { approval, .. }
        | Rule::ContractCall { approval, .. } => *approval,
        Rule::Swap { .. } => ApprovalMode::Always,
    }
}

/// The swap-order decision function — pure, like [`evaluate`]. Swaps NEVER auto-allow in v1:
/// a valid order is ALWAYS `NeedsApproval`. `now` is unix-secs (injected so the fn stays pure).
/// `wallet` is the daemon's unlocked address (the receiver/owner binding).
pub fn evaluate_order(order: &SwapOrder, policy: &Policy, wallet: Address, now: u64) -> Decision {
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    if order.receiver == Address::ZERO {
        return Decision::Deny {
            reason: deny_reasons::RECEIVER_ZERO.into(),
        };
    }
    if order.receiver != wallet {
        return Decision::Deny {
            reason: deny_reasons::RECEIVER_NOT_WALLET.into(),
        };
    }
    // A zero sell amount is a garbage order (nothing to sell) and would let the shaped-approve
    // gate admit an `approve(relayer, 0)`; refuse it outright. (`buy_amount_min == 0` is left
    // valid: a max-slippage market sell is legitimate and the human sees it on the card.)
    if order.sell_amount.is_zero() {
        return Decision::Deny {
            reason: deny_reasons::ZERO_AMOUNT.into(),
        };
    }
    // The `Swap` rule's token lattice. NO Swap rule ⇒ `swap_tokens()` returns `DenyAll` ⇒
    // `OFF_SWAP_LIST` (NOT `NO_RULE`): a missing Swap rule reads as "no token is allowed",
    // preserving the existing parity tag. Both sell+buy must be present for `Only`.
    match policy.swap_tokens() {
        Allowlist::DenyAll => {
            return Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into(),
            };
        }
        Allowlist::Only(v) if !v.contains(&order.sell_token) || !v.contains(&order.buy_token) => {
            return Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into(),
            };
        }
        _ => {}
    }
    if order.valid_to as u64 > now.saturating_add(86_400) {
        return Decision::Deny {
            reason: deny_reasons::VALID_TO_TOO_FAR.into(),
        };
    }
    Decision::NeedsApproval {
        request_id: RequestId::ZERO,
    }
}

/// The message-signing decision function — pure, like [`evaluate`] and [`evaluate_order`].
/// Messages never auto-allow in v1: any safe request is held for human approval. The `wallet`
/// argument is present so the parity signature has the daemon-bound account available as the
/// policy grows; it is intentionally unused by the first pass.
pub fn evaluate_message(message: &SignMessage, policy: &Policy, _wallet: Address) -> Decision {
    if policy.revoked {
        return Decision::Deny {
            reason: deny_reasons::REVOKED.into(),
        };
    }
    match &message.kind {
        SignMessageKind::PersonalSign { .. } => Decision::NeedsApproval {
            request_id: RequestId::ZERO,
        },
        SignMessageKind::TypedDataV4(review) => {
            if review
                .domain_chain_id
                .is_some_and(|chain_id| chain_id != message.chain_id)
            {
                return Decision::Deny {
                    reason: deny_reasons::CHAINID_MISMATCH.into(),
                };
            }
            Decision::NeedsApproval {
                request_id: RequestId::ZERO,
            }
        }
        SignMessageKind::EthSign { .. } => Decision::Deny {
            reason: deny_reasons::ETH_SIGN_REFUSED.into(),
        },
        SignMessageKind::Authorization7702 { .. } => Decision::Deny {
            reason: deny_reasons::DELEGATION_REFUSED.into(),
        },
    }
}

/// Shape check: does the calldata match the kind? The real Railgun adapter calldata is
/// validated downstream (`10-kohaku-shield.md`); this only enforces the coarse invariant
/// the policy gate relies on.
///
/// The Shield invariant matters now that Shield routes to the signing path: a
/// `Shield`/`Unshield` MUST carry non-empty calldata. Without it, an `Intent{kind:Shield,
/// calldata: empty}` would fall through the daemon's broadcast as a **plain native ETH send**
/// to `intent.to` (no private note ever created) while wire-labelled "Shield" — a key-less
/// client could thereby move ETH to an arbitrary address under the Shield label. Requiring
/// calldata closes that. (The deeper `to == RelayAdapt(chain)` check lives downstream — the
/// contract crate is pure policy with zero chain knowledge and no railgun dep, by charter.)
fn calldata_ok(intent: &Intent) -> bool {
    match intent.kind {
        // A plain send carries no calldata (the daemon builds the tx from to/value/token).
        IntentKind::Send => intent.calldata.is_empty(),
        // A contract write / Railgun deposit / withdraw all carry an encoded call. An empty
        // payload for any of these would degrade into a bare native send — reject it.
        IntentKind::ContractCall | IntentKind::Shield | IntentKind::Unshield => {
            !intent.calldata.is_empty()
        }
    }
}

#[cfg(test)]
mod evaluate_order_tests {
    use super::*;
    use alloy_primitives::B256;

    const NOW: u64 = 1_700_000_000;
    const WALLET_BYTE: u8 = 0x11;

    /// The wallet the daemon binds owner/receiver to in these vectors.
    fn wallet() -> Address {
        Address::repeat_byte(WALLET_BYTE)
    }

    /// A base v1 policy: not revoked, `Swap` tokens = `Any` (any token allowed). It also
    /// carries `Send`/`Shield` rules so it reads as a realistic full policy, but
    /// `evaluate_order` only ever inspects the `Swap` rule's `tokens`.
    fn base_policy() -> Policy {
        Policy {
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            revoked: false,
            daily_cap_wei: U256::from(1000u64),
            auto_shield_min_wei: U256::from(10u64),
            spent_today_wei: U256::ZERO,
            rules: vec![
                Rule::Swap {
                    tokens: Allowlist::Any,
                },
                Rule::Send {
                    approval: ApprovalMode::OverCap,
                    per_tx_cap_wei: Some(U256::from(50u64)),
                    recipients: Allowlist::Any,
                },
                Rule::Shield {
                    approval: ApprovalMode::OverCap,
                    per_tx_cap_wei: None,
                },
            ],
        }
    }

    /// A well-formed order whose receiver == wallet and whose `valid_to` sits inside the
    /// 24h horizon. Sub-tests mutate one field at a time off this baseline.
    fn base_order() -> SwapOrder {
        SwapOrder {
            chain_id: 11155111,
            owner: wallet(),
            sell_token: Address::repeat_byte(0xA1),
            buy_token: Address::repeat_byte(0xB2),
            sell_amount: U256::from(1_000_000u64),
            buy_amount_min: U256::from(900_000u64),
            receiver: wallet(),
            valid_to: (NOW + 3600) as u32,
            app_data: B256::repeat_byte(0xCD),
        }
    }

    /// Replace the policy's `Swap` rule's token allowlist (the only field `evaluate_order`
    /// reads), preserving the other rules so the fixture stays a realistic full policy.
    fn with_swap_tokens(mut p: Policy, tokens: Allowlist) -> Policy {
        for rule in &mut p.rules {
            if let Rule::Swap { tokens: t } = rule {
                *t = tokens;
                return p;
            }
        }
        p
    }

    #[test]
    fn revoked_denies() {
        let mut p = base_policy();
        p.revoked = true;
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::REVOKED.into()
            }
        );
    }

    #[test]
    fn receiver_zero_denies() {
        let order = SwapOrder {
            receiver: Address::ZERO,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::RECEIVER_ZERO.into()
            }
        );
    }

    #[test]
    fn receiver_not_wallet_denies() {
        let order = SwapOrder {
            receiver: Address::repeat_byte(0x22),
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::RECEIVER_NOT_WALLET.into()
            }
        );
    }

    #[test]
    fn zero_sell_amount_denies() {
        let order = SwapOrder {
            sell_amount: alloy_primitives::U256::ZERO,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::ZERO_AMOUNT.into()
            }
        );
    }

    #[test]
    fn empty_swap_list_allows_any_token() {
        // `Swap` tokens = `Any` (the base policy): a well-formed order needs approval, never
        // denied. (The "empty" name is historical — the v2 lattice spells "any" explicitly.)
        assert!(matches!(
            evaluate_order(&base_order(), &base_policy(), wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn sell_off_list_denies() {
        // buy_token present, sell_token absent.
        let p = with_swap_tokens(
            base_policy(),
            Allowlist::Only(vec![Address::repeat_byte(0xB2)]),
        );
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn buy_off_list_denies() {
        // sell_token present, buy_token absent.
        let p = with_swap_tokens(
            base_policy(),
            Allowlist::Only(vec![Address::repeat_byte(0xA1)]),
        );
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn both_off_list_denies() {
        let p = with_swap_tokens(
            base_policy(),
            Allowlist::Only(vec![Address::repeat_byte(0xEE)]),
        );
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn both_on_list_needs_approval() {
        let p = with_swap_tokens(
            base_policy(),
            Allowlist::Only(vec![Address::repeat_byte(0xA1), Address::repeat_byte(0xB2)]),
        );
        assert!(matches!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn valid_to_at_horizon_is_allowed() {
        // Boundary: valid_to == now + 86_400 (exactly 24h) is INSIDE the horizon.
        let order = SwapOrder {
            valid_to: (NOW + 86_400) as u32,
            ..base_order()
        };
        assert!(matches!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn valid_to_one_past_horizon_denies() {
        // Boundary: valid_to == now + 86_401 is one second too far.
        let order = SwapOrder {
            valid_to: (NOW + 86_401) as u32,
            ..base_order()
        };
        assert_eq!(
            evaluate_order(&order, &base_policy(), wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::VALID_TO_TOO_FAR.into()
            }
        );
    }

    #[test]
    fn no_swap_rule_denies_off_swap_list() {
        // Default-deny: a policy with NO Swap rule has `swap_tokens() == DenyAll`, so a
        // well-formed order is `OFF_SWAP_LIST` (NOT `no_rule`) — preserving the swap parity tag.
        let p = Policy {
            rules: vec![Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            }],
            ..base_policy()
        };
        assert_eq!(
            evaluate_order(&base_order(), &p, wallet(), NOW),
            Decision::Deny {
                reason: deny_reasons::OFF_SWAP_LIST.into()
            }
        );
    }

    #[test]
    fn well_formed_order_needs_approval_with_zero_placeholder() {
        // A valid order never auto-allows: it is ALWAYS NeedsApproval, and the pure fn
        // returns the ZERO placeholder id (the stateful caller mints the real one).
        assert_eq!(
            evaluate_order(&base_order(), &base_policy(), wallet(), NOW),
            Decision::NeedsApproval {
                request_id: RequestId::ZERO
            }
        );
    }
}

#[cfg(test)]
mod message_signing_tests {
    use super::*;
    use crate::{MessageSigningRisk, SignMessage, SignMessageKind, TypedDataReview};
    use alloy_primitives::B256;

    const CHAIN_ID: u64 = 11155111;

    fn wallet() -> Address {
        Address::repeat_byte(0x11)
    }

    /// A base v1 policy. `evaluate_message` only reads `revoked`, so the rules are immaterial
    /// here; a single `Shield` rule keeps the fixture minimal and valid.
    fn base_policy() -> Policy {
        Policy {
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            revoked: false,
            daily_cap_wei: U256::from(1000u64),
            auto_shield_min_wei: U256::from(10u64),
            spent_today_wei: U256::ZERO,
            rules: vec![Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            }],
        }
    }

    fn personal_message() -> SignMessage {
        SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::PersonalSign {
                message: b"Sign in to Deckard".as_slice().into(),
            },
        }
    }

    fn typed_message(chain_id: u64) -> SignMessage {
        SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::TypedDataV4(TypedDataReview {
                domain_name: Some("Permit2".into()),
                domain_version: Some("1".into()),
                domain_chain_id: Some(chain_id),
                verifying_contract: Some(Address::repeat_byte(0x22)),
                primary_type: "PermitSingle".into(),
                digest: B256::repeat_byte(0x42),
                risks: vec![MessageSigningRisk::PermitLike],
                permit: None,
            }),
        }
    }

    #[test]
    fn personal_sign_always_needs_approval() {
        assert_eq!(
            evaluate_message(&personal_message(), &base_policy(), wallet()),
            Decision::NeedsApproval {
                request_id: RequestId::ZERO
            }
        );
    }

    #[test]
    fn typed_data_chainid_mismatch_denies() {
        assert_eq!(
            evaluate_message(&typed_message(1), &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::CHAINID_MISMATCH.into()
            }
        );
    }

    #[test]
    fn eth_sign_is_refused() {
        let message = SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::EthSign {
                digest: B256::repeat_byte(0x33),
            },
        };
        assert_eq!(
            evaluate_message(&message, &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::ETH_SIGN_REFUSED.into()
            }
        );
    }

    #[test]
    fn eip7702_delegation_refused() {
        let message = SignMessage {
            chain_id: CHAIN_ID,
            origin: "https://example.test".into(),
            kind: SignMessageKind::Authorization7702 {
                delegate: Address::repeat_byte(0x44),
                nonce: 7,
            },
        };
        assert_eq!(
            evaluate_message(&message, &base_policy(), wallet()),
            Decision::Deny {
                reason: deny_reasons::DELEGATION_REFUSED.into()
            }
        );
    }
}

#[cfg(test)]
mod policy_v2_tests {
    //! Tests specific to the v2 rule-list shape: default-deny on a missing rule, the
    //! `validate` gate (version + duplicate action), and the per-action accessors.
    use super::*;
    use crate::intent::IntentKind;
    use alloy_primitives::Bytes;

    fn send_intent(to: Address, value: u64) -> Intent {
        Intent {
            chain_id: 1,
            to,
            token: None,
            value: U256::from(value),
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        }
    }

    fn policy_with(rules: Vec<Rule>) -> Policy {
        Policy {
            version: POLICY_VERSION,
            default_effect: Effect::Deny,
            revoked: false,
            daily_cap_wei: U256::from(1000u64),
            auto_shield_min_wei: U256::from(10u64),
            spent_today_wei: U256::ZERO,
            rules,
        }
    }

    #[test]
    fn no_matching_rule_denies_no_rule() {
        // A policy with only a Shield rule denies a Send with the NEW default-deny tag.
        let p = policy_with(vec![Rule::Shield {
            approval: ApprovalMode::Never,
            per_tx_cap_wei: None,
        }]);
        assert_eq!(
            evaluate(&send_intent(Address::repeat_byte(0x22), 10), &p),
            Decision::Deny {
                reason: deny_reasons::NO_RULE.into()
            }
        );
    }

    #[test]
    fn empty_rules_denies_everything() {
        let p = policy_with(vec![]);
        assert_eq!(
            evaluate(&send_intent(Address::repeat_byte(0x22), 10), &p),
            Decision::Deny {
                reason: deny_reasons::NO_RULE.into()
            }
        );
    }

    #[test]
    fn validate_rejects_wrong_version() {
        let p = Policy {
            version: 2,
            ..policy_with(vec![])
        };
        assert_eq!(p.validate(), Err(PolicyError::UnsupportedVersion(2)));
    }

    #[test]
    fn validate_rejects_duplicate_action() {
        let p = policy_with(vec![
            Rule::Send {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
                recipients: Allowlist::Any,
            },
            Rule::Send {
                approval: ApprovalMode::Always,
                per_tx_cap_wei: None,
                recipients: Allowlist::DenyAll,
            },
        ]);
        assert_eq!(p.validate(), Err(PolicyError::DuplicateAction("send")));
    }

    #[test]
    fn validate_accepts_a_clean_v1_policy() {
        let p = policy_with(vec![
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(50u64)),
                recipients: Allowlist::Any,
            },
            Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            },
            Rule::Swap {
                tokens: Allowlist::Any,
            },
        ]);
        assert_eq!(p.validate(), Ok(()));
    }

    #[test]
    fn deny_all_recipients_denies_send() {
        // An explicit `DenyAll` recipient set denies every recipient (the lattice ⊥).
        let p = policy_with(vec![Rule::Send {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: None,
            recipients: Allowlist::DenyAll,
        }]);
        assert_eq!(
            evaluate(&send_intent(Address::repeat_byte(0x22), 10), &p),
            Decision::Deny {
                reason: deny_reasons::OFF_ALLOWLIST.into()
            }
        );
    }

    #[test]
    fn accessors_read_the_matching_rule() {
        let p = policy_with(vec![
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(50u64)),
                recipients: Allowlist::Only(vec![Address::repeat_byte(0xAA)]),
            },
            Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            },
            Rule::Swap {
                tokens: Allowlist::Only(vec![Address::repeat_byte(0xCC)]),
            },
        ]);
        // per_tx_cap_for: Some for Send, None for Shield (no cap) and a no-rule action.
        assert_eq!(p.per_tx_cap_for(IntentKind::Send), Some(U256::from(50u64)));
        assert_eq!(p.per_tx_cap_for(IntentKind::Shield), None);
        assert_eq!(p.per_tx_cap_for(IntentKind::Unshield), None);
        // approval_for: Some for the intent kinds, None for a no-rule action.
        assert_eq!(
            p.approval_for(IntentKind::Send),
            Some(ApprovalMode::OverCap)
        );
        assert_eq!(
            p.approval_for(IntentKind::Shield),
            Some(ApprovalMode::Never)
        );
        assert_eq!(p.approval_for(IntentKind::ContractCall), None);
        // recipients_for: Send's set, Any for Shield, DenyAll floor for a no-rule action.
        assert_eq!(
            p.recipients_for(IntentKind::Send),
            &Allowlist::Only(vec![Address::repeat_byte(0xAA)])
        );
        assert_eq!(p.recipients_for(IntentKind::Shield), &Allowlist::Any);
        assert_eq!(
            p.recipients_for(IntentKind::ContractCall),
            &Allowlist::DenyAll
        );
        // swap_tokens: the Swap rule's set.
        assert_eq!(
            p.swap_tokens(),
            &Allowlist::Only(vec![Address::repeat_byte(0xCC)])
        );
    }

    #[test]
    fn swap_tokens_floor_is_deny_all_with_no_swap_rule() {
        let p = policy_with(vec![Rule::Shield {
            approval: ApprovalMode::Never,
            per_tx_cap_wei: None,
        }]);
        assert_eq!(p.swap_tokens(), &Allowlist::DenyAll);
    }

    // ── #185 cap-enforced-on-shields (TRUST-CRITICAL) ─────────────────────────────────────────
    // A Shield now carries a per-tx cap, and `evaluate` (the ONE gate the mock AND the daemon
    // call) enforces it on the shield path exactly as it does for a Send. The bug this locks
    // shut: a 0.15 deposit auto-broadcast under a stated 0.1 per-move cap because
    // `per_tx_cap_for(Shield)` returned `None` and the per-tx check silently short-circuited.

    /// A Shield intent with the non-empty calldata `calldata_ok` requires. `to`/`chain_id` are
    /// immaterial to `evaluate` (shields carry no recipient allowlist), so only `value` varies.
    fn shield_intent(value: u64) -> Intent {
        Intent {
            chain_id: 1,
            to: Address::repeat_byte(0x33),
            token: None,
            value: U256::from(value),
            calldata: Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]),
            kind: IntentKind::Shield,
        }
    }

    /// A demo-shaped shield rule: `over_cap` approval + a per-tx cap, under a high daily wall so
    /// the per-tx cap is the ONLY fence that can trip (proving per-tx enforcement on the shield
    /// path, not the daily wall).
    fn shield_cap_policy(per_tx: u64) -> Policy {
        Policy {
            daily_cap_wei: U256::from(1_000_000u64),
            ..policy_with(vec![Rule::Shield {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(per_tx)),
            }])
        }
    }

    #[test]
    fn shield_over_per_tx_cap_needs_approval() {
        // THE regression: a shield OVER the stated per-move cap ASKS, never auto-broadcasts.
        let p = shield_cap_policy(100);
        assert!(
            matches!(
                evaluate(&shield_intent(150), &p),
                Decision::NeedsApproval { .. }
            ),
            "a shield over the per-tx cap must be held for approval, not auto-allowed (#185)"
        );
    }

    #[test]
    fn shield_within_per_tx_cap_allows() {
        // The other half: a within-cap shield still auto-allows (the fix doesn't over-block).
        let p = shield_cap_policy(100);
        assert_eq!(evaluate(&shield_intent(50), &p), Decision::Allow);
        // Boundary: exactly at the cap is within (the check is strictly `>`).
        assert_eq!(evaluate(&shield_intent(100), &p), Decision::Allow);
    }

    #[test]
    fn shield_over_per_tx_cap_denies_under_never_mode() {
        // With `never` approval there is no card to authorise an over-cap move, so it DENIES
        // (fail-closed) rather than silently broadcasting.
        let p = Policy {
            daily_cap_wei: U256::from(1_000_000u64),
            ..policy_with(vec![Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: Some(U256::from(100u64)),
            }])
        };
        assert_eq!(
            evaluate(&shield_intent(150), &p),
            Decision::Deny {
                reason: deny_reasons::OVER_CAP.into()
            }
        );
    }

    #[test]
    fn shield_per_tx_cap_is_read_by_the_accessor() {
        // Locks the `per_tx_cap_for` arm that was the bug: it must now return the shield rule's cap.
        let p = shield_cap_policy(100);
        assert_eq!(
            p.per_tx_cap_for(IntentKind::Shield),
            Some(U256::from(100u64))
        );
    }

    #[test]
    fn authority_for_over_cap_matches_evaluate() {
        // Pin `authority_for.over_cap` (the UI's "is this a breach?" signal) to `evaluate`'s
        // verdict across a value matrix, so the Allowed-by line can never claim headroom the
        // engine doesn't back (the honest-enforced-cap invariant).
        let p = shield_cap_policy(100);
        for value in [1u64, 50, 100, 101, 150, 1_000_000, 2_000_000] {
            let auth = p
                .authority_for(IntentKind::Shield, U256::from(value))
                .expect("shield rule governs the kind");
            let asks = matches!(
                evaluate(&shield_intent(value), &p),
                Decision::NeedsApproval { .. }
            );
            assert_eq!(
                auth.over_cap, asks,
                "authority_for.over_cap must equal evaluate's ask-verdict at value {value}"
            );
        }
    }

    #[test]
    fn authority_for_reports_daily_remaining_after_the_move() {
        // The "$X of $Y daily left after this" figures come straight off the policy, so the UI
        // never recomputes cap math. Y = daily cap; X = daily cap − (spent + value).
        let p = Policy {
            daily_cap_wei: U256::from(1000u64),
            spent_today_wei: U256::from(200u64),
            ..policy_with(vec![Rule::Shield {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(500u64)),
            }])
        };
        let auth = p
            .authority_for(IntentKind::Shield, U256::from(300u64))
            .unwrap();
        assert_eq!(auth.rule_label, "Shield rule");
        assert_eq!(auth.daily_cap_wei, U256::from(1000u64));
        // 1000 − (200 + 300) = 500 left after this move.
        assert_eq!(auth.daily_remaining_after_wei, U256::from(500u64));
        assert!(
            !auth.over_cap,
            "300 is within the 500 per-tx and 1000 daily caps"
        );
    }

    #[test]
    fn authority_for_is_none_without_a_governing_rule() {
        // Default-deny: no rule for the kind ⇒ no authority to cite (the review shows the deny).
        let p = policy_with(vec![]);
        assert_eq!(p.authority_for(IntentKind::Shield, U256::from(1u64)), None);
    }
}

#[cfg(test)]
mod rule_serde_tests {
    //! Locks in the by-hand `Rule` serde: the exact internally-tagged JSON shape, strict
    //! field handling (unknown key / wrong-action key / missing required field), the
    //! omitted-allowlist default, and a CBOR round-trip (the reason the serde is hand-rolled
    //! at all — `#[serde(tag)]` + `ciborium` + `alloy` breaks, see the `Rule` type doc).
    use super::*;

    #[test]
    fn send_rule_json_shape_is_internally_tagged() {
        let rule = Rule::Send {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: Some(U256::from(5u64)),
            recipients: Allowlist::Any,
        };
        let json: serde_json::Value = serde_json::to_value(&rule).unwrap();
        assert_eq!(json["action"], "send");
        assert_eq!(json["approval"], "over_cap");
        assert_eq!(json["recipients"], "any");
        // per_tx_cap_wei is present (Some) and encoded as a 0x-hex string.
        assert_eq!(json["per_tx_cap_wei"], "0x5");
    }

    #[test]
    fn none_per_tx_cap_is_omitted() {
        let rule = Rule::Send {
            approval: ApprovalMode::Always,
            per_tx_cap_wei: None,
            recipients: Allowlist::Any,
        };
        let json: serde_json::Value = serde_json::to_value(&rule).unwrap();
        assert!(
            json.get("per_tx_cap_wei").is_none(),
            "a None per_tx_cap_wei must be omitted, got {json}"
        );
    }

    #[test]
    fn omitted_recipients_decodes_to_deny_all() {
        // The default-deny floor: an omitted allowlist field is `DenyAll`, never "any".
        let rule: Rule =
            serde_json::from_str(r#"{"action":"send","approval":"over_cap"}"#).unwrap();
        assert_eq!(
            rule,
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: None,
                recipients: Allowlist::DenyAll,
            }
        );
    }

    #[test]
    fn recipients_any_and_array_decode() {
        let any: Rule =
            serde_json::from_str(r#"{"action":"send","approval":"over_cap","recipients":"any"}"#)
                .unwrap();
        assert!(matches!(
            any,
            Rule::Send {
                recipients: Allowlist::Any,
                ..
            }
        ));
        let only: Rule = serde_json::from_str(
            r#"{"action":"send","approval":"over_cap","recipients":["0xAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAaAa"]}"#,
        )
        .unwrap();
        assert_eq!(
            only,
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: None,
                recipients: Allowlist::Only(vec![Address::repeat_byte(0xAA)]),
            }
        );
    }

    #[test]
    fn unknown_field_in_rule_is_rejected() {
        let bad: Result<Rule, _> =
            serde_json::from_str(r#"{"action":"send","approval":"over_cap","bogus":1}"#);
        assert!(
            bad.is_err(),
            "an unknown field inside a rule must be rejected"
        );
    }

    #[test]
    fn wrong_action_field_is_rejected() {
        // `tokens` belongs to `swap`, not `send` — a no-op field that would lie to the author.
        let bad: Result<Rule, _> =
            serde_json::from_str(r#"{"action":"send","approval":"over_cap","tokens":"any"}"#);
        assert!(
            bad.is_err(),
            "a field belonging to a different action must be rejected"
        );
    }

    #[test]
    fn missing_required_field_and_unknown_action_error() {
        let missing: Result<Rule, _> = serde_json::from_str(r#"{"action":"send"}"#);
        assert!(missing.is_err(), "a missing required field must error");
        let unknown: Result<Rule, _> =
            serde_json::from_str(r#"{"action":"teleport","approval":"never"}"#);
        assert!(unknown.is_err(), "an unknown action must error");
    }

    #[test]
    fn non_any_allowlist_string_is_rejected() {
        let bad: Result<Rule, _> =
            serde_json::from_str(r#"{"action":"send","approval":"over_cap","recipients":"all"}"#);
        assert!(bad.is_err(), "a non-\"any\" allowlist string must error");
    }

    #[test]
    fn rule_cbor_roundtrips() {
        // The reason the serde is hand-rolled: an internally-tagged derive breaks CBOR with
        // alloy types. Each variant must survive a ciborium round-trip.
        for rule in [
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(2u64)),
                recipients: Allowlist::Only(vec![Address::repeat_byte(0xAA)]),
            },
            Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            },
            // #185: a Shield WITH a per-tx cap must also survive CBOR (the new wire field).
            Rule::Shield {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(7u64)),
            },
            Rule::Unshield {
                approval: ApprovalMode::Always,
                per_tx_cap_wei: None,
            },
            Rule::Swap {
                tokens: Allowlist::Any,
            },
            Rule::ContractCall {
                approval: ApprovalMode::OverCap,
                targets: Allowlist::Only(vec![Address::repeat_byte(0xCC)]),
            },
        ] {
            let mut cbor = Vec::new();
            ciborium::into_writer(&rule, &mut cbor).expect("cbor encode");
            let back: Rule = ciborium::from_reader(&cbor[..]).expect("cbor decode");
            assert_eq!(back, rule, "rule did not survive a CBOR round-trip");
        }
    }

    #[test]
    fn shield_per_tx_cap_json_shape_and_omission() {
        // #185: the shield rule's optional per-tx cap emits as a 0x-hex string when present and is
        // OMITTED when `None` (matching Send/Unshield), and decodes back to the same value.
        let capped = Rule::Shield {
            approval: ApprovalMode::OverCap,
            per_tx_cap_wei: Some(U256::from(5u64)),
        };
        let json: serde_json::Value = serde_json::to_value(&capped).unwrap();
        assert_eq!(json["action"], "shield");
        assert_eq!(json["per_tx_cap_wei"], "0x5");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, capped);

        let uncapped: Rule =
            serde_json::from_str(r#"{"action":"shield","approval":"never"}"#).unwrap();
        assert_eq!(
            uncapped,
            Rule::Shield {
                approval: ApprovalMode::Never,
                per_tx_cap_wei: None,
            }
        );
        let uncapped_json: serde_json::Value = serde_json::to_value(&uncapped).unwrap();
        assert!(
            uncapped_json.get("per_tx_cap_wei").is_none(),
            "a None shield per_tx_cap_wei must be omitted, got {uncapped_json}"
        );
    }
}

#[cfg(test)]
mod demo_shape_check {
    use super::*;
    #[test]
    fn demo_json_from_the_plan_decodes_and_validates() {
        // The exact policy.demo.json shape (#185: the shield rule now carries a per-tx cap so a
        // large deposit can't auto-broadcast past the stated per-move limit).
        let json = r#"{ "version":1, "default":"deny",
            "daily_cap_wei":"500000000000000000", "auto_shield_min_wei":"10000000000000000",
            "rules":[ {"action":"shield","approval":"over_cap","per_tx_cap_wei":"100000000000000000"},
                      {"action":"send","approval":"over_cap","per_tx_cap_wei":"100000000000000000","recipients":"any"},
                      {"action":"swap","tokens":"any"} ] }"#;
        let p: Policy = serde_json::from_str(json).expect("demo policy decodes");
        assert!(p.validate().is_ok(), "demo policy validates");
        assert_eq!(
            p.approval_for(IntentKind::Send),
            Some(ApprovalMode::OverCap)
        );
        assert_eq!(p.recipients_for(IntentKind::Send), &Allowlist::Any);
        assert_eq!(p.swap_tokens(), &Allowlist::Any);
        // #185: the demo shield rule is now capped at 0.1 ETH per move (equal to the send cap),
        // so a 0.15 ETH shield asks instead of auto-broadcasting.
        assert_eq!(
            p.per_tx_cap_for(IntentKind::Shield),
            Some(U256::from(100_000_000_000_000_000u128)),
            "the demo shield rule carries the 0.1 ETH per-tx cap (#185)"
        );
    }
}
