# Deckard — next-phase build handoff

> ⚠ **Stale snapshot.** Live build status is in [`/STATUS.md`](../STATUS.md) (single source of truth). The
> "Built so far" / paths below predate the `crates/` workspace restructure, the encrypted keystore, the signer
> daemon, and the Helios/shield integration. Kept for the GPUI gotchas + ground rules; use `/STATUS.md` for state.

For a fresh session to pick up implementation cold. Read this + `SPEC-v0-epic.md` (backlog C1–C9) +
the repo's `DESIGN.md`. Project memory also carries the state.

## Where things are
- **Repo:** `/Users/hellno/dev/misc/hacks/deckard` (git; remote `origin` = github.com/hellno/deckard
  private; `starter` = hellno/deck). Branch `main`. Run: `cargo run` (binary `deckard`).
- **Specs (in this repo):** `specs/SPEC-v0.md`, `specs/SPEC-v0-epic.md`, `specs/strategy.md`.
  Design: `DESIGN.md` (repo root).
- **Built so far (v0):** real alloy wallet (`src/wallet.rs`, EIP-55 address persisted to OS config
  dir), Portfolio (`src/welcome.rs`, mock balances), Receive QR + copy (`src/receive.rs`), cmd-K
  palette (`src/palette.rs`), theme (`src/theme.rs`), routing (`src/shell.rs`).

## Ground rules (do not violate)
1. **Audited libs only — no hand-rolled crypto/security.** alloy for keys/signing/provider/tx/ENS,
   alloy-contract for Multicall3, Helios (a16z) for the light client, CoW API for swaps.
2. **`DESIGN.md` is the UI source of truth** (amber `#F2A43B` on near-black, mono for money/addresses,
   minimal motion, clear-signing affordances).
3. **Build-verify every step:** `cargo build` must stay green. Commit per logical unit.
4. **`/cso` (security review) gates chunks 3 and 4** (keys + funds). A third-party audit gates real
   mainnet money (funded grant line item per strategy).

## GPUI gotchas (learned this session — saves hours)
- Views are `impl Shell` methods returning `impl IntoElement`; `Route` enum + `render()` match dispatch.
- Extract `cx.theme()` Copy color tokens into locals BEFORE any `cx.listener(...)` (theme borrows cx).
- Stateful `div().id("x").on_click(cx.listener(...))` needs `use gpui::{InteractiveElement, StatefulInteractiveElement}`.
- Conditional child: `.children(self.flag.then(|| self.render_x(cx)))`.
- Clipboard: `cx.write_to_clipboard(gpui::ClipboardItem::new_string(s))`.
- gpui-component: `Button::new(id).primary()/.ghost().label(..).on_click(..)`, `h_flex()/v_flex()`,
  Styled helpers `.gap_N()/.px_N()/.py_Np5()/.rounded_lg()/.size(px())`, `IconName::*` (verify the
  variant exists before using).
- **Async:** GPUI has its own executor; alloy needs tokio. Run ALL network on a single background
  tokio thread, bridge results to the GPUI main thread via channels + `cx.spawn`. Never block the
  main thread. (Eng-review decision.)
- Screenshot to verify: needs Ghostty (host app) Screen Recording permission AND the Mac unlocked;
  the `deckard-watch4.sh` pattern (launch + frontmost + diff screen center + `screencapture -x`) works.

## The chunks (dependency order)

### Chunk 1 — INFRA (do first; unblocks 2 & 4)  [spec C3]
- **Goal:** an async bridge + an `EthProvider` over alloy; reads work end to end.
- **Do:** single background tokio runtime thread owning network; channel/`cx.spawn` bridge to UI.
  Add `alloy-provider`. `EthProvider` = one alloy JSON-RPC client; default to a public RPC for now,
  with a settings field for a custom RPC URL (omakase). (Helios sidecar = the trustless default,
  next sub-step: bundle the helios binary, spawn/supervise it on localhost, point the provider at it.)
- **Optional but recommended:** split a `deckard-core` crate (no GPUI dep) holding wallet/provider/
  balances/signing — headless + unit-testable. Eng-review wanted this while the surface is small.
- **Done when:** the app fetches the live ETH balance of the wallet address over RPC and logs/shows it
  without blocking the UI; provider has a mock-transport unit test.

### Chunk 2 — LIVE BALANCES  [spec C4]
- **Goal:** Portfolio shows real holdings.
- **Do:** `alloy-contract` `sol!` bindings for Multicall3 (`0xcA11bde05977b3631167028862bE2a173976CA11`);
  bundle a curated token list (Uniswap default subset); batch native + ERC-20 balances via Multicall3.
  USD via the **Chainlink Feed Registry** for majors, ETH-denominated for the long tail (no price API —
  per the resolved price decision). Local-first cache; background refresh. The ONE allowed loading
  state is first sync; everywhere else renders from cache.
- **Done when:** Portfolio (`welcome.rs`) shows real balances for an imported address matching a block
  explorer within rounding; refresh is non-blocking; missing-token caveat noted in UI.

### Chunk 3 — SEED BACKUP + KEYSTORE  [spec C2]  (/cso gate)
- **Goal:** the wallet is recoverable from a BIP-39 phrase and the key is encrypted at rest.
- **Do:** replace `PrivateKeySigner::random()` in `wallet.rs` with alloy `MnemonicBuilder`
  (`alloy_signer_local::MnemonicBuilder::<English>` — `mnemonic` feature already enabled): generate a
  12/24-word phrase → derive `m/44'/60'/0'/0/0` → signer. Encrypted-at-rest keystore: password-derived
  (scrypt/argon2) baseline (portable to keyring-less Linux; eng-review flag), OS keystore optional.
  Onboarding: create vs import, mandatory backup + confirm-a-subset, seed-reveal protection
  (blurred/hold-to-reveal/auto-hide, never silent clipboard — DESIGN.md).
- **Migration note:** the existing persisted `wallet.key` is a RAW random key (no phrase). Loading it via
  `from_slice` keeps the same address but there's no recoverable mnemonic — treat as "imported key,"
  and only new wallets get a phrase. Changing derivation for a fresh wallet changes the address.
- **Done when:** create → see phrase → confirm → restart → same address; phrase recovers the key;
  key never written in plaintext; reveal flow gated. Unit tests: BIP-39 vectors, keystore round-trip.
  Then run `/cso`.

### Chunk 4 — SEND / SWAP  [spec C5/C6]  (/cso + audit gate, testnet first)
- **Send:** alloy `TransactionRequest` (EIP-1559 fees, nonce, gas estimate), ENS resolution via alloy,
  sign with the wallet signer, broadcast via the provider, pending/confirmed status.
- **Swap (CoW):** quote (CoW order-book REST) → detect missing ERC-20 allowance → `approve` to the GPv2
  vault relayer (separate tx + prompt) → build order → **EIP-712 sign (alloy)** → submit → poll fill.
  Order lifecycle state machine (quote → approve? → sign → submit → filled/expired/cancelled/rejected),
  every state visible. Uniswap fallback via alloy-contract router.
- **CRITICAL test:** the CoW EIP-712 order hash MUST be verified against CoW's reference vectors — a
  wrong struct loses/silently fails orders.
- **Test harness:** Anvil mainnet-fork for send + swap. Tiny real caps only after `/cso` + audit.
- **Done when:** send to an ENS name + raw address on the fork; CoW quote→approve→sign→submit on the
  fork; EIP-712 vector test passes; every failure surfaces a named state, never a silent stall.

## Suggested session split
- Session A: Chunk 1 (infra). Session B: Chunk 2 (balances) and/or Chunk 3 (seed) — independent, can be
  parallel. Session C: Chunk 4 (send/swap), after `/cso`.
- Optional per chunk: a small **API-scout workflow** first (verified snippets for the exact crate
  versions: alloy-provider+Multicall3, CoW order+EIP-712, alloy MnemonicBuilder HD, Helios embed) to
  de-risk version churn. Then a focused single-agent build → `/review` the diff → `/cso` for 3 & 4.
