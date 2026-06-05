//! Wallet — a real, self-custodial Ethereum keypair via the **alloy** stack
//! (alloy-signer-local), the audited library the reth/Paradigm ecosystem builds
//! on. No hand-rolled crypto: key generation, the EIP-55 address, and (later)
//! signing all come from alloy. v0 generates a key on first run and persists it
//! (hex) to the platform config dir so the address is stable across launches.
//! BIP-39 seed backup is the next increment (the `mnemonic` feature is enabled).

use std::fs;
use std::path::PathBuf;

use alloy_signer_local::PrivateKeySigner;
use directories::ProjectDirs;

pub struct Wallet {
    /// 0x-prefixed, EIP-55 checksummed address (from alloy's `Address` Display).
    pub address: String,
    #[allow(dead_code)] // the signer drives send/swap signing in a later increment
    signer: PrivateKeySigner,
}

impl Wallet {
    /// Load the persisted key, or generate and persist a fresh one.
    pub fn load_or_generate() -> Self {
        if let Some(path) = key_path() {
            if let Ok(hex) = fs::read_to_string(&path) {
                if let Some(signer) =
                    hex_decode(hex.trim()).and_then(|b| PrivateKeySigner::from_slice(&b).ok())
                {
                    return Self::from_signer(signer);
                }
            }
            let signer = PrivateKeySigner::random();
            let bytes = signer.to_bytes();
            let _ = fs::write(&path, hex_encode(&bytes[..]));
            return Self::from_signer(signer);
        }
        Self::from_signer(PrivateKeySigner::random())
    }

    fn from_signer(signer: PrivateKeySigner) -> Self {
        let address = signer.address().to_string(); // EIP-55 checksummed
        Self { address, signer }
    }

    /// Middle-truncated address for tight UI, e.g. `0xA1b2…9F3c`.
    pub fn short(&self) -> String {
        let a = &self.address;
        if a.len() >= 12 {
            format!("{}…{}", &a[..6], &a[a.len() - 4..])
        } else {
            a.clone()
        }
    }
}

fn key_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("com", "deckard", "Deckard")?;
    let dir = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("wallet.key"))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
