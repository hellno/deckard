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

- **Agent surface — `deckard-mcp` (MCP sidecar + CLI).** One key-less binary that is both a
  CLI and an MCP stdio server for Claude Desktop, exposing the `mcp.v0.1` 6-tool profile:
  `deckard_wallet_address`, `deckard_wallet_balance`, `deckard_policy_get`, `deckard_shield`,
  `deckard_execute`, and `deckard_revoke_all`. It holds no key and never signs — every write
  is an `Intent` proposed to `deckard-signerd`, which enforces policy and signs. The raw
  `propose` and `simulate` tools specced earlier were cut from launch (see
  `docs/build/30-mcp-shape.md`). `wallet_balance` is public-only in v0.1 (the shielded field
  is an honest "unavailable" string, never a fake `0`).
- **Demo recipes + env contract.** `just demo` / `just demo-fund` / `just demo-check` stand up
  a forked Sepolia, a funded EOA, and the app against `policy.demo.json`. The app reads its
  configuration from `DECKARD_CONFIG_DIR`, `DECKARD_SOCKET_PATH`, `DECKARD_CHAIN_ID`,
  `DECKARD_RPC_URL`, `DECKARD_VERIFIED_READS`, and `DECKARD_DEMO_FORK_BLOCK` (documented in
  `CONTRIBUTING.md`), and shows a **"DEMO FORK — not mainnet"** banner when running on a fork.
- **`THREAT-MODEL.md`.** Documents the trust boundaries, the mainnet guardrail, and the one
  override env var (kept out of every reason string and tool response).

### Changed

- **`deckard-signerd` mainnet guardrail + Resolve flow.** On chain 1, every policy auto-`Allow`
  is downgraded to `NeedsApproval` and held `Pending`; a human resolves it through the app's
  hold-to-confirm (`Resolve`), so a prompt-injected agent can't move real funds hands-free
  within the caps. A daemon-side `Shield.to == RelayAdapt` pre-check landed as
  defense-in-depth.

### Fixed

- **Agent-path shields now show up on refresh.** The in-app refresh re-scans the shielded
  balance too (`refresh_portfolio` previously refetched only the public portfolio, so a
  shield driven through `deckard-mcp` stayed invisible until the next unlock).
- **The Atlas policy card renders the signer's live policy.** The agent home previously
  showed hardcoded placeholder values (a weekly budget, a session-key expiry, an autonomy
  line — none of which exist); it now fetches the daemon's policy via `PolicyGet` and
  renders the same fence `deckard_policy_get` shows an MCP client, including spent-today,
  the approval mode, and the STOP state.
- **README/CONTRIBUTING quickstarts now build `deckard-mcp` before invoking it.** The
  previous instructions called a binary that is not on `PATH` in a fresh clone.

### Security

- **Reason / RPC redaction.** Denial reasons and the logged RPC URL are sanitized so that
  RPC paths, userinfo, and API keys never leak into a reason string, a tool response, or the
  startup log. The mainnet-override env var is documented only in `THREAT-MODEL.md` and never
  printed.

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
