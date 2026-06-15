//! QA helper — seal a THROWAWAY test vault so clicky GUI QA skips onboarding.
//!
//! Seals anvil's well-known dev mnemonic under a FIXED passphrase with FAST Argon2
//! params, into `DECKARD_CONFIG_DIR`. Because the KDF cost is baked into the vault
//! header, the resulting vault unlocks near-instantly everywhere. The app then boots
//! straight to the **Unlock** screen (no Create / seed-reveal / backup challenge) —
//! the tester just types the passphrase this prints.
//!
//!   just qa-vault     # seal the vault (this example)
//!   just qa           # launch the app against the same DECKARD_CONFIG_DIR
//!
//! Account 0 of this mnemonic (`0xf39Fd6…92266`) is prefunded with 10000 ETH on any
//! anvil chain — including a Sepolia fork — so the QA wallet needs no funding step.
//!
//! WARNING: throwaway test seed, NEVER for real funds. This file lives only under
//! `examples/`, so it is never linked into the shipped `deckard` binary. The
//! production create/import paths are untouched and keep `KdfParams::PRODUCTION`.

use std::path::PathBuf;

use deckard_core::{config::VAULT_FILE, KdfParams, Vault};

/// Anvil's canonical dev mnemonic. Account 0 = m/44'/60'/0'/0/0 =
/// `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266`, prefunded by anvil on every chain.
const MNEMONIC: &str = "test test test test test test test test test test test junk";
/// Fixed QA passphrase (>= 8 chars). Type this on the Unlock screen.
const PASS: &str = "deckard-qa";

fn main() {
    // Resolve the target config dir. NEVER fall back to the real platform keystore:
    // honour DECKARD_CONFIG_DIR, else use an explicit throwaway temp dir.
    let dir = match std::env::var_os("DECKARD_CONFIG_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from("/tmp/deckard-qa"),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("qa-vault: cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }

    // Fast Argon2 (8 MiB / t=1 / p=1) — the floor `validate()` allows. Baked into the
    // vault header, so unlock is fast in both the app and the daemon.
    let kdf = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };

    let vault = match Vault::import_mnemonic(MNEMONIC, PASS, kdf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("qa-vault: seal failed: {e}");
            std::process::exit(1);
        }
    };

    // Derive the address for the QA log (we print only the address + the known QA
    // passphrase reminder — never seed/key material).
    let addr = match vault.unlock(PASS).and_then(|u| u.primary_address()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("qa-vault: derive address failed: {e}");
            std::process::exit(1);
        }
    };

    let path = dir.join(VAULT_FILE);
    if let Err(e) = vault.write_atomic(&path) {
        eprintln!("qa-vault: write {} failed: {e}", path.display());
        std::process::exit(1);
    }

    println!("qa-vault: sealed a throwaway QA vault (fast KDF)");
    println!("  config dir : {}", dir.display());
    println!("  vault file : {}", path.display());
    println!("  address    : {addr}  (anvil account 0 — prefunded on any anvil/fork)");
    println!("  passphrase : {PASS}");
    println!();
    println!("Next: `just qa` -> the app boots to Unlock; type the passphrase above.");
}
