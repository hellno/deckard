# Deckard

A fast, keyboard-first, **self-custodial Ethereum wallet** for people who live onchain.
Native (macOS + Linux), trustless by construction, open source.

> Forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which permits
> relicensing). Now its own project: Rust + [GPUI](https://www.gpui.rs/), licensed AGPL-3.0-or-later.

## Status

**Live build status: [`STATUS.md`](STATUS.md) — the single source of truth** (demo beats, crates, risks).

Working today: encrypted BIP-39 keystore + onboarding, live on-chain balances (Multicall3), receive (QR),
command palette, and the amber-on-near-black design system (`DESIGN.md`). Reads are **Helios-verified** (no
third-party RPC trusted by default). A process-isolated signer daemon (`deckard-signerd`) holds the key and
gates every write. The **shield** hero (auto-private via Railgun) is wired + black-box tested on an anvil fork.
Next: receive-watcher, the agent (MCP) surface, Send/Swap UI. See `STATUS.md` for the beat-by-beat picture.

## Roadmap

- **v0** — the wallet (above).
- **next** — live balances (alloy provider + Multicall3, Helios light client), Send/Swap
  (alloy + CoW Swap), and the BIP-39 seed-backup flow. All on audited libraries.
- **v1** — the sovereign autopilot: policy-bounded automation over your own keys.

## Build & run

```sh
cargo run        # debug build + run
just run         # same, via the task runner
just bundle      # build a macOS Deckard.app
```

Requires a recent stable Rust toolchain.

## Design

The visual + interaction system (typography, color, spacing, the command palette, and the
clear-signing/trust affordances) lives in [`DESIGN.md`](DESIGN.md) — the source of truth for any
UI work.

## License

AGPL-3.0-or-later. See [`LICENSE`](LICENSE) and third-party attributions in [`NOTICE`](NOTICE).
