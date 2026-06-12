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

/// Whether an env override forces the macOS screen-capture block **off** (screen capture
/// ALLOWED), regardless of the persisted `capture_block` privacy setting. **Default OFF** —
/// the trust feature is honored unless an operator opts in with
/// `DECKARD_ALLOW_SCREEN_CAPTURE=1`. The one intended use is an automated agent recording the
/// demo GIF: it can launch the app with capture guaranteed un-blocked instead of having to
/// reach into the settings UI first. Recognizes `1` / `true` / `yes` / `on` (case-insensitive,
/// trimmed); anything else (including unset/empty) leaves the block under the setting's control.
pub fn screen_capture_allowed() -> bool {
    screen_capture_allowed_from(
        std::env::var("DECKARD_ALLOW_SCREEN_CAPTURE")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`screen_capture_allowed`] — takes the raw env value so it is testable without
/// touching process-global state. Only an explicit truthy spelling enables the override, so the
/// trust default (block honored per setting) is fail-safe against typos and stray values.
pub(crate) fn screen_capture_allowed_from(value: Option<&str>) -> bool {
    matches!(
        value.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
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

    #[test]
    fn screen_capture_allowed_defaults_off_and_parses_truthy_values() {
        // Unset / empty / unknown / falsey → the capture block stays under the setting's
        // control (override OFF). The trust default is fail-safe against typos.
        assert!(!screen_capture_allowed_from(None));
        assert!(!screen_capture_allowed_from(Some("")));
        assert!(!screen_capture_allowed_from(Some("0")));
        assert!(!screen_capture_allowed_from(Some("false")));
        assert!(!screen_capture_allowed_from(Some("nope")));
        // Only the documented truthy spellings, case/whitespace-insensitive, enable it.
        for on in ["1", "true", "yes", "on", " ON ", "True", "Yes"] {
            assert!(
                screen_capture_allowed_from(Some(on)),
                "{on:?} should allow screen capture"
            );
        }
    }
}
