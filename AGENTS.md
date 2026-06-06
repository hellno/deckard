# AGENTS.md — Deckard

Guidance for any coding agent (Codex, Claude Code, etc.) working in this repo. Claude Code also
reads `CLAUDE.md`; the two are kept in sync. Full rationale: `docs/AGENTIC-ENGINEERING.md`.

## What this is
Deckard is a native, self-custodial **Ethereum wallet** (GPUI + Rust, macOS + Linux). It is
**security-sensitive**: it holds private keys, BIP-39 seed phrases, and an encrypted keystore.
Treat key material with care.

## Layout
- `src/*.rs` — the `deckard` GPUI app (the view layer / shell).
- `crates/deckard-core` — the headless engine (Ethereum provider, balances, HD keys, encrypted
  keystore). **No GPUI dependency; fully unit-testable.** Most logic belongs here, not in the app.

## Commands
- Iterate fast: `cargo check -p deckard-core` (GPUI-free — seconds, not minutes).
- Lint: `just check` — clippy `-D warnings` on BOTH the default and `--features tray` configs.
- Format: `just fmt` (`cargo fmt`); CI gates `cargo fmt --all --check`.
- Test: `cargo test --workspace`.
- Bump the git GPUI stack: `just bump-gpui` (the ONLY way to change those pins).

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

## Design
Before any visual/UI change, read `DESIGN.md` (and the constraints in `CLAUDE.md`). Ground design in
the real reference screenshots, not remembered descriptions.
