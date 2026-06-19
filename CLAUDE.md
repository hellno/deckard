# Deckard — agent notes

## Planning artifacts
Work items and epics (PRDs, tasks, spikes) live as **GitHub issues**, not files in the repo — use an
epic issue with linked sub-issues. Research, ADRs/decision records, and design specs live in `docs/`.
**Only open (create) issues after the user confirms.** Draft in chat first; create on the go-ahead.

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

## Branch hygiene — required before edits

Before changing files, always establish the current branch and its source-of-truth status:

1. Run `git status --short --branch`.
2. If on `main`, run `git fetch origin --prune`, fast-forward from `origin/main`, then create a new
   feature branch before editing.
3. If not on `main`, run `git fetch origin --prune` and check whether the branch has already been
   merged into current `origin/main`.
   - If it has been merged, switch back to `main`, fast-forward from `origin/main`, and create a new
     feature branch before editing.
   - If it has not been merged, inspect the branch's upstream/ahead/behind state and update local
     state before editing. Do not stack unrelated work on a stale or merged branch.
4. If there are uncommitted changes, identify whether they are user changes before switching,
   rebasing, stashing, or applying patches.

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

## Visual verification (GUI changes)
Touching any `crates/deckard-app` view? Tests alone don't prove a screen renders — **run the app and
attach before/after screenshots to the PR** (the un-gated Send PR #54 is the template: Portfolio with
the action enabled + the clear-signing review card). On macOS: `just run`. In a headless Linux /
Claude-on-the-web session, follow `docs/dev/headless-gui-screenshots.md` (software Vulkan + Xvfb) to
launch, drive, and capture. QA and review flag a GUI PR that ships without screenshots.

## Language
In docs, UI copy, and refusal/error strings: prefer plain words over crypto jargon, and explain a
term once where it first appears (e.g. "shield — move funds to a private balance").

## Command palette reachability
Every user-facing action must be reachable from the ⌘K palette. When you add an action, add a
`Command` to `crates/deckard-app/src/palette_commands.rs` (id, title, fuzzy aliases, shortcut,
optional curated icon) and handle its `id` in `Shell::run_palette_command` (`shell.rs`). The palette
is the universal control plane; an action that isn't in it is invisible to a keyboard-first operator.
QA and review flag any action that ships without its command.
