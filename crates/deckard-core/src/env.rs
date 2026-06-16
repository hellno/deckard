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

/// Whether the demo swap stub is engaged (`DECKARD_DEMO_SWAP_STUB`). **Default OFF — production
/// never stubs.** A real CoW order can't be accepted+open from a local Sepolia fork (the live
/// orderbook validates real-Sepolia balances), so `just demo` / `install --demo` turn this ON to
/// route quote/submit through an HONEST in-fork stub instead. This is a pure ON/OFF FLAG: the fork
/// RPC the stub credits balances on comes from [`demo_swap_fill_rpc`] (`DECKARD_RPC_URL`), NOT from
/// this value — the flag and the URL are deliberately separate knobs. Recognizes `1` / `true` /
/// `yes` / `on` (case-insensitive, trimmed); anything else (including unset/empty) leaves the real
/// orderbook in place.
pub fn demo_swap_stub() -> bool {
    demo_swap_stub_from(std::env::var("DECKARD_DEMO_SWAP_STUB").ok().as_deref())
}

/// Pure core of [`demo_swap_stub`] — only an explicit truthy spelling enables the stub, so the
/// production default (real orderbook) is fail-safe against typos and stray values.
pub(crate) fn demo_swap_stub_from(value: Option<&str>) -> bool {
    matches!(
        value.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// The fork RPC URL the demo swap stub credits simulated buy-token balances on, sourced from the
/// standard `DECKARD_RPC_URL` (in demo mode that is the local anvil fork at `http://127.0.0.1:8545`,
/// which serves the `anvil_setStorageAt` cheatcode). Only consulted when [`demo_swap_stub`] is on; a
/// blank/unset value is `None`, in which case the stub returns a synthetic uid with the buy balance
/// left UN-credited (honest — never a fabricated fill).
pub fn demo_swap_fill_rpc() -> Option<String> {
    demo_swap_fill_rpc_from(std::env::var("DECKARD_RPC_URL").ok().as_deref())
}

/// Pure core of [`demo_swap_fill_rpc`] — trims and treats blank/unset as `None`.
pub(crate) fn demo_swap_fill_rpc_from(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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

    #[test]
    fn demo_swap_stub_defaults_off_and_parses_truthy_values() {
        // Unset / blank / falsey / non-truthy → no stub (the real CoW orderbook is used). A bare
        // URL is NOT truthy: the flag and the fill URL are separate knobs now.
        for off in [
            None,
            Some(""),
            Some("   "),
            Some("0"),
            Some("http://127.0.0.1:8545"),
        ] {
            assert!(
                !demo_swap_stub_from(off),
                "{off:?} should leave the real orderbook in place"
            );
        }
        // Only the documented truthy spellings, case/whitespace-insensitive, enable it.
        for on in ["1", "true", "yes", "on", " ON ", "True", "Yes"] {
            assert!(
                demo_swap_stub_from(Some(on)),
                "{on:?} should enable the stub"
            );
        }
    }

    #[test]
    fn demo_swap_fill_rpc_trims_and_treats_blank_as_unset() {
        for none in [None, Some(""), Some("   ")] {
            assert_eq!(
                demo_swap_fill_rpc_from(none),
                None,
                "{none:?} → no fill RPC"
            );
        }
        for url in ["http://127.0.0.1:8545", "  http://127.0.0.1:8545 "] {
            assert_eq!(
                demo_swap_fill_rpc_from(Some(url)),
                Some("http://127.0.0.1:8545".to_string())
            );
        }
    }
}
