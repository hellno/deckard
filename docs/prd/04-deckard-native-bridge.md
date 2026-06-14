# PRD-04 — Deckard-native bridge (universal dapp reach, owned end-to-end)

> Phase 2 of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). Delivers "connect to **any**
> dapp" through a transport Deckard **fully owns** — our wire, our UX, local-first, **no third-party
> relay, no embedded browser, no store as a trust anchor**. This is the deliberate replacement for
> WalletConnect ([shelved](./x-walletconnect-shelved.md)). **Gated on PRD-01 (resolver auth),
> PRD-02 (clear-signing v2), PRD-05 (per-origin permissions), and an external audit** (`SECURITY.md`).

## Why this exists

The maintainer wants **universal reach** (users will want arbitrary dapps) **without** the "external
bad stuff": no bundled browser engine (rejected, ADR §1) and no WalletConnect (a centralized relay that
leaks metadata, a Project-ID dependency, and a QR/relay UX the maintainer rejects — `research §10–14`).

The resolution: dapps already speak **EIP-1193**. So we reach all of them by injecting a *standard*
EIP-1193 + EIP-6963 provider (`research §7, 21–22`) through a **first-party connector** in the user's
own browser, and carry requests over a **wire Deckard defines** to a key-less proposer that speaks the
existing `deckard-contract` wire to `deckard-signerd`. Same proven pattern as `deckard-mcp`
(`docs/build/30-mcp-shape.md`) — a thin, key-less translator — but for dapps, and entirely local.

The payoffs are exactly the vision: **no relay → no third-party metadata leak** (a real privacy win
over WalletConnect, on-brand offline-first); **native, instant approval cards → the UX we control**
(no QR dance, no relay latency); universal reach without becoming a browser.

## Goals

- A **key-less proposer process** (working name `deckard-bridged`) mirroring `deckard-mcp`: speaks the
  owned wire to the connector and the `deckard-contract` wire to the daemon. Holds **no key**, **cannot
  `resolve`** (PRD-01 enforces this even if it tried), submits only **typed** intents/messages via the
  PRD-02 builders — never raw arbitrary calldata that skips a typed path.
- A **first-party browser connector** that injects EIP-1193 + EIP-6963 (`rdns: sh.deckard` or similar),
  giving universal reach (every dapp works) without per-dapp integration.
- A **Deckard-owned local wire** between connector and `deckard-bridged`. **Native messaging is the
  default** (stdio host, gated by `allowed_origins`/`allowed_extensions`, **not web-reachable**) —
  chosen over a localhost RPC port precisely because a localhost port is reachable by any page via
  DNS-rebinding / cross-site WebSocket hijacking (`research §2–4`, the Frame weakness).
- Map dapp methods → Deckard surfaces: `eth_sendTransaction` → `Intent`; `personal_sign` /
  `eth_signTypedData_v4` → `SignMessage` (PRD-02); `wallet_switchEthereumChain`/`addEthereumChain` →
  guarded per PRD-02. **Refuse** `eth_sign` and (v1) EIP-7702 delegation.
- Per-origin scope enforced via PRD-05 (accounts × chains × methods).

## Non-goals

- WalletConnect (shelved) and embedded webview (rejected).
- **Mobile.** A browser-side connector is desktop-only. Mobile universal-reach is an explicit open
  problem (ADR Consequences), NOT solved here. Keep `deckard-bridged`'s core protocol logic
  platform-agnostic so a future mobile mechanism can reuse it, but do not build for mobile now
  (driver #1: don't compromise desktop for it).
- Generic ABI decoding (PRD-02 scope: high-risk shapes + ERC-7730 + blind-sign fallback).

## Design

### Process topology (mirror `deckard-mcp`)

```
dapp ──EIP-1193──► Deckard connector ──owned wire (native messaging)──► deckard-bridged ──contract wire──► deckard-signerd
       (any site)   (first-party,                                        (key-less,                         (holds key,
                     injects std provider)                                no resolve)                        policy gate)
                                                                                │
                                                          native clear-signing card ◄── GPUI app
                                                          (control channel, PRD-01)
```

`deckard-bridged` is to dapps what `deckard-mcp` is to LLM agents. Reuse the daemon socket, the
`Propose`/`Execute`/`Status` poll loop, and the native approval card unchanged.

### The owned wire (key decision — record the choice)

- **Default: native messaging** (Chrome `connectNative` / Firefox `runtime.connectNative`). The host is
  an OS-installed stdio binary gated by a manifest `allowed_origins` (exact extension id, no wildcards)
  and runs as the user (`research §3, 12`). It is **not reachable from web pages** — the decisive
  advantage over a localhost port. Caveats to handle: MV3 service-worker lifecycle (keep the port alive,
  reconnect on disconnect, `research §5`); the host-manifest registration lives in user-writable paths
  (`research §4`) — install it ourselves and document the integrity expectation.
- **Rejected sub-option: localhost RPC** (Frame's `ws://127.0.0.1`). Web-reachable; would require
  origin-allowlist + token + Host-header validation just to approach native-messaging's isolation
  (`research §2`). Only consider if a browser without native-messaging support must be served, and then
  only with those defenses.

### The first-party connector (trust is bounded, not store-based)

- Minimal, open-source, **key-less**: it injects a standard provider and relays JSON-RPC to the host.
  It holds no key, has no signing power, and **cannot reach the daemon's `Resolve`** (PRD-01).
- **"No store as a trust anchor":** browser stores (Chrome Web Store, AMO) are distribution channels we
  may use, but they are **not** where security comes from. The connector being a recurring attack target
  (`research §6`: Great Suspender, Cyberhaven) is *contained by design* — a fully compromised connector
  can only **propose**; it cannot sign, self-approve, or exfiltrate a key, and every effect is
  clear-signed (PRD-02) against attacker-controllable origin (`research §29`). Prefer self-distributed/
  self-signed where the browser allows (e.g. Firefox self-distribution); treat the store listing as
  convenience, not trust.
- Inject via **EIP-6963** (announce with a stable `rdns`), with `window.ethereum` as legacy fallback,
  so Deckard coexists with other wallets instead of fighting over the global (`research §7, 21–22`).

### UX (the explicit anti-WalletConnect)

- Connecting is a native flow: the dapp requests, Deckard raises a **native connect card** showing the
  (unverified) origin + requested scope (PRD-05); approval is local and instant — no QR, no relay.
- Every signing request renders the PRD-02 clear-signing card. Origin shown as unverified unless
  corroborated (PRD-05). Reuse `DESIGN.md` card anatomy, danger-early, hold-to-confirm.
- Sessions are listable and revocable from ⌘K + settings governance (next to "Pause all agents").

## Acceptance tests

- **Spike first** (mirror `spikes/`): prove a real dapp → connector → native-messaging host →
  `deckard-bridged` → daemon round-trip for `eth_requestAccounts` + one `personal_sign`, with the native
  card rendering. Commit a short report before the full build.
- `bridged_is_keyless`: memory/fd scan of `deckard-bridged` finds no key (reuse the `deckard-mcp`
  red-team harness, `docs/build/00-test-harness.md`).
- `bridged_cannot_resolve`: `deckard-bridged` attempting `Resolve` is refused by PRD-01's control-channel
  gate (cross-PRD integration test).
- `wire_not_web_reachable`: assert the chosen wire (native messaging) exposes no listening TCP/WS port a
  web page could reach; if a localhost fallback is ever used, a cross-origin page request without the
  token/Host check is rejected.
- `method_out_of_scope_refused`: a dapp method/chain/account outside the PRD-05 per-origin grant is
  refused; nothing reaches the daemon.
- `eth_sign_refused` / `delegation_refused`: dapp requests for `eth_sign` and EIP-7702 authorizations are
  refused (defense-in-depth with PRD-02).
- `eip6963_announced`: the connector announces via EIP-6963 with the stable `rdns` and coexists with a
  second injected provider.
- Transcript hygiene extended to `deckard-bridged` (no secret/RPC-token leakage).

## Definition of Done

PRD-series DoD (see [`README.md`](./README.md)) **plus**:
- ⌘K commands: connect, disconnect, list sessions, revoke session, STOP-all-sessions.
- The owned-wire choice (native messaging vs localhost) and the connector distribution/trust model are
  documented; the "store is not a trust anchor; trust is bounded by key-less + clear-signing" argument
  is written into `SECURITY.md` + `THREAT-MODEL.md` as a new "dapp bridge" surface.
- The desktop-only / mobile-open-question is recorded honestly.
- Off by default — the bridge is an opt-in network/browser feature in an offline-first wallet; it never
  installs a host or opens a connection silently.
- New dependencies (if any) explicitly approved and reviewed against `deny.toml`/`Cargo.lock`.
- **External audit sign-off recorded** before shipping beyond testnet (`SECURITY.md`).

## Risks & fallbacks

- **First-party connector is a standing supply-chain target** (`research §6`). Contained by design
  (key-less, no-resolve, clear-signed, bounded blast radius); keep it minimal and open-source; consider
  reproducible builds + signature pinning between connector and host.
- **Native-messaging host registration is user-writable** (`research §4`) — any same-uid code can
  overwrite the manifest. This is the same uid boundary the whole model already assumes; document it and
  lean on PRD-01 (the host is key-less and can't self-approve regardless).
- **MV3 lifecycle flakiness** (`research §5`) — reconnect logic + heartbeat; covered by the spike.
- **We're inventing a wire.** Keep it small and typed; reuse `deckard-contract` shapes wherever possible
  so the connector↔host protocol is a thin envelope, not a second contract.
- **Mobile remains unsolved** — acknowledged; revisit as a separate effort if/when mobile is funded.

## Sources

`docs/research/10-dapp-connectivity.md §1–7 (extension/bridge), §22–29 (boundary), §30–37 (signing &
scope)`; `docs/build/30-mcp-shape.md` (the proposer pattern to mirror); EIP-1193, EIP-6963;
developer.chrome.com native-messaging; github.blog localhost-dangers; the shelved WalletConnect
rationale ([`x-walletconnect-shelved.md`](./x-walletconnect-shelved.md)).
