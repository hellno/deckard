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
    }
}

/// Load the policy from `path`, falling back to [`default_policy`] on any problem.
///
/// `spent_today_wei` and `revoked` are forced to their fresh-start values regardless of what
/// the file says: spend tracking is in-memory, and a daemon boots *armed* (the brake is a
/// live STOP, not a persisted flag).
pub fn load_policy(path: &Path) -> Policy {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return default_policy(),
    };
    match serde_json::from_slice::<Policy>(&bytes) {
        Ok(mut p) => {
            p.spent_today_wei = U256::ZERO;
            p.revoked = false;
            p
        }
        Err(_) => default_policy(),
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
}
