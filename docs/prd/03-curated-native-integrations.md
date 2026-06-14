# PRD-03 — Curated native dapp integrations (Phase 0 connectivity)

> Phase 0 of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). The "curated/allowlisted dapps
> first" requirement, realized as **native Deckard surfaces** — no browser, no extension, no relay, no
> external proposer. This is the first user-visible value and the most secure interpretation of
> "connect to dapps."

## Why this exists

The maintainer chose **curated/allowlisted dapps first** (ADR driver #2). Taken to its logical, most
secure conclusion, the curated set doesn't need a generic *dapp-connection transport at all*: each
curated integration is a **native screen that builds a typed `Intent`/`SwapOrder` in Rust** and routes
it through the existing daemon → policy → clear-signing path — exactly how **Shield** (Railgun) and the
**CoW swap** path already work (`swap_order.rs`, `SignOrder`). This adds zero new attack surface, stays
fully on-brand ("calm sovereign control, not a casino"), works offline-first, and ships *before* the
Phase-2 connectivity machinery (PRD-04/05).

## Goals

- A small, vetted set of integrations as **native surfaces**, each producing a typed proposal the
  daemon already (or via PRD-02) understands. Initial set: **token swap** (extend existing) and **one
  bridge**. Each is a wallet action, reachable from ⌘K, with clear-signing on confirm.
- A **curated integration registry** in-repo (compile-time or signed config) listing the allowed
  protocols, their contract addresses per chain, and the `IntentKind`/order they build — the
  "allowlist" is the set of code paths we shipped, not a URL allowlist.
- Reuse `Policy.allow_swap_tokens` / `allow_to` fences; no new bypass.

## Non-goals

- Generic / open dapp access (PRD-04, the Deckard-native bridge). No `window.ethereum`, no external
  transport here — these are native Rust code paths only.
- Per-origin permissions (PRD-05) — there is no untrusted origin; the integration *is* Deckard code.
- New signing primitives beyond what PRD-02 adds (a swap needs Permit2/EIP-712 → that's PRD-02's job;
  this PRD consumes it).

## Design

### Integration shape

Each curated integration is a module under the app (or a small `deckard-integrations` area if it grows)
that:
1. Fetches the quote/route from the protocol's API over the existing verified-read/RPC path (reads
   only; never a signing path).
2. Builds a typed `Intent` (`Send`/`ContractCall`/`Shield`) or `SwapOrder` against **registry-pinned
   contract addresses** for the active `chain_id`.
3. Calls `Propose`/`ProposeOrder` → renders the clear-signing card → on hold-confirm,
   `Execute`/`SignOrder`.

The daemon's `calldata_ok` shape check and `evaluate`/`evaluate_order` fences apply unchanged — a
native integration is just another well-behaved proposer using the *typed* builders (never raw
arbitrary calldata, per the `deckard-mcp` rule).

### Curated registry

- A versioned, in-repo list: `{ protocol, chains: { chain_id: { contracts… } }, kind }`. Compile-time
  `const`/`include_str!` of a JSON checked into the repo (offline-first; no network fetch to learn what
  is allowed). If it must update without a release, ship it as a **signed** config (verify a maintainer
  signature before trusting — never an unsigned downloaded allowlist).
- Addresses are pinned and reviewed; a swap/bridge to an off-registry contract is impossible from this
  surface (it would require a code change + review).

### UI (`DESIGN.md`)

- Swap already exists; extend per the **Amount input** + **Clear-signing review card** specs.
- Bridge: a new contextual action on the wallet (Send/Receive/Swap siblings), same review-card confirm,
  same status-glyph activity rows. Bridges cross chains — the card must show source chain, destination
  chain, destination address, and asset, danger-early on any mismatch.

## Acceptance tests

- `swap_builds_pinned_addresses`: the swap path only ever targets registry-pinned contracts for the
  active chain; an attempt to target another address is rejected before propose.
- `bridge_proposal_clear_signs`: a bridge proposal renders source/dest chain + dest address on the card
  and routes through the normal approval path.
- `off_registry_denied`: a constructed proposal to a non-registry contract via this surface is refused.
- Existing swap parity tests (`crates/deckard-signerd/tests/swap_parity.rs`) stay green.
- Registry signature check (if signed-config path chosen): a tampered/unsigned registry is rejected
  loudly (mirror the `policy_store.rs` "loud fallback" discipline).

## Definition of Done

PRD-series DoD **plus**: each integration is a ⌘K `Command`; the curated registry and its trust model
(compile-time vs signed) are documented; the bridge card matches `DESIGN.md`; a screenshot in the PR.

## Risks & fallbacks

- **Protocol API as an input channel.** Quotes/routes from a protocol API are untrusted input — the
  daemon still re-derives/validates the typed proposal against pinned addresses and the policy fence;
  never sign what the API returns verbatim.
- **Bridge complexity** (two chains, longer settlement). v1: pick one well-understood bridge; show
  honest pending/failed states (`DESIGN.md` required states). Don't ship a bridge whose effects we
  can't clear-sign.
- **Scope creep toward "just open any dapp".** That is explicitly PRD-04 and gated on an audit; keep
  this surface curated-code-only.

## Sources

`docs/adr/0001-dapp-connectivity-architecture.md` (Phase 0); existing `swap_order.rs`, `SignOrder`,
`swap_parity.rs`, Railgun Shield path (`docs/build/10-kohaku-shield.md`); `DESIGN.md`.
