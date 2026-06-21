//! Where Deckard keeps per-user state on disk. The encrypted keystore (`vault.bin`) and the
//! signer policy (`policy.json`) live in the platform config dir; the GUI app, onboarding,
//! and the signer daemon all resolve the **same** path through here so they never drift.
//!
//! This is a pure resolver — it does not create the directory. The writer (`Vault::write_atomic`)
//! creates the parent as needed; readers treat a missing file as "not set up yet."

use std::ffi::OsString;
use std::path::PathBuf;

use directories::ProjectDirs;

/// The encrypted keystore filename inside [`config_dir`].
pub const VAULT_FILE: &str = "vault.bin";
/// The signer policy filename inside [`config_dir`].
pub const POLICY_FILE: &str = "policy.json";
/// The durable daily-spend counter filename inside [`config_dir`] (issue #108). Single-writer:
/// only the signer daemon writes it; it survives restart so the daily cap isn't zeroed on every
/// crash/OOM/update.
pub const SPEND_FILE: &str = "spend.json";

/// The config dir every Deckard process resolves through, so the GUI app, onboarding, the
/// signer daemon, and the demo all agree on where `vault.bin` / `policy.json` / `settings.json`
/// live.
///
/// Resolution: the `DECKARD_CONFIG_DIR` env override first (so `just demo` can isolate the
/// throwaway vault/settings/policy in one directory that never bleeds into the everyday
/// keystore), else the platform dir — `~/Library/Application Support/com.deckard.Deckard` on
/// macOS, `$XDG_CONFIG_HOME/deckard` (or `~/.config/deckard`) on Linux. `None` only when no
/// override is set AND the OS has no home directory at all. Previously only the daemon honored
/// the override, which split the app's vault from the daemon's (the `NoVault` demo bug).
pub fn config_dir() -> Option<PathBuf> {
    let override_dir = override_dir_from(std::env::var_os("DECKARD_CONFIG_DIR"));
    let platform =
        || ProjectDirs::from("com", "deckard", "Deckard").map(|d| d.config_dir().to_path_buf());
    resolve_config_dir(override_dir, platform)
}

/// Map the raw `DECKARD_CONFIG_DIR` value to an override path. An EMPTY value (`export
/// DECKARD_CONFIG_DIR=`) is treated as **unset**: `PathBuf::from("")` would join to a
/// CWD-relative `vault.bin`, silently relocating the keystore — the exact silent split this
/// resolver exists to prevent. Pure so the empty-handling is unit-testable without env mutation.
fn override_dir_from(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|d| !d.is_empty()).map(PathBuf::from)
}

/// Pure resolver for [`config_dir`]: the explicit `DECKARD_CONFIG_DIR` override wins, else the
/// platform fallback is computed lazily. Split out so the precedence is unit-testable without
/// mutating process-global env.
fn resolve_config_dir(
    override_dir: Option<PathBuf>,
    platform: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    override_dir.or_else(platform)
}

/// The encrypted keystore path (`<config_dir>/vault.bin`).
pub fn vault_path() -> Option<PathBuf> {
    Some(config_dir()?.join(VAULT_FILE))
}

/// The signer policy path (`<config_dir>/policy.json`).
pub fn policy_path() -> Option<PathBuf> {
    Some(config_dir()?.join(POLICY_FILE))
}

/// The durable daily-spend counter path (`<config_dir>/spend.json`).
pub fn spend_path() -> Option<PathBuf> {
    Some(config_dir()?.join(SPEND_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_wins_over_the_platform_dir() {
        let forced = PathBuf::from("/tmp/deckard-demo");
        // The platform fallback must NOT be consulted when the override is present.
        let resolved = resolve_config_dir(Some(forced.clone()), || {
            panic!("platform fallback must not run when DECKARD_CONFIG_DIR is set")
        });
        assert_eq!(resolved, Some(forced));
    }

    #[test]
    fn empty_override_is_treated_as_unset() {
        // A set-but-empty `DECKARD_CONFIG_DIR=` must NOT become `PathBuf::from("")` (which would
        // resolve the vault CWD-relative) — it falls through to the platform dir.
        assert_eq!(override_dir_from(Some(OsString::new())), None);
        assert_eq!(override_dir_from(None), None);
        assert_eq!(
            override_dir_from(Some(OsString::from("/tmp/deckard-demo"))),
            Some(PathBuf::from("/tmp/deckard-demo"))
        );
    }

    #[test]
    fn falls_back_to_the_platform_dir_when_unset() {
        let platform = PathBuf::from("/home/op/.config/deckard");
        assert_eq!(
            resolve_config_dir(None, || Some(platform.clone())),
            Some(platform)
        );
        // No override and no home directory → None (the honest "can't resolve" signal).
        assert_eq!(resolve_config_dir(None, || None), None);
    }
}
