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

- **Starter policy presets, chosen at launch** ([#135]). Set `DECKARD_POLICY_PRESET=<name>` to seed
  the agent's rulebook with one of three named starting points, all default-deny on the v1 policy
  schema: **shield-only** auto-allows shielding (shield — move funds into your own private balance)
  under the daily cap and denies send and swap, so the agent can only move funds into your private
  balance and never send to a third party or trade; **ask-me-everything** raises a human-approval
  card for every shield, send, and swap, so nothing auto-signs; and **locked** has no rules at all,
  so every action is denied (a frozen rulebook, distinct from the runtime STOP an operator flips in
  an emergency). The preset applies only on a first run with no `policy.json` yet — an authored
  `policy.json` always wins — and an unknown name logs a warning and falls back to the friendly
  default (shield auto-allowed, send and swap always carded, under the 0.2 ETH daily cap), which is
  also selectable explicitly as `default` (the same rulebook you get with no preset set). Live
  in-app switching between presets is not in this release.

### Changed

- **Policy is now versioned and default-deny** ([#135]). The `policy.json` file changed shape.
  It is now `{ "version": 1, "default": "deny", "daily_cap_wei", "auto_shield_min_wei", "rules": [...] }`,
  where each rule grants one kind of action (`send`, `shield`, `unshield`, `swap`, `contract_call`)
  and carries its own settings — for example a send rule's `approval` (`never` / `over_cap` / `always`),
  its per-transaction cap (`per_tx_cap_wei`), and its `recipients` (the string `"any"` or a list of
  allowed addresses). An action with no matching rule is denied. The old flat shape
  (`{ per_tx_cap_wei, allow_to, require_approval, … }`) and its `allow_to: []` "any recipient" default
  are removed; `recipients` replaces `allow_to`, and an omitted `recipients` now denies every send
  rather than allowing all of them.
- **A policy file with no `version` key is now rejected** ([#135]). Such a file is treated as a stale
  pre-v1 file: the daemon refuses it, logs loudly, and falls back to a most-restrictive deny-all policy
  that approves nothing — so the demo's auto-shield quietly stops working. **`just demo` now upgrades a
  legacy demo policy for you**: it detects a v0 `~/.deckard/demo/policy.json` (no `version` key), backs
  it up to `policy.json.v0.bak`, and installs the v1 file. A v1 or hand-edited file is left untouched.
  If you don't use `just demo`, reinstall by hand: `rm ~/.deckard/demo/policy.json` then `just demo`,
  or `cp policy.demo.json ~/.deckard/demo/policy.json`.

[#135]: https://github.com/hellno/deckard/issues/135

### Fixed

- **Bundled macOS app (`just bundle`) could not unlock the wallet** ([#134]). `cargo bundle`
  ships only the `deckard` binary, but the release signer resolver launches `deckard-signerd`
  exclusively as a provenance-verified sibling under `Contents/MacOS/` (finding C1 — no `$PATH`
  or env fallback). The daemon was therefore never found or spawned, the socket was never bound,
  and Unlock failed with `connect …/signerd.sock: No such file or directory`. `just bundle` now
  builds the daemon in release and co-bundles it next to `deckard`, so the `.app` is
  self-contained. (Downloaded — vs locally-built — `.app`s additionally need the
  `com.apple.quarantine` xattr stripped; see [`docs/RELEASING.md`](docs/RELEASING.md).)

[#134]: https://github.com/hellno/deckard/issues/134

## [0.0.1-alpha] - 2026-06-22

First tagged alpha. Deckard is a self-custodial wallet built around a key-less proposer
and a process-isolated signer: an agent (or the app) can propose a transaction, but only
the daemon holds the key, and on any real-value chain a human approves before it signs.
The trust mechanisms are built and de-risked and the core flows work end to end; this is
still experimental software. See [`STATUS.md`](STATUS.md) for the beat-by-beat state and
[`DESIGN.md`](DESIGN.md) for the design system.

Licensed under **AGPL-3.0-or-later** (see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE)).
Forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which
permits relicensing), now its own project.

Workspace (a virtual Cargo workspace, all crates under `crates/`): `deckard-app` (the GPUI
app, binary `deckard`), `deckard-core` (the headless engine: provider / verified reads,
balances, HD keys, keystore, and the key-less shield builder), `deckard-contract` (the
frozen wire contract), `deckard-signerd` (the process-isolated signer daemon), `deckard-mcp`
(the agent sidecar), `deckard-browser-bridge` (the dapp bridge), and `deckard-wallet-client`
(the socket client).

### Added

**Wallet and money**

- **Encrypted BIP-39 keystore.** Generate or import a seed; it is sealed at rest in an
  Argon2id + XChaCha20-Poly1305 envelope and never leaves the device.
- **Onboarding.** A four-step create flow: set a passphrase with a live strength meter, back
  up the recovery phrase behind a hold-to-reveal grid, verify it by position, then a Ready
  screen with your new, copyable address. A progress rail shows where you are.
- **Live on-chain balances**, read over an `alloy` provider and batched through Multicall3,
  with a public / shielded composition and a roughly 20-second auto-refresh on the wallet home.
- **Helios-verified reads.** Mainnet reads are checked against an embedded Helios light client
  instead of trusting one RPC, with a `ReadStatus` badge surfaced in the app.
- **Send native ETH** to a `0x` address or ENS name, with a clear-signing review.
- **Swap tokens via CoW Protocol**: a live quote, a minimum-received and slippage review, and
  an off-chain order that settles on the CoW orderbook.
- **Shield to a private Railgun balance** (the shield hero): move public ETH into a private
  balance through the key-less shield builder, signed and broadcast by the daemon.
- **Receive.** Your address plus a QR code, with one-click copy.

**Agents and oversight**

- **First-class agent surface.** Agents appear in the sidebar beside wallets; select one to
  see and edit its policy (per-transaction cap, daily budget, allowed actions and assets) and
  reach Pause, Rotate, Adjust, and Revoke.
- **Activity feed.** A daemon-backed, day-grouped audit log of every proposed, approved,
  denied, and executed action (who acted, the real amount, the on-chain result with tx hash,
  and whether it was auto- or human-approved), with a top STOP that revokes all agent
  authority and zeroizes the key.
- **`deckard-mcp` agent sidecar.** One key-less binary that is both a CLI and an MCP stdio
  server for Claude Desktop, exposing the `mcp.v0.1` six-tool profile (wallet address, balance,
  policy, shield, execute, revoke-all). It holds no key; every write is an intent the daemon
  gates and signs.
- **Headless agent runner.** `just demo-agent` drives a hands-free watch-and-shield loop
  against the live policy, respecting over-cap refusals and human denials.
- **Chain capability registry.** One source of truth in `deckard-core` for per-chain RPC,
  explorer, native asset, protocol support, and verified-reads trust tier. Launch refuses to
  start if the RPC reports a chain id that does not match.

**Dapp connectivity**

- **EIP-1193 browser bridge.** A local loopback bridge exposes `eth_chainId`, `eth_accounts`,
  and `eth_requestAccounts` to an unpacked browser extension, announces itself via EIP-6963 so
  dapps can discover it among other injected providers, and emits basic provider events.

**Interface**

- **Bundled fonts and an enforced design system.** The app ships Schibsted Grotesk for the UI
  and JetBrains Mono for money and addresses, and routes every screen through a shared
  `widgets.rs` vocabulary so address truncation, caution lines, and status glyphs cannot drift
  per file. The two-signal actor model (amber = human, cyan = agent) is wired throughout.
- **Editorial layout.** An oversized monospace balance hero, whitespace-and-hairline hierarchy
  instead of cards, and a compact agent presence row on the home that links to the agent's
  policy.
- **Command palette v2.** Fuzzy search across every command, arrow-key navigation, inline
  shortcuts, and frecency ranking. Every user-facing action is reachable from `⌘K`.

**Project and tooling**

- **Source-only release workflow.** A `v*` tag runs the full CI gate, then publishes a GitHub
  Release built from the version's CHANGELOG section. `scripts/release-check.sh` validates the
  tag shape, that every crate is at the tagged version, and that the changelog section exists.
- **Demo recipes.** `just demo` / `demo-fund` / `demo-deposit` / `demo-check` / `demo-bridge`
  stand up a forked Sepolia, a funded wallet, and the app against `policy.demo.json`, with a
  "DEMO FORK, not mainnet" banner when running on a fork.
- **QA harnesses.** Playwright suites that smoke-test the browser extension and the WalletBeat
  provider surface on bundled Chromium, deterministic and fund-free.
- **Supply-chain gates.** A blocking `cargo-deny` advisories gate, a bans / licenses / sources
  gate, and a daily off-PR re-scan that files tracking issues on new advisories.
- **Contributor surface.** `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`,
  `THREAT-MODEL.md`, `docs/RELEASING.md`, and GitHub issue and PR templates.
- **Agentic-engineering policy.** A workspace-wide lint / CI policy that denies `todo!`,
  `dbg!`, and ignored `Result`s, with a documented Definition of Done.

### Changed

- **The confirm gesture is a `⌘↵` key-cap, not hold-to-confirm.** A short arm-delay keeps a
  keypress carried over from the previous screen from approving a money move.
- **Clear-signing review is transaction-as-hero.** The amount dominates in monospace, the
  recipient and network are prominent, fees and routes are quiet, and irreversibility reads in
  red.
- **The agent guardrail defaults to deny on every real-value chain.** On mainnet, every L2
  mainnet, and any unknown chain id, an auto-`Allow` is downgraded to a human approval.
  Sepolia and local anvil stay hands-free by design; adding a new real chain can never silently
  turn the brake off.
- **Daemon error reasons are redacted at the boundary.** RPC URLs are scrubbed to
  `scheme://host` so API keys never reach an agent transcript, and the Railgun viewing key is
  held in `Zeroizing` with a redacting `Debug`.
- **`cargo-deny` yanks warn instead of block**, so a yanked transitive dependency does not gate
  unrelated PRs; real advisories still block immediately.

### Fixed

- **Agent-path shields show up on refresh.** The refresh button resyncs the shielded balance
  too, so a shield driven through `deckard-mcp` is visible immediately instead of at the next
  unlock.
- **The Atlas policy card renders the daemon's live policy** (cap, daily budget, spent today,
  allowlist, approval mode, STOP state) instead of hardcoded placeholders.
- **Swap confirms in one hold.** It waits for the relayer approval to mine, then submits,
  instead of asking you to approve and then hold a second time.
- **Repeating the same swap amount works.** The replay guard treats an idempotent approval
  differently from a send or shield, which stay strictly double-spend guarded.

### Security

- **Process isolation.** The signing key lives only inside `deckard-signerd`, reached over a
  `0600` Unix domain socket; the GUI never holds it. STOP zeroizes the in-memory secret.
- **Capability-gated approval.** The daemon honors a human `Resolve` only on a private socket
  pair inherited from the app, so a same-uid process can propose intents but cannot self-approve.
- **Hardened daemon launch.** The daemon is spawned with a cleared environment (no
  `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES` / inherited `$PATH`) and, in release builds, only after
  verifying the one canonical daemon binary beside the app. This closes loader-injection and
  binary-substitution paths to the key.
- **Durable daily spend cap.** The cap is written before signing (reserve-before-sign), survives
  a restart, rolls over forward-only on the UTC day, and is not reset by a daemon crash-loop.
- **Keystore fails closed on CSPRNG failure.** Every secure random fill returns an error rather
  than panicking if the OS CSPRNG is unavailable, so onboarding degrades gracefully instead of
  crashing mid-backup.
- **Verified reads by default** on mainnet (Helios), never trusting a single third-party RPC.
- **Keystore at rest** in an Argon2id + XChaCha20-Poly1305 envelope; **secrets stay in
  `Zeroizing`** and are never logged or `Debug`-printed.
- **Frozen Deny-reason vocabulary.** Refusals route through roughly thirty documented `const`
  tags, with a build-time scan that rejects raw literals, so an agent can recover against a
  stable, typo-free vocabulary.
- **`#![forbid(unsafe_code)]` in `deckard-core`** (the trust core); the app crate sets
  `unsafe_code = "deny"`.

### Notes

- **Definition of Done** (all must hold): `cargo fmt --all --check` is clean; `just check` is
  green (clippy `-D warnings` on both the default and `--features tray` configs);
  `cargo test --workspace` is green; no new or changed dependencies without explicit approval.
- **Commands:** `just run` (build signerd + run the app), `just core` (the fast engine loop),
  `just check` (lint both configs), `cargo test --workspace`, `just demo` (forked-Sepolia demo),
  `just bundle` (a macOS `.app`), and `just release-check <tag>`. The toolchain is pinned in
  `rust-toolchain.toml`.

### Known limitations

This is an **alpha**: not production-ready and **not safe for real funds**. Use testnet or
throwaway keys only, never a real mainnet key. No third-party security audit has been done.

- **Source-only.** No prebuilt binary is shipped; build from source. The manual macOS `.app`
  path is unsigned and testnet-only (see [`docs/RELEASING.md`](docs/RELEASING.md)). Signed,
  notarized builds are tracked separately.
- **Verified reads are mainnet-only.** Other chains read from a trusted RPC and never wear the
  verified badge; the guardrail treats them, and unknown chains, as real-value by default.
- **The browser bridge is a scaffold.** It answers `chainId` / `accounts` / `requestAccounts`
  for discovery; it does not yet sign or send a transaction over the bridge.
- **Touch ID unlock is not available yet** (it depends on a signed build).
- **Inbound funds are not auto-detected in the app.** The headless agent runner watches for
  deposits, but the app has no on-screen receive-watcher.
- **Network-dependent tests are `#[ignore]`.** The `shield_e2e` and swap suites need a local
  `anvil` plus an archive RPC and are not run by default `cargo test`; see the test caveats in
  [`STATUS.md`](STATUS.md).

[Unreleased]: https://github.com/hellno/deckard/compare/v0.0.1-alpha...HEAD
[0.0.1-alpha]: https://github.com/hellno/deckard/releases/tag/v0.0.1-alpha
