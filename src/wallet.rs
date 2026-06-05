//! Vault file location + legacy-key migration helpers.
//!
//! The keypair itself now lives encrypted in `deckard-core`'s keystore (`vault.bin`);
//! this module only resolves where that file lives and detects the legacy plaintext
//! `wallet.key` from the pre-keystore build so onboarding can migrate it.

use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;

/// The platform config dir (`~/Library/Application Support/com.deckard.Deckard` on macOS).
fn config_dir() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("com", "deckard", "Deckard")?;
    let dir = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Where the encrypted keystore lives.
pub fn vault_path() -> Option<PathBuf> {
    Some(config_dir()?.join("vault.bin"))
}

/// The legacy plaintext key from the pre-keystore build (raw 32-byte hex).
fn legacy_key_path() -> Option<PathBuf> {
    Some(config_dir()?.join("wallet.key"))
}

/// True once an encrypted vault exists (the app is past first-run).
pub fn vault_exists() -> bool {
    vault_path().map(|p| p.exists()).unwrap_or(false)
}

/// The legacy plaintext key hex, if a pre-keystore `wallet.key` is present.
pub fn legacy_key_hex() -> Option<String> {
    let p = legacy_key_path()?;
    // A raw key is ~66 bytes of hex; refuse to slurp anything larger.
    if fs::metadata(&p).ok()?.len() > 256 {
        return None;
    }
    let s = fs::read_to_string(&p).ok()?;
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Best-effort delete of the legacy plaintext key after it has been migrated into the
/// encrypted vault. (On APFS the bytes may persist in snapshots — onboarding warns the
/// user to move funds to a freshly created wallet.)
pub fn delete_legacy_key() {
    if let Some(p) = legacy_key_path() {
        let _ = fs::remove_file(p);
    }
}
