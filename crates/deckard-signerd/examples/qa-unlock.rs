//! QA helper — unlock the throwaway `qa-vault` signer daemon.
//!
//! This is only for the deterministic test vault produced by
//! `deckard-core --example qa-vault`. It never prints seed material, private keys, or the
//! passphrase; the only success output is the public address.

use deckard_contract::UnlockOutcome;
use deckard_signerd::{socket, SignerClient};

const QA_PASS: &str = "deckard-qa";

fn main() -> anyhow::Result<()> {
    let socket_path = std::env::var_os("DECKARD_SOCKET_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(socket::default_socket_path);

    let client = SignerClient::new(socket_path);
    match client.unlock_blocking(QA_PASS)? {
        UnlockOutcome::Unlocked { address } => {
            println!("qa-unlock: unlocked throwaway QA wallet {address:#x}");
            Ok(())
        }
        UnlockOutcome::BadPassphrase => anyhow::bail!("qa-unlock: QA passphrase was rejected"),
        UnlockOutcome::NoVault => anyhow::bail!("qa-unlock: no QA vault exists for this config"),
    }
}
