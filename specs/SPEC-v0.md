# Deckard v0 — Requirements (locked 2026-06-04)

Feeds the full build spec (`/spec`). v0 is the wallet; the sovereign AI autopilot loop is v1.

## What v0 is
A craft-forward, native desktop, self-custodial wallet for **onchain operators**. **Zero AI.**
"What if Linear / Superhuman built a wallet." The sovereign autopilot loop layers on in v1, on
top of a wallet people already open every day.

## Platform & stack
- Native desktop: **macOS + Linux**.
- **Rust + GPUI + gpui-component**, forked from the `hellno/deck` starter.
- Key storage: device keystore (macOS Keychain / Secure Enclave; Linux Secret Service / keyring),
  abstracted behind one interface.

## Account & key model
- **Managed EOA** (self-custody; key lives on device). Deckard signs only after the user approves.
- First run: **create a fresh HD wallet OR import an existing key/seed.** Mandatory BIP-39 seed
  backup flow.
- Multiple accounts via HD derivation.
- Smart-account migration is deferred to v1 (when bounded autonomy lands).

## Core jobs (v0 must-haves)
1. **Multichain token balances at a glance** (native + ERC-20).
2. **Send / receive** (paste / scan / ENS).
3. **In-app swap.**
- Deferred to v0.1: activity history + labels; DeFi positions.

## Chains
- **Ethereum mainnet only for v0.** Resolved 2026-06-04: rather than compromise on an L2, v0 ships
  L1-only, where the full trustless stack works (Helios light client + CoW). L2s are deferred to
  v0.1, pending a per-L2 light-client story (Helios covers OP Stack + Linea, not Arbitrum).
- **Tradeoff (flagged):** mainnet gas makes sends/swaps pricier for operators. Accepted for v0 to
  keep the sovereignty stack pure and scope simple; cheaper L2s are the top v0.1 priority.

## Data layer (omakase, local-first, trustless by default)
- **Bundle a Helios light client as the default.** Helios is a Rust crate (a16z), embeds directly
  in the GPUI app (also compiles to WASM), and gives trustless reads of Ethereum mainnet with no
  third-party RPC trusted. This is the cyberpunk core: the wallet trusts no one's RPC out of the box.
- **Omakase + override:** Helios local-first by default; advanced users point at their own RPC URL.
  Sensible default, full sovereignty available.
- Token balances via a **curated token list + multicall** over the light client, not a centralized
  portfolio API.
- **Honest tradeoff (flagged):** no indexer means "all your tokens at a glance" depends on the token
  list (may miss long-tail / airdropped tokens) and is heavier than a portfolio API. Accepted for
  the thesis; revisit if UX suffers (middle path: a self-hostable / open indexer users can point at).

## Swap
- **CoW Swap** on Ethereum mainnet (MEV-protected, decentralized), on-thesis and deployed on L1.
  Uniswap direct as fallback.

## Craft bar (all three are the standard)
- **Command palette + total keyboard control** (cmd-K everything).
- **Local-first, zero-spinner speed** (cached, background sync, no loading states).
- **Opinionated, minimal, gorgeous UI.**

## Explicitly OUT of v0
- Any AI / autopilot / policy engine (that is v1).
- CEX integrations / API keys.
- Smart accounts (EOA now; migrate in v1).
- DeFi positions, activity history (v0.1).
- Mobile.

## Name & license
- **Deckard.** **AGPL-3.0.**

## Resolved / open for the spec
- **Resolved:** chains = Ethereum mainnet only (v0); data layer = bundled Helios light client
  (omakase) + bring-your-own-RPC; swap = CoW (mainnet); L2s deferred to v0.1.
- **Open:** long-tail token discovery without an indexer; mainnet gas-cost UX (estimates, warnings);
  Helios integration shape (Rust crate vs WASM) inside the GPUI app.
