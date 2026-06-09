//! Railgun key derivation — the consensus-critical seed → 0zk-key path.
//!
//! Railgun derives its spending (babyjubjub) and viewing (ed25519) private keys via a
//! SLIP-0010-style hardened HMAC-SHA512 chain (NOT secp256k1 BIP-32) along
//! - spending  `m/44'/1984'/0'/0'/index'`
//! - viewing   `m/420'/1984'/0'/0'/index'`
//!
//! and feeds each node's 32-byte `chainKey` straight into the babyjubjub / ed25519 public-key
//! functions (Railgun engine `bip32.ts` + `wallet-node.ts` @ `e2913b39`). The one place
//! Railgun diverges from textbook SLIP-0010: the master HMAC key is the literal
//! **`"babyjubjub seed"`**, NOT `"ed25519 seed"`. Get ANY step wrong — that seed constant, the
//! hardened-only paths, the byte order, the pubkey curve — and the synced private balance
//! reads $0 forever. That is silent, and it is the worst failure mode a trust wallet can have.
//!
//! So this module is **gated by a known-answer test**. [`known_answer_ok`] re-derives a fixed
//! mnemonic and compares the resulting 0zk address to Railgun's OWN engine test vector (an
//! independent source). The app must not display any shielded balance unless it returns true,
//! and the same comparison is a `#[test]` so a derivation drift also turns CI red.
//!
//! Construction note: `railgun`'s `ByteKey::from_bytes` is crate-private, so the only public
//! way to build a `SpendingKey`/`ViewingKey` from our derived bytes is `HexKey::from_hex` —
//! hence the small local hex encoder (kept dependency-free).

use hmac::{Hmac, Mac};
use sha2::Sha512;

use railgun::account::address::RailgunAddress;
use railgun::account::chain::ChainId;
use railgun::crypto::keys::{HexKey, SpendingKey, ViewingKey};

type HmacSha512 = Hmac<Sha512>;

/// The hardened-key offset (SLIP-0010 / BIP-32). Every Railgun path segment is hardened —
/// which is also why ed25519 derivation is even possible (it supports hardened only).
const HARDENED: u32 = 0x8000_0000;
/// The fixed prefix of the spending-key path `m/44'/1984'/0'/0'/index'`.
const SPENDING_PREFIX: [u32; 4] = [44, 1984, 0, 0];
/// The fixed prefix of the viewing-key path `m/420'/1984'/0'/0'/index'`.
const VIEWING_PREFIX: [u32; 4] = [420, 1984, 0, 0];

/// Railgun's own known-answer vectors — engine `src/test/config.test.ts` (the mnemonic) +
/// `src/wallet/__tests__/railgun-wallet.test.ts` (account-0 `getAddress({ type: EVM, id })`).
/// Two chains so the test pins BOTH the key derivation AND the chain-id encoding. (The
/// engine's no-arg `getAddress()` default is the distinct ALL-chains address, NOT id=1.)
const KAT_MNEMONIC: &str = "test test test test test test test test test test test junk";
/// `getAddress({ type: ChainType.EVM, id: 1 })` — used by [`known_answer_ok`] at runtime.
/// (The chain-2 vector lives in the test, which is its only consumer.)
const KAT_ADDRESS_CHAIN1: &str = "0zk1qyk9nn28x0u3rwn5pknglda68wrn7gw6anjw8gg94mcj6eq5u48t7unpd9kxwatwq9ma02nutwtcqc979wnce0qwly4y7w4rls5cq040g7z8eagshxrw56ltkfa";

/// A derived Railgun keypair — the private spending + viewing keys. The public master key
/// and the 0zk address are computed from these by `railgun`.
pub struct RailgunKeys {
    pub spending: SpendingKey,
    pub viewing: ViewingKey,
}

/// HMAC-SHA512 keyed by `key` over the concatenation of `parts`, split into the
/// `(left[0..32], right[32..64])` halves SLIP-0010 uses for `(key, chainCode)`.
fn hmac_split(key: &[u8], parts: &[&[u8]]) -> anyhow::Result<([u8; 32], [u8; 32])> {
    // HMAC accepts a key of any length, so `new_from_slice` never errors here; propagate
    // rather than unwrap (deckard-core forbids unwrap/expect in non-test code).
    let mut mac = HmacSha512::new_from_slice(key).map_err(|e| anyhow::anyhow!("hmac key: {e}"))?;
    for p in parts {
        mac.update(p);
    }
    let out = mac.finalize().into_bytes();
    // `split_at` + `try_into` instead of `out[..32]` to satisfy the no-raw-indexing lint.
    let (left, right) = out.split_at(32);
    let l: [u8; 32] = left
        .try_into()
        .map_err(|_| anyhow::anyhow!("hmac left split"))?;
    let r: [u8; 32] = right
        .try_into()
        .map_err(|_| anyhow::anyhow!("hmac right split"))?;
    Ok((l, r))
}

/// Railgun's SLIP-0010-style derivation: master from `seed` (keyed by the literal
/// `"babyjubjub seed"`, Railgun's custom constant — its one divergence from textbook
/// SLIP-0010), then a hardened child-key derivation per segment; returns the final node's
/// 32-byte private key (Railgun's `chainKey`).
fn derive_chain_key(seed: &[u8], path: &[u32]) -> anyhow::Result<[u8; 32]> {
    // Master: I = HMAC-SHA512("babyjubjub seed", seed); key = I_L, chainCode = I_R.
    let (mut key, mut chain_code) = hmac_split(b"babyjubjub seed", &[seed])?;
    // Child (hardened only): I = HMAC-SHA512(chainCode, 0x00 || key || ser32(idx | HARDENED)).
    for &segment in path {
        let index = (segment | HARDENED).to_be_bytes();
        let (k, cc) = hmac_split(&chain_code, &[&[0u8], &key, &index])?;
        key = k;
        chain_code = cc;
    }
    Ok(key)
}

/// Lowercase hex (no `0x`) of a 32-byte key — the input to `railgun`'s public `HexKey::from_hex`
/// (its `ByteKey::from_bytes` is crate-private). Dependency-free, no raw indexing.
fn to_hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(64);
    for &b in bytes {
        // Writing to a String is infallible; discard the formatter Result explicitly.
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Derive the Railgun spending + viewing keys for account `index` from BIP-39 entropy.
///
/// Mirrors [`crate::UnlockedVault::account_signer`] (entropy stays in core, derived per call).
/// The BIP-39 seed uses an EMPTY passphrase, matching Railgun engine's
/// `Mnemonic.toSeed(mnemonic)`.
pub fn railgun_keys_from_entropy(entropy: &[u8], index: u32) -> anyhow::Result<RailgunKeys> {
    let mnemonic = bip39::Mnemonic::from_entropy(entropy)
        .map_err(|e| anyhow::anyhow!("bip39 from_entropy: {e}"))?;
    // `to_seed_normalized` (vs `to_seed`) avoids needing the `unicode-normalization` feature;
    // an empty passphrase is already normalized.
    let seed = mnemonic.to_seed_normalized("");

    let mut spending_path = SPENDING_PREFIX.to_vec();
    spending_path.push(index);
    let mut viewing_path = VIEWING_PREFIX.to_vec();
    viewing_path.push(index);

    let spend_bytes = derive_chain_key(&seed, &spending_path)?;
    let view_bytes = derive_chain_key(&seed, &viewing_path)?;

    let spending = SpendingKey::from_hex(&to_hex(&spend_bytes))
        .map_err(|e| anyhow::anyhow!("railgun spending key: {e}"))?;
    let viewing = ViewingKey::from_hex(&to_hex(&view_bytes))
        .map_err(|e| anyhow::anyhow!("railgun viewing key: {e}"))?;
    Ok(RailgunKeys { spending, viewing })
}

/// The 0zk address string for account `index` on `chain_id`, derived from BIP-39 entropy.
pub fn railgun_address_from_entropy(
    entropy: &[u8],
    chain_id: u64,
    index: u32,
) -> anyhow::Result<String> {
    let keys = railgun_keys_from_entropy(entropy, index)?;
    let address =
        RailgunAddress::from_private_keys(keys.spending, keys.viewing, ChainId::evm(chain_id));
    Ok(address.to_string())
}

/// The runtime gate: re-derive Railgun's own known mnemonic and compare the 0zk address to the
/// engine's published vector. **The app must not display a shielded balance unless this is
/// true** — a wrong derivation would otherwise show a silent, wrong (typically $0) balance.
pub fn known_answer_ok() -> bool {
    let Ok(mnemonic) = bip39::Mnemonic::parse(KAT_MNEMONIC) else {
        return false;
    };
    let entropy = mnemonic.to_entropy();
    match railgun_address_from_entropy(&entropy, 1, 0) {
        Ok(addr) => addr == KAT_ADDRESS_CHAIN1,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate. The expected address is Railgun's OWN engine test vector, so a pass means our
    /// SLIP-0010 + babyjubjub/ed25519 + bech32m path matches the canonical implementation; a
    /// fail means shielded balances would read wrong, so it must block the build.
    #[test]
    fn known_answer_matches_railgun_engine_vector() {
        let mnemonic = bip39::Mnemonic::parse(KAT_MNEMONIC).unwrap();
        let entropy = mnemonic.to_entropy();
        assert_eq!(
            railgun_address_from_entropy(&entropy, 1, 0).unwrap(),
            KAT_ADDRESS_CHAIN1,
            "Railgun derivation drifted from the engine vector — shielded balances would be wrong"
        );
        // A second chain pins the chain-id encoding, not just the keys.
        const KAT_ADDRESS_CHAIN2: &str = "0zk1qyk9nn28x0u3rwn5pknglda68wrn7gw6anjw8gg94mcj6eq5u48t7unpd9kxwatwqfma02nutwtcqc979wnce0qwly4y7w4rls5cq040g7z8eagshxrw5aha7vd";
        assert_eq!(
            railgun_address_from_entropy(&entropy, 2, 0).unwrap(),
            KAT_ADDRESS_CHAIN2
        );
        assert!(known_answer_ok(), "runtime gate must agree with the KAT");
    }

    /// Different account indices must yield different addresses (the path's last segment).
    #[test]
    fn distinct_indices_distinct_addresses() {
        let mnemonic = bip39::Mnemonic::parse(KAT_MNEMONIC).unwrap();
        let entropy = mnemonic.to_entropy();
        let a0 = railgun_address_from_entropy(&entropy, 1, 0).unwrap();
        let a1 = railgun_address_from_entropy(&entropy, 1, 1).unwrap();
        assert_ne!(a0, a1);
    }
}
