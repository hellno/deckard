# Epic: Deckard v0 — native self-custodial operator wallet

> ⚠ **Live build status is in [`/STATUS.md`](../STATUS.md)** (single source of truth). The status fields in
> this epic are the *original plan* and are now stale (C1–C3 and much of v1 are built). Use this doc for the
> backlog/requirements, `/STATUS.md` for where things actually stand.

Status: spec (pre-implementation) — **see `/STATUS.md` for actual progress.** Source requirements: `SPEC-v0.md`. Strategy: `strategy.md`.
Target repo: fork of `hellno/deck`. License: AGPL-3.0. Chain: Ethereum mainnet only.

## Context

Onchain operators (paid in crypto, live onchain, refuse custodians) manage runway, conversions,
and payments by hand across wallets, and stay exposed to CEX freezes. Existing good wallets are
mobile/extension; there is no fast, keyboard-driven, gorgeous **native desktop** wallet that is
also trustless by construction. Deckard v0 is that wallet: a craft-forward operator wallet with
**zero AI** (the sovereign AI autopilot is v1, layered on this daily-use base). The differentiator
is the trust model: keys on device, a bundled light client so no third-party RPC is trusted by
default, open source.

## Current state (the `deck` starter, verified 2026-06-04)

`hellno/deck` is a private GPUI desktop starter. Stack from `Cargo.toml`: `gpui 0.2`,
`gpui-component 0.5`, `gpui-component-assets 0.5`, `serde`, `directories`, optional `tray-icon`.
License `0BSD`. `cargo bundle` config emits `Deck.app` (macOS) / `.deb` (Linux). `just` task runner,
CI workflow present.

Architecture (verified in `src/`):
- `main.rs` — `Application::new().with_assets(...).run(...)`; `gpui_component::init`; declares actions
  via `gpui::actions!(deck, [Quit, About, OpenSettings, ToggleTheme, NewItem, GoBack])`; binds keys
  (`secondary-q/n/,` etc., `secondary` = Cmd on macOS / Ctrl on Linux); sets native menus; opens one
  window with `Shell` wrapped in `gpui_component::Root`.
- `shell.rs` — `Shell` is the single root view. Owns `Settings`, a `Route` enum (`Welcome | Settings`),
  `InputState`, and handles view-local actions; `navigate(route)` + `cx.notify()` drives routing.
  Pages are `impl Shell` methods split across `welcome.rs` / `settings_view.rs`.
- `settings.rs` — `Settings` struct persisted to the platform config dir (`directories`).
- `theme.rs` — accent + light/dark install.

No crypto, networking, or async runtime yet. The wallet is greenfield on top of a working,
themed, packaged native shell. Routing, settings persistence, theming, and packaging are reusable
as-is.

## Proposed change — architecture

```
                         ┌─────────────────────────── Deckard (GPUI app) ───────────────────────────┐
  device keystore  ◀────▶│  core::keystore   core::signer        ui::palette (cmd-K)                 │
  (Keychain /            │      (seed,           (sign tx /        ui routes: Portfolio | Send |     │
   Secret Service)       │   HD accounts)        EIP-712)          Receive | Swap | Settings         │
                         │        │                 │                    │                           │
                         │        ▼                 ▼                    ▼                           │
                         │  core::wallet  ──▶  core::eth (provider)  ──▶ core::balances / send / swap │
                         │                          │                                                │
                         └──────────────────────────┼────────────────────────────────────────────────┘
                                                    ▼
                         ┌─ default: bundled Helios light client (trustless, in-process) ─┐
                         └─ override: user RPC URL ───────────────────────────────────────┘
                                                    │
                                                    ▼  Ethereum mainnet
                            Multicall3 (balances) · ENS · CoW Swap API / Uniswap router
```

Async (eng review): a **single background `tokio` runtime thread owns all network** (alloy provider,
CoW/HTTP); results bridge onto the GPUI main thread via channels + `cx.spawn`. The GPUI main thread
never makes a network call. Helios runs as its own process (C3), so its runtime never enters this app.

Crate split (eng review): a **`deckard-core` crate with ZERO GPUI dependency** holds keystore,
provider, balances, signing, and swap — headless and fully unit-testable. The GPUI app is a thin view
layer over it. If GPUI does not work out, the engine is portable.

## Child issues

| # | Title | Priority | Effort (human / CC) | Depends on |
|---|-------|----------|---------------------|------------|
| C1 | Fork + rebrand + relicense to AGPL-3.0 | P1 | 0.5d / ~20m | — |
| C2 | Key management core (HD, keystore, backup) | P1 | 3d / ~1h | C1 |
| C3 | Eth provider: embed Helios + BYO-RPC | P1 | 2.5d / ~45m | C1 |
| C4 | Balances: token list + Multicall3 + portfolio view | P1 | 2.5d / ~45m | C2, C3 |
| C5 | Send / receive (ENS, QR, gas, broadcast) | P1 | 3d / ~1h | C2, C3 |
| C6 | Swap: CoW Swap (Uniswap fallback) | P2 | 3d / ~1h | C2, C3, C4 |
| C7 | Command palette + full keyboard control | P1 | 2.5d / ~45m | C1 |
| C8 | Craft pass: local-first caching, states, polish | P2 | 2d / ~40m | C4, C5 |
| C9 | Packaging: signed .app + Linux build + CI | P2 | 1.5d / ~30m | C1 |

```
C1 ─┬─ C2 ─┬─ C4 ─┬─ C6
    │      │      └─ C8
    ├─ C3 ─┘
    ├─ C5 (after C2,C3)
    ├─ C7 (parallel)
    └─ C9 (last)
```

Sequencing rationale: C1 unblocks everything. C2 (keys) and C3 (provider) are the two pillars and
can proceed in parallel; nothing onchain works without both. C4/C5 are the core daily jobs and need
both pillars. C6 (swap) is highest-integration, lowest-table-stakes, so P2. C7 (palette) is the craft
signature and only depends on the shell, so it runs in parallel. C8 and C9 are finish work.

## Per-child detail

### C1 — Fork + rebrand + relicense
Rename crate `deck`→`deckard`, `APP_NAME`, bundle identifier (`com.deckard.app` or similar), swap
icon, set README to the Deckard README, replace `0BSD` with **AGPL-3.0** (+ `NOTICE`). **Acceptance:**
builds and runs as "Deckard"; `LICENSE` is AGPL-3.0; **dependency-license audit done** (`cargo deny`
or `cargo license`) confirming GPUI/Zed-lineage and all deps are AGPL-compatible. Flag any GPL/Apache
edge cases.

### C2 — Key management core
- Crates: `alloy-signer-local` (`MnemonicBuilder`, BIP-39), `keyring` v3 (Keychain / Secret Service),
  `eth-keystore` or libsodium/`chacha20poly1305` for at-rest encryption.
- Generate a 12/24-word mnemonic; import an existing mnemonic/private key; derive multiple accounts
  (BIP-44 `m/44'/60'/0'/0/i`).
- **At-rest model (eng review flagged; security review decides):** **password-derived encrypted
  keystore (scrypt/argon2) as the portable baseline** — works on headless / keyring-less Linux where
  OS Secret Service is absent. Optionally cache the unlock in the OS keystore (`keyring`) when present.
  Never log or copy the seed to clipboard without an explicit reveal+timeout.
- Mandatory backup flow on create: show words, confirm a subset, then enable funding.
- **Acceptance:** create/import → derive 3 accounts → restart app → accounts persist, signing works;
  seed never written in plaintext; reveal flow gated. Unit tests for derivation vectors (BIP-39 test
  vectors), keystore round-trip, and import.

### C3 — Eth provider (Helios sidecar + BYO-RPC) — REVISED per eng review D1
- Ship the **Helios binary as a bundled sidecar** serving a localhost RPC. The app supervises it:
  spawn on launch, health-check, restart on crash, shut down on exit. Bundle the helios binary per OS.
- `EthProvider` is then a **single `alloy` JSON-RPC client**; "local Helios vs BYO-RPC" is just which
  URL it points at (omakase default = the local Helios port; override = user RPC URL in settings).
- Handle Helios sync state (first-sync is the ONE acceptable loading moment; cache the checkpoint);
  surface a clear "syncing / synced / sidecar down" status.
- **Acceptance:** fresh launch spawns Helios, it syncs and serves `eth_getBalance`/`eth_call`
  trustlessly via localhost; toggling to a custom RPC works; killing the sidecar shows a clear status
  and auto-restarts; provider has tests against a mocked transport.

### C4 — Balances + portfolio
- Bundle a curated token list (Uniswap default list subset); read native ETH + ERC-20 balances via
  **Multicall3** (`0xcA11bde05977b3631167028862bE2a173976CA11`) through the provider.
- Portfolio view route: list of holdings, balances, USD value via the **Chainlink Feed Registry** for
  major tokens (simpler + trustless; Uniswap v3 TWAP deferred to v0.1 for the long tail), ETH-
  denominated for everything without a feed. No price API. Local-first cache; background refresh.
- **Acceptance:** portfolio shows native + listed ERC-20 balances for an imported address, matching a
  block explorer within rounding; refresh is non-blocking; missing-token caveat documented in UI.

### C5 — Send / receive
- Receive: show active account address + `qrcode`-rendered QR; copy.
- Send: recipient via paste or **ENS** (alloy ENS resolution) — desktop camera scan deferred; amount
  with max; gas estimate + EIP-1559 fees; build, sign (C2), broadcast via provider; pending/confirmed
  status.
- **Acceptance:** send native + ERC-20 to an ENS name and a raw address on a fork/testnet harness;
  correct nonce/gas; status updates; rejects bad addresses loudly. Integration test on an Anvil fork.

### C6 — Swap
- **CoW Swap** primary: fetch quote (CoW API), build order, sign EIP-712 with C2 signer, submit to
  the order book, track fill. **Uniswap** fallback: quote + swap via Universal Router/`SwapRouter`.
- **ERC-20 approval step (eng review):** the first swap of a token needs an `approve` to the CoW GPv2
  vault relayer — a separate tx + signing prompt. Detect missing allowance, prompt, then place the order.
- **Order lifecycle state machine (eng review):** quote → (approve?) → sign → submit → {filled |
  expired | cancelled | rejected}. Every state is shown; expiry and no-fill are first-class, not silent.
- Review screen: rate, fee, slippage, min-received; explicit confirm.
- **Acceptance:** quote → review → (approve) → sign → submit on mainnet-fork harness; MEV-protected via
  CoW; fallback works; **EIP-712 order hash verified against CoW reference vectors**; every failure
  (expiry, no-fill, rejection, allowance) surfaces a named state, never a silent stall.

### C7 — Command palette + keyboard control
- Cmd-K palette: fuzzy-filtered (`nucleo`/`fuzzy-matcher`) command list (navigate routes, send, swap,
  switch account, copy address, toggle theme). Full keyboard nav for every primary flow; visible focus.
- Built on gpui-component primitives (modal/list/input) since it has no palette out of the box.
- **Acceptance:** every primary action reachable from cmd-K and by keyboard with no mouse; palette
  opens <50ms; arrow/enter/esc semantics correct.

### C8 — Craft pass
Local-first caching so views render instantly from cache then refresh; designed empty / loading /
error / partial states; visual polish to the opinionated bar. **Acceptance:** no spinner on any view
except first Helios sync; every list has a real empty state; errors are human-readable.

### C9 — Packaging
`cargo bundle` macOS `.app` (codesign + notarize note), Linux `.deb`/AppImage; CI builds both;
release artifacts. **Acceptance:** double-click installable build on macOS + Linux from CI.

## Tech stack decisions (made — implementer decides nothing here)
- Ethereum: **`alloy`** (provider, signer-local, contract `sol!`, ENS, EIP-712). Not ethers-rs (sunset).
- Light client: **`helios`** (a16z) embedded as a library.
- Keystore: **`keyring`** v3 + encrypted-at-rest seed.
- Balances: **Multicall3** + bundled token list.
- Swap: **CoW** REST + EIP-712; Uniswap router fallback.
- Palette fuzzy match: **`nucleo`**.
- Async: **`tokio`**, bridged to GPUI's main thread via `cx.spawn`.

## Acceptance criteria (epic)
1. Create or import a wallet, back up the seed, and persist accounts across restarts.
2. View native + listed ERC-20 balances on Ethereum mainnet, read through a bundled Helios light
   client by default, with a working custom-RPC override.
3. Send to an ENS name or address and receive via address/QR.
4. Swap two assets via CoW (Uniswap fallback) with an explicit review step.
5. Every primary action is reachable by keyboard and from a cmd-K palette.
6. No third-party RPC is trusted in the default config.
7. AGPL-3.0; dependency licenses audited compatible.
8. Builds and packages on macOS + Linux.

## Testing plan
| Layer | What | Count |
|-------|------|-------|
| Unit | BIP-39 vectors, keystore round-trip, address validation, fee math, provider abstraction, **CoW EIP-712 order hash vs CoW reference vectors (CRITICAL)** | +14 |
| Integration | send + swap on an Anvil mainnet-fork; Helios sync; balance reads vs known address | +6 |
| E2E (manual, pre-automation) | create → fund → balance → send → swap → restart, keyboard-only | checklist |

## Security notes (this is a wallet holding funds — full pass via /cso or /security-review)
Threat-model: seed at rest, clipboard exposure, the reveal flow, signing-prompt clarity (clear-signing
of what is being signed), CoW order EIP-712 correctness, dependency supply chain (Helios, alloy, CoW),
RPC trust (mitigated by Helios), update channel. No telemetry. The security review is a gate before
any mainnet build, and the audit is a funded grant line item per `strategy.md`.

## Out of scope (v0)
AI / autopilot / policy engine (v1) · L2s (v0.1) · smart accounts (v1) · CEX integrations · DeFi
positions · activity history (v0.1) · mobile · desktop camera QR scan (paste/ENS only in v0).

## Open decisions (surface to the user)
1. **Price / USD source — RESOLVED 2026-06-05:** on-chain oracles (Uniswap v3 TWAP / Chainlink feeds)
   for major tokens, with ETH-denominated display as the honest fallback for the long tail. No price
   API, no centralized feed. Keeps the trustless default end to end.
2. **Seed at-rest model:** OS-keystore-wrapped key (no per-launch password) vs app passphrase
   (eth-keystore). Recommend keystore-wrapped + optional passphrase; confirm in the security review.

## Rollback / risk
Pre-release software; "rollback" = revert PRs / pin a prior tagged build. No shared state or server.
Highest risk is C2/C5/C6 (funds). Mitigation: Anvil-fork tests, tiny real amounts first, and the
security review gate before distributing any mainnet build.

## Effort
~20.5 dev-days human across C1–C9 (parallelizable to ~3–4 calendar weeks solo), or roughly a few
focused days with CC assistance, plus the review gates below.

## Next steps — run before implementing
1. **`/plan-eng-review`** — architecture + tests gate. Pressure-test the async/GPUI bridge, the
   provider abstraction, the keystore design, and the CoW signing path.
2. **`/plan-design-review`** or **`/design-consultation`** — the craft bar IS the product; lock the
   design system (type, color, spacing, motion, the palette UX) before building views.
3. **`/cso`** or **`/security-review`** — wallet threat model (keys, signing, supply chain) before any
   funds-touching code ships.
Suggested order: eng-review → design → security, then implement C1–C9 in dependency order.

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | complete | Strategy locked (`strategy.md`); Codex challenge digested. |
| Eng Review | `/plan-eng-review` | Architecture & tests | 1 | issues_addressed | 1 decision (D1 Helios sidecar) + 5 architecture findings folded in; CoW EIP-712 test marked critical. |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | recommended next (the craft bar is the product). |
| Security | `/cso` or `/security-review` | Keys + funds threat model | 0 | — | required gate before any mainnet build. |

- **ENG DECISIONS:** D1 Helios = bundled sidecar (localhost RPC). Folded in: `deckard-core` crate with
  no GPUI dep; single background tokio thread owns network; password-derived keystore baseline (Linux
  portability); CoW ERC-20 approval step + order lifecycle state machine; Chainlink Feed Registry for
  prices (Uniswap TWAP deferred to v0.1); CoW EIP-712 order-hash test marked CRITICAL.
- **UNRESOLVED:** keystore at-rest final model deferred to the security review.
- **VERDICT:** Eng review complete, spec updated. Next: `/plan-design-review` (design system + palette
  UX), then `/cso` or `/security-review` (hard gate before funds-touching code), then implement C1-C9.
