//! Where Deckard keeps per-user state on disk. The encrypted keystore (`vault.bin`) and the
//! signer policy (`policy.json`) live in the platform config dir; the GUI app, onboarding,
//! and the signer daemon all resolve the **same** path through here so they never drift.
//!
//! This is a pure resolver — it does not create the directory. The writer (`Vault::write_atomic`)
//! creates the parent as needed; readers treat a missing file as "not set up yet."

use std::path::PathBuf;

use directories::ProjectDirs;

/// The encrypted keystore filename inside [`config_dir`].
pub const VAULT_FILE: &str = "vault.bin";
/// The signer policy filename inside [`config_dir`].
pub const POLICY_FILE: &str = "policy.json";

/// The platform config dir: `~/Library/Application Support/com.deckard.Deckard` on macOS,
/// `$XDG_CONFIG_HOME/deckard` (or `~/.config/deckard`) on Linux. `None` only if the OS has
/// no home directory at all.
pub fn config_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("com", "deckard", "Deckard")?;
    Some(dirs.config_dir().to_path_buf())
}

/// The encrypted keystore path (`<config_dir>/vault.bin`).
pub fn vault_path() -> Option<PathBuf> {
    Some(config_dir()?.join(VAULT_FILE))
}

/// The signer policy path (`<config_dir>/policy.json`).
pub fn policy_path() -> Option<PathBuf> {
    Some(config_dir()?.join(POLICY_FILE))
}
