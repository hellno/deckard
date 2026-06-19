# AGENTS.md — Deckard

Guidance for any coding agent (Codex, Claude Code, etc.) working in this repo. Claude Code also
reads `CLAUDE.md`; the two are kept in sync. Full rationale: `docs/AGENTIC-ENGINEERING.md`.

## What this is
Deckard is a native, self-custodial **Ethereum wallet** (GPUI + Rust, macOS + Linux). It is
**security-sensitive**: it holds private keys, BIP-39 seed phrases, and an encrypted keystore.
Treat key material with care.

## Layout (virtual Cargo workspace — every crate lives under `crates/`)
- `crates/deckard-app` — the `deckard` GPUI desktop app (the view layer / shell; binary `deckard`).
- `crates/deckard-core` — the headless engine (Ethereum provider + verified reads, balances, HD keys,
  encrypted keystore, key-less shield builder). **No GPUI dependency; fully unit-testable.** Most logic
  belongs here, not in the app.
- `crates/deckard-contract` — the frozen wire contract (Intent / Decision / Policy / RPC / ReadStatus).
- `crates/deckard-signerd` — the process-isolated signer daemon (owns the key; UDS server).
- `crates/deckard-wallet-client` — shared key-less signer client/account/chain/error primitives for local interfaces.
- `crates/deckard-mcp` — MCP/agent interface over shared wallet capabilities.
- `crates/deckard-browser-bridge` — EIP-1193 dapp/browser interface over shared wallet capabilities.

## Commands
- Iterate fast: `just core` — clippy + test the GPUI-free engine (`deckard-core`) without building the
  gpui app (the heavy verified-reads/shield deps compile once, then it's fast). Use while working on the
  engine; the full DoD still applies before done. UI work needs `just check`.
- Lint: `just check` — clippy `-D warnings` on the whole workspace + the app's `--features tray` config.
- Format: `just fmt` (`cargo fmt`); CI gates `cargo fmt --all --check`.
- Test: `cargo test --workspace`.
- Bump the git GPUI stack: `just bump-gpui` (the ONLY way to change those pins).

## Branch hygiene — required before edits
Before changing files, always establish the current branch and its source-of-truth status:

1. Run `git status --short --branch`.
2. If on `main`, run `git fetch origin --prune`, fast-forward from `origin/main`, then create a new
   feature branch before editing.
3. If not on `main`, run `git fetch origin --prune` and check whether the branch has already been
   merged into current `origin/main`.
   - If it has been merged, switch back to `main`, fast-forward from `origin/main`, and create a new
     feature branch before editing.
   - If it has not been merged, inspect the branch's upstream/ahead/behind state and update local
     state before editing. Do not stack unrelated work on a stale or merged branch.
4. If there are uncommitted changes, identify whether they are user changes before switching,
   rebasing, stashing, or applying patches.

## Definition of done (all must hold; show command output as evidence)
1. `cargo fmt --all --check` clean
2. `just check` green (both feature configs)
3. `cargo test --workspace` green
4. No new/changed dependencies in `Cargo.toml` or `Cargo.lock` unless explicitly approved

Never report a task complete while any of these is red.

## Code constraints (see docs/AGENTIC-ENGINEERING.md for the full rationale)
**Enforced workspace-wide** (CI fails the build): `todo!` / `dbg!` denied; `unused_must_use` denied;
`deckard-core` is `#![forbid(unsafe_code)]` and the app crate is `unsafe_code = "deny"` (a new `unsafe`
block needs a reviewed `// SAFETY:` comment + explicit `#[allow]`); `std::mem::forget` /
`core::mem::forget` / `rand::thread_rng` denied (use `drop()` / `OsRng`).

**Enforced in `deckard-core`** via crate-level `#![deny(...)]`: no `.unwrap()` / `.expect()` / `panic!` /
raw slice indexing in non-test code — propagate with `Result` / `?` and parse untrusted bytes through the
bounded `Reader` in `keystore.rs`. The app crate may `unwrap` infallible GPUI handles; the engine must
not. Genuinely-unrecoverable boundaries use a scoped `#[allow]` + `// reason` (see `eth.rs`), never a
bare `unwrap`.

**Always:** never log or `Debug`-print a seed, key, or passphrase. Secrets live in `Zeroizing`.

## Language
In docs, UI copy, and refusal/error strings: prefer plain words over crypto jargon, and explain a
term once where it first appears (e.g. "shield — move funds to a private balance").

## Design
Before any visual/UI change, read `DESIGN.md` (and the constraints in `CLAUDE.md`). Ground design in
the real reference screenshots, not remembered descriptions.
