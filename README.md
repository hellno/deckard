# Deckard

A fast, keyboard-first, **self-custodial Ethereum wallet** for people who live onchain.
Native (macOS + Linux), trustless by construction, open source.

> ## ⚠️ v0.0.1-alpha — EXPERIMENTAL. NOT for production.
> This is **pre-1.0, experimental software** with **no third-party security audit**.
> **Do NOT use it with real funds or real mainnet keys.** Use **testnet / throwaway keys only.**
> It can lose your money or break at any time. You have been warned.

![Deckard demo](docs/demo.gif)

> _Draft demo from the current build: onboarding → verified split-balance → command palette → receive → shield-to-private. The final cut is swapped in once the in-flight UI polish lands ([`docs/RELEASING.md`](docs/RELEASING.md) §4)._

> Forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (0BSD, which permits
> relicensing). Now its own project: Rust + [GPUI](https://www.gpui.rs/), licensed AGPL-3.0-or-later.

For the bigger picture — *why* Deckard exists and where it's going — read [the why](docs/launch-pitch.md).

## Status

**Live build status: [`STATUS.md`](STATUS.md) — the single source of truth** (demo beats, crates, risks).

**Working today:**

- Encrypted BIP-39 keystore + onboarding (Argon2id + XChaCha20-Poly1305 at rest; secrets in `Zeroizing`).
- Live on-chain balances (Multicall3).
- Receive (address + QR).
- Command palette.
- The amber-on-near-black design system (`DESIGN.md`).
- **Helios-verified reads** — no third-party RPC is trusted by default.
- A **process-isolated signer daemon** (`deckard-signerd`) over a Unix socket that holds the key and
  gates every write.
- The **shield** hero (auto-private via [Railgun](https://railgun.org/)) is **wired + black-box tested
  on an anvil fork**.

**Not done yet (do not expect these to work):**

- **Send** UI is gated ("next release").
- **Swap** is a TODO.
- The **agent / MCP surface** (`deckard-mcp`) is **not built**.
- The **receive-watcher** auto-detect is a TODO.
- Some tests are `#[ignore]` (need `anvil` + an archive RPC) — see the test caveats in `STATUS.md`.

See `STATUS.md` for the beat-by-beat picture and the honest test caveats.

## Build & run

```sh
just run                 # build the signer daemon, then build + run the app (debug)
just core                # fast engine-only inner loop (clippy + tests for deckard-core, no GPUI build)
just check               # lint both feature configs (clippy -D warnings: default AND --features tray)
cargo test --workspace   # run the test suite
just bundle              # build a macOS Deckard.app
```

The Rust toolchain is pinned in `rust-toolchain.toml`. Install [`just`](https://github.com/casey/just)
with `brew install just` (macOS) or `cargo install just` (any platform). Under the hood `just run`,
`just core`, `just check`, and `cargo test` are plain `cargo`, so you can run them by hand too.

**Linux:** the app builds with `cargo`, but GPUI needs the same system libraries Zed does — a Vulkan
loader plus the X11/Wayland and font/clipboard dev packages. Install them per
[Zed's Linux build dependencies](https://github.com/zed-industries/zed/blob/main/docs/src/development/linux.md)
before `just run`. Note that `just bundle` / `just open` / `just icon` are **macOS-only** (they shell
out to `cargo-bundle` (osx), `open`, `sips`, and `iconutil`); on Linux use `cargo build` / `cargo run`
directly.

**Definition of done** (all must hold before a change is finished):

1. `cargo fmt --all --check` is clean.
2. `just check` is green (clippy `-D warnings` on **both** the default and `--features tray` configs).
3. `cargo test --workspace` is green.
4. No new or changed dependencies in `Cargo.toml` / `Cargo.lock` unless explicitly approved.

### Crate layout

Virtual Cargo workspace; all crates live under `crates/`:

- **`deckard-app`** — the GPUI app (binary `deckard`).
- **`deckard-core`** — the headless engine: provider / verified-reads, balances, HD keys, keystore, and the
  key-less shield builder. `#![forbid(unsafe_code)]`.
- **`deckard-contract`** — the frozen wire contract (`Intent` / `Decision` / `Policy` / `RPC` / `ReadStatus`).
- **`deckard-signerd`** — the signer daemon that holds the key and gates writes.

## Security architecture

Real, and already built:

- **Process-isolated signer daemon** (`deckard-signerd`) over a Unix domain socket: it holds the key and
  gates every write; the app and engine are key-less.
- **Helios light-client verified reads** — no third-party RPC is trusted by default.
- **Keystore at rest** = Argon2id key derivation + an XChaCha20-Poly1305 envelope; secrets stay in `Zeroizing`.
- `deckard-core` is `#![forbid(unsafe_code)]`; the workspace lint policy denies `todo!`, `dbg!`, and ignored
  `Result`s.

This is alpha software with **no external audit yet** — treat the above as design intent under active review,
not a guarantee. Report anything sensitive privately (see below).

## Design

The visual + interaction system (typography, color, spacing, the command palette, and the
clear-signing / trust affordances) lives in [`DESIGN.md`](DESIGN.md) — the source of truth for any
UI work.

## Contributing

This is **alpha software**, and money software at that — it should not be reviewed by one person. I'm
especially looking for a **security-minded co-maintainer** for the signer/policy/keystore surface. If
reviewing self-custody enforcement is your thing, please reach out.

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to build, test, and submit changes.
- [`SECURITY.md`](SECURITY.md) — how to report a vulnerability **privately**
  (security contact: [contact-removed-see-SECURITY.md]).
- [`CHANGELOG.md`](CHANGELOG.md) — what changed, release by release.

Repo: <https://github.com/hellno/deckard>.

## License

AGPL-3.0-or-later. See [`LICENSE`](LICENSE) and third-party attributions in [`NOTICE`](NOTICE).
