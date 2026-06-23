# Nix dev shell

## 1. Title

Add a flake-based Nix development shell for Deckard.

## 2. Context

Deckard's Linux development workflow needs native GUI/system libraries for GPUI and tray builds, plus Rust, Node/Playwright QA tooling, `just`, and Foundry for Anvil-backed tests. Earlier local setup used an ad hoc `nix-shell -p pkg-config gtk3 xdotool ...` wrapper because system-wide `apt` installs required sudo. This task makes that local environment explicit and repo-local.

This is developer-environment / workflow work. It should not change wallet runtime behavior.

## 3. Source Of Truth

- User instructions: add `flake.nix` and `flake.lock`; no legacy `shell.nix` compatibility needed.
- Repo guidance: `AGENTS.md`, `PLANS.md`.
- CI dependencies: `.github/workflows/ci.yml` Linux system dependency list.
- Tooling source of truth: `rust-toolchain.toml`, `package.json`, `justfile`.

No conflicting sources found. `rust-toolchain.toml` remains the Rust version source of truth; the Nix shell should provide system/tooling dependencies, not replace the Rust pin.

## 4. Current State Analysis

- The repo has no `flake.nix`, `flake.lock`, or `shell.nix`.
- Local Nix is installed with flakes enabled.
- CI installs Linux GPUI/tray dependencies via `apt`.
- Node QA scripts exist for Playwright extension and WalletBeat lanes.
- Foundry is used by tests/demos; this workstation already has Foundry under `~/.foundry/bin`.

## 5. Target State

- `nix develop` opens a usable Deckard dev shell on Linux.
- The shell includes the native libraries/tooling needed for `just check` and normal QA commands.
- `flake.lock` pins nixpkgs for reproducibility.
- No `shell.nix` is added.
- No Rust, Cargo, npm, or application dependency lockfiles are changed.

## 6. Security And Trust Invariants

- Private keys, seed phrases, passphrases, and decrypted keystore material are never logged, copied into plans, or written to artifacts.
- The shell must not introduce real wallet material or secrets.
- The signer/key ownership boundaries remain unchanged.
- This task does not change wallet RPCs, chain behavior, signing, policies, or verified-read display.
- Real-value chains continue to fail closed according to existing code/tests.

## 7. UX And Design Constraints

No UI or visual changes. `DESIGN.md` is not applicable.

## 8. Execution Plan

1. Create a feature branch from current `main`.
2. Add this execution plan.
3. Add `flake.nix` with a Linux dev shell mirroring CI's system dependencies and local QA tooling.
4. Generate `flake.lock` with `nix flake lock`.
5. Verify `nix flake check` and at least one command through `nix develop`.
6. Run repo DoD commands if feasible from the shell; if a full command is blocked by environment/time, record exactly why.
7. Commit, push, and open a PR.

## 9. Validation Criteria

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Task-specific checks:

- `nix flake check`
- `nix develop --command just --version`
- `nix develop --command cargo fmt --all --check`
- Prefer `nix develop --command just check` and `nix develop --command cargo test --workspace` before PR if runtime permits.

Browser/WalletBeat QA is not required for this environment-only change unless the shell affects those commands directly; the shell should include Node/pnpm so those lanes are possible.

## 10. Failure Signals

- `nix develop` cannot evaluate or enter the shell.
- `nix flake check` fails.
- `cargo fmt --all --check`, `just check`, or `cargo test --workspace` fail due to missing libraries in the shell.
- The flake tries to replace `rust-toolchain.toml` as the Rust version authority.
- The change modifies application dependency lockfiles or wallet behavior.

## 11. Risks And Tradeoffs

- Nix package names may differ from Ubuntu package names; verification must prove the shell actually satisfies native linking.
- Foundry availability in nixpkgs may vary. If not included directly, the shell may preserve `~/.foundry/bin` on `PATH` and document that Foundry remains installed by Foundry's own toolchain.
- Full workspace checks may take significant time because Deckard has a heavy Rust dependency tree.

## 12. Out Of Scope

- Adding `shell.nix` legacy compatibility.
- Rewriting CI to use Nix.
- Packaging Deckard as a Nix app/package.
- Changing Rust, Cargo, npm, Playwright, or wallet runtime behavior.
- Adding or changing Playwright/WalletBeat tests.

## 13. Status Notes

- 2026-06-23: Created plan after pulling `origin/main`, reading updated `AGENTS.md` / `PLANS.md`, and confirming no Nix files currently exist.
- 2026-06-23: Added `flake.nix` and generated `flake.lock` from `nixpkgs/nixos-unstable`.
- 2026-06-23: Verified `nix flake check`, `nix develop --command cargo fmt --all --check`, `nix develop --command just check`, and `nix develop --command cargo test --workspace` all pass locally.
