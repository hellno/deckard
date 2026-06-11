//! Settings — typed preferences persisted to the platform config directory.
//!
//! This is the **mainstream, dependency-light Rust pattern**: a `serde` struct
//! written as JSON into the OS config dir (`directories` finds it — on macOS
//! that's `~/Library/Application Support/<id>/settings.json`). No database, no
//! framework. See `docs/LEARNINGS.md` for how this compares to `confy` and to
//! Zed's layered settings system.

use std::path::PathBuf;

use gpui_component::ThemeMode;
use serde::{Deserialize, Serialize};

/// The settings filename inside the shared config dir.
const SETTINGS_FILE: &str = "settings.json";

/// The chain the daemon signs for when neither env nor settings pick one (mainnet).
pub const DEFAULT_CHAIN_ID: u64 = 1;

/// Persisted theme preference. We keep our own enum (rather than reusing
/// gpui-component's `ThemeMode`) so the on-disk format is ours to control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeModePref {
    #[default]
    Dark,
    Light,
}

impl ThemeModePref {
    pub fn to_gpui(self) -> ThemeMode {
        match self {
            ThemeModePref::Dark => ThemeMode::Dark,
            ThemeModePref::Light => ThemeMode::Light,
        }
    }
}

/// Everything the app remembers between launches. Add fields freely — the
/// `#[serde(default)]` makes older config files forward-compatible.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme_mode: ThemeModePref,
    pub display_name: String,
    pub launch_minimized: bool,
    /// Custom Ethereum RPC URL (bring-your-own-RPC). Empty = the bundled default.
    /// The trustless default (a local Helios light client) supersedes this later.
    pub rpc_url: String,
    /// A read-only address or ENS name to view instead of the active wallet. Empty =
    /// show the wallet. Lets an operator watch any address (e.g. `vitalik.eth`).
    pub watch_address: String,
    /// Privacy mask: replace every money figure with fixed-length bullets. Unlike the
    /// seed reveal (momentary, default-hidden), the mask is **persisted-once-on** — a
    /// stated preference that survives relaunch. Default OFF.
    pub mask_balances: bool,
    /// macOS screen-capture block (NSWindow sharingType = none) tied to the mask. Opt-in,
    /// **default OFF** — for a demo recording it stays off, or the capture itself is
    /// blocked. Only takes effect in a `--features tray` macOS build (reuses that dep).
    pub capture_block: bool,
    /// The chain the daemon signs for, when no `DECKARD_CHAIN_ID` env override is set. `None`
    /// (the default) = mainnet ([`DEFAULT_CHAIN_ID`]). There's no settings UI for this yet —
    /// it's the middle tier of `env > settings > default` so multi-chain config has a home —
    /// but `just demo` drives the chain via the env override, not this field.
    pub chain_id: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeModePref::Dark,
            display_name: String::new(),
            launch_minimized: false,
            rpc_url: String::new(),
            watch_address: String::new(),
            mask_balances: false,
            capture_block: false,
            chain_id: None,
        }
    }
}

impl Settings {
    /// The effective RPC URL: the `DECKARD_RPC_URL` env override > the user's custom setting >
    /// the bundled default. The env tier lets `just demo` (and the supervised daemon, which
    /// reads the same var) point every process at the local fork without editing settings.
    pub fn effective_rpc(&self) -> String {
        resolve_rpc(
            std::env::var("DECKARD_RPC_URL").ok().as_deref(),
            &self.rpc_url,
        )
    }

    /// The effective chain id the daemon signs for: the `DECKARD_CHAIN_ID` env override >
    /// the persisted [`Settings::chain_id`] > [`DEFAULT_CHAIN_ID`]. Threaded to the signer
    /// launch, the shield builder, and the Railgun sync so they never disagree (the supervisor
    /// passes this resolved id to the daemon instead of clobbering the env with a hardcoded 1).
    pub fn effective_chain_id(&self) -> u64 {
        resolve_chain_id(
            std::env::var("DECKARD_CHAIN_ID").ok().as_deref(),
            self.chain_id,
        )
    }
}

/// Pure resolver for [`Settings::effective_rpc`]: env override (if non-empty) > the setting
/// (if non-empty) > the bundled default. Split out so the precedence is unit-testable.
pub(crate) fn resolve_rpc(env: Option<&str>, setting: &str) -> String {
    if let Some(url) = env.map(str::trim).filter(|s| !s.is_empty()) {
        return url.to_string();
    }
    let setting = setting.trim();
    if setting.is_empty() {
        deckard_core::DEFAULT_RPC.to_string()
    } else {
        setting.to_string()
    }
}

/// Pure resolver for [`Settings::effective_chain_id`]: a parseable env override wins, else the
/// persisted setting, else [`DEFAULT_CHAIN_ID`]. Only an unset-or-empty env value falls through;
/// a non-empty value that is NOT a valid `u64` is a loud startup failure (matching the wording of
/// `deckard-signerd` and `deckard-mcp`, which both hard-error on the same input). Silently
/// ignoring a typo'd override (e.g. `DECKARD_CHAIN_ID=sepolia`) would resolve toward MAINNET —
/// the wrong direction — and the supervisor pins the daemon's env to this resolved id.
///
/// Fatal-at-startup boundary: an unparsable explicit override is operator misconfiguration the
/// app cannot recover from, so a clear panic is correct here (the app crate permits this at
/// genuinely-unrecoverable startup boundaries — see `deckard-core/src/eth.rs`).
#[allow(clippy::expect_used)]
pub(crate) fn resolve_chain_id(env: Option<&str>, setting: Option<u64>) -> u64 {
    if let Some(raw) = env.map(str::trim).filter(|s| !s.is_empty()) {
        return raw
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("DECKARD_CHAIN_ID must be a u64, got {raw:?}"));
    }
    setting.unwrap_or(DEFAULT_CHAIN_ID)
}

/// True when the app is pointed at a **local development fork** rather than a public network:
/// the resolved RPC is a loopback address AND the chain isn't mainnet. Both conditions matter —
/// a local mainnet archive node (loopback, chain 1) is still real mainnet data, and a remote
/// testnet (public host, chain ≠ 1) isn't a fork. Drives the status-strip "DEMO FORK — not
/// mainnet" caution (DESIGN: an amber alert icon + risk text inline, never a colored slab).
pub(crate) fn is_fork_mode(rpc_url: &str, chain_id: u64) -> bool {
    chain_id != DEFAULT_CHAIN_ID && rpc_is_loopback(rpc_url)
}

/// Whether an RPC URL points at the local loopback interface (a local anvil fork lives at
/// `127.0.0.1` / `localhost`). Parses `scheme://[user@]host[:port]/…` defensively: anything
/// unrecognized is treated as non-loopback (fail toward "this is a real network").
pub(crate) fn rpc_is_loopback(url: &str) -> bool {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    // IPv6 literal `[::1]` / `[::1]:port` → take what's inside the brackets; otherwise
    // `host[:port]` → drop a single trailing `:port` (IPv4 / hostnames have at most one colon).
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    // Parse as an IP so the whole 127.0.0.0/8 + ::1 range counts but a *hostname* like
    // `127.0.0.1.evil.com` (which resolves wherever an attacker wants) never matches.
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

impl Settings {
    fn path() -> Option<PathBuf> {
        // Route through deckard-core so settings.json lands in the SAME dir as the vault +
        // policy — and honors `DECKARD_CONFIG_DIR`, so `just demo` keeps every file in one
        // isolated directory (the default resolves to the same platform path as before, so
        // existing users' settings are found unchanged).
        deckard_core::config_dir().map(|dir| dir.join(SETTINGS_FILE))
    }

    /// Human-readable path, shown in the settings UI so users know where prefs live.
    pub fn config_path_display() -> String {
        Self::path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unavailable>".to_string())
    }

    /// Load from disk, falling back to defaults on a missing/corrupt file.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write to disk (best-effort; creates the config dir if needed).
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_resolution_prefers_env_then_setting_then_default() {
        let default = deckard_core::DEFAULT_RPC;
        // Env override wins over everything, even a custom setting.
        assert_eq!(
            resolve_rpc(Some("http://127.0.0.1:8545"), "https://my.rpc"),
            "http://127.0.0.1:8545"
        );
        // Blank/whitespace env is ignored → the setting is used.
        assert_eq!(resolve_rpc(Some("   "), "https://my.rpc"), "https://my.rpc");
        assert_eq!(resolve_rpc(None, "https://my.rpc"), "https://my.rpc");
        // Neither env nor setting → the bundled default.
        assert_eq!(resolve_rpc(None, ""), default);
        assert_eq!(resolve_rpc(Some(""), "  "), default);
        // Trimmed.
        assert_eq!(resolve_rpc(Some(" https://x "), ""), "https://x");
    }

    #[test]
    fn chain_id_resolution_prefers_env_then_setting_then_default() {
        assert_eq!(resolve_chain_id(Some("11155111"), Some(5)), 11_155_111);
        assert_eq!(resolve_chain_id(Some(" 11155111 "), None), 11_155_111);
        // Unset OR empty/whitespace env falls through to setting/default.
        assert_eq!(resolve_chain_id(Some(""), Some(5)), 5);
        assert_eq!(resolve_chain_id(Some("   "), None), DEFAULT_CHAIN_ID);
        assert_eq!(resolve_chain_id(None, Some(5)), 5);
        assert_eq!(resolve_chain_id(None, None), DEFAULT_CHAIN_ID);
    }

    #[test]
    #[should_panic(expected = "DECKARD_CHAIN_ID must be a u64")]
    fn chain_id_resolution_rejects_unparsable_env_override() {
        // A typo'd demo env like `DECKARD_CHAIN_ID=sepolia` must be a loud startup failure, not a
        // silent fall-through toward mainnet (parity with signerd + mcp, which both hard-error).
        let _ = resolve_chain_id(Some("sepolia"), Some(5));
    }

    #[test]
    fn loopback_detection_covers_anvil_urls() {
        for local in [
            "http://127.0.0.1:8545",
            "http://localhost:8545",
            "http://127.0.0.1",
            "https://localhost/",
            "http://[::1]:8545",
            "http://user@127.0.0.1:8545/path",
        ] {
            assert!(rpc_is_loopback(local), "{local} should be loopback");
        }
        for remote in [
            "https://ethereum-rpc.publicnode.com",
            "https://mainnet.infura.io/v3/KEY",
            "https://127.0.0.1.evil.com", // host is 127.0.0.1.evil.com, not loopback
        ] {
            assert!(!rpc_is_loopback(remote), "{remote} should NOT be loopback");
        }
    }

    #[test]
    fn fork_mode_needs_both_loopback_and_non_mainnet() {
        // The demo: local fork on Sepolia → fork mode.
        assert!(is_fork_mode("http://127.0.0.1:8545", 11_155_111));
        // A local mainnet archive node is still real mainnet data → NOT a fork.
        assert!(!is_fork_mode("http://127.0.0.1:8545", DEFAULT_CHAIN_ID));
        // A remote testnet isn't a local fork.
        assert!(!is_fork_mode("https://sepolia.example.com", 11_155_111));
        // Plain mainnet over a public RPC.
        assert!(!is_fork_mode(deckard_core::DEFAULT_RPC, DEFAULT_CHAIN_ID));
    }
}
