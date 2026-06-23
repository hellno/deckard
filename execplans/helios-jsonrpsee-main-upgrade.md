# Helios Jsonrpsee Main Upgrade

## 1. Title

Helios Jsonrpsee Main Upgrade

## 2. Context

- Discovery and dependency-security work for Deckard's verified-read path.
- User asked whether a merged upstream Helios change that disables unused `jsonrpsee`
  packages unblocks Deckard from security notifications or local dependency comments.
- Affects `deckard-core`'s optional `verified-reads` dependency on `helios-ethereum`.

## 3. Source Of Truth

- User request in this workspace.
- Repo guidance: `AGENTS.md`, `/Users/hellno/.codex/RTK.md`, `PLANS.md`.
- External upstream: a16z/helios PR #804 and current `master` ref.
- Relevant code files: `crates/deckard-core/Cargo.toml`, `Cargo.lock`.

## 4. Current State Analysis

- Deckard pins `helios-ethereum` to tag `0.11.1` (`204c998a`).
- That Helios tag enables `jsonrpsee = { version = "0.19.0", features = ["full"] }`
  in `helios-core`, which pulls unused HTTP, WebSocket, and WASM client transport crates.
- Upstream PR #804 was merged after the tag and narrows Helios `jsonrpsee` features to
  `server`, `macros`, and `client-core`.

## 5. Target State

- Determine whether pinning Deckard to current Helios `master` compiles.
- Confirm whether the unused `jsonrpsee` transport crates leave the active Deckard graph.
- Do not change runtime behavior or verified-read trust labels.

## 6. Security And Trust Invariants

- No key material, seed phrases, passphrases, decrypted keystore data, or RPC secrets are logged,
  printed, or written to artifacts.
- Verified reads must continue to be tagged by `ReadStatus`; unverified reads are never displayed
  as verified.
- This dependency test must not add decrypt/sign capability to key-less crates.

## 7. UX And Design Constraints

- No UI work.

## 8. Execution Plan

1. Verify upstream Helios PR/ref and current Deckard pin.
2. Update `helios-ethereum` from tag `0.11.1` to current upstream commit for a local test.
3. Refresh `Cargo.lock`.
4. Inspect dependency graph for removed `jsonrpsee` transports and old `rustls-webpki`.
5. Run focused and required validation as time permits, recording failures honestly.

## 9. Validation Criteria

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Task-specific checks:

- `cargo update -p helios-ethereum`
- Dependency graph checks for `jsonrpsee-http-client`, `jsonrpsee-ws-client`,
  `jsonrpsee-wasm-client`, and `rustls-webpki 0.101`.
- At least a focused `deckard-core` check with `verified-reads`.

## 10. Failure Signals

- Deckard no longer compiles against Helios `master`.
- Removed client transports remain in the active dependency graph solely because of Helios.
- Lockfile gains unrelated dependency churn.

## 11. Risks And Tradeoffs

- Helios `master` is not a release tag, so a production pin should use an explicit commit and
  may still need a comment explaining why this is intentionally not tag `0.11.1`.
- Full DoD can be expensive because GPUI and ZK dependencies are large.

## 12. Out Of Scope

- Changing Helios runtime behavior.
- Solving unrelated advisories such as optional DNS resolver features unless the graph proves they
  are affected by this bump.

## 13. Status Notes

- 2026-06-23: Created plan after confirming upstream PR #804 and current branch hygiene.
- 2026-06-23: Pinned `helios-ethereum` to a16z/helios `43a8c9f3`; Cargo removed
  `jsonrpsee-http-client`, `jsonrpsee-ws-client`, `jsonrpsee-wasm-client`,
  `jsonrpsee-client-transport`, `hyper-rustls 0.24`, `rustls 0.21`, and
  `rustls-webpki 0.101`.
- 2026-06-23: `cargo tree` confirms active `jsonrpsee` features are only `server`,
  `macros`, and `client-core`; the old `jsonrpsee` client transport packages no longer
  match package ID lookups.
- 2026-06-23: `cargo deny check advisories` reported the three `rustls-webpki`
  ignores as unused, so `deny.toml` now drops them. The Hickory DNS ignores remain
  because Helios still enables `reqwest/hickory-dns`.
- 2026-06-23: Validation green:
  `cargo fmt --all --check`; `cargo check -p deckard-core --features verified-reads`;
  `just core`; `just check`; `cargo deny check advisories`;
  `TMPDIR=/tmp cargo test --workspace`.
- 2026-06-23: Plain `cargo test --workspace` failed once on macOS because the default
  `/var/folders/...` temp path made a test Unix socket exceed `SUN_LEN`; the exact test
  passed with `TMPDIR=/tmp`, as did the full workspace test run.
