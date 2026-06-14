# Dapp connectivity — PRD series

These PRDs execute [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). Each is written to be
picked up by an independent agent: motivation, scope, implementation guidance mapped to real crates/
files, and a Definition of Done tied to the project's gates (`docs/AGENTIC-ENGINEERING.md`).

**Definition of Done — applies to every PRD below** (paste command output as evidence; never claim
done while red):

1. `cargo fmt --all --check` clean.
2. `just check` green — clippy `-D warnings` on BOTH default and `--features tray`.
3. `cargo test --workspace` green.
4. No new/changed dependencies (`Cargo.toml`/`Cargo.lock`) unless explicitly approved in the PRD.
5. `deckard-core` crate-level lints respected (no `unwrap`/`expect`/`panic!`/raw indexing in non-test
   code; `#![forbid(unsafe_code)]`). New untrusted-byte parsing goes through a bounded `Reader`.
6. No secret ever logged or `Debug`-printed (seeds/keys/passphrases/viewing keys stay in `Zeroizing`).
7. Every new user-facing action has a ⌘K `Command` (`palette_commands.rs` + `Shell::run_palette_command`).
8. Any visual/UI surface matches `DESIGN.md` (clear-signing card anatomy, amber=human/cyan=agent,
   danger-early, hold-to-confirm).

## Dependency graph

```
PRD-01 Resolver auth ─────────────┐
                                  ├─► PRD-04 Deckard-native bridge ─► (Phase 2, post-audit)
PRD-02 Clear-signing v2 ──────────┤   (universal reach, owned wire)
        │                         └─► PRD-05 Per-origin permissions + registry
        └─► PRD-03 Curated native integrations (Phase 0, ships first)
```

| PRD | Title | Phase | Blocks on | Ships |
|-----|-------|-------|-----------|-------|
| [01](./01-resolver-authentication.md) | Resolver authentication (capability-gated `Resolve`) | 1a | — | independently; closes residual-risk #1 |
| [02](./02-clear-signing-and-message-intents.md) | Clear-signing v2 + message-signing intents | 1b | — | independently; needed by PRD-03 & PRD-04 |
| [03](./03-curated-native-integrations.md) | Curated native dapp integrations | 0 | PRD-02 (for permit/EIP-712) | first user-visible value |
| [04](./04-deckard-native-bridge.md) | Deckard-native bridge (universal reach, owned wire) | 2 | PRD-01, PRD-02, PRD-05 | post-audit |
| [05](./05-per-origin-permissions-and-registry.md) | Per-origin permissions, registry & anti-phishing | 2 | PRD-02 | with PRD-04 |
| [x](./x-walletconnect-shelved.md) | ~~WalletConnect transport~~ | — | — | **SHELVED / rejected** (rationale recorded) |

**Recommended execution order:** PRD-01 and PRD-02 in parallel (foundational, independent) → PRD-03
(first shippable value) → PRD-05 → PRD-04 (gated on an external audit per `SECURITY.md`).

**Connectivity model (ADR 0001, second-pass requirements):** Deckard pursues **universal dapp reach**
but **owns the transport end-to-end** — no embedded browser, no WalletConnect relay, no store as a
trust anchor. Reach comes from injecting a standard EIP-1193/6963 provider via a first-party,
key-less connector over a Deckard-owned local wire (native messaging). WalletConnect is shelved with
rationale; the embedded webview is rejected.
