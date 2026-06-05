# Deckard keystore — locked design (v0)

Status: **locked** 2026-06-05. Validated by a 3-model adversarial review (Claude research + 2 Claude
skeptics + GPT-5.5/xhigh hostile audit). Verdict: **SHIP-WITH-ADDITIONS** — the architecture is the
best audited software-at-rest construction for a native EOA wallet; ship with the additions below.
This is the blueprint for Chunk 3 and the input to `/cso`.

## Threat model
Holds a mainnet EOA seed. Defends well against **cold theft** of the vault file (disk image, Time
Machine, backup, `chmod`-readable copy) → offline cracking. Does **not** defend against live malware
running as the user during an unlocked session (no software hot wallet can; the seed must enter RAM to
sign). We state this honestly and minimise the unlocked-RAM window.

## What we store, and where
- Encrypt the **BIP-39 entropy** (16 B for 12-word / 32 B for 24-word) — never the mnemonic string,
  never the 64-byte seed, never a derived key. Re-derive on unlock.
- `~/Library/Application Support/com.deckard.Deckard/vault.bin` — the versioned, authenticated vault
  (Argon2id-wrapped DEK + XChaCha20-Poly1305-encrypted entropy). `chmod 0600`. Atomic write:
  `vault.tmp → fsync → rename → fsync(dir)`. Never partially written, never silently regenerated.
- macOS Keychain (Phase-2 only) — a *copy of the DEK*, biometry-gated. Never the entropy, never the
  passphrase. A convenience cache over the mandatory passphrase, never the only copy.
- Nothing in plaintext anywhere. The legacy plaintext `wallet.key` is migrated then deleted.

## Crypto construction (audited primitives only)
- **KDF:** Argon2id (`argon2` 0.5, RustCrypto). Calibrated to **~0.5–1 s on target hardware**
  (≈256–512 MiB, t tuned, p=1) — NOT the OWASP server-login floor. Params live in the authenticated
  header; upgrade-on-unlock re-seals with a fresh DEK + fresh nonces when params change.
- **AEAD:** XChaCha20-Poly1305 (`chacha20poly1305` 0.10, RustCrypto, NCC-audited). 192-bit random
  nonces (no reuse risk). Poly1305 tag is the MAC — no bolt-on keccak-MAC, no AES-128-CTR.
- **Envelope:** random 32-B DEK encrypts the entropy; the DEK is wrapped by `KEK = Argon2id(passphrase,
  salt)`. (Phase-2: the DEK is *also* stored in the biometry-gated Keychain.)
- **RNG:** `OsRng` only, for entropy, DEK, salt, and all nonces. Exact length validation everywhere.
- **Secrets in memory:** `Zeroizing` for entropy, DEK, KEK, derived child key, and the transient
  mnemonic. We do **not** rely on alloy's `PrivateKeySigner` zeroizing (it does not: no `Drop`, derives
  `Clone`) — see "Signing" below.

## Vault binary layout (authenticated)
Canonical, fixed-order header; **every field is AEAD AAD** so a tamperer who can write `vault.bin`
cannot downgrade KDF params or flip the biometric flag without failing decryption.

```
magic "DKRD" | format_version u8 | secret_kind u8 (0=entropy16,1=entropy32,2=raw32-imported)
 | vault_id [16]B | kdf_id u8 (1=argon2id) | argon2 m u32 | t u32 | p u32 | salt [16]B
 | bip39_passphrase_used u8 (v0: always 0) | flags u8 (bit0 = keychain-DEK present)
 | wrap_nonce [24]B | wrapped_dek [32+16]B | entropy_nonce [24]B | ct_entropy [len+16]B
```
Domain-separated AAD per AEAD call:
- DEK wrap: `aad = "DKRDv1/wrap/passphrase" || canonical_header_without_ciphertexts`
- entropy : `aad = "DKRDv1/payload" || canonical_header || wrapped_dek`

**Parse before Argon2:** strict bounds — known `kdf_id`/version, `p == 1`, `m`/`t` within sane
min/max, exact salt/nonce/ct lengths — so a hostile header can't OOM/hang the app before the AEAD can
reject it.

## Keys & signing (memory hygiene)
- Generate entropy with `OsRng`; convert to a mnemonic via `coins_bip39::Mnemonic::<English>::
  new_from_entropy(..)` (already in-tree via alloy-signer-local). **Do not** use
  `MnemonicBuilder::build_random()` (it uses `thread_rng` and `write_to()` writes the phrase in
  plaintext) — entropy is Deckard-owned.
- On unlock: derive the selected child private key **once** into `Zeroizing<[u8;32]>`
  (path `m/44'/60'/0'/0/i`). Drop the phrase + seed immediately.
- Per signing batch: `PrivateKeySigner::from_slice(&key)` **only after `key.len() == 32`**, sign, drop
  the signer. alloy is the transient ECDSA boundary, never the long-lived key holder.
- BIP-39 passphrase: v0 supports **empty only**, recorded in authenticated metadata. A non-empty BIP-39
  passphrase is a *separate* recovery secret if ever added — never reuse the vault passphrase.
- Idle-lock: default 15 min (configurable). `lock()` zeroizes the in-memory child key (not just a UI
  route flip). A memory dump after lock yields nothing we control.

## Multi-account
Account metadata is **derived, never trusted from writable config**. Receive addresses are recomputed
from the seed on unlock. Anti-funding gate: Receive is enabled only after backup-confirm **and** an
atomic vault write succeeds.

## Unlock UX (macOS-first)
- **v0 ships passphrase unlock (Argon2id).** The passphrase is the durable ground-truth secret — set at
  create/import, required every launch and after idle-lock. With ~0.5–1 s Argon2 this needs a visible
  progress affordance.
- **Touch ID is Phase-2**, *blocked on the codesign/notarize pipeline*: the macOS data-protection
  keychain requires an App ID entitlement (`com.apple.application-identifier`) + provisioning profile +
  Developer ID + Hardened Runtime + notarization. An unsigned `cargo bundle` build cannot `SecItemAdd`
  the biometric DEK (`errSecMissingEntitlement` -34018). When it lands: `biometryCurrentSet` (not bare
  `.userPresence`), `kSecUseDataProtectionKeychain=true`, `kSecAttrSynchronizable=false`, stable
  service/account = `vault_id`; SecItem calls run **off** the GPUI thread; passphrase always falls back
  so an invalidated item never bricks access.

## Onboarding flow
- **Create:** generate entropy → 12-word (default) / 24-word → set passphrase (strength meter, confirm
  twice) → **mandatory backup** (blurred, hold-to-reveal, auto-re-blur on blur/timeout, never silent
  clipboard) → **confirm-a-subset** (3–4 random word positions) → only then write vault + enable
  funding → offer Touch ID (Phase-2, shown disabled-with-reason until signing lands).
- **Import:** paste mnemonic (BIP-39 checksum-validated) or raw private key (flagged "imported — no
  recovery phrase") → set passphrase → write vault.
- **Seed reveal (Settings):** re-authenticate first → same blurred/hold-to-reveal/auto-hide treatment.

## Legacy migration (current plaintext `wallet.key`)
The existing dev `wallet.key` is raw plaintext hex. On first run of the keystore build: reject anything
but an **exact 32-byte** key (strip optional `0x`, require exactly 64 hex chars — `from_slice`
zero-pads short slices, a silent-corruption footgun), force-set a passphrase, encrypt into `vault.bin`
as `secret_kind=raw32-imported` (no phrase), then delete `wallet.key`. **Treat any wallet that ever
existed as plaintext as compromised** (APFS copy-on-write / snapshots / Time Machine retain it) →
strongly recommend creating a fresh wallet and moving funds. Never silently generate a new key on a
corrupt/oversized read — surface an error.

## Crates (Chunk 3)
`argon2 = "0.5"` · `chacha20poly1305 = "0.10"` · `zeroize = { version = "1", features = ["derive"] }`
· `rand = "0.8"` (OsRng) · `coins-bip39 = "0.12"` (in-tree; entropy↔mnemonic). Phase-2 (deferred):
`security-framework`, `objc2-local-authentication`, `objc2-security`, `keyring = "4"` (Linux).

## Test matrix (deckard-core, headless)
BIP-39 official vectors (entropy→phrase→address) · vault round-trip (create→lock→unlock) ·
wrong-passphrase fails closed (one generic error, no oracle) · tamper each region (flip a byte in
header / wrapped_dek / ct_entropy) fails closed · KDF-cap rejection (hostile params) · exact-32-byte
raw-key import (reject short/long) · account derivation vectors · migration failure modes ·
upgrade-on-unlock re-seal. (Touch ID path is hardware-integration-tested, not unit-tested.)

## Residual risks for `/cso`
Weak user passphrase collapses the guarantee (meter is advisory) · third-party crate internals
(coins-bip39 seed, GPUI text input/clipboard during reveal) may hold un-zeroized copies · unlocked-RAM
window vs live malware (inherent) · APFS secure-delete is best-effort (migration) · Touch ID read path
(`kSecUseAuthenticationContext` + LAContext) must be hardware-verified to actually enforce biometry.

## Linux fast-follow
Identical vault.bin + passphrase path (the primary path). Convenience DEK cache via `keyring`
(Secret Service) when present; absent on headless → passphrase-only, unchanged. Zero crypto divergence.

## Implementation review (codex GPT-5.5/xhigh, 2026-06-05)
Static audit of the shipped `keystore.rs`. **Confirmed correct:** entropy-not-phrase storage, domain-
separated AAD binding the canonical header + `wrapped_dek`, OsRng for all randomness, generic unlock
error, exact-32-byte raw-key reject, BIP-39↔alloy agreement (known-vector test), Argon2 off the UI
thread. **Verdict: SHIP-WITH-FIXES** — fixes applied this session:
- `Vault::read` now size-caps the file (`MAX_VAULT_BYTES`) before reading → no multi-GB OOM.
- `write_atomic` now fsyncs the parent dir after rename, and opens the temp file at `0600` (no perms window).
- Parser rejects trailing garbage (`Reader::finish`) and a non-zero BIP-39-passphrase flag (byte-exact auth).
- `import_raw_key` validates the secp256k1 scalar before persisting.
- KDF parse-caps tightened to `m ≤ 1 GiB, t ≤ 10`; `PRODUCTION` raised to **256 MiB / t=3** (the calibrated target).
- `legacy_key_hex` size-capped before read.
- New tests: trailing-garbage, non-canonical-flag, invalid-scalar, oversized-ct rejection.

**Deferred residual (for `/cso` + a Phase-2 secret input):** the GPUI text inputs hold passphrase /
phrase copies as `String`/`SharedString`/Rope (and undo history) that we don't control or zeroize.
Mitigation needs a custom no-history secret input + clearing inputs after use. Flagged, not yet done.
