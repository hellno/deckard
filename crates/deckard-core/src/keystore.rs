//! The encrypted keystore — Deckard's trust core. See `specs/keystore-design.md` for
//! the full rationale (validated by a 3-model adversarial review).
//!
//! Model: **envelope-encrypt the BIP-39 entropy** (never the phrase/seed/key). A random
//! 32-byte data key (DEK) encrypts the entropy with XChaCha20-Poly1305; the DEK is
//! wrapped by `KEK = Argon2id(passphrase, salt)`. A versioned, fully-authenticated
//! header (passed as AEAD associated data) makes downgrade/tamper attempts fail closed.
//! Secrets live in `Zeroizing` buffers; the alloy signer is only ever transient.

use std::path::Path;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::rngs::OsRng;
use rand::RngCore;
use zeroize::Zeroizing;

use alloy_primitives::Address;
use alloy_signer_local::{coins_bip39::English, MnemonicBuilder, PrivateKeySigner};

const MAGIC: &[u8; 4] = b"DKRD";
const FORMAT_VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24; // XChaCha20-Poly1305
const DEK_LEN: usize = 32;
const TAG_LEN: usize = 16;
const WRAPPED_DEK_LEN: usize = DEK_LEN + TAG_LEN; // 48
const VAULT_ID_LEN: usize = 16;

// Domain-separation strings bound into the two AEAD layers (per the adversarial review).
const AAD_WRAP: &[u8] = b"DKRDv1/wrap/passphrase";
const AAD_PAYLOAD: &[u8] = b"DKRDv1/payload";

/// Parse-time caps applied to a header BEFORE running Argon2, so a hostile vault can't
/// drive the app into an OOM/hang before the AEAD ever gets a chance to reject it.
const MIN_M_KIB: u32 = 8 * 1024; // 8 MiB (allows fast test vaults)
const MAX_M_KIB: u32 = 1024 * 1024; // 1 GiB — a hostile header can't force a larger Argon2 alloc
const MAX_T: u32 = 10;
const MAX_CT_ENTROPY: usize = 1024;
/// Hard cap on the whole on-disk vault (header + wraps + capped ciphertext, with margin):
/// `Vault::read` refuses to slurp anything larger, so a hostile multi-GB file can't OOM us.
const MAX_VAULT_BYTES: u64 = 4096;

/// Argon2id cost parameters, stored in the (authenticated) header so they can be raised
/// later via upgrade-on-unlock without breaking old vaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory cost, in KiB.
    pub m_kib: u32,
    /// Time cost (iterations).
    pub t: u32,
    /// Parallelism (pinned to 1).
    pub p: u32,
}

impl KdfParams {
    /// Production default: 256 MiB / t=3 / p=1 — the locked design's calibrated target
    /// (~0.5–1s on Apple Silicon), and strictly harder than every surveyed wallet (which
    /// use PBKDF2 or scrypt). The header carries the params, so `/cso` can raise them and
    /// old vaults still open (upgrade-on-unlock re-seals).
    pub const PRODUCTION: KdfParams = KdfParams {
        m_kib: 256 * 1024,
        t: 3,
        p: 1,
    };
    /// Fast params for tests only — NOT for real vaults.
    #[cfg(test)]
    pub const FAST_TEST: KdfParams = KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    };

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.p == 1, "unsupported Argon2 parallelism");
        anyhow::ensure!(
            (MIN_M_KIB..=MAX_M_KIB).contains(&self.m_kib),
            "Argon2 memory cost out of bounds"
        );
        anyhow::ensure!(
            (1..=MAX_T).contains(&self.t),
            "Argon2 time cost out of bounds"
        );
        Ok(())
    }
}

/// What the encrypted secret actually is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    /// 16 bytes of BIP-39 entropy (a 12-word phrase).
    Entropy16,
    /// 32 bytes of BIP-39 entropy (a 24-word phrase).
    Entropy32,
    /// A raw 32-byte private key imported with no recovery phrase.
    RawKey,
}

impl SecretKind {
    fn id(self) -> u8 {
        match self {
            SecretKind::Entropy16 => 0,
            SecretKind::Entropy32 => 1,
            SecretKind::RawKey => 2,
        }
    }
    fn from_id(id: u8) -> anyhow::Result<Self> {
        Ok(match id {
            0 => SecretKind::Entropy16,
            1 => SecretKind::Entropy32,
            2 => SecretKind::RawKey,
            _ => anyhow::bail!("unknown secret kind"),
        })
    }
    fn has_phrase(self) -> bool {
        !matches!(self, SecretKind::RawKey)
    }
}

/// How many words a freshly generated mnemonic should have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordCount {
    Twelve,
    TwentyFour,
}

/// The fixed-size, authenticated header (everything except the two ciphertexts).
#[derive(Clone)]
struct Header {
    secret_kind: SecretKind,
    vault_id: [u8; VAULT_ID_LEN],
    kdf: KdfParams,
    salt: [u8; SALT_LEN],
    /// Whether a non-empty BIP-39 passphrase was used (v0: always false).
    bip39_passphrase: bool,
    /// bit0 = a biometric DEK copy exists in the OS keychain (Phase-2; v0: 0).
    flags: u8,
    wrap_nonce: [u8; NONCE_LEN],
    entropy_nonce: [u8; NONCE_LEN],
}

impl Header {
    /// The canonical authenticated prefix: every header field, fixed order, no ciphertext.
    fn core_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(96);
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.push(self.secret_kind.id());
        b.extend_from_slice(&self.vault_id);
        b.push(KDF_ARGON2ID);
        b.extend_from_slice(&self.kdf.m_kib.to_le_bytes());
        b.extend_from_slice(&self.kdf.t.to_le_bytes());
        b.extend_from_slice(&self.kdf.p.to_le_bytes());
        b.extend_from_slice(&self.salt);
        b.push(self.bip39_passphrase as u8);
        b.push(self.flags);
        b.extend_from_slice(&self.wrap_nonce);
        b.extend_from_slice(&self.entropy_nonce);
        b
    }
}

/// The on-disk vault: header + the two ciphertexts. Carries no plaintext secret.
#[derive(Clone)]
#[must_use]
pub struct Vault {
    header: Header,
    wrapped_dek: [u8; WRAPPED_DEK_LEN],
    ct_entropy: Vec<u8>,
}

impl Vault {
    /// Generate a fresh HD wallet. Returns the sealed vault and the mnemonic phrase to
    /// show the user for backup (in a `Zeroizing` buffer — never log or persist it).
    pub fn create(
        passphrase: &str,
        words: WordCount,
        kdf: KdfParams,
    ) -> anyhow::Result<(Vault, Zeroizing<String>)> {
        let n = match words {
            WordCount::Twelve => 16,
            WordCount::TwentyFour => 32,
        };
        let mut entropy = Zeroizing::new(vec![0u8; n]);
        OsRng.fill_bytes(&mut entropy);

        let phrase = entropy_to_phrase(&entropy)?;
        let kind = if n == 16 {
            SecretKind::Entropy16
        } else {
            SecretKind::Entropy32
        };
        let vault = Self::seal(kind, &entropy, passphrase, kdf)?;
        Ok((vault, phrase))
    }

    /// Import an existing BIP-39 phrase (checksum-validated).
    pub fn import_mnemonic(
        phrase: &str,
        passphrase: &str,
        kdf: KdfParams,
    ) -> anyhow::Result<Vault> {
        let mnemonic = bip39::Mnemonic::parse(phrase.trim())
            .map_err(|_| anyhow::anyhow!("invalid recovery phrase"))?;
        let entropy = Zeroizing::new(mnemonic.to_entropy());
        let kind = match entropy.len() {
            16 => SecretKind::Entropy16,
            32 => SecretKind::Entropy32,
            // 15/18/21-word phrases are valid BIP-39 but uncommon for wallets; accept
            // 12 and 24 only so the round-trip kind is unambiguous.
            _ => anyhow::bail!("only 12- or 24-word phrases are supported"),
        };
        Self::seal(kind, &entropy, passphrase, kdf)
    }

    /// Import a raw private key. Requires an EXACT 32-byte key (alloy's `from_slice`
    /// would otherwise zero-pad a short slice — a silent-corruption footgun), and that the
    /// bytes are a valid secp256k1 scalar — so we never persist an unusable vault.
    pub fn import_raw_key(hex: &str, passphrase: &str, kdf: KdfParams) -> anyhow::Result<Vault> {
        let key = parse_exact_32(hex)?;
        PrivateKeySigner::from_slice(&key)
            .map_err(|_| anyhow::anyhow!("not a valid private key"))?;
        Self::seal(SecretKind::RawKey, &key, passphrase, kdf)
    }

    /// Envelope-encrypt `secret` under `passphrase`.
    fn seal(
        kind: SecretKind,
        secret: &[u8],
        passphrase: &str,
        kdf: KdfParams,
    ) -> anyhow::Result<Vault> {
        kdf.validate()?;

        let mut vault_id = [0u8; VAULT_ID_LEN];
        let mut salt = [0u8; SALT_LEN];
        let mut wrap_nonce = [0u8; NONCE_LEN];
        let mut entropy_nonce = [0u8; NONCE_LEN];
        let mut dek = Zeroizing::new([0u8; DEK_LEN]);
        OsRng.fill_bytes(&mut vault_id);
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut wrap_nonce);
        OsRng.fill_bytes(&mut entropy_nonce);
        OsRng.fill_bytes(dek.as_mut_slice());

        let header = Header {
            secret_kind: kind,
            vault_id,
            kdf,
            salt,
            bip39_passphrase: false,
            flags: 0,
            wrap_nonce,
            entropy_nonce,
        };
        let core = header.core_bytes();

        // Wrap the DEK with the passphrase-derived KEK, binding the header as AAD.
        let kek = derive_kek(passphrase.as_bytes(), &salt, &kdf)?;
        let wrap_aad = [AAD_WRAP, &core].concat();
        let wrapped = aead_encrypt(kek.as_slice(), &wrap_nonce, dek.as_slice(), &wrap_aad)?;
        anyhow::ensure!(wrapped.len() == WRAPPED_DEK_LEN, "unexpected wrap length");
        let mut wrapped_dek = [0u8; WRAPPED_DEK_LEN];
        wrapped_dek.copy_from_slice(&wrapped);

        // Encrypt the entropy with the DEK, binding header + wrapped DEK as AAD.
        let payload_aad = [AAD_PAYLOAD, &core, &wrapped_dek].concat();
        let ct_entropy = aead_encrypt(dek.as_slice(), &entropy_nonce, secret, &payload_aad)?;

        Ok(Vault {
            header,
            wrapped_dek,
            ct_entropy,
        })
    }

    /// Decrypt the vault with `passphrase`, yielding an in-memory unlocked wallet.
    /// A wrong passphrase and a tampered vault both surface as the same generic error
    /// (no oracle leaking which).
    pub fn unlock(&self, passphrase: &str) -> anyhow::Result<UnlockedVault> {
        self.header.kdf.validate()?;
        let core = self.header.core_bytes();

        let kek = derive_kek(passphrase.as_bytes(), &self.header.salt, &self.header.kdf)?;
        let wrap_aad = [AAD_WRAP, &core].concat();
        let dek = Zeroizing::new(
            aead_decrypt(
                kek.as_slice(),
                &self.header.wrap_nonce,
                &self.wrapped_dek,
                &wrap_aad,
            )
            .map_err(|_| unlock_failed())?,
        );

        let payload_aad = [AAD_PAYLOAD, &core, &self.wrapped_dek].concat();
        let secret = Zeroizing::new(
            aead_decrypt(
                dek.as_slice(),
                &self.header.entropy_nonce,
                &self.ct_entropy,
                &payload_aad,
            )
            .map_err(|_| unlock_failed())?,
        );

        Ok(UnlockedVault {
            kind: self.header.secret_kind,
            secret,
        })
    }

    /// Serialize to the on-disk byte format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.header.core_bytes();
        b.extend_from_slice(&self.wrapped_dek);
        // ct_entropy is bounded by MAX_CT_ENTROPY (1024) at both seal and parse time (and `read`
        // caps the whole file at MAX_VAULT_BYTES), so this cast can never truncate — to_bytes stays
        // infallible by construction.
        b.extend_from_slice(&(self.ct_entropy.len() as u32).to_le_bytes());
        b.extend_from_slice(&self.ct_entropy);
        b
    }

    /// Parse the on-disk byte format, applying strict caps BEFORE any heavy work.
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Vault> {
        let mut r = Reader::new(bytes);
        anyhow::ensure!(r.take(4)? == MAGIC, "not a Deckard vault");
        anyhow::ensure!(r.u8()? == FORMAT_VERSION, "unsupported vault version");
        let secret_kind = SecretKind::from_id(r.u8()?)?;
        let mut vault_id = [0u8; VAULT_ID_LEN];
        vault_id.copy_from_slice(r.take(VAULT_ID_LEN)?);
        anyhow::ensure!(r.u8()? == KDF_ARGON2ID, "unsupported KDF");
        let kdf = KdfParams {
            m_kib: r.u32()?,
            t: r.u32()?,
            p: r.u32()?,
        };
        kdf.validate()?; // cap-check BEFORE deriving anything
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(r.take(SALT_LEN)?);
        // v0 supports only an empty BIP-39 passphrase; require the canonical byte so the
        // value is byte-exact under AAD (no `2`-decodes-as-`true`-re-encodes-as-`1` drift).
        anyhow::ensure!(r.u8()? == 0, "unsupported BIP-39 passphrase flag");
        let bip39_passphrase = false;
        let flags = r.u8()?;
        let mut wrap_nonce = [0u8; NONCE_LEN];
        wrap_nonce.copy_from_slice(r.take(NONCE_LEN)?);
        let mut entropy_nonce = [0u8; NONCE_LEN];
        entropy_nonce.copy_from_slice(r.take(NONCE_LEN)?);
        let mut wrapped_dek = [0u8; WRAPPED_DEK_LEN];
        wrapped_dek.copy_from_slice(r.take(WRAPPED_DEK_LEN)?);
        let ct_len = r.u32()? as usize;
        anyhow::ensure!(ct_len <= MAX_CT_ENTROPY, "ciphertext too large");
        let ct_entropy = r.take(ct_len)?.to_vec();
        r.finish()?; // reject trailing garbage — the whole file must be consumed

        Ok(Vault {
            header: Header {
                secret_kind,
                vault_id,
                kdf,
                salt,
                bip39_passphrase,
                flags,
                wrap_nonce,
                entropy_nonce,
            },
            wrapped_dek,
            ct_entropy,
        })
    }

    /// Atomically write the vault to `path` with `0600` perms: write a temp file, fsync,
    /// rename over the target. Never leaves a partially written vault.
    pub fn write_atomic(&self, path: &Path) -> anyhow::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        {
            // Open the temp file already at 0600 — no window where it exists world-readable.
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(&self.to_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        // fsync the directory so the rename itself is durable across a crash/power loss.
        if let Some(dir) = path.parent() {
            if let Ok(dirf) = std::fs::File::open(dir) {
                let _ = dirf.sync_all();
            }
        }
        Ok(())
    }

    /// Read and parse a vault from `path`, refusing an implausibly large file before
    /// reading it into memory (a hostile multi-GB `vault.bin` can't OOM us).
    pub fn read(path: &Path) -> anyhow::Result<Vault> {
        let meta = std::fs::metadata(path)?;
        anyhow::ensure!(
            meta.len() <= MAX_VAULT_BYTES,
            "vault file is implausibly large"
        );
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Parse + unlock from in-memory bytes as a SINGLE authentication step. Every failure —
    /// malformed/truncated/tampered bytes, hostile KDF params, wrong passphrase, AEAD rejection —
    /// collapses to the same generic [`unlock_failed`] *message*, so the unlock path is not an
    /// error-message oracle distinguishing "wrong passphrase" from "tampered/corrupt vault".
    ///
    /// Scope: this equalizes the rendered error, NOT timing — a parse reject returns before Argon2,
    /// while a wrong passphrase runs it, so failure latency still differs. That residual is accepted
    /// deliberately: an attacker who holds the vault file can already parse it, and a desktop wallet
    /// doesn't expose unlock latency remotely; padding every malformed-file failure with a full
    /// Argon2 pass would cost ~1s for no real gain in this threat model.
    ///
    /// Use this (or [`Vault::open`]) on the unlock path; `from_bytes`/`unlock` stay available where a
    /// specific diagnostic is intentionally wanted and is NOT attacker-facing.
    pub fn open_bytes(bytes: &[u8], passphrase: &str) -> anyhow::Result<UnlockedVault> {
        Self::from_bytes(bytes)
            .and_then(|v| v.unlock(passphrase))
            .map_err(|_| unlock_failed())
    }

    /// Read a vault file and unlock it in one step, with the same generic-error (no-oracle)
    /// contract as [`Vault::open_bytes`]. This is what the unlock screen calls.
    pub fn open(path: &Path, passphrase: &str) -> anyhow::Result<UnlockedVault> {
        Self::read(path)
            .and_then(|v| v.unlock(passphrase))
            .map_err(|_| unlock_failed())
    }

    pub fn secret_kind(&self) -> SecretKind {
        self.header.secret_kind
    }
}

/// An unlocked wallet held in memory only while the app is unlocked. Drops zeroize the
/// secret. The alloy signer is reconstructed transiently per call and never stored.
#[must_use]
pub struct UnlockedVault {
    kind: SecretKind,
    secret: Zeroizing<Vec<u8>>, // entropy (16/32) or a raw 32-byte key
}

impl UnlockedVault {
    /// Derive account `index` (path `m/44'/60'/0'/0/index`) as a transient signer.
    pub fn account_signer(&self, index: u32) -> anyhow::Result<PrivateKeySigner> {
        match self.kind {
            SecretKind::Entropy16 | SecretKind::Entropy32 => {
                let phrase = entropy_to_phrase(&self.secret)?;
                Ok(MnemonicBuilder::<English>::default()
                    .phrase(phrase.as_str())
                    .index(index)?
                    .build()?)
            }
            SecretKind::RawKey => {
                anyhow::ensure!(index == 0, "imported raw key has only one account");
                anyhow::ensure!(self.secret.len() == 32, "raw key must be 32 bytes");
                Ok(PrivateKeySigner::from_slice(&self.secret)?)
            }
        }
    }

    /// The address of account `index`.
    pub fn account_address(&self, index: u32) -> anyhow::Result<Address> {
        Ok(self.account_signer(index)?.address())
    }

    /// The primary account address (index 0).
    pub fn primary_address(&self) -> anyhow::Result<Address> {
        self.account_address(0)
    }

    /// The recovery phrase, for the gated reveal flow. Errors for raw-key imports.
    pub fn reveal_phrase(&self) -> anyhow::Result<Zeroizing<String>> {
        anyhow::ensure!(
            self.kind.has_phrase(),
            "imported raw key has no recovery phrase"
        );
        entropy_to_phrase(&self.secret)
    }

    /// The wallet's own Railgun **0zk address** for account `index` on `chain_id` — the shield
    /// auto-fill recipient and the source of the viewing key. Derived from the same BIP-39
    /// entropy as `account_signer` (the seed never leaves core). Errors for a raw-key import
    /// (it has no mnemonic). Gated with `shield` since it leans on the railgun key types; the
    /// derivation itself is KAT-verified in [`crate::railgun_keys`].
    #[cfg(feature = "shield")]
    pub fn railgun_address(&self, chain_id: u64, index: u32) -> anyhow::Result<String> {
        anyhow::ensure!(
            self.kind.has_phrase(),
            "imported raw key has no Railgun 0zk address"
        );
        crate::railgun_keys::railgun_address_from_entropy(&self.secret, chain_id, index)
    }
}

// --- crypto helpers ---

fn derive_kek(
    passphrase: &[u8],
    salt: &[u8],
    kdf: &KdfParams,
) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32))
        .map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(passphrase, salt, kek.as_mut_slice())
        .map_err(|e| anyhow::anyhow!("argon2: {e}"))?;
    Ok(kek)
}

fn aead_encrypt(
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    msg: &[u8],
    aad: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(key.len() == 32, "bad AEAD key length");
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .encrypt(XNonce::from_slice(nonce), Payload { msg, aad })
        .map_err(|_| anyhow::anyhow!("encryption failed"))
}

fn aead_decrypt(
    key: &[u8],
    nonce: &[u8; NONCE_LEN],
    ct: &[u8],
    aad: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(key.len() == 32, "bad AEAD key length");
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| anyhow::anyhow!("decryption failed"))
}

/// One generic error for wrong-passphrase OR tamper — no oracle.
fn unlock_failed() -> anyhow::Error {
    anyhow::anyhow!("could not unlock — wrong passphrase, or the vault was tampered with")
}

/// Pick `k` distinct word positions (0-indexed, sorted) to quiz during backup
/// confirmation, chosen with the OS CSPRNG so a user can't pre-learn which to write.
pub fn random_word_positions(word_count: usize, k: usize) -> Vec<usize> {
    use rand::seq::SliceRandom;
    let mut idx: Vec<usize> = (0..word_count).collect();
    idx.shuffle(&mut OsRng);
    let mut chosen: Vec<usize> = idx.into_iter().take(k.min(word_count)).collect();
    chosen.sort_unstable();
    chosen
}

/// BIP-39 entropy bytes → mnemonic phrase (in a zeroizing buffer).
fn entropy_to_phrase(entropy: &[u8]) -> anyhow::Result<Zeroizing<String>> {
    let mnemonic =
        bip39::Mnemonic::from_entropy(entropy).map_err(|e| anyhow::anyhow!("bip39: {e}"))?;
    Ok(Zeroizing::new(mnemonic.to_string()))
}

/// Parse a hex private key that MUST be exactly 32 bytes (64 hex chars, optional `0x`).
fn parse_exact_32(hex: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let h = hex.trim().strip_prefix("0x").unwrap_or(hex.trim());
    anyhow::ensure!(
        h.len() == 64,
        "private key must be exactly 32 bytes (64 hex chars)"
    );
    let mut out = Zeroizing::new(vec![0u8; 32]);
    // h.len() == 64 (checked above) → exactly 32 two-char chunks, matching out's 32 slots.
    // iter_mut().zip() avoids raw indexing (clippy::indexing_slicing).
    for (slot, chunk) in out.iter_mut().zip(h.as_bytes().chunks(2)) {
        let s = std::str::from_utf8(chunk).map_err(|_| anyhow::anyhow!("invalid hex"))?;
        *slot = u8::from_str_radix(s, 16).map_err(|_| anyhow::anyhow!("invalid hex"))?;
    }
    Ok(out)
}

/// A tiny bounds-checked byte reader for parsing the vault format.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> anyhow::Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.buf.len())
            .ok_or_else(|| anyhow::anyhow!("vault truncated"))?;
        // `.get(range)` instead of `self.buf[pos..end]`: bounds-checked, no raw slice indexing.
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| anyhow::anyhow!("vault truncated"))?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> anyhow::Result<u8> {
        self.take(1)?
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("vault truncated"))
    }
    fn u32(&mut self) -> anyhow::Result<u32> {
        let b: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("vault truncated"))?;
        Ok(u32::from_le_bytes(b))
    }
    /// Assert the whole buffer was consumed (no trailing bytes).
    fn finish(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.pos == self.buf.len(), "trailing bytes after vault");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PW: &str = "correct horse battery staple";

    #[test]
    fn create_unlock_round_trip_and_stable_address() {
        let (vault, phrase) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        assert_eq!(phrase.split_whitespace().count(), 12);
        let unlocked = vault.unlock(PW).unwrap();
        let addr1 = unlocked.primary_address().unwrap();

        // Round-trip through bytes → same address, same phrase.
        let bytes = vault.to_bytes();
        let reparsed = Vault::from_bytes(&bytes).unwrap();
        let unlocked2 = reparsed.unlock(PW).unwrap();
        assert_eq!(addr1, unlocked2.primary_address().unwrap());
        assert_eq!(*phrase, *unlocked2.reveal_phrase().unwrap());
    }

    #[test]
    fn wrong_passphrase_fails_closed() {
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        assert!(vault.unlock("wrong passphrase").is_err());
    }

    #[test]
    fn unlock_failures_share_one_message() {
        // Every authentication failure must surface the IDENTICAL message via the open_bytes()
        // contract, so the unlock UI can't reveal whether the passphrase was wrong or the vault was
        // tampered/corrupt. (Message-level only — timing is a documented, accepted residual; see
        // Vault::open_bytes. anyhow::Error has no PartialEq, so compare rendered strings.)
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let good = vault.to_bytes();

        // A correct passphrase still unlocks through the same path.
        assert!(Vault::open_bytes(&good, PW).is_ok());

        // `.err()` not `.unwrap_err()`: UnlockedVault is deliberately not Debug (no-leak), so
        // unwrap_err (which would format the Ok value) won't compile — itself a guard.
        let baseline = Vault::open_bytes(&good, "definitely the wrong passphrase")
            .err()
            .expect("wrong passphrase must fail to unlock")
            .to_string();

        // Tampers spanning the parser (magic/version/KDF/trailing/truncation) AND the AEAD layer —
        // all must collapse to the same message.
        let mut cases: Vec<Vec<u8>> = vec![
            b"not a deckard vault".to_vec(), // bad magic
            good[..good.len() - 1].to_vec(), // truncated
        ];
        let mut bad_version = good.clone();
        bad_version[4] = 0xFF; // version byte, right after the 4-byte magic
        cases.push(bad_version);
        let mut bad_kdf = good.clone();
        let m_off = 4 + 1 + 1 + VAULT_ID_LEN + 1; // m_kib: after magic+ver+kind+vault_id+kdf_id
        bad_kdf[m_off..m_off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(bad_kdf);
        let mut bad_aead = good.clone();
        let last = bad_aead.len() - 1;
        bad_aead[last] ^= 0x01; // flip a ciphertext/tag byte → AEAD rejects
        cases.push(bad_aead);
        let mut trailing = good.clone();
        trailing.push(0x00); // trailing garbage
        cases.push(trailing);

        for (i, bad) in cases.iter().enumerate() {
            let got = Vault::open_bytes(bad, PW)
                .err()
                .expect("a tampered/corrupt vault must fail to unlock")
                .to_string();
            assert_eq!(
                got, baseline,
                "case {i} produced a distinguishable unlock error"
            );
        }
    }

    #[test]
    fn open_file_path_collapses_to_generic() {
        use std::io::Write;
        // Vault::open (the on-disk path do_unlock uses) must collapse read/size-cap/parse/AEAD
        // failures to the same generic message as a wrong passphrase — never a distinct IO error.
        let path = std::env::temp_dir().join("deckard-open-contract-test.bin");
        let write = |bytes: &[u8]| {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(bytes).unwrap();
        };
        let open_err = || {
            Vault::open(&path, PW)
                .err()
                .expect("must fail to unlock")
                .to_string()
        };

        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        write(&vault.to_bytes());
        assert!(
            Vault::open(&path, PW).is_ok(),
            "a valid vault file must unlock"
        );
        let baseline = Vault::open(&path, "wrong passphrase")
            .err()
            .expect("wrong passphrase must fail")
            .to_string();

        write(b"not a deckard vault");
        assert_eq!(open_err(), baseline, "garbage file leaked a distinct error");
        write(&vec![0u8; 5000]); // > MAX_VAULT_BYTES (4096) → size-cap reject
        assert_eq!(
            open_err(),
            baseline,
            "oversized file leaked a distinct error"
        );
        let _ = std::fs::remove_file(&path);
        assert_eq!(open_err(), baseline, "missing file leaked a distinct error");
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // Frozen REAL v1 vault blobs (FORMAT_VERSION 1, FAST_TEST KDF), captured once from known inputs:
    // ENTROPY16 = the canonical 12-word "abandon…about"; ENTROPY32 = the 24-word "abandon…art";
    // RAWKEY = 0x46*32. To regenerate after an intentional format bump, temporarily restore the
    // gen_compat_fixtures generator (git history) and re-bake.
    const ENTROPY16_HEX: &str = "444b5244010079a6a367eeb270d770dcf64ea3eecea801002000000100000001000000d2d7d128338ed86d7c7dd636345084a700007f1a9945fceea918f61828d5b3e4f2ea6ea7df273870ee3c02832b5966fb0a6fe0e839403d6264de00245da2de44162d03df796bd78e979470ba93c90c3998440873de456b3011a7d9135356eab7cec475ed48444e3ec3d386c0776cf238a5da200000005ebb566131a4f7c68ffdc0d6c7087b6ce3166baac57a6ce3ff408316eb90249f";
    const ENTROPY32_HEX: &str = "444b52440101c23da1d7c70914433c40a2573da3e7330100200000010000000100000089acc8ec1a8b0a563601760609bf3d530000f8f1e8ca056fad16d8416189ea7e0f2d23f93d78bc4f5bdd6da976e6bd67bb502e354788a409a361d099f7a5a812e2dd37eeb9bd34070045b64b7820e59a10618cd3a8f496f1df57e759704f8d919604764d1c262e2b58f69d0d83c80192ff633000000033ed76cbcade5104bc27c0ae0391b2f0c44124d3c2d3c878c8f9d4a7d67811553b13e617ae10f719223a78f37e204fb4";
    const RAWKEY_HEX: &str = "444b524401024ded6e18c437a15f07f0ec8043534b7001002000000100000001000000e2c48852b4d1e57e184ae6b73ab00011000095dc73698f21c7cfb28984a9f2da03f00ab54c722330e1f0afe4e79025b6bf37811038ccdfe9f48b7a1ce6754998e049143b30f340ebcab827cb61ec2a1e8065436187dccec035995ba5b9199d6e6340bce0e3bac70d0c32f0bf3ce94455a3a430000000caf0392f2f3b04eb0ce6b9d00c13ca62a54cf862e25248063f0bfbfaf55ba18ac08e2e325c7f1a084cdfc1a25f3c2a54";

    #[test]
    fn decode_compat_v1_fixtures() {
        // If a future format/parser change ever stops an old on-disk vault from parsing + unlocking
        // to the same address, that's a backward-incompatible break (lost funds) — caught here.
        let fixtures: &[(&str, SecretKind, &str)] = &[
            (
                ENTROPY16_HEX,
                SecretKind::Entropy16,
                "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
            ),
            (
                ENTROPY32_HEX,
                SecretKind::Entropy32,
                "0xF278cF59F82eDcf871d630F28EcC8056f25C1cdb",
            ),
            (
                RAWKEY_HEX,
                SecretKind::RawKey,
                "0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F",
            ),
        ];
        for (hex, kind, want_addr) in fixtures {
            let bytes = unhex(hex);
            let vault = Vault::from_bytes(&bytes).expect("v1 fixture must still parse");
            assert_eq!(vault.secret_kind(), *kind);
            let addr = vault
                .unlock(PW)
                .expect("v1 fixture must still unlock")
                .primary_address()
                .unwrap();
            assert_eq!(addr.to_string(), *want_addr, "v1 fixture address drifted");
        }
    }

    #[test]
    fn tamper_each_region_fails_closed() {
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let good = vault.to_bytes();
        // Flip a byte in the header (m_kib region ~ byte 23), the wrapped DEK, and the
        // entropy ciphertext; each must fail to unlock.
        for idx in [7usize, good.len() - 1, good.len() - 20] {
            let mut bad = good.clone();
            bad[idx] ^= 0x01;
            // Some flips break the parser (truncation/caps), others break the AEAD;
            // either way the vault must NOT unlock.
            let unlocked = Vault::from_bytes(&bad).and_then(|v| v.unlock(PW));
            assert!(unlocked.is_err(), "tamper at {idx} unexpectedly unlocked");
        }
    }

    #[test]
    fn bip39_known_vector_derives_known_address() {
        // Standard BIP-39 test phrase → well-known account-0 address (m/44'/60'/0'/0/0).
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let vault = Vault::import_mnemonic(phrase, PW, KdfParams::FAST_TEST).unwrap();
        let addr = vault.unlock(PW).unwrap().primary_address().unwrap();
        assert_eq!(
            addr.to_string(),
            "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
        );
    }

    #[test]
    fn import_raw_key_requires_exact_32_bytes() {
        // Exactly 32 bytes → ok.
        let k = "0x4646464646464646464646464646464646464646464646464646464646464646";
        let v = Vault::import_raw_key(k, PW, KdfParams::FAST_TEST).unwrap();
        assert_eq!(v.secret_kind(), SecretKind::RawKey);
        assert!(v.unlock(PW).unwrap().primary_address().is_ok());
        // A short key must be rejected (NOT zero-padded).
        assert!(Vault::import_raw_key("0xabcd", PW, KdfParams::FAST_TEST).is_err());
        // A raw-key vault has no recovery phrase.
        assert!(v.unlock(PW).unwrap().reveal_phrase().is_err());
    }

    #[test]
    fn trailing_bytes_rejected() {
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let mut bytes = vault.to_bytes();
        bytes.push(0x00); // one extra byte
        assert!(
            Vault::from_bytes(&bytes).is_err(),
            "trailing garbage must be rejected"
        );
    }

    #[test]
    fn noncanonical_bip39_flag_rejected() {
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let mut bytes = vault.to_bytes();
        // bip39_passphrase byte sits after magic(4)+ver(1)+kind(1)+vault_id(16)+kdf_id(1)
        // +m(4)+t(4)+p(4)+salt(16) = offset 51.
        let off = 4 + 1 + 1 + VAULT_ID_LEN + 1 + 4 + 4 + 4 + SALT_LEN;
        bytes[off] = 1;
        assert!(
            Vault::from_bytes(&bytes).is_err(),
            "non-zero BIP-39 flag must be rejected"
        );
    }

    #[test]
    fn invalid_raw_scalar_rejected() {
        // An all-zero "key" is not a valid secp256k1 scalar — reject before persisting.
        let zero = "0x0000000000000000000000000000000000000000000000000000000000000000";
        assert!(Vault::import_raw_key(zero, PW, KdfParams::FAST_TEST).is_err());
    }

    #[test]
    fn oversized_vault_bytes_rejected() {
        // from_bytes must reject a ciphertext length beyond the cap.
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let mut bytes = vault.to_bytes();
        // The ct length u32 is the 4 bytes right before ct_entropy (the tail).
        let ct_actual = bytes.len(); // we'll just corrupt the declared length to huge
        let len_pos = ct_actual - vault.ct_entropy.len() - 4;
        bytes[len_pos..len_pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            Vault::from_bytes(&bytes).is_err(),
            "absurd ct length must be rejected"
        );
    }

    #[test]
    fn hostile_kdf_params_rejected_before_work() {
        let (vault, _) = Vault::create(PW, WordCount::Twelve, KdfParams::FAST_TEST).unwrap();
        let mut bytes = vault.to_bytes();
        // m_kib lives right after magic(4)+ver(1)+kind(1)+vault_id(16)+kdf_id(1) = offset 23.
        let off = 4 + 1 + 1 + VAULT_ID_LEN + 1;
        bytes[off..off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            Vault::from_bytes(&bytes).is_err(),
            "absurd m_kib must be rejected"
        );
    }
}
