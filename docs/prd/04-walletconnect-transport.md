# PRD-04 — WalletConnect v2 transport (`deckard-wcd`)

> Phase 2 of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). The **primary generic
> dapp-connectivity transport**: reaches desktop today and mobile later, no browser extension to ship
> or get hijacked. **Gated on PRD-01 (resolver auth), PRD-02 (clear-signing v2), PRD-05 (per-origin
> permissions), and an external security audit** (`SECURITY.md`). Do not start before its blockers.

## Why this exists

When Deckard wants to connect to dapps *it didn't integrate natively* (PRD-03), it needs a generic
transport. Research (`research §8–15`) found WalletConnect v2 dominates the alternatives for Deckard:

- **Reaches mobile** (driver #1) — the only one of {extension, embedded webview, WalletConnect} that
  does. An eventual mobile app reuses the same transport; no rewrite.
- **No extension artifact** to maintain in two hostile stores or get supply-chain-hijacked
  (`research §6`).
- **E2E encrypted** (ChaCha20-Poly1305 / X25519); the relay cannot read sign/tx payloads
  (`research §9`).
- **Protocol-level scope** via CAIP-25 namespaces (chains × methods × accounts) — maps cleanly onto
  per-origin policy (PRD-05).

The costs are real and this PRD must confront them: a **centralized Reown relay + Project-ID
dependency** (privacy/offline-first tension, `research §10–11`) and **no maintained Rust wallet-side
SDK** — only the low-level relay client; the Sign protocol must be built in-house (`research §14`).

## Goals

- A **key-less proposer process, `deckard-wcd`**, mirroring `deckard-mcp`: it speaks WalletConnect to
  dapps and the existing `deckard-contract` wire to `deckard-signerd`. It holds **no key**, **cannot
  `resolve`** (PRD-01 guarantees that even if it tried), and submits only **typed** intents/messages
  through the PRD-02 builders — never raw arbitrary calldata that skips a typed path.
- Implement WC v2 pairing (`wc:` URI), sessions, and **CAIP-25 scope negotiation** restricting a
  connected dapp to approved chains × methods × accounts (PRD-05 owns the policy; this owns the wire).
- Map WC method calls → Deckard intents: `eth_sendTransaction` → `Intent`; `personal_sign` /
  `eth_signTypedData_v4` → `SignMessage` (PRD-02); `wallet_switchEthereumChain`/`addEthereumChain` →
  guarded per PRD-02. Refuse `eth_sign` and (v1) delegation.
- **Verify-API origin attestation** surfaced on the card as MATCH/UNVERIFIED/MISMATCH/THREAT — never as
  the sole defense (`research §13, 29`); the card still clear-signs real effects.
- **Relay-privacy posture**: document and implement what a privacy-focused wallet must do (below).

## Non-goals

- A browser extension (separate optional future PRD; explicitly secondary per ADR).
- Generic ABI decoding (PRD-02 scope: high-risk shapes + ERC-7730 + blind-sign fallback).
- Mobile app itself (driver #1: don't compromise desktop for it) — but **do not pick a design that
  forecloses mobile** (keep `deckard-wcd`'s core protocol logic platform-agnostic).

## Design

### Process topology (reuse the `deckard-mcp` shape — `docs/build/30-mcp-shape.md`)

```
dapp ──WalletConnect (relay, E2E)──► deckard-wcd ──deckard-contract wire──► deckard-signerd
                                     (key-less,                            (holds key, policy gate)
                                      no resolve)                                  │
                                                                  native clear-signing card ◄─ GPUI app
                                                                  (control channel, PRD-01)
```

`deckard-wcd` is to dapps what `deckard-mcp` is to LLM agents: a thin, key-less translator. Reuse the
same daemon socket, the same `Propose`/`Execute`/`Status` poll loop, and the same native approval card.

### Build vs borrow the WC Sign layer (decide first; record the decision)

No maintained Rust wallet SDK exists (`research §14`). Options, in preference order:
1. **Build the Sign protocol in Rust** on `WalletConnectRust` (relay client + RPC types). Most work;
   keeps everything native and dependency-light; full control of the crypto + scoping. **No new
   heavyweight deps without approval** (DoD #4) — the relay client + an X25519/ChaCha20 stack (some
   already in-tree via the keystore) need an explicit dependency review.
2. A separate sidecar in another language only if (1) is infeasible — but that reintroduces a non-Rust
   surface; avoid unless forced.

This PRD's first deliverable is a **spike** (mirror `spikes/`): pin `WalletConnectRust`'s exact surface,
prove a pairing + one `personal_sign` round-trip against a test dapp, and write the build/borrow
recommendation before committing the full implementation.

### Relay privacy (the offline-first tension, `research §10–11`)

- Route relay egress through the **same network path Deckard already uses** (so it inherits any
  proxy/VPN/Tor the user configures); document that the relay sees IP + timing + topic metadata.
- Rotate topics per session; don't reuse pairing topics.
- Expose a **custom relay URL** setting (for future self-host / testing), defaulting to the public
  relay, clearly labeled. Note the Project-ID analytics dependency honestly in-app and in docs.
- WalletConnect is **opt-in and off by default** — it is a network feature in an offline-first wallet;
  the user turns it on, sees the relay-dependency notice, and it never silently phones home.

### Scope & methods

- Negotiate CAIP-25 with **required namespaces empty/minimal** (per Reown guidance, `research §12`) and
  drive real scope from optional namespaces + Deckard's per-origin policy (PRD-05).
- Method allowlist per session; an out-of-scope method request is refused with a typed error and shown
  to the user.

## Acceptance tests

- Spike report committed: pairing + `personal_sign` round-trip proven; build/borrow recommendation.
- `wcd_is_keyless`: memory/fd scan of `deckard-wcd` finds no key (reuse the `deckard-mcp` red-team
  harness, `docs/build/00-test-harness.md`).
- `wcd_cannot_resolve`: `deckard-wcd` attempting `Resolve` is refused by PRD-01's control-channel gate
  (cross-PRD integration test).
- `method_out_of_scope_refused`: a dapp requesting a method outside the negotiated CAIP-25 scope is
  refused; nothing reaches the daemon.
- `sign_request_clear_signs`: a `personal_sign`/EIP-712 from a dapp renders the PRD-02 card with origin
  attestation state; off-chain danger flags fire.
- `relay_opt_in_default_off`: with WC disabled, no relay connection is opened (assert no socket/egress).
- Transcript hygiene extended to `deckard-wcd` (no secret/RPC-token leakage).

## Definition of Done

PRD-series DoD **plus**: ⌘K commands for connect/disconnect/list-sessions/STOP-all-sessions; the relay
privacy posture documented in `SECURITY.md` + `THREAT-MODEL.md` (new "WalletConnect transport"
surface); off-by-default verified; **an external audit sign-off recorded** before this ships beyond
testnet (per `SECURITY.md`). New deps (if any) explicitly approved and added to `deny.toml`/`Cargo.lock`
review.

## Risks & fallbacks

- **In-house Sign protocol is substantial.** Fallback: ship PRD-03 (native integrations) as the user
  value while this bakes; WC is not on the critical path to first value.
- **Relay centralization/availability** (`research §11`): the custom-relay-URL setting + opt-in posture
  contain it; track WC's decentralized-relay progress before relying on self-host.
- **Verify API is not bulletproof** (`research §13`): treat as one signal; the clear-signing card +
  per-origin policy (PRD-05) + blocklist are the real defenses.
- **Spoofable dapp metadata** (`research §13`): never let the claimed name substitute for decoded
  effects (`research §29`).

## Sources

`docs/research/10-dapp-connectivity.md §8–15, 29`; specs.walletconnect.com (pairing-uri, crypto-keys,
sign/namespaces, core/verify); docs.reown.com/cloud/relay; github.com/WalletConnect/WalletConnectRust;
`docs/build/30-mcp-shape.md` (the proposer pattern to mirror).
