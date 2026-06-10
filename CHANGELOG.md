# Changelog

All notable changes to **Deckard** — a native, self-custodial Ethereum wallet
(GPUI + Rust; macOS + Linux) — are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Pre-1.0 alpha.** Deckard is experimental software under active development. While
> pre-1.0, the public surface (APIs, the frozen wire contract, on-disk keystore format,
> CLI/commands) may change between releases without a major-version bump, per semver's
> `0.y.z` rules. **Do not use Deckard with real funds or real mainnet keys** — testnet
> or throwaway keys only. No third-party security audit has been performed.

## [Unreleased]

### Added

### Changed

### Fixed

## [0.0.1-alpha] - 2026-06-10

First tagged alpha. This is a security-sensitive, experimental wallet: the core
trust mechanisms are built and de-risked, but the end-to-end demo flow is not yet
wired together. See [`STATUS.md`](STATUS.md) for the authoritative, beat-by-beat
state of the build, and [`DESIGN.md`](DESIGN.md) for the design system.

Licensed under **AGPL-3.0-or-later** (see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE)).
Forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which
permits relicensing), now its own project.

Workspace layout (a virtual Cargo workspace, all crates under `crates/`):

- **`deckard-app`** — the GPUI application (binary `deckard`).
- **`deckard-core`** — the headless engine: provider / verified reads, balances,
  HD keys, keystore, and the key-less shield builder.
- **`deckard-contract`** — the frozen wire contract (`Intent` / `Decision` /
  `Policy` / RPC / `ReadStatus`).
- **`deckard-signerd`** — the process-isolated signer daemon.

### Added

- **Encrypted BIP-39 keystore + onboarding.** Self-custodial seed-vault flow that
  generates or imports a BIP-39 seed and stores it encrypted at rest.
- **Live on-chain balances.** Real balances over an `alloy` provider, batched through
  Multicall3.
- **Helios-verified reads.** Reads are verified against a Helios light client — **no
  third-party RPC is trusted by default** — with a `ReadStatus` badge surfaced in the
  app.
- **Receive.** Your address plus a QR code.
- **Command palette.** Keyboard-first navigation across the app.
- **Design system (`DESIGN.md`).** The amber-on-near-black visual language, including
  the two-signal actor model (amber = human, cyan = agent), wired into onboarding,
  portfolio, receive, palette, and settings.
- **Process-isolated signer daemon (`deckard-signerd`).** A separate process holds the
  key and gates every write over a Unix domain socket, with a policy gate, a
  `propose → Decision → execute` flow, and a STOP control that zeroizes the in-memory
  secret. The app talks to it through a socket signer client.
- **The shield hero (auto-private via Railgun).** The auto-shield mechanism — the
  `deckard-core` key-less shield builder plus daemon broadcast — is **wired and
  black-box tested on an anvil fork.** (See *Known limitations* for what is not yet
  reachable from the app or an agent.)
- **Agentic-engineering policy.** A workspace-wide lint / CI / supply-chain policy
  that denies `todo!`, `dbg!`, and ignored `Result`s, with a documented Definition of
  Done (see *Notes*).

### Security

- **Process isolation.** The signing key lives only inside `deckard-signerd`, a
  separate process reached over a Unix domain socket; the GUI never holds it.
- **Verified reads by default.** Chain reads are checked against a Helios light client
  rather than trusting any single third-party RPC.
- **Keystore at rest.** The seed is sealed in an Argon2id + XChaCha20-Poly1305
  envelope.
- **Secrets in `Zeroizing`.** Seeds, keys, and passphrases are held in zeroizing
  buffers and are never logged or `Debug`-printed.
- **`#![forbid(unsafe_code)]` in `deckard-core`** (the trust core); the app crate sets
  `unsafe_code = "deny"`.

### Notes

- **Definition of Done** (all must hold for a change to be considered done):
  1. `cargo fmt --all --check` is clean.
  2. `just check` is green — clippy `-D warnings` on **both** the default and
     `--features tray` configurations.
  3. `cargo test --workspace` is green.
  4. No new or changed dependencies in `Cargo.toml` / `Cargo.lock` unless explicitly
     approved.
- **Commands:** `just run` (build signerd + run the app), `just core` (the fast engine
  inner loop), `just check` (lint both configs), `cargo test --workspace`, and
  `just bundle` (build a macOS `.app`). The toolchain is pinned in
  `rust-toolchain.toml`.

### Known limitations

This is an **alpha**. It is **not** production-ready and **not** safe for real funds —
use testnet or throwaway keys only, and never a real mainnet key. No third-party
security audit has been done.

- **Send UI is gated** — marked "next release"; not available in this build.
- **Swap is a TODO** — the button is disabled.
- **No agent / MCP surface** — `deckard-mcp` is **not built**; only the wire contract
  and the daemon socket exist for it to build on.
- **Receive-watcher auto-detect is a TODO** — inbound funds are not auto-detected and
  the shield is **not** yet triggerable from the app or an agent. The shield hero is
  reachable only from the test/manual path, not on-screen.
- **Some tests are `#[ignore]`** — the network-dependent suites (notably
  `signerd/shield_e2e`) need a local `anvil` plus an archive RPC and are not run by
  default `cargo test`; the default-on `anvil_e2e` silently skips if `anvil` is
  missing, and some unit tests exercise mocked transports / a fake recording daemon
  rather than a live chain. See the test caveats in [`STATUS.md`](STATUS.md).

[Unreleased]: https://github.com/hellno/deckard/compare/v0.0.1-alpha...HEAD
[0.0.1-alpha]: https://github.com/hellno/deckard/releases/tag/v0.0.1-alpha
