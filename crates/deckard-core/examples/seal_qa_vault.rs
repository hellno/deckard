//! Seal a throwaway QA / screenshot vault for anvil account 0 with FAST KDF params, so a
//! launched app boots straight to Unlock (no onboarding) and a single passphrase unlocks it.
//!
//! The phrase is anvil's well-known dev mnemonic, so account 0 is the prefunded anvil EOA — a
//! vault sealed here controls a funded account on a local anvil. NEVER point this at a real
//! config dir: the keystore it writes is intentionally weakly-KDF'd and uses a public seed.
//!
//! Usage: `cargo run -p deckard-core --example seal_qa_vault -- <config_dir> <passphrase>`

use std::path::Path;

use deckard_core::{KdfParams, Vault};

/// Anvil's default dev mnemonic — account 0 is prefunded at the BIP-44 path the keystore derives.
const ANVIL_MNEMONIC: &str = "test test test test test test test test test test test junk";

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: seal_qa_vault <config_dir> <passphrase>"))?;
    let pass = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: seal_qa_vault <config_dir> <passphrase>"))?;

    // Deliberately weak, fast params (8 MiB / t=1) — this is a throwaway demo vault, not a
    // real keystore; the production params are far heavier.
    let kdf = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };
    let vault = Vault::import_mnemonic(ANVIL_MNEMONIC, &pass, kdf)?;
    std::fs::create_dir_all(&dir)?;
    vault.write_atomic(&Path::new(&dir).join("vault.bin"))?;
    println!("sealed QA vault into {dir} (anvil account 0)");
    Ok(())
}
