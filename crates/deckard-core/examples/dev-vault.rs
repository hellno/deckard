//! Dev helper — seal a THROWAWAY vault with a FRESH random wallet.
//!
//! Same idea as `qa-vault`, but instead of anvil's well-known dev mnemonic (whose
//! account 0 is swept by bots the instant it gets ETH on a public testnet), this
//! generates a brand-new HD wallet via the production create path (`Vault::create`,
//! OsRng entropy). Use it for a real-Sepolia clicky run where you faucet your own
//! testnet ETH to a unique address that's actually yours.
//!
//!   DECKARD_CONFIG_DIR=/tmp/deckard-montreal-sepolia \
//!     cargo run -q -p deckard-core --example dev-vault
//!
//! Prints ONLY the derived address + the (known, throwaway) passphrase — the backup
//! phrase is generated, sealed, and immediately dropped (zeroized); it is NEVER logged.
//! Fast Argon2 params, baked into the vault header, so unlock is near-instant.
//!
//! WARNING: throwaway wallet. Lives only under `examples/`, so it is never linked into
//! the shipped `deckard` binary.

use std::path::PathBuf;

use deckard_core::{config::VAULT_FILE, KdfParams, Vault, WordCount};

/// Fixed dev passphrase (>= 8 chars). Override with `DECKARD_DEV_PASS`.
const DEFAULT_PASS: &str = "deckard-qa";

fn main() {
    let pass = match std::env::var("DECKARD_DEV_PASS") {
        Ok(p) if !p.is_empty() => p,
        _ => DEFAULT_PASS.to_string(),
    };

    // Resolve the target config dir. NEVER fall back to the real platform keystore.
    let dir = match std::env::var_os("DECKARD_CONFIG_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from("/tmp/deckard-dev"),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("dev-vault: cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }

    // Fast Argon2 (8 MiB / t=1 / p=1) — the floor `validate()` allows. Baked into the
    // vault header, so unlock is fast in both the app and the daemon.
    let kdf = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };

    // Generate a fresh wallet. The returned phrase is dropped immediately (Zeroizing).
    let vault = match Vault::create(&pass, WordCount::Twelve, kdf) {
        Ok((v, _phrase)) => v,
        Err(e) => {
            eprintln!("dev-vault: create failed: {e}");
            std::process::exit(1);
        }
    };

    // Derive the address for the log (we print ONLY the address — never seed/key material).
    let addr = match vault.unlock(&pass).and_then(|u| u.primary_address()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("dev-vault: derive address failed: {e}");
            std::process::exit(1);
        }
    };

    let path = dir.join(VAULT_FILE);
    if let Err(e) = vault.write_atomic(&path) {
        eprintln!("dev-vault: write {} failed: {e}", path.display());
        std::process::exit(1);
    }

    println!("dev-vault: sealed a throwaway vault (fresh random wallet, fast KDF)");
    println!("  config dir : {}", dir.display());
    println!("  vault file : {}", path.display());
    println!("  address    : {addr}");
    println!("  passphrase : {pass}");
}
