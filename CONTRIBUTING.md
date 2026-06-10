# Contributing to Deckard

Thanks for your interest in Deckard — a native, self-custodial Ethereum wallet built in
Rust + GPUI for macOS and Linux. Contributions are welcome, and this guide should get you
from clone to your first green PR.

## Read this first: alpha, and security-sensitive

Deckard is **`0.0.1-alpha` — experimental, pre-1.0 software.** It is **not production-ready**.

This wallet holds private keys, BIP-39 seed phrases, and an encrypted keystore. That makes
every change here **security-sensitive**. Treat key material with care:

- **Testnet and throwaway keys only.** Do **not** run Deckard with real funds or real mainnet
  keys, and never paste a real seed phrase into it. No third-party security audit has been done yet.
- **Never log, `Debug`-print, or otherwise expose a seed, key, or passphrase.** Secrets live in
  `Zeroizing` and stay there.
- If you find a vulnerability, **do not open a public issue** — see
  [Reporting security issues](#reporting-security-issues) below.

## Getting started

### 1. Clone

```sh
git clone https://github.com/hellno/deckard
cd deckard
```

### 2. Toolchain

The Rust toolchain is **pinned in `rust-toolchain.toml`** (currently `1.95.0`, in lockstep with
Zed's GPUI). With `rustup` installed, the pinned toolchain (plus `rustfmt` and `clippy`) is selected
automatically when you build inside the repo — you don't need to switch channels by hand.

### 3. Install `just`

The task runner is [`just`](https://github.com/casey/just):

```sh
brew install just      # or: cargo install just
```

Everything in the `justfile` is plain `cargo` + macOS built-ins, so you can also run the underlying
commands by hand if you prefer.

### 4. Crate layout

Deckard is a **virtual Cargo workspace** — the root carries no `[package]`; all four crates live
under `crates/`:

| Crate | Role |
|---|---|
| `deckard-app` | The GPUI desktop app (the view layer / shell; binary `deckard`). |
| `deckard-core` | The headless engine: Ethereum provider + verified reads, balances, HD keys, encrypted keystore, key-less shield builder. **No GPUI dependency; fully unit-testable** — most logic belongs here, not in the app. |
| `deckard-contract` | The frozen wire contract (`Intent` / `Decision` / `Policy` / `RPC` / `ReadStatus`). |
| `deckard-signerd` | The process-isolated signer daemon — owns the key and gates writes over a Unix-domain socket. |

### 5. The inner loop

- **`just core`** — the fast inner loop. Clippy + tests for the GPUI-free engine (`deckard-core`)
  **without building the GPUI app.** The heavy verified-reads / shield deps compile once, then it's
  quick. Reach for this while iterating on the keystore, provider, balances, or shield builder.
- **`just check`** — the full lint pass (clippy `-D warnings` across the whole workspace **and** the
  app's `--features tray` config). Required for any UI work, and part of the Definition of Done below.
- **`just run`** — builds `deckard-signerd`, then runs the app (the signerd binary is spawned as a
  sibling). This is the command you'll use most for UI work.
- **`cargo test --workspace`** — runs the full test suite.
- **`just bundle`** — builds a distributable macOS `Deckard.app`.

> The fast `just core` loop is for iterating — the **full Definition of Done still applies before
> you're done.**

## Definition of Done

All of these must hold before a change is complete (reproduced verbatim from `AGENTS.md`):

1. `cargo fmt --all --check` clean
2. `just check` green (both feature configs)
3. `cargo test --workspace` green
4. No new/changed dependencies in `Cargo.toml` or `Cargo.lock` unless explicitly approved

**Never report a task complete while any of these is red. Paste the command output as evidence —
don't claim done while red.**

## Code constraints

The full rationale (what we enforce and *why*, plus the deliberately-rejected rules) lives in
[`docs/AGENTIC-ENGINEERING.md`](docs/AGENTIC-ENGINEERING.md). The summary:

**In `deckard-core`** (the trust core), enforced via crate-level `#![deny(...)]` — clippy fails the
build: **no `.unwrap()` / `.expect()` / `panic!` / raw slice indexing in non-test code.** Propagate
errors with `Result` / `?`, and parse untrusted bytes through the bounded `Reader` in `keystore.rs`.
The app crate may `unwrap` infallible GPUI handles; the engine must not. A genuinely-unrecoverable
boundary uses a scoped `#[allow]` + a `// reason` comment (see `eth.rs`) — never a bare `unwrap`.

**Workspace-wide**, enforced by `[workspace.lints]` + `clippy.toml` (CI fails the build):

- `todo!` and `dbg!` are denied; ignored `Result`s (`unused_must_use`) are denied.
- `deckard-core` is `#![forbid(unsafe_code)]`; the app crate is `unsafe_code = "deny"` (a new
  `unsafe` block needs a reviewed `// SAFETY:` comment + an explicit `#[allow]`).
- `std::mem::forget` / `core::mem::forget` and `rand::thread_rng` are denied — use `drop()` /
  `OsRng`.

**Always, every crate:** never log or `Debug`-print a seed, key, or passphrase. Secrets stay in
`Zeroizing`.

## Design changes

Before any visual or UI change, **read [`DESIGN.md`](DESIGN.md) first** — it is the source of truth
for fonts, color, spacing, the information architecture, and component states. A few load-bearing
rules:

- **Ground every design decision in real reference screenshots** (Linear, Conductor, Splits,
  Superhuman, Stripe), never in remembered descriptions — that's how the first drafts went wrong.
- **The two-signal actor model:** **amber = the human** ("where you are" / caution); **cyan = the
  agent** (the machine actor). The two never mean the same thing.

Don't deviate from `DESIGN.md` without explicit maintainer approval.

## Workflow

- **Branch off `main`**, do your work, and open a **PR back to `main`.**
- **Expect adversarial cross-model review.** Changes go through review by a second model (e.g. Codex
  GPT-5.5 as a cross-model reviewer) — design for that scrutiny.
- **Keep dependencies frozen.** No new or changed `Cargo.toml` / `Cargo.lock` deps without explicit
  approval. The git GPUI stack is bumped **only** via `just bump-gpui` — never hand-edit those pins.
- Satisfy the full **Definition of Done** before requesting review, and include the command output
  as evidence.

## Where to look

| File | What it is |
|---|---|
| [`AGENTS.md`](AGENTS.md) / `CLAUDE.md` | Guidance for coding agents (and a fast human orientation). |
| [`STATUS.md`](STATUS.md) | The honest, live status of what's built vs. partial vs. TODO. **Treat it as ground truth.** |
| `specs/` and `docs/build/` | The specs and build notes. |
| [`docs/AGENTIC-ENGINEERING.md`](docs/AGENTIC-ENGINEERING.md) | The *why* behind the lint policy, CI gates, and code constraints. |
| [`DESIGN.md`](DESIGN.md) | The design system — required reading before any UI change. |

### What works today (and what doesn't)

Per `STATUS.md` — don't overstate maturity:

- **Working:** encrypted BIP-39 keystore + onboarding; live on-chain balances (Multicall3); receive
  (address + QR); the command palette; the amber-on-near-black design system; Helios-verified reads
  (no third-party RPC trusted by default); the process-isolated signer daemon (`deckard-signerd`)
  that holds the key and gates writes; the shield hero (auto-private via Railgun), which is **wired
  and black-box tested on an anvil fork.**
- **Not done / partial:** the **Send** UI is gated ("next release"); **Swap** is TODO; the agent /
  MCP surface (`deckard-mcp`) is **not built**; the receive-watcher auto-detect is TODO. Some tests
  are `#[ignore]` (they need `anvil` + an archive RPC) — see the test caveats in `STATUS.md`.

## Reporting security issues

**Do not open a public issue for a security vulnerability.** Report it privately per
[`SECURITY.md`](SECURITY.md), or directly to the maintainer at **holgerniessner@me.com**.

## License

Deckard is licensed under **AGPL-3.0-or-later** (see [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE)).
It was forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which permits
relicensing). By contributing, you agree your contributions are licensed under AGPL-3.0-or-later.
