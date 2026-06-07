# Key Management & Security Patterns

> How software and hardware Ethereum wallets protect keys at rest, on-device, and at signing time in 2026 — and where Deckard's locked Argon2id + XChaCha20-Poly1305 envelope sits relative to the field. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- The de-facto software-wallet key-at-rest format is the **Web3 Secret Storage Definition v3**: AES-128-CTR cipher, PBKDF2-HMAC-SHA256 (mandatory) or scrypt (optional) KDF, and a bolt-on **keccak-256 MAC** = `KECCAK(DK[16..31] ++ ciphertext)` for integrity [1]. Geth, ethers, Foundry/`cast`, and the Rust `eth-keystore` crate all implement it [1][2][3].
- Deckard's locked envelope (**Argon2id** + **XChaCha20-Poly1305**) is cryptographically stronger on both axes but **not interoperable** with that format. Argon2id is OWASP's top-recommended KDF; XChaCha20-Poly1305 is an AEAD whose Poly1305 tag authenticates intrinsically, replacing the separate keccak MAC and removing AES-CTR malleability [4][5].
- Deckard v0's **plaintext-hex private key on disk is below the universal field floor** — no mainstream wallet stores cleartext keys at rest [1][3][6]. Shipping the encrypted envelope is the single highest-value security change.
- alloy / `eth-keystore` give you scrypt + AES-128-CTR Web3-Secret-Storage out of the box, **not** Argon2id/XChaCha — that envelope is a custom layer built from RustCrypto's `argon2`, `chacha20poly1305`, and `zeroize` [7][8][9].
- The Rust primitives are mature and audited: `k256` (NCC Group 2023, two high-sev issues found and fixed), `chacha20poly1305` (NCC Group, no significant findings), reference `argon2`, and `zeroize` for non-optimizable memory wiping [9][10][11].
- Apple's **Secure Enclave only supports NIST P-256 (secp256r1)** — it cannot hold or sign with Ethereum's secp256k1 keys, so a Secure-Enclave-backed EOA is impossible without a smart account [12][13].
- Passkeys / WebAuthn sign with secp256r1; on-chain verification is now cheap via **RIP-7212** (precompile at `0x100`, 3450 gas, live on L2s) and on **mainnet** via **EIP-7951** (`0x100`, 6900 gas), shipped in the **Fusaka fork on Dec 3 2025** — but only usable through a smart account [14][15][16].
- **EIP-7702** (Pectra, mainnet **May 7 2025**) lets an EOA delegate its code without changing address — the lowest-friction path to account-abstraction features, but a real phishing surface: within weeks the vast majority of mainnet delegations pointed at drainer contracts [17][18].
- Institutional infra removed single-key risk via **TEEs + MPC/sharding** (Turnkey, Privy, Web3Auth, Lit); the TEE-plus-policy-engine pattern maps closely onto Deckard's operator-wallet vision [19][20][21][22].
- **Clear signing** (EIP-712 + **ERC-7730**, Ledger-led, Draft since Feb 2024) gives machine-readable transaction intent — directly relevant to letting an LLM (or user) understand what a signature does before approving [23][24].

## The field-standard keystore: Web3 Secret Storage v3

The canonical software-wallet key-at-rest format, documented on ethereum.org and originating from go-ethereum, is a JSON file (`<uuid>.json`) with a `crypto` object holding `cipher`, `cipherparams.iv`, `ciphertext`, `kdf`, `kdfparams`, and `mac`, plus top-level `id` (UUID) and `version: 3` [1]. The specifics:

- **Cipher**: AES-128-CTR is mandatory for minimal compliance; the encryption key is the leftmost 16 bytes of the derived key (`DK[0..15]`) [1].
- **KDF**: PBKDF2-HMAC-SHA256 must be supported (params `c`, `salt`, `dklen ≥ 32`); scrypt (`n`, `r`, `p`, `salt`, `dklen`) is an optional alternative. The PBKDF2 test vector uses `c = 262144`, `dklen = 32` [1].
- **Integrity**: NOT an AEAD tag but a separate keccak-256 MAC, `KECCAK(DK[16..31] ++ ciphertext)` [1].

AES-128-CTR is **unauthenticated** — the keccak MAC is what prevents ciphertext tampering, a bolt-on that modern AEADs make unnecessary [1].

Foundry's `cast wallet import` writes per-account encrypted JSON to `~/.foundry/keystores` in this exact v3 format (scrypt KDF, AES-128-CTR, keccak MAC), and its `--unsafe-password` / plaintext path is explicitly flagged "not recommended" [3][6]. This is the modern recommendation for replacing plaintext `PRIVATE_KEY` env vars in dev tooling, and confirms that even developer CLIs encrypt keys at rest with a memory-hard-ish KDF.

## How Deckard's Argon2id + XChaCha20-Poly1305 compares

Deckard's locked envelope is **stronger but non-standard**.

| Axis | Web3 Secret Storage v3 | Deckard envelope |
|---|---|---|
| KDF | PBKDF2-HMAC-SHA256 (mandatory) / scrypt (optional) | **Argon2id** (memory-hard) |
| Cipher | AES-128-CTR (unauthenticated) | **XChaCha20** (256-bit, 192-bit nonce) |
| Integrity | separate keccak-256 MAC | **Poly1305 AEAD tag** (intrinsic) |
| Interop | Geth / MetaMask / Foundry | Deckard-only |

OWASP's Password Storage Cheat Sheet lists **Argon2id first** (minimum 19 MiB memory, `t=2`, `p=1`), names scrypt as the fallback, and reserves PBKDF2 (600,000+ iterations) for FIPS-140 compliance [4]. Argon2id resists GPU/ASIC cracking far better than PBKDF2 or scrypt. XChaCha20-Poly1305 is an AEAD: the Poly1305 tag authenticates the ciphertext intrinsically (so no separate keccak MAC), and the 192-bit XChaCha nonce can be randomly generated without collision worry, unlike AES-CTR's 128-bit IV [5]. This is the same modern construction family as `age` and libsodium `secretbox`. (Note: "stronger" and "below the field floor" are well-grounded engineering judgments rather than spec-verifiable facts, but they follow directly from the primary evidence.)

The cost is **portability**: a Deckard keystore cannot be opened by Geth, MetaMask, or Foundry. The field-standard mitigation is to (a) ship a **BIP-39 mnemonic backup** — the true cross-wallet portability layer — and optionally (b) offer a **Web3-Secret-Storage (scrypt + AES-128-CTR) export** so users can recover into any standard wallet [1][8].

## The v0 plaintext-hex problem

Persisting the raw secp256k1 private key as plaintext hex in the OS config dir is below the universal field floor. MetaMask and Rabby keep the seed+keys in an encrypted "vault" blob — `browser-passworder` derives an AES key from the password via PBKDF2 and encrypts with AES-GCM — unlocked by a password and only briefly held in memory during signing [25][26]. Geth, Foundry/`cast`, and ethers all write Web3-Secret-Storage JSON [1][3]. Any local-disk read (malware, backup sync, lost laptop, shoulder-surf of the config file) is instant total compromise.

**Corrected (verified against the primary `browser-passworder` source):** an earlier claim that "MetaMask uses PBKDF2 with only 10,000 iterations" is **outdated**. MetaMask's `browser-passworder` repo shows 10,000 as the *legacy* `OLD_DERIVATION_PARAMS`; since v4.2.0 (Nov 13, 2023) the library default jumped to 900,000 and the **extension was configured to 600,000** iterations to match OWASP's 2023 guidance [25][27]. The 10,000-iteration figure was a real *historical* weakness (and the mobile app historically used ~5,000 with AES-CBC), but the present-tense framing is wrong for current MetaMask. The broader point stands: Argon2id is a stronger KDF than PBKDF2 for the same UX [4].

## Seed handling: BIP-39 / BIP-32 / BIP-44

BIP-39 encodes entropy (128–256 bits, a multiple of 32) plus a SHA-256 checksum (ENT/32 bits) into 11-bit indices over a 2048-word list (128 bits → 12 words, 256 → 24 words) [28]. The seed is then `PBKDF2-HMAC-SHA512(password = mnemonic, salt = "mnemonic" + passphrase, 2048 iterations, 64-byte output)` [28]. The optional passphrase (the informal "25th word") yields a completely different wallet tree for each value — useful plausible-deniability UX, with the harsh property that there is **no recovery if forgotten** [28]. BIP-32 turns the seed into an HD key tree; BIP-44 defines `m/purpose'/coin_type'/account'/change/address_index`, with Ethereum at `m/44'/60'/0'/0/0`. Because all EVM chains share `coin_type 60` and identical address/signature schemes, one mnemonic reproduces the same `0x` address everywhere. For Deckard, BIP-39 backup is the portability/recovery layer (importable into MetaMask/Ledger) and the passphrase is a cheap optional defense-in-depth feature.

## OS-level protection: Keychain, Secure Enclave, and the secp256r1 wall

Apple's Secure Enclave (SEP) generates and holds EC keys with optional Touch ID access control, but **only on the NIST P-256 (secp256r1) curve** — `SecureEnclave.P256` is the only EC type it exposes [12][13]. Ethereum uses secp256k1, which the SEP cannot hold or sign with. So you cannot put an Ethereum EOA key in the Secure Enclave. Two realistic patterns:

1. **Pragmatic ("Touch ID later")**: use the Keychain / Secure Enclave to protect a *wrapping key or passphrase* that decrypts the Argon2id+XChaCha keystore; the actual secp256k1 key lives in the encrypted file [12].
2. **Smart-account route**: a P-256 SEP key becomes an on-chain signer verified via RIP-7212/EIP-7951 — requires account abstraction Deckard doesn't have [14][15].

On Linux the equivalent unlock-secret store is the freedesktop Secret Service API (GNOME Keyring / KWallet over D-Bus).

For Rust, `keyring-rs` (v4.0.1, May 2026) is the cross-platform credential store (macOS Keychain, Windows Credential Manager, Linux/BSD Secret Service); its macOS backend uses the **login Keychain, not the Secure Enclave**, and is not biometric-gated by default [29][30]. The v4 README advises depending on `keyring-core` + per-platform store crates rather than the umbrella crate. For SEP / Touch-ID-guarded P-256 keys you need the **experimental** `iqlusioninc/keychain-services.rs` (a thin wrapper over Keychain Services / `SecAccessControl`), explicitly flagged as possibly having memory-safety bugs [13]. A sound design: store the keystore-unlock secret (not the raw key) in keyring/Keychain for the no-passphrase-each-time UX, and reserve `keychain-services.rs` for Phase-2 Touch ID.

## Rust signing & keystore ecosystem

`alloy-signer-local` (v2.x) is the canonical signer: the default `PrivateKeySigner` uses the pure-Rust `k256` crate; an optional `secp256k1` (libsecp256k1 C-bindings) backend produces identical signatures; there's also a YubiHSM2 signer [7]. Encrypted keystores sit behind the `keystore` feature, which wraps the `eth-keystore` crate (Web3 Secret Storage, scrypt for encryption, scrypt+pbkdf2 for decryption, AES via `aes`/`ctr`); the `mnemonic` feature enables BIP-39 [7][8]. The `eth-keystore-rs` crate is minimalist (latest 0.5.0, Apache-2.0, low activity) and has **no Argon2id/XChaCha support** [8][31].

**Key gap for Deckard**: alloy/`eth-keystore` give scrypt + AES-128-CTR out of the box, but the Argon2id + XChaCha20 envelope is a custom layer built with RustCrypto's `argon2` + `chacha20poly1305` + `zeroize`, feeding decrypted bytes into alloy's `PrivateKeySigner` — and keep `eth-keystore` for an export path [7][8][9].

The primitives are audited/mature: `k256` is constant-time secp256k1 (NCC Group's 2023 Entropy/Rust review found two high-severity issues, since fixed — so pin a current version); `chacha20poly1305` was NCC-audited with no significant findings and runs in constant time; RustCrypto's `argon2` is the reference Argon2id; `zeroize` performs volatile, non-optimizable wiping via `write_volatile` + atomic fences (but cannot defend against Spectre-class microarchitectural leakage) [9][10][11]. Wrap in-memory keys/seeds in `Zeroizing<...>`, and prefer `k256` (no C toolchain) unless libsecp256k1 perf is needed.

## Hardware wallets and the stronger single-key fix

Ledger and Trezor hold keys in a certified Secure Element: Ledger uses ST33 chips at CC EAL5+/EAL6+; Trezor Safe 3/5 use Infineon OPTIGA Trust M (V3) at EAL6+, with the Trezor Safe 7 (2025/26) adding the open/auditable TROPIC01 element [32][33]. The SE enforces a PIN without storing it. (Caveat, March 2025: Ledger researchers showed Trezor still runs crypto on the general MCU, a voltage-glitch surface [43].) Both ship EIP-712 typed-data display; Trezor's Sept 2025 firmware added EIP-712 message-hash display [34]. For a desktop EOA wallet, hardware-wallet support is the strongest available single-key-risk reduction.

## Clear signing: EIP-712 and ERC-7730

EIP-712 lets dapps present typed structured data so wallets can show fields instead of an opaque hash, but type info alone isn't enough to render safe human intent. **ERC-7730** (Draft, created Feb 2024, Ledger-led, authors Castillo/Aoun et al.) standardizes a JSON "clear-signing" descriptor for both EVM calldata and EIP-712 messages, with `context`, `metadata`, `display`, and `includes` sections [23][24]. A public registry (`ethereum/clear-signing-erc7730-registry`) holds descriptors and is deliberately treated as untrusted, recommending cryptographic provenance + multi-party governance [35]. For an operator-wallet, ERC-7730 descriptors are the mechanism to show the user *and the LLM* what a transaction means before an autonomous signature.

## Higher up the stack: MPC/TSS, TEEs, passkeys, smart-EOAs

The institutional/embedded-wallet field removed single-key risk two ways [19][20][21][22]:

- **TEE-isolated signing**: Turnkey decrypts and signs inside AWS Nitro secure enclaves with attestation; raw keys never leave, and transaction policies (limits, multisig, roles) are enforced *inside* the TEE [19]. Privy combines AWS Nitro TEEs with Shamir Secret Sharing (a 2-of-2 enclave-share / auth-share model, reconstructed only ephemerally in-enclave) — acquired by Stripe June 2025 [20].
- **MPC/TSS where the key is never reconstructed**: Web3Auth tKey uses 2/3 SSS (device / OAuth-network / recovery shares) with TSS producing partial signatures; Lit Protocol uses DKG + threshold TSS across nodes, minting each key as a Programmable Key Pair (ERC-721) [21][22].

The TEE-plus-policy-engine pattern maps almost exactly onto the operator-wallet vision: enforce spending/action policy in a trusted boundary the LLM cannot bypass.

**Passkeys as on-chain signers**: WebAuthn / Secure Enclave / Android Keystore all sign secp256r1. On-chain verification needs a P-256 verifier: **RIP-7212** (Final) — precompile at `0x100`, 3450 gas, 160-byte input `(hash, r, s, qx, qy)` — is live on Arbitrum (RIP-7212 support AIP'd in ArbOS 30, activated in ArbOS 31 "Bianca"), OP-Stack chains (Base/Optimism), Polygon zkEVM and others, making passkey verification roughly as cheap as `ECRECOVER` [14][36]. **EIP-7951** brings it to **mainnet** at `0x100`, 6900 gas, fixing two RIP-7212 edge cases (reject point-at-infinity; compare `r' ≡ r (mod n)`); it shipped in the **Fusaka hard fork, mainnet Dec 3 2025 (21:49:11 UTC, epoch 411392)** [15][16]. The flagship production user is **Coinbase Smart Wallet** on Base: owners are stored as `bytes` to allow both Ethereum-address and secp256r1 passkey owners, with signatures wrapped in a `SignatureWrapper`/`WebAuthnAuth` struct; the actual on-chain verifier (`base-org/webauthn-sol`) tries the RIP-7212 precompile and falls back to the open-source FreshCryptoLib Solidity verifier [37][38]. Passkeys can only be an *on-chain signer* through a smart account — for a plain EOA they're an excellent local-keystore unlock factor.

**EIP-7702 smart-EOA delegation** (Pectra, mainnet **May 7 2025**): a `SetCode` (0x04) transaction points an EOA at an implementation contract, and the EVM executes that code as the EOA without changing the address — enabling batching, gas sponsorship, and alt-auth with no migration [17][39]. But it broke the "EOAs cannot execute code" assumption: per **Wintermute's** research, within ~4 weeks **97% of mainnet 7702 delegations** pointed to copy-pasted sweeper/drainer contracts (the "CrimeEnjoyor" family), with individual losses of $1.54M and ~$146K to 7702 phishing [17][18][40]. (Wintermute later framed ~48% of 7702 *uses* as crime-linked — a different measure that shouldn't be conflated with the 97%-of-delegations figure [40].) Signing a 7702 authorization is signing away your account's code, so the wallet must surface the delegate target (ideally with ERC-7730 metadata) and warn on unknown delegates.

**Social recovery** uses guardians (a quorum of trusted addresses) to authorize a new signer. Argent pioneered it with a guardian quorum and a 36-hour delay during which the owner can `cancelRecovery` [41][42]. In 2025 it's delivered via account-abstraction modules (ERC-4337, ERC-7579). It is impossible on a plain EOA — it requires smart-account logic.

## What this means for Deckard

- Deckard v0's plaintext-hex key on disk is below the floor every comparable wallet meets; the locked Argon2id + XChaCha20-Poly1305 envelope closes the largest gap and uses primitives (RustCrypto `argon2` / `chacha20poly1305` / `zeroize`, alloy's `k256`) that are already audited and pure-Rust [4][5][9][10][11].
- The envelope is cryptographically ahead of the Web3 Secret Storage field standard but **not interoperable** with it; BIP-39 mnemonic backup is the genuine cross-wallet recovery layer, and an optional `eth-keystore` (scrypt + AES-128-CTR) export exists as a portability escape hatch [1][8][28].
- The Secure Enclave's secp256r1-only constraint means Touch ID can gate the *unlock secret* for the keystore today, but cannot hold the Ethereum key itself — full SEP/passkey signing is gated on account abstraction Deckard doesn't yet have [12][13].
- For the operator-wallet vision, the recurring industry pattern is a **policy engine inside a trust boundary the signer cannot bypass** (Turnkey/Privy TEEs); a local equivalent — enforced spending/action limits between the LLM and the signing key — is the analogous self-custodial control [19][20].
- **ERC-7730 clear-signing descriptors** are the natural source of machine-readable transaction intent for an LLM to reason about *before* an autonomous signature, complementing EIP-712 [23][24][35].
- The now-mainnet P-256 precompile (EIP-7951, Fusaka Dec 3 2025) and EIP-7702 delegation are the two infrastructure pieces that would let a future Deckard add passkey signers and smart-account features to the *same* EOA address — both also introduce new signing-time risks (7702 drainer phishing) the UI must surface [15][17][18].
- Hardware-wallet (Ledger/Trezor) support is the strongest off-the-shelf single-key-risk reduction available to a desktop EOA wallet, independent of any smart-account work [32][33].

## Open questions

- Should Deckard's keystore JSON adopt a versioned, self-describing header (KDF params, AEAD, nonce) so future migrations (e.g. Argon2id parameter bumps, or to a different AEAD) are backward-readable?
- What exact Argon2id parameters should Deckard ship for a desktop CPU profile — OWASP's 19 MiB / `t=2` / `p=1` floor, or a higher memory cost given desktop hardware headroom? [4]
- For the Touch-ID path, is the experimental `keychain-services.rs` mature enough to depend on, or should Deckard wrap the platform Security framework directly / via its own FFI? [13]
- What is the right Linux story for biometric/hardware-gated unlock, given Secret Service (GNOME Keyring / KWallet) has no Secure-Enclave equivalent?
- For the operator-wallet, where does the policy boundary live in a local-first app with no TEE — a separate signing process, OS sandbox, or a future hardware/enclave dependency? [19]
- If/when Deckard adds account abstraction, is EIP-7702 delegation on the existing EOA preferable to a fresh ERC-4337/7579 account, given 7702's address-preservation benefit but added delegation-phishing surface? [17][18]

## Sources

1. Web3 Secret Storage Definition (v3) — https://ethereum.org/developers/docs/data-structures-and-encoding/web3-secret-storage/ — (docs, high)
2. eth-keystore crate docs — https://docs.rs/eth-keystore — (docs, high)
3. Foundry — `cast wallet` reference (incl. `cast wallet decrypt-keystore`) — https://getfoundry.sh/cast/reference/wallet/ — (docs, high)
4. OWASP Password Storage Cheat Sheet (Argon2id / scrypt / PBKDF2) — https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html — (docs, high)
5. RustCrypto AEADs — chacha20poly1305 (NCC audit, constant-time AEAD) — https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305 — (github, high)
6. Foundry — `cast wallet` reference (incl. `cast wallet import`; Web3 Secret Storage v3, `--unsafe-password` flagged) — https://getfoundry.sh/cast/reference/wallet/ — (docs, high)
7. alloy-signer-local crate docs (k256 default; `keystore`/`mnemonic` features) — https://docs.rs/alloy-signer-local — (docs, high)
8. eth-keystore-rs GitHub (scrypt+pbkdf2, AES-128-CTR; no Argon2/XChaCha) — https://github.com/roynalnaruto/eth-keystore-rs — (github, high)
9. RustCrypto elliptic-curves — k256 (NCC audit 2023; constant-time secp256k1) — https://github.com/RustCrypto/elliptic-curves/tree/master/k256 — (github, high)
10. NCC Group Entropy/Rust Cryptography Review (2023-08-25; two high-sev k256 findings) — https://www.nccgroup.com/research-blog/public-report-entropyrust-cryptography-review/ — (other, high)
11. zeroize crate docs (volatile, non-optimizable memory wiping) — https://docs.rs/zeroize/latest/zeroize/ — (docs, high)
12. Apple — Protecting keys with the Secure Enclave (CryptoKit `SecureEnclave.P256`, P-256 only) — https://developer.apple.com/documentation/cryptokit/secureenclave/p256 — (docs, high)
13. keychain-services.rs (experimental macOS Keychain/SEP Rust bindings, Touch ID) — https://github.com/iqlusioninc/keychain-services.rs — (github, high)
14. RIP-7212 secp256r1 precompile spec (Final; `0x100`, 3450 gas) — https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md — (spec, high)
15. EIP-7951 secp256r1 mainnet precompile (`0x100`, 6900 gas, two security fixes) — https://eips.ethereum.org/EIPS/eip-7951 — (spec, high)
16. EF Blog — Fusaka Mainnet Announcement (mainnet Dec 3 2025, includes EIP-7951) — https://blog.ethereum.org/2025/11/06/fusaka-mainnet-announcement — (blog, high)
17. Zealynx — EIP-7702 wallet security (auditor view; SetCode 0x04, delegation phishing) — https://www.zealynx.io/research/smart-contracts/eip-7702-wallet-security — (blog, medium)
18. CertiK — Pectra EIP-7702 trust assumptions — https://www.certik.com/blog/pectras-eip-7702-redefining-trust-assumptions-of-externally-owned-accounts — (blog, medium)
19. Turnkey — Non-custodial key management (AWS Nitro enclaves, in-enclave policy) — https://docs.turnkey.com/security/non-custodial-key-mgmt — (docs, high)
20. Privy — Wallet security architecture (AWS Nitro TEE + Shamir Secret Sharing) — https://docs.privy.io/security/wallet-infrastructure/architecture — (docs, high)
21. Web3Auth Full MPC / tKey architecture (2/3 SSS + TSS) — https://hackmd.io/@torus/Hyv8HjO8i — (docs, medium)
22. Lit Protocol — 60 Days of Autonomous Signing (DKG + threshold TSS, PKPs) — https://spark.litprotocol.com/60-days-of-autonomous-signing/ — (blog, medium)
23. ERC-7730 Structured Data Clear Signing Format (Draft, Feb 2024, Ledger-led) — https://eips.ethereum.org/EIPS/eip-7730 — (spec, high)
24. Ledger — ERC-7730 v2 & the evolution of clear signing — https://www.ledger.com/blog-the-evolution-of-clear-signing — (blog, medium)
25. MetaMask browser-passworder source (OLD_DERIVATION_PARAMS=10k vs default 900k; AES-GCM) — https://github.com/MetaMask/browser-passworder/blob/main/src/index.ts — (github, high)
26. Rabby Wallet README (MetaMask-derived key management) — https://github.com/RabbyHub/Rabby/blob/develop/README.md — (github, high)
27. MetaMask browser-passworder releases (v4.2.0, Nov 13 2023; configurable KDF) — https://github.com/MetaMask/browser-passworder/releases — (github, high)
28. BIP-39 specification (PBKDF2-HMAC-SHA512, 2048 iters, salt "mnemonic"+passphrase) — https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki — (spec, high)
29. keyring-rs GitHub (cross-platform credential store; macOS = login Keychain, not SEP) — https://github.com/hwchen/keyring-rs — (github, high)
30. keyring crate docs (Secret Service backend) — https://docs.rs/keyring/latest/keyring/ — (docs, high)
31. alloy-rs/alloy (alloy-signer-local source) — https://github.com/alloy-rs/alloy — (github, high)
32. Trezor — Secure Elements in Trezor Safe devices (OPTIGA Trust M, EAL6+; TROPIC01) — https://trezor.io/learn/security-privacy/how-trezor-keeps-you-safe/secure-elements-in-trezor-safe-devices — (docs, high)
33. Ledger — Why Secure Elements matter (ST33, EAL5+/EAL6+) — https://www.ledger.com/why-secure-elements-make-a-crucial-difference-to-hardware-wallet-security — (docs, medium)
34. Trezor Suite/Firmware Sept 2025 update (EIP-712 typed-data display) — https://forum.trezor.io/t/update-trezor-suite-trezor-firmware-september-2025-update-is-here/24843 — (forum, medium)
35. ethereum/clear-signing-erc7730-registry spec (untrusted-registry model) — https://github.com/ethereum/clear-signing-erc7730-registry/blob/master/specs/erc-7730.md — (github, high)
36. Arbitrum AIP — Support RIP-7212 (ArbOS 30 deployment) — https://forum.arbitrum.foundation/t/aip-support-rip-7212-for-account-abstraction-wallets-arbos-30/23298 — (forum, high)
37. Coinbase Smart Wallet README (owners as bytes; secp256r1 passkey owners; SignatureWrapper) — https://github.com/coinbase/smart-wallet/blob/main/README.md — (github, high)
38. base-org/webauthn-sol — WebAuthn.sol (tries RIP-7212 precompile, falls back to FreshCryptoLib) — https://github.com/base-org/webauthn-sol/blob/main/src/WebAuthn.sol — (github, high)
39. EF Blog — Pectra Mainnet Announcement (mainnet May 7 2025) — https://blog.ethereum.org/2025/04/23/pectra-mainnet — (blog, high)
40. Protos — coverage of Wintermute's EIP-7702 research (delegation/crime statistics) — https://protos.com/48-of-ethereum-eip-7702-uses-linked-to-crime-says-wintermute/ — (other, medium)
41. Argent — How to recover my wallet with guardians (36-hour delay, cancelRecovery) — https://support.argent.xyz/hc/en-us/articles/360007338877-How-to-recover-my-wallet-with-guardians-onchain-complete-guide — (docs, high)
42. OpenZeppelin — Argent vulnerability report (recoveryPeriod / cancelRecovery / guardian model) — https://blog.openzeppelin.com/argent-vulnerability-report — (other, high)
43. The Block — Trezor discloses vulnerability in Safe 3 (March 2025; Ledger Donjon researchers, crypto on general MCU / voltage-glitch surface) — https://www.theblock.co/post/346018/trezor-discloses-vulnerability-safe-3-crypto-wallet-rival-ledger — (news, medium)
