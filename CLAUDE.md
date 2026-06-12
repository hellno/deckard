# Deckard — agent notes

## Design System
Always read `DESIGN.md` before making any visual or UI decision. Fonts, colors, spacing,
the two-signal actor model (amber = human, cyan = agent), the sidebar/contextual-views IA,
component states, and the clear-signing / seed-reveal trust affordances are defined there.
Do not deviate without explicit user approval. In QA/review, flag anything that doesn't match
`DESIGN.md`.

Ground all design work in **real reference screenshots** (Linear, Conductor, Splits, Superhuman,
Stripe), never in remembered descriptions — that is how the first drafts went wrong. The
interactive, dogfooded reference lives at
`~/.gstack/projects/hellno-deckard/designs/deckard-foundation-preview.html`.

## Engineering & verification
The full rationale (what we enforce and *why*, plus deliberately-rejected rules) is in
`docs/AGENTIC-ENGINEERING.md`. The quick reference:

**Iterate fast:** `just core` — clippy + tests the GPUI-free engine (`deckard-core`) in seconds, no
gpui build. Reach for it while working on the engine; the full Definition of Done below still applies
before you're done. (For UI work you must build the app: `just check`.)

**Definition of done** (ALL must hold; paste the command output as evidence — do not claim done while red):
1. `cargo fmt --all --check` is clean
2. `just check` is green — runs clippy `-D warnings` on BOTH the default and `--features tray` configs
3. `cargo test --workspace` is green
4. No new or changed dependencies (`Cargo.toml` / `Cargo.lock`) unless explicitly approved
   (the git GPUI stack is bumped only via `just bump-gpui` — never hand-edit those pins)

## Code constraints

**Enforced workspace-wide** by `[workspace.lints]` + `clippy.toml` (CI fails the build):
- `todo!` and `dbg!` are denied; ignored `Result`s (`unused_must_use`) are denied.
- `deckard-core` is `#![forbid(unsafe_code)]`; the app crate is `unsafe_code = "deny"`.
- `std::mem::forget` / `core::mem::forget` and `rand::thread_rng` are denied — use `drop()` / `OsRng`.

**Enforced in `deckard-core`** (the trust core) via crate-level `#![deny(...)]` — clippy fails the build:
- No `.unwrap()` / `.expect()` / `panic!` / raw slice indexing in non-test code — propagate with
  `Result` / `?` and parse untrusted bytes through the bounded `Reader` in `keystore.rs`. The app
  crate may `unwrap` infallible GPUI handles; the engine must not. The two startup-fatal `expect`s in
  `eth.rs` carry a documented local `#[allow]` — match that pattern (a `// reason` + scoped `#[allow]`)
  for any genuinely-unrecoverable boundary, don't reach for a bare `unwrap`.

**Always, every crate:** never log or `Debug`-print a seed, key, or passphrase — secrets stay in `Zeroizing`.

## Language
In docs, UI copy, and refusal/error strings: prefer plain words over crypto jargon, and explain a
term once where it first appears (e.g. "shield — move funds to a private balance").
