//! Runtime environment knobs for **local-fork / demo mode**.
//!
//! `just demo` points the app + daemon at a local anvil fork of Sepolia (the
//! `shield_e2e` configuration). Two things have to change for that fork to behave —
//! and both must be switchable **at runtime**, without rebuilding the heavy GPUI app
//! with different cargo features:
//!
//! - **verified reads off** — the embedded Helios light client is mainnet-only and
//!   re-bootstraps on every failed `Balance`; against a Sepolia fork it never verifies
//!   and stalls reads for seconds-to-minutes. The demo runs with `verified-reads`
//!   compiled IN but disabled by env, so reads fall back to the raw fork RPC, tagged
//!   `Unsynced` (honest non-verification, never a fabricated `Verified`).
//! - **shielded-sync pinned to the fork block** — the live Subsquid index only covers
//!   the real chain, so on a fork it must stop at the fork block and let the RpcSyncer
//!   pick up the fork-local shield event from there (mirrors `shield_e2e`'s
//!   `.with_latest_block(FORK_BLOCK)`).
//!
//! Each knob is a `DECKARD_*` env var with a safe **production default**; the parsing is
//! a pure, unit-tested function and a thin wrapper reads the real environment.

/// Whether verified (Helios) reads are enabled. **Default ON.** The demo turns them OFF
/// with `DECKARD_VERIFIED_READS=0` because embedded Helios is mainnet-only and would stall
/// a `Balance` read against a Sepolia fork. Recognizes `0` / `false` / `off` / `no`
/// (case-insensitive, trimmed) as off; anything else (including an empty value) leaves
/// verification on.
pub fn verified_reads_enabled() -> bool {
    verified_reads_enabled_from(std::env::var("DECKARD_VERIFIED_READS").ok().as_deref())
}

/// Pure core of [`verified_reads_enabled`] — takes the raw env value so it is testable
/// without touching process-global state.
pub(crate) fn verified_reads_enabled_from(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(s) => !matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
    }
}

/// The block to pin the shielded-balance `SubsquidSyncer` at, if any
/// (`DECKARD_DEMO_FORK_BLOCK`). **Unset (the production default) → unpinned live sync.**
/// `just demo` sets it to the `shield_e2e` fork block (10_822_990). A non-numeric value is
/// ignored (treated as unset) rather than crashing the sync worker.
pub fn demo_fork_block() -> Option<u64> {
    demo_fork_block_from(std::env::var("DECKARD_DEMO_FORK_BLOCK").ok().as_deref())
}

/// Pure core of [`demo_fork_block`] — parses the raw env value.
pub(crate) fn demo_fork_block_from(value: Option<&str>) -> Option<u64> {
    value.and_then(|s| s.trim().parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_reads_defaults_on_and_parses_falsey_values() {
        // Unset / empty / unknown → verification stays ON (fail safe).
        assert!(verified_reads_enabled_from(None));
        assert!(verified_reads_enabled_from(Some("")));
        assert!(verified_reads_enabled_from(Some("1")));
        assert!(verified_reads_enabled_from(Some("true")));
        assert!(verified_reads_enabled_from(Some("yolo")));
        // The documented off spellings, case/whitespace-insensitive.
        for off in ["0", "false", "off", "no", " OFF ", "False", "No"] {
            assert!(
                !verified_reads_enabled_from(Some(off)),
                "{off:?} should disable verified reads"
            );
        }
    }

    #[test]
    fn demo_fork_block_parses_only_a_valid_u64() {
        assert_eq!(demo_fork_block_from(None), None);
        assert_eq!(demo_fork_block_from(Some("")), None);
        assert_eq!(demo_fork_block_from(Some("not-a-number")), None);
        assert_eq!(demo_fork_block_from(Some("10822990")), Some(10_822_990));
        assert_eq!(demo_fork_block_from(Some("  10822990 ")), Some(10_822_990));
    }
}
