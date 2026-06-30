//! Loading the signer [`Policy`] and tracking the daily spend.
//!
//! The policy is read from `policy.json` in the config dir. There are **two distinct
//! fallbacks** (ADR 0005 §5):
//!
//! * **No file** ⇒ the FRIENDLY built-in [`default_policy`] (a sane first-run policy:
//!   shield freely, send with an always-card, swap any token, under a 0.2 ETH daily cap).
//! * **A file that exists but does NOT load** (malformed JSON, a legacy v0 flat policy with
//!   no `version` key, or a v1 file that fails [`Policy::validate`]) ⇒ the MOST-RESTRICTIVE
//!   [`deny_all_policy`] (every action denied, `NO_RULE`). This is loud: a typo'd policy must
//!   never silently degrade into a permissive default.
//!
//! `spent_today_wei` is **in-memory only**, rolls over at UTC midnight, and resets on daemon
//! restart — cross-restart persistence is owned by the durable `SpendStore` (#108). There is
//! no `SetPolicy` mutation API yet.

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

/// Load the policy from `path` with the two fallbacks: a missing file quietly uses the
/// FRIENDLY [`default_policy`]; a file that exists but did not load falls back LOUDLY to the
/// MOST-RESTRICTIVE [`deny_all_policy`] (every action denied) — a silent fallback would let a
/// typo'd `policy.json` run a permissive default with nobody the wiser.
pub fn load_policy(path: &Path) -> Policy {
    match load_policy_outcome(path) {
        PolicyLoad::Loaded(p) => p,
        PolicyLoad::DefaultMissing => {
            eprintln!(
                "signerd: no policy file at {} — using the built-in default policy",
                path.display()
            );
            default_policy()
        }
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
