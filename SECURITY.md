# Security Policy

Deckard is a native, self-custodial Ethereum wallet (GPUI + Rust; macOS + Linux).
Because it holds private keys, BIP-39 seed phrases, and an encrypted keystore, its
security posture matters more than almost anything else in the project. This document
states honestly what protects you, what does **not** yet, and how to report a problem
privately.

---

## ⚠️ Alpha warning — read this first

> **Deckard is `0.0.1-alpha`: EXPERIMENTAL, UNAUDITED, PRE-1.0 software.**
>
> **Do NOT store real funds in it. Do NOT use real mainnet keys or a seed phrase that
> controls real value.** Use **testnet keys or throwaway keys only.**
>
> No third-party security audit has been performed. This is not production-ready software.
> **Use it entirely at your own risk.** If you load a key that controls funds you care
> about, you may lose them.

This is alpha software released for evaluation and development. Treat every wallet you
create or import in Deckard as disposable until a release explicitly says otherwise.

---

## Supported versions

Security fixes are **best-effort during the alpha** and target only the latest alpha build.
There is **no LTS guarantee and no backport guarantee before 1.0** — APIs, the on-disk
keystore format, and the security model may all change without notice.

| Version       | Supported                          | Notes                                                     |
| ------------- | ---------------------------------- | --------------------------------------------------------- |
| `0.0.1-alpha` | ✅ Best-effort security fixes only | Latest alpha. No LTS. Testnet / throwaway keys only.      |
| `< 0.0.1`     | ❌ Unsupported                     | Pre-release / development snapshots.                      |

Always run the latest available build. Older alpha builds receive no fixes.

---

## Reporting a vulnerability

**Please report security issues privately. Do NOT open a public GitHub issue, pull
request, or discussion for a security vulnerability** — public disclosure before a fix is
available can put users' keys and funds at risk.

Use **either or both** of these private channels:

1. **GitHub private vulnerability reporting** (preferred) — go to the repository's
   **Security** tab → **Report a vulnerability**:
   <https://github.com/hellno/deckard/security/advisories/new>
2. **Email** the maintainer directly: **[contact-removed-see-SECURITY.md]**
   (PGP available on request — ask in your first message if you want to encrypt details.)

### What to include

To help us triage and reproduce quickly, please send:

- **Description** — what the issue is, in your own words.
- **Reproduction** — exact steps, environment (OS, build/commit), and any proof-of-concept.
- **Impact** — what an attacker can do (e.g. key/seed exposure, signing without consent,
  bypassing the policy gate, forging a "verified" read, keystore downgrade/tamper).
- **Suggested fix or mitigation**, if you have one.

### Our commitment

- **Acknowledgement:** best-effort, typically **within ~72 hours** during the alpha. Deckard
  is currently maintained by a single person, so please allow for time-zone and availability
  delays.
- **Coordinated disclosure:** we will work with you on a fix and a disclosure timeline. We
  ask that you give us a reasonable window to ship a fix before any public write-up, and we're
  happy to credit you in the advisory (or keep you anonymous — your choice).
- **No bounty program** exists during the alpha. We deeply appreciate responsible reports
  regardless.

Please act in good faith: test only against your **own** wallets / testnet keys, never
against other users' funds, and don't run destructive, privacy-invasive, or
denial-of-service tests against shared infrastructure.

---

## Security model — what actually protects you

These mechanisms are built and exercised in tests (with the honest caveats noted in the
section below). The architecture is deliberately defense-in-depth around the key.

- **Process-isolated signer daemon (`deckard-signerd`).** The private key lives in a
  **separate daemon process**, not in the GPUI app. The daemon owns the key and **gates
  every state-changing operation** (every write/signing request) behind an explicit
  policy check, communicating with the app over a **Unix domain socket (UDS)**. The UI
  cannot sign on its own — it can only *propose* an intent, which the daemon turns into a
  `Decision` and may execute. On lock/STOP the daemon **zeroizes** the in-memory key.

- **Verified reads via a Helios light client.** By default Deckard does **not trust a
  third-party RPC** for on-chain reads. Reads are validated against an embedded
  **Helios light client**, and the app surfaces a `ReadStatus` so you can see whether
  what you're looking at is light-client-**verified** or not. A lying or compromised RPC
  cannot silently feed you a false balance or state.

- **Keystore encrypted at rest.** The wallet secret is sealed with an authenticated
  envelope: a random data-encryption key (DEK) is wrapped by **Argon2id**(passphrase, salt)
  and the secret is encrypted with **XChaCha20-Poly1305**. The vault stores the **BIP-39
  entropy** (not the mnemonic string, not the 64-byte seed, not a derived key); the seed is
  re-derived on unlock. The vault header is fully **authenticated (AAD)** so a tamperer who
  can write the file cannot downgrade KDF parameters or flip flags without failing
  decryption. The file is written atomically at `0600`. A wrong passphrase fails closed
  with a single generic error (no oracle).

- **Secrets confined to `Zeroizing`, never logged.** Seeds, keys, the DEK/KEK, derived
  child keys, and the transient mnemonic are held in `Zeroizing` buffers and wiped when
  dropped. It is a hard project rule to **never log or `Debug`-print** a seed, key, or
  passphrase, anywhere, in any crate. On an explicit Lock/STOP the daemon zeroizes the
  in-memory key — a real zeroize, not just a UI route change.

- **A hardened trust core with a strict lint policy.** `deckard-core` (the engine that does
  provider/verified-reads, balances, HD-key derivation, the keystore, and the key-less
  shield builder) is **`#![forbid(unsafe_code)]`**. In `deckard-core`, **`.unwrap()`,
  `.expect()`, `panic!`, and raw slice indexing are denied** in non-test code — errors are
  propagated with `Result`/`?`, and untrusted bytes are parsed through a bounded reader with
  strict length/parameter caps (so a hostile keystore file can't OOM, hang, or corrupt state
  before the AEAD can reject it). Workspace-wide, **`todo!`, `dbg!`, ignored `Result`s
  (`unused_must_use`), `std::mem::forget`, and `rand::thread_rng` are denied** — the app
  crate is `unsafe_code = "deny"`, and randomness comes from `OsRng`. CI fails the build on
  any violation.

### Threat model, stated honestly

The keystore defends well against **cold theft of the vault file** (a stolen disk image,
Time Machine snapshot, backup, or world-readable copy → offline cracking). It does **not**
defend against **live malware running as your user during an unlocked session** — no
software hot wallet can, because the seed must enter RAM to sign. We minimize the
unlocked-RAM window (explicit Lock/STOP zeroizes the key), but it is inherent — and there
is **no automatic idle-lock yet** (see caveats below), so an unlocked session stays
unlocked until you lock it. A **weak passphrase** collapses the at-rest guarantee — beyond an
8-character minimum there is no passphrase-strength enforcement, so choose a strong one.

---

## Known limitations & honest caveats

We would rather under-promise. The following are real, current gaps — do not read past
them as solved:

- **No third-party security audit yet.** An external audit is a **planned, funded
  line item**, but it has not happened. Until it does, treat all guarantees above as
  *self-assessed*, not independently verified.

- **Single-maintainer review.** Deckard is currently designed, built, and reviewed by
  **one person** (with cross-model adversarial review as a supplement, not a substitute).
  **We are actively seeking a security co-maintainer.** If that's you, please reach out.

- **Some critical paths are only test-covered behind `#[ignore]`.** Several end-to-end
  tests (notably the shield flow and live-network paths) require `anvil` and an archive RPC
  and are marked `#[ignore]`, so they **do not run under a default `cargo test`**. Some
  read-path tests run against a *mocked* transport rather than live Helios, and some app
  tests use a *fake* recording daemon rather than the real signer + chain. See
  [`STATUS.md`](./STATUS.md) for the per-test caveats. The daemon STOP/zeroize and
  propose→`Decision`→execute tests **do** run by default.

- **Feature surface is partial — do not assume more than is built.** The encrypted
  keystore + onboarding, live on-chain balances, receive (address + QR), the command
  palette, Helios-verified reads, and the process-isolated signer daemon are working. The
  **Shield** hero (auto-private via Railgun) is **wired and black-box tested on an Anvil
  fork** — but **Send is gated** ("next release"), **Swap is a TODO**, the **agent / MCP
  surface (`deckard-mcp`) is NOT built**, and the **receive-watcher auto-detect is a TODO**.
  Do not rely on Send, Swap, MCP/agent automation, or the receive-watcher; they are not
  finished.

- **Supply-chain items are tracked, not closed.** Deckard vendors a native-only fork of
  `eip-1193-provider` (`vendor/eip-1193-provider`, to dodge a `wasm-bindgen` exact-pin
  conflict), and the upstream **Railgun** dependency's licensing is being resolved. Both are
  tracked items to settle before any non-alpha release.

- **No automatic idle-lock yet.** An automatic idle/auto-lock is *specified* (default
  15 min) in [`specs/keystore-design.md`](./specs/keystore-design.md) but is **not yet
  implemented** — no timer exists in any crate. Today the daemon zeroizes the in-memory key
  **only on an explicit Lock/STOP command**, never on a timer. Until idle-lock ships, an
  unlocked session remains unlocked (key in RAM) until you lock it yourself.

- **No Touch ID / biometric unlock yet.** Biometric unlock is a Phase-2 item blocked on the
  macOS codesign/notarize pipeline. v0 ships **passphrase-only** unlock (Argon2id). The
  passphrase is the durable ground-truth secret.

---

## License & provenance

Deckard is licensed **AGPL-3.0-or-later** (see [`LICENSE`](./LICENSE) and
[`NOTICE`](./NOTICE)). It was forked from the [`deck`](https://github.com/hellno/deck) GPUI
starter (0BSD, which permits relicensing). Source repository:
<https://github.com/hellno/deckard>.

---

*Last reviewed for `0.0.1-alpha`. This policy reflects an alpha, unaudited build and will be
tightened as the project matures toward 1.0. Nothing here should be read as a guarantee of
security for real funds.*
