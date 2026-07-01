//! Loading the signer [`Policy`] and tracking the daily spend.
//!
//! The policy is read from `policy.json` in the config dir. There are **two distinct
//! fallbacks** (ADR 0005 §5):
//!
//! * **No file** ⇒ a starter posture: the [`Preset`] named by `DECKARD_POLICY_PRESET` if set
//!   (`shield-only` | `ask-me-everything` | `locked` | `default`), otherwise the FRIENDLY
//!   built-in [`default_policy`] (a sane first-run policy: shield freely, send with an
//!   always-card, swap any token, under a 0.2 ETH daily cap). A preset is a no-file affordance
//!   only — an authored `policy.json` always wins over it.
//! * **A file that exists but does NOT load** (malformed JSON, a legacy v0 flat policy with
//!   no `version` key, or a v1 file that fails [`Policy::validate`]) ⇒ the MOST-RESTRICTIVE
//!   [`deny_all_policy`] (every action denied, `NO_RULE`). This is loud: a typo'd policy must
//!   never silently degrade into a permissive default (nor can a preset revive it).
//!
//! `spent_today_wei` is **in-memory only**, rolls over at UTC midnight, and resets on daemon
//! restart — cross-restart persistence is owned by the durable `SpendStore` (#108). Presets are
//! selected at launch; there is no runtime `SetPolicy` mutation API yet (deferred to #29).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::U256;

use deckard_contract::{Allowlist, ApprovalMode, Effect, Policy, Rule, POLICY_VERSION};

/// Default rolling daily cap: 0.2 ETH.
pub const DEFAULT_DAILY_CAP_WEI: u128 = 200_000_000_000_000_000;
/// Default auto-shield threshold: 0.01 ETH.
pub const DEFAULT_AUTO_SHIELD_MIN_WEI: u128 = 10_000_000_000_000_000;

/// The FRIENDLY built-in policy used when `policy.json` is absent (a normal first run, ADR
/// 0005 §5): shield freely (no card), send to any recipient with an always-card and no per-tx
/// cap, swap any token — all under the 0.2 ETH daily cap. Default-deny still holds for every
/// action without a rule (e.g. `Unshield`/`ContractCall`).
pub fn default_policy() -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(DEFAULT_DAILY_CAP_WEI),
        auto_shield_min_wei: U256::from(DEFAULT_AUTO_SHIELD_MIN_WEI),
        spent_today_wei: U256::ZERO,
        rules: vec![
            Rule::Shield {
                approval: ApprovalMode::Never,
            },
            Rule::Send {
                approval: ApprovalMode::Always,
                per_tx_cap_wei: None,
                recipients: Allowlist::Any,
            },
            Rule::Swap {
                tokens: Allowlist::Any,
            },
        ],
    }
}

/// The MOST-RESTRICTIVE deny-all policy used when `policy.json` EXISTS but did not load
/// (malformed, legacy v0, or fails `validate`). No rules ⇒ every action is `NO_RULE`-denied,
/// and the caps are zero. A typo'd policy fails closed, never open.
pub fn deny_all_policy() -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::ZERO,
        auto_shield_min_wei: U256::ZERO,
        spent_today_wei: U256::ZERO,
        rules: vec![],
    }
}

/// `shield-only` preset (ADR 0005 §5, #135 PR4): the safest useful agent — shield auto-allowed
/// under the daily cap, every other action default-deny (no `Send`, no `Swap`). An agent under
/// this posture can only move funds INTO your own private balance; it can never send to a third
/// party or trade.
pub fn shield_only_policy() -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(DEFAULT_DAILY_CAP_WEI),
        auto_shield_min_wei: U256::from(DEFAULT_AUTO_SHIELD_MIN_WEI),
        spent_today_wei: U256::ZERO,
        rules: vec![Rule::Shield {
            approval: ApprovalMode::Never,
        }],
    }
}

/// `ask-me-everything` preset (ADR 0005 §5, #135 PR4): shield, send, and swap are all enabled
/// but every one raises a human-approval card — send to any recipient with no per-tx cap, swap
/// any token (a swap is always carded regardless). Under the 0.2 ETH daily cap. The posture for
/// an operator who wants the agent to propose freely but sign nothing without them.
pub fn ask_me_everything_policy() -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(DEFAULT_DAILY_CAP_WEI),
        auto_shield_min_wei: U256::from(DEFAULT_AUTO_SHIELD_MIN_WEI),
        spent_today_wei: U256::ZERO,
        rules: vec![
            Rule::Shield {
                approval: ApprovalMode::Always,
            },
            Rule::Send {
                approval: ApprovalMode::Always,
                per_tx_cap_wei: None,
                recipients: Allowlist::Any,
            },
            Rule::Swap {
                tokens: Allowlist::Any,
            },
        ],
    }
}

/// `locked` preset (ADR 0005 §5, #135 PR4): a frozen rulebook — no rules, so every action is
/// `NO_RULE`-denied. This is a *chosen* posture (a switchable safe default the operator selects),
/// deliberately distinct from the runtime `revoked` STOP brake an operator flips in an emergency.
/// Byte-identical to [`deny_all_policy`], but reached by intent rather than as a load fallback.
pub fn locked_policy() -> Policy {
    deny_all_policy()
}

/// A named starter posture for the agent's Rules (ADR 0005 §5, #135 PR4). Each is a complete,
/// default-deny [`Policy`] on the v1 schema — a one-word way to set the agent's autonomy without
/// hand-editing `policy.json`. Selected at daemon start via `DECKARD_POLICY_PRESET` when no
/// `policy.json` exists; a real file ALWAYS wins, so a preset never overrides an authored policy
/// and never weakens the strict load classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// The FRIENDLY first-run posture (same as [`default_policy`]): shield auto-allowed, send
    /// always-carded to any recipient, swap always-carded, under the 0.2 ETH daily cap.
    Default,
    /// The safest useful agent: shield auto-allowed, everything else denied ([`shield_only_policy`]).
    ShieldOnly,
    /// Every action raises a card ([`ask_me_everything_policy`]).
    AskMeEverything,
    /// A frozen rulebook — every action denied ([`locked_policy`]).
    Locked,
}

impl Preset {
    /// Every preset, for enumeration (CLI help / the ⌘K picker).
    pub const ALL: [Preset; 4] = [
        Preset::Default,
        Preset::ShieldOnly,
        Preset::AskMeEverything,
        Preset::Locked,
    ];

    /// This preset's wire name (the `DECKARD_POLICY_PRESET` value / CLI token).
    pub fn name(self) -> &'static str {
        match self {
            Preset::Default => "default",
            Preset::ShieldOnly => "shield-only",
            Preset::AskMeEverything => "ask-me-everything",
            Preset::Locked => "locked",
        }
    }

    /// A one-line description for help text and the picker.
    pub fn summary(self) -> &'static str {
        match self {
            Preset::Default => "shield auto-allowed, send always-carded, swap always-carded",
            Preset::ShieldOnly => {
                "shield auto-allowed, everything else denied (safest useful agent)"
            }
            Preset::AskMeEverything => {
                "every action asks — shield, send, and swap all human-approved"
            }
            Preset::Locked => "no rules — every action denied (a frozen rulebook)",
        }
    }

    /// Parse a preset by its wire name. Hyphen and underscore spellings are both accepted and the
    /// match is case-insensitive; `None` for an unknown name.
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "default" => Some(Preset::Default),
            "shield-only" => Some(Preset::ShieldOnly),
            "ask-me-everything" => Some(Preset::AskMeEverything),
            "locked" => Some(Preset::Locked),
            _ => None,
        }
    }

    /// The complete [`Policy`] this preset installs.
    pub fn policy(self) -> Policy {
        match self {
            Preset::Default => default_policy(),
            Preset::ShieldOnly => shield_only_policy(),
            Preset::AskMeEverything => ask_me_everything_policy(),
            Preset::Locked => locked_policy(),
        }
    }
}

/// The preset selected via the `DECKARD_POLICY_PRESET` env var, if set to a known name. An unknown
/// (typo'd) value is a LOUD no-op — it warns and returns `None`, so the friendly default applies
/// rather than bricking the agent into deny-all. Only ever consulted when there is no
/// `policy.json` (the [`PolicyLoad::DefaultMissing`] arm).
pub fn preset_from_env() -> Option<Preset> {
    let raw = std::env::var("DECKARD_POLICY_PRESET").ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    match Preset::from_name(&raw) {
        Some(p) => Some(p),
        None => {
            eprintln!(
                "signerd: ⚠ DECKARD_POLICY_PRESET={raw:?} is not a known preset \
                 (shield-only | ask-me-everything | locked | default); ignoring it and using \
                 the built-in default policy",
            );
            None
        }
    }
}

/// How a policy load resolved — split out from [`load_policy`] so the fallback cases are
/// unit-testable (the logging wrapper can't be asserted on).
#[derive(Debug)]
pub enum PolicyLoad {
    /// `policy.json` parsed cleanly and passed [`Policy::validate`].
    Loaded(Policy),
    /// No file at the path — a normal first run; the FRIENDLY [`default_policy`] applies quietly.
    DefaultMissing,
    /// The file EXISTS but didn't load (unreadable, not JSON, a legacy v0 policy with no
    /// `version` key, or a v1 file that fails `validate`). The MOST-RESTRICTIVE
    /// [`deny_all_policy`] applies (every action denied), and this is loud: a typo'd policy
    /// must never silently degrade into a permissive default.
    DefaultInvalid(String),
}

/// Resolve a policy load without logging. The classification is strict (ADR 0005 §5): a file
/// that exists but does not cleanly parse-and-validate is `DefaultInvalid` (⇒ deny-all), never
/// reinterpreted. A legacy v0 flat policy (no `version` key) is rejected with a SPECIFIC
/// message rather than reinterpreting its `allow_to: [] = any` semantics. On success
/// `spent_today_wei`/`revoked` are forced to their fresh-start values: they are
/// `#[serde(default)]` (a file may carry them, but the durable `SpendStore`/#108 counter is the
/// source of truth — a file can never inject a spend) and the daemon boots armed.
pub fn load_policy_outcome(path: &Path) -> PolicyLoad {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PolicyLoad::DefaultMissing,
        Err(e) => return PolicyLoad::DefaultInvalid(format!("read failed: {e}")),
    };
    // Parse to a generic Value first so a missing `version` key (a legacy v0 flat policy) is
    // detected and rejected with a specific message — rather than the v1 decoder's generic
    // "missing field `version`" parse error.
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return PolicyLoad::DefaultInvalid(format!("not json: {e}")),
    };
    if value.get("version").is_none() {
        return PolicyLoad::DefaultInvalid("legacy v0 policy — rewrite to v1".into());
    }
    let mut p: Policy = match serde_json::from_value(value) {
        Ok(p) => p,
        Err(e) => return PolicyLoad::DefaultInvalid(format!("parse failed/invalid: {e}")),
    };
    if let Err(e) = p.validate() {
        return PolicyLoad::DefaultInvalid(format!("parse failed/invalid: {e}"));
    }
    p.spent_today_wei = U256::ZERO;
    p.revoked = false;
    PolicyLoad::Loaded(p)
}

/// Load the policy from `path`, consulting `DECKARD_POLICY_PRESET` for the no-file case. See
/// [`load_policy_with_preset`] for the full classification.
pub fn load_policy(path: &Path) -> Policy {
    load_policy_with_preset(path, preset_from_env())
}

/// Load the policy from `path` with the two fallbacks: a missing file uses a starter posture — the
/// selected `preset` if one was given, otherwise the FRIENDLY [`default_policy`]; a file that
/// exists but did not load falls back LOUDLY to the MOST-RESTRICTIVE [`deny_all_policy`] (every
/// action denied) — a silent fallback would let a typo'd `policy.json` run a permissive default
/// with nobody the wiser. A preset is a NO-FILE affordance only: an authored `policy.json` always
/// wins, and a preset can never turn the loud deny-all fallback back into something permissive.
pub fn load_policy_with_preset(path: &Path, preset: Option<Preset>) -> Policy {
    match load_policy_outcome(path) {
        PolicyLoad::Loaded(p) => p,
        PolicyLoad::DefaultMissing => match preset {
            Some(p) => {
                eprintln!(
                    "signerd: no policy file at {} — using the '{}' starter preset \
                     (DECKARD_POLICY_PRESET): {}",
                    path.display(),
                    p.name(),
                    p.summary(),
                );
                p.policy()
            }
            None => {
                eprintln!(
                    "signerd: no policy file at {} — using the built-in default policy",
                    path.display()
                );
                default_policy()
            }
        },
        PolicyLoad::DefaultInvalid(why) => {
            eprintln!(
                "signerd: ⚠ POLICY FALLBACK — {} exists but did not load ({why}); \
                 running the MOST-RESTRICTIVE DENY-ALL policy (every action denied). \
                 Fix the file and restart the daemon.",
                path.display()
            );
            deny_all_policy()
        }
    }
}

/// Days since the Unix epoch in UTC — the rollover key for `spent_today_wei`. (Chrono-free:
/// integer division of the wall-clock seconds.)
pub fn current_utc_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_contract::IntentKind;

    #[test]
    fn absent_file_yields_default() {
        // No file ⇒ the FRIENDLY built-in: default-deny, the 0.2 ETH daily cap, armed, and a
        // Send rule that allows any recipient.
        let p = load_policy(Path::new("/nonexistent/deckard/policy.json"));
        assert_eq!(p.default_effect, Effect::Deny);
        assert_eq!(p.daily_cap_wei, U256::from(DEFAULT_DAILY_CAP_WEI));
        assert!(!p.revoked);
        match p.rule_for(IntentKind::Send) {
            Some(Rule::Send { recipients, .. }) => assert_eq!(*recipients, Allowlist::Any),
            other => panic!("expected a Send rule with Any recipients, got {other:?}"),
        }
    }

    #[test]
    fn load_resets_spent_and_revoked() {
        let dir = std::env::temp_dir().join(format!("deckard-policy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        // The FRIENDLY default, serialized to disk. `spent_today_wei`/`revoked` are
        // `#[serde(skip)]`, so they never make it to the file — but the loader forces them to
        // fresh-start values regardless.
        std::fs::write(&path, serde_json::to_vec(&default_policy()).unwrap()).unwrap();

        let loaded = load_policy(&path);
        assert_eq!(
            loaded.spent_today_wei,
            U256::ZERO,
            "spend is in-memory; never trusted"
        );
        assert!(!loaded.revoked, "daemon boots armed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_file_yields_deny_all() {
        let dir = std::env::temp_dir().join(format!("deckard-policy-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let p = load_policy(&path);
        assert!(
            p.rules.is_empty(),
            "malformed file must fall back to deny-all"
        );
        assert_eq!(p.daily_cap_wei, U256::ZERO, "deny-all has a zero daily cap");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A legacy v0 flat policy (no `version` key) is REJECTED with a specific message rather
    /// than reinterpreted — its `allow_to: [] = any` semantics would silently flip the
    /// recipient axis. The loud `DefaultInvalid` arm ⇒ deny-all.
    #[test]
    fn legacy_v0_file_is_rejected() {
        let dir = std::env::temp_dir().join(format!("deckard-policy-v0-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(
            &path,
            br#"{"per_tx_cap_wei":"1","daily_cap_wei":"1","allow_to":[],"require_approval":"OverCap"}"#,
        )
        .unwrap();
        match load_policy_outcome(&path) {
            PolicyLoad::DefaultInvalid(why) => {
                assert!(why.contains("legacy v0"), "unexpected cause: {why}")
            }
            other => panic!("expected DefaultInvalid (legacy v0), got {other:?}"),
        }
        let p = load_policy(&path);
        assert!(
            p.rules.is_empty(),
            "a legacy v0 file must fall back to deny-all"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every preset is a valid, default-deny, armed, fresh-start policy — so any of them can be
    /// installed without the loud `DefaultInvalid` fallback ever firing.
    #[test]
    fn presets_are_valid_default_deny_and_armed() {
        for preset in Preset::ALL {
            let p = preset.policy();
            assert!(p.validate().is_ok(), "{} must validate", preset.name());
            assert_eq!(p.default_effect, Effect::Deny, "{}", preset.name());
            assert!(!p.revoked, "{} boots armed", preset.name());
            assert_eq!(p.spent_today_wei, U256::ZERO, "{}", preset.name());
            // Byte-stable in JSON (the MCP surface): every preset round-trips unchanged.
            let bytes = serde_json::to_vec(&p).unwrap();
            let back: Policy = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(back, p, "{} must round-trip", preset.name());
        }
    }

    /// `shield-only` is the safest useful agent: shield auto-allowed, send and swap denied.
    #[test]
    fn shield_only_allows_only_shield() {
        let p = shield_only_policy();
        match p.rule_for(IntentKind::Shield) {
            Some(Rule::Shield { approval }) => assert_eq!(*approval, ApprovalMode::Never),
            other => panic!("expected an auto-allow Shield rule, got {other:?}"),
        }
        assert!(
            p.rule_for(IntentKind::Send).is_none(),
            "shield-only must deny Send"
        );
        assert_eq!(
            *p.swap_tokens(),
            Allowlist::DenyAll,
            "shield-only must deny Swap (no Swap rule ⇒ deny-all floor)"
        );
    }

    /// `ask-me-everything` enables shield/send/swap but cards every one.
    #[test]
    fn ask_me_everything_cards_every_action() {
        let p = ask_me_everything_policy();
        match p.rule_for(IntentKind::Shield) {
            Some(Rule::Shield { approval }) => assert_eq!(*approval, ApprovalMode::Always),
            other => panic!("expected an always-card Shield rule, got {other:?}"),
        }
        match p.rule_for(IntentKind::Send) {
            Some(Rule::Send {
                approval,
                per_tx_cap_wei,
                recipients,
            }) => {
                assert_eq!(*approval, ApprovalMode::Always);
                assert_eq!(*per_tx_cap_wei, None);
                assert_eq!(*recipients, Allowlist::Any);
            }
            other => panic!("expected an always-card Send rule, got {other:?}"),
        }
        assert_eq!(
            *p.swap_tokens(),
            Allowlist::Any,
            "swap enabled (always carded regardless)"
        );
    }

    /// `locked` is a frozen rulebook: no rule matches any action (default-deny everywhere), and it
    /// is a chosen posture, NOT the runtime `revoked` STOP brake.
    #[test]
    fn locked_denies_every_action() {
        let p = locked_policy();
        assert!(p.rules.is_empty(), "locked carries no rules");
        assert!(!p.revoked, "locked is a posture, not the runtime STOP");
        for kind in [
            IntentKind::Send,
            IntentKind::Shield,
            IntentKind::Unshield,
            IntentKind::ContractCall,
        ] {
            assert!(
                p.rule_for(kind.clone()).is_none(),
                "locked must deny {kind:?}"
            );
        }
        assert_eq!(
            *p.swap_tokens(),
            Allowlist::DenyAll,
            "locked must deny Swap"
        );
    }

    /// Every preset parses back from its own wire name; hyphen/underscore/case are tolerated; an
    /// unknown name is `None`.
    #[test]
    fn preset_name_round_trips() {
        for preset in Preset::ALL {
            assert_eq!(Preset::from_name(preset.name()), Some(preset));
        }
        assert_eq!(Preset::from_name("Shield_Only"), Some(Preset::ShieldOnly));
        assert_eq!(Preset::from_name("  locked  "), Some(Preset::Locked));
        assert_eq!(Preset::from_name("nonsense"), None);
    }

    /// A preset is a NO-FILE affordance only: it applies when `policy.json` is absent, but an
    /// authored file ALWAYS wins over the preset.
    #[test]
    fn preset_applies_only_when_no_file() {
        // No file + a preset ⇒ the preset installs.
        let locked = load_policy_with_preset(
            Path::new("/nonexistent/deckard/policy.json"),
            Some(Preset::Locked),
        );
        assert!(
            locked.rules.is_empty(),
            "locked preset applies with no file"
        );

        // A valid file present ⇒ the file wins, the preset is ignored.
        let dir =
            std::env::temp_dir().join(format!("deckard-policy-preset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(&path, serde_json::to_vec(&default_policy()).unwrap()).unwrap();
        let loaded = load_policy_with_preset(&path, Some(Preset::Locked));
        assert!(
            loaded.rule_for(IntentKind::Send).is_some(),
            "an authored file wins over the preset"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A malformed file plus even the MOST-permissive preset must STILL fall back to the loud
    /// deny-all: a preset is a no-file affordance and can never revive an unloadable `policy.json`
    /// into something permissive (the `DefaultInvalid` arm structurally ignores the preset). Locks
    /// the invariant against a future refactor that moves preset handling up the loader.
    #[test]
    fn preset_cannot_revive_a_malformed_file() {
        let dir =
            std::env::temp_dir().join(format!("deckard-policy-revive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let p = load_policy_with_preset(&path, Some(Preset::AskMeEverything));
        assert!(
            p.rules.is_empty(),
            "a malformed file must stay deny-all even with a permissive preset"
        );
        assert_eq!(p.daily_cap_wei, U256::ZERO, "deny-all has a zero daily cap");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fallback is CLASSIFIED correctly: absent → quiet `DefaultMissing`, a present file
    /// that has a `version` key but a bad field → the loud `DefaultInvalid` arm.
    #[test]
    fn fallback_distinguishes_missing_from_invalid() {
        assert!(matches!(
            load_policy_outcome(Path::new("/nonexistent/deckard/policy.json")),
            PolicyLoad::DefaultMissing
        ));

        let dir = std::env::temp_dir().join(format!("deckard-policy-loud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(
            &path,
            br#"{"version":1,"default":"deny","daily_cap_wei":"oops","auto_shield_min_wei":"0","rules":[]}"#,
        )
        .unwrap();
        match load_policy_outcome(&path) {
            PolicyLoad::DefaultInvalid(why) => {
                assert!(why.contains("parse failed"), "unexpected cause: {why}")
            }
            other => panic!("expected DefaultInvalid, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
