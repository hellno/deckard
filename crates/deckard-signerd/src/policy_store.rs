//! Loading the signer [`Policy`] and tracking the daily spend.
//!
//! The policy is read from `policy.json` in the config dir; a **sane default** is used if the
//! file is absent or malformed (fail-safe: a tight cap, no allowlist, approval-over-cap).
//! `spent_today_wei` is **in-memory only**, rolls over at UTC midnight, and resets on daemon
//! restart — cross-restart persistence is a documented v1 limitation / fast-follow. There is
//! no `SetPolicy` mutation API yet.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy_primitives::U256;

use deckard_contract::{ApprovalMode, Policy};

/// Default per-tx cap: 0.05 ETH.
pub const DEFAULT_PER_TX_CAP_WEI: u128 = 50_000_000_000_000_000;
/// Default rolling daily cap: 0.2 ETH.
pub const DEFAULT_DAILY_CAP_WEI: u128 = 200_000_000_000_000_000;
/// Default auto-shield threshold: 0.01 ETH.
pub const DEFAULT_AUTO_SHIELD_MIN_WEI: u128 = 10_000_000_000_000_000;

/// The fail-safe default policy used when `policy.json` is absent or unreadable.
pub fn default_policy() -> Policy {
    Policy {
        per_tx_cap_wei: U256::from(DEFAULT_PER_TX_CAP_WEI),
        daily_cap_wei: U256::from(DEFAULT_DAILY_CAP_WEI),
        spent_today_wei: U256::ZERO,
        allow_to: vec![], // empty = any recipient; the caps still apply
        auto_shield_min_wei: U256::from(DEFAULT_AUTO_SHIELD_MIN_WEI),
        require_approval: ApprovalMode::OverCap,
        revoked: false,
        // Empty = any token allowed; the daemon may later populate from `tokens_for(chain_id)`.
        allow_swap_tokens: vec![],
    }
}

/// How a policy load resolved — split out from [`load_policy`] so the fallback cases are
/// unit-testable (the logging wrapper can't be asserted on).
#[derive(Debug)]
pub enum PolicyLoad {
    /// `policy.json` parsed cleanly.
    Loaded(Policy),
    /// No file at the path — a normal first run; the default applies quietly.
    DefaultMissing,
    /// The file EXISTS but didn't load (unreadable or malformed). The default applies, but
    /// this is loud: a typo'd policy would otherwise silently run the default
    /// any-recipient policy.
    DefaultInvalid(String),
}

/// Resolve a policy load without logging. `spent_today_wei` and `revoked` are forced to
/// their fresh-start values regardless of what the file says: spend tracking is in-memory,
/// and a daemon boots *armed* (the brake is a live STOP, not a persisted flag).
pub fn load_policy_outcome(path: &Path) -> PolicyLoad {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PolicyLoad::DefaultMissing,
        Err(e) => return PolicyLoad::DefaultInvalid(format!("read failed: {e}")),
    };
    match serde_json::from_slice::<Policy>(&bytes) {
        Ok(mut p) => {
            p.spent_today_wei = U256::ZERO;
            p.revoked = false;
            PolicyLoad::Loaded(p)
        }
        Err(e) => PolicyLoad::DefaultInvalid(format!("parse failed: {e}")),
    }
}

/// Load the policy from `path`, falling back to [`default_policy`] on any problem — and
/// falling back LOUDLY when a file exists but doesn't load. A silent fallback would let a
/// typo'd `policy.json` run the default (any-recipient) policy with nobody the wiser.
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
                 RUNNING THE BUILT-IN DEFAULT POLICY instead (per-tx 0.05 ETH, daily 0.2 ETH, \
                 any recipient, approval over cap). Fix the file and restart the daemon.",
                path.display()
            );
            default_policy()
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

    #[test]
    fn absent_file_yields_default() {
        let p = load_policy(Path::new("/nonexistent/deckard/policy.json"));
        assert_eq!(p.per_tx_cap_wei, U256::from(DEFAULT_PER_TX_CAP_WEI));
        assert_eq!(p.daily_cap_wei, U256::from(DEFAULT_DAILY_CAP_WEI));
        assert!(!p.revoked);
        assert!(p.allow_to.is_empty());
    }

    #[test]
    fn load_resets_spent_and_revoked() {
        let dir = std::env::temp_dir().join(format!("deckard-policy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        // A file that (maliciously or staleley) claims spent + revoked.
        let on_disk = Policy {
            spent_today_wei: U256::from(999u64),
            revoked: true,
            ..default_policy()
        };
        std::fs::write(&path, serde_json::to_vec(&on_disk).unwrap()).unwrap();

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
    fn malformed_file_yields_default() {
        let dir = std::env::temp_dir().join(format!("deckard-policy-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        assert_eq!(
            load_policy(&path).per_tx_cap_wei,
            U256::from(DEFAULT_PER_TX_CAP_WEI)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The fallback is CLASSIFIED correctly: absent → quiet default, present-but-broken →
    /// the loud `DefaultInvalid` arm (the one `load_policy` shouts about).
    #[test]
    fn fallback_distinguishes_missing_from_invalid() {
        assert!(matches!(
            load_policy_outcome(Path::new("/nonexistent/deckard/policy.json")),
            PolicyLoad::DefaultMissing
        ));

        let dir = std::env::temp_dir().join(format!("deckard-policy-loud-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");
        std::fs::write(&path, b"{ \"per_tx_cap_wei\": tyop }").unwrap();
        match load_policy_outcome(&path) {
            PolicyLoad::DefaultInvalid(why) => {
                assert!(why.contains("parse failed"), "unexpected cause: {why}")
            }
            other => panic!("expected DefaultInvalid, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
