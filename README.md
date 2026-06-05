# Deckard

A fast, keyboard-first, **self-custodial Ethereum wallet** for people who live onchain.
Native (macOS + Linux), trustless by construction, open source.

> Forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which permits
> relicensing). Now its own project: Rust + [GPUI](https://www.gpui.rs/), licensed AGPL-3.0-or-later.

## Status — v0 (working today)

- **Real self-custodial wallet** via [alloy](https://github.com/alloy-rs/alloy)
  (`alloy-signer-local`): a secp256k1 keypair, EIP-55 address, key persisted to the OS config
  dir. No hand-rolled crypto.
- **Portfolio** screen: address, balance, Send / Receive / Swap, holdings.
- **Receive**: a real scannable QR plus copy-to-clipboard.
- **Command palette** (`cmd-K`) and the amber-on-near-black design system (see `DESIGN.md`).
- Light / dark.

Representative / not yet wired (next): live balances, Send/Swap execution, BIP-39 seed backup.

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
