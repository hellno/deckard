//! Frecency store for the command palette — persisted per-command usage so the most-used
//! actions surface first in the empty palette. App-layer (touches the filesystem); the matcher
//! stays pure in palette_commands. Keyed by stable command id; new commands start unseen.
//!
//! Same on-disk pattern as `settings.rs`: a `serde` map written as JSON into the shared config
//! dir (`deckard_core::config_dir()`, which honors `DECKARD_CONFIG_DIR`). No panics on IO —
//! a missing/unreadable/corrupt file loads as empty; writes are best-effort.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The usage filename inside the shared config dir.
const USAGE_FILE: &str = "palette_usage.json";

/// One week in seconds — the recency half-life: a command unused for a week counts for half.
const HALF_LIFE_SECS: f32 = 604_800.0;

/// Per-command tally: how many times it was run and when it was last run (unix seconds).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct Entry {
    count: u32,
    last_used: u64,
}

/// Persisted frecency store. The on-disk shape is the inner `HashMap` (the path is runtime-only,
/// derived from the config dir, so it is `#[serde(skip)]`).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PaletteUsage {
    /// command id → usage entry.
    entries: HashMap<String, Entry>,
    /// Where this store persists. Not serialized; resolved at load time.
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl PaletteUsage {
    /// Load `<config_dir>/palette_usage.json`. Empty (never panics) on a missing/unreadable/parse
    /// error. Uses the SAME config dir resolution as `settings.rs` so the file lands alongside
    /// `settings.json`, the vault, and the policy.
    pub fn load() -> Self {
        let path = Self::path();
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|contents| serde_json::from_str::<HashMap<String, Entry>>(&contents).ok())
            .unwrap_or_default();
        Self { entries, path }
    }

    /// Record one use of `id`: `count += 1`, `last_used = now`. Persists best-effort
    /// (IO errors are ignored — usage stats are not load-bearing).
    pub fn record(&mut self, id: &str) {
        let entry = self.entries.entry(id.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_used = now_unix_secs();
        self.save();
    }

    /// Frecency weight at `now` (0.0 if unseen): frequency × recency_decay with a one-week
    /// half-life, i.e. `(count as f32) * 0.5_f32.powf(age_secs / 604_800.0)`. `age_secs` is
    /// `saturating_sub` so a clock that ran backwards (or a future `last_used`) reads as age 0
    /// (no decay, no underflow) rather than panicking.
    pub fn frecency(&self, id: &str, now_unix_secs: u64) -> f32 {
        let Some(entry) = self.entries.get(id) else {
            return 0.0;
        };
        let age_secs = now_unix_secs.saturating_sub(entry.last_used) as f32;
        (entry.count as f32) * 0.5_f32.powf(age_secs / HALF_LIFE_SECS)
    }

    /// `<config_dir>/palette_usage.json`, via the same resolver `settings.rs` uses.
    fn path() -> Option<PathBuf> {
        deckard_core::config_dir().map(|dir| dir.join(USAGE_FILE))
    }

    /// Write the entries to disk (best-effort; creates the config dir if needed). Mirrors
    /// `Settings::save` — any IO/serialize failure is silently dropped.
    fn save(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Current unix time in whole seconds, or 0 if the clock is before the epoch (unreachable in
/// practice — keeps the helper infallible so callers never have to handle a `SystemTime` error).
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a store directly from entries with no backing path, so `record`/`save` no-op on disk
    /// and the tests never touch the real config dir.
    fn store(entries: &[(&str, u32, u64)]) -> PaletteUsage {
        let mut map = HashMap::new();
        for &(id, count, last_used) in entries {
            map.insert(id.to_string(), Entry { count, last_used });
        }
        PaletteUsage {
            entries: map,
            path: None,
        }
    }

    #[test]
    fn unseen_id_has_zero_frecency() {
        let usage = store(&[]);
        assert_eq!(usage.frecency("never", 1_000_000), 0.0);
    }

    #[test]
    fn record_increments_count() {
        // No backing path → `record` mutates in memory and the best-effort save no-ops.
        let mut usage = store(&[]);
        usage.record("shield");
        assert_eq!(usage.entries.get("shield").map(|e| e.count), Some(1));
        usage.record("shield");
        assert_eq!(usage.entries.get("shield").map(|e| e.count), Some(2));
    }

    #[test]
    fn older_last_used_decays_below_newer_for_equal_counts() {
        let now = 10 * HALF_LIFE_SECS as u64; // comfortably past several half-lives
                                              // Same count; one used a week ago, one used just now.
        let usage = store(&[("old", 3, now - HALF_LIFE_SECS as u64), ("new", 3, now)]);
        let old = usage.frecency("old", now);
        let new = usage.frecency("new", now);
        assert!(
            old < new,
            "older last_used should decay smaller: {old} !< {new}"
        );
        // The one-week-old entry decayed by exactly the half-life (count 3 → ~1.5).
        assert!((old - 1.5).abs() < 1e-4, "expected ~1.5, got {old}");
        assert!((new - 3.0).abs() < 1e-4, "expected 3.0, got {new}");
    }

    #[test]
    fn missing_or_garbage_file_loads_empty_without_panic() {
        // Parse-failure path: garbage JSON deserializes to the default empty map.
        let entries =
            serde_json::from_str::<HashMap<String, Entry>>("not json at all").unwrap_or_default();
        assert!(entries.is_empty());

        // And a store with no path (the missing-file fallback) is empty and frecency stays 0.0.
        let usage = PaletteUsage::default();
        assert!(usage.entries.is_empty());
        assert_eq!(usage.frecency("anything", now_unix_secs()), 0.0);
    }
}
