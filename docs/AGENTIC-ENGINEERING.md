# Agentic engineering: lints, constraints & CI for building fast with coding agents

> **Audience:** maintainers of Deckard *and* the upstream **Deck** GPUI starter it was forked
> from. Most of this is general to any GPUI + Rust project; the few **wallet-specific** rules are
> tagged `🔐` so Deck can drop them. This is the *why* behind the config; the *what to run* lives in
> `CLAUDE.md` / `AGENTS.md`.

## The thesis

A coding agent is fast but has no taste and no memory of *this* repo's intent. The cheapest way to
make an agent (or a hurried human) reliably produce good code is to **make the compiler and CI say
no for you.** Every rule below converts a class of mistake from "caught in review, maybe" into
"caught at `cargo check`, always."

Three principles, in priority order:

1. **The manifest is the source of truth, not CI flags.** If the lint policy only lives in a CI
   `-D warnings` flag, the agent doesn't see it until the build is already red. Put it in
   `Cargo.toml` (`[workspace.lints]`) and `clippy.toml` so `rust-analyzer` and `cargo check` show
   the *exact same* policy at the moment code is written. **Shorten the feedback loop to zero.**
2. **CI is the only gate that matters.** Pre-commit hooks, editor warnings, and good intentions are
   all bypassable (`git commit --no-verify`, "I'll fix it later"). An agent will confidently report
   "done" while red. So anything you actually care about must *block merge* in CI.
3. **Prefer compile-time over runtime, and `warn` → fix → `deny` over big-bang.** Land a new lint as
   `warn`, clear the backlog, *then* flip to `deny`. A rule that breaks `main` on day one gets
   reverted; a rule that lands green stays forever.

These match what the paradigm Rust projects do — we cross-checked
[reth](https://github.com/paradigmxyz/reth) (an Ethereum node, our closest analog),
[alloy](https://github.com/alloy-rs/alloy) (which we depend on),
[Zed](https://github.com/zed-industries/zed) (our GPUI source), tokio, ripgrep, and the
[Embark Studios shared lint set](https://github.com/EmbarkStudios/rust-ecosystem/blob/main/lints.rs).
The recurring pattern is identical: a `[workspace.lints]` table, a `clippy.toml`, a checked-in
`rustfmt.toml`, `cargo-deny` in CI, and a CI matrix that runs fmt + clippy + test on every push.

---

## Tier 1 — the do-now set

Each item: **what**, **why it helps an agent**, the **snippet**, and the **tradeoff**.

### 1. Move the lint policy into the manifest — `[workspace.lints]`

**What.** A lint table in the root `Cargo.toml`, inherited by every crate via `[lints] workspace =
true`. Today our `-D warnings` policy exists *only* as a CLI flag in `just check` and CI.

```toml
# root Cargo.toml
[workspace.lints.rust]
unused_must_use = "deny"   # an ignored Result from a fallible crypto/IO call is a silent bug
unsafe_code     = "deny"   # deny (not forbid) at the root — see §6 for why the escape hatch matters

[workspace.lints.clippy]
all       = { level = "warn", priority = -1 }  # priority = -1 is required on a lint *group*
todo      = "deny"          # a stray todo!() left on a code path panics in production
dbg_macro = "deny"          # dbg!(x) is debug noise — and 🔐 dbg!(seed) is a key leak
```

Then in **every member crate** under `crates/` (`deckard-app`, `deckard-core`, `deckard-contract`,
`deckard-signerd`):

```toml
[lints]
workspace = true
```

> **Gotcha (verified):** `[workspace.lints]` is *not* inherited automatically — each crate must opt in
> with `[lints] workspace = true`. The root `Cargo.toml` is a **virtual** workspace manifest (no
> `[package]`), so it carries the `[workspace.lints]` table but takes no `[lints]` line itself.

**Why for an agent.** The agent now sees `clippy::all` and the deny-list in-editor and at
`cargo check`, identical to what CI enforces — no more "looked fine locally, red in CI." `todo` /
`dbg_macro` at `deny` make two classic agent habits (leaving a `todo!()` stub, leaving a `dbg!`)
into hard compile errors.

**Tradeoff.** We keep `clippy::all` at `warn` in the manifest (not `deny`) and keep `-D warnings`
in CI. That's deliberate: `warn` lets you iterate locally without every style nit blocking
`cargo build`, while CI still fails on any warning. Setting `clippy::all = "deny"` would make a
*future toolchain bump* (new clippy lints) break the build for unrelated reasons. `warn` + CI gate
is the reth/alloy convention.

### 2. A project lint config — `clippy.toml`

**What.** New file at the workspace root.

```toml
msrv = "1.95.0"                 # match rust-toolchain.toml; governs which API suggestions clippy makes
allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests  = true

# crypto/eth jargon that would otherwise trip clippy::doc_markdown. ".." keeps clippy's defaults.
doc-valid-idents = ["..", "BIP-39", "BIP-32", "EIP-1559", "ERC-20", "XChaCha20",
                    "Argon2id", "ChaCha20-Poly1305", "secp256k1", "keccak", "Multicall3"]

# 🔐 wallet-specific footguns, banned by construction (both unused today → zero migration):
disallowed-methods = [
  { path = "std::mem::forget", reason = "skips Drop, so skips zeroize scrubbing of key material; use drop()" },
  { path = "rand::thread_rng", reason = "use OsRng for anything key-derived (keystore.rs already does)" },
]
```

**Why for an agent.** `disallowed-methods` is the highest-signal-per-line tool clippy offers: it
turns "don't use X here, we have reasons" tribal knowledge into a compile error *with the reason
printed*. An agent reaching for `thread_rng()` to generate entropy gets told, at `cargo check`, to
use `OsRng`. The `allow-*-in-tests` keys let `deckard-core`'s crate-level restriction lints
(`#![deny(clippy::unwrap_used, expect_used, panic, indexing_slicing, …)]`) coexist with normal test
code that unwraps freely.

**Tradeoff.** None material — both disallowed methods are already unused, so this is pure
prevention. Deck: keep `doc-valid-idents` (trim the crypto words), drop the two `disallowed-methods`
or replace with your own footguns.

### 3. Deterministic formatting — `rustfmt.toml`

**What.** New file at the workspace root. All keys are **stable-channel** (our pinned 1.95 stable
`cargo fmt` honors them; nightly-only keys like `imports_granularity` / `group_imports` are
deliberately excluded — stable silently ignores them, which is worse than not setting them).

```toml
edition = "2021"
max_width = 100
```

Intentionally minimal — `edition` + the width the code is already written to (both rustfmt defaults
today), so landing it is near-zero churn. Style idioms (e.g. redundant field names) are left to
`clippy::all`. An opinionated `use_small_heuristics = "Max"` was tried and dropped: it reflowed ~2×
more code for no correctness gain.

**Why for an agent.** Without a checked-in config, two agents (or two rustfmt versions) format the
same code differently, producing noisy diffs that bury the real change. A pinned config makes the
diff deterministic, and `cargo fmt --all --check` in CI (§5) makes "did you run fmt?" a yes/no gate
instead of a review comment.

**Tradeoff.** The repo wasn't `cargo fmt`-clean to begin with, so enabling the `--check` gate needs
**one baseline `cargo fmt --all` commit** (formatting-only — rustfmt never changes semantics).

### 4. Supply-chain gate — `deny.toml` + a `cargo-deny` CI job

**What.** `cargo-deny` checks the dependency tree for security advisories, banned/duplicate crates,
disallowed licenses, and untrusted sources. New `deny.toml`:

```toml
[advisories]
yanked = "warn"   # see "Advisory policy" below (#82) — a bare yank shouldn't block unrelated PRs
ignore = []   # add { id = "RUSTSEC-…", reason = "…" } only with written justification

[licenses]
version = 2
confidence-threshold = 0.9
# ⚠️ SEED THIS from a real `cargo deny check licenses` run before making the job blocking —
# the alloy + git-gpui tree pulls a wide license surface and a blind list WILL red the build.
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause",
         "ISC", "Unicode-3.0", "Zlib", "MPL-2.0", "Unlicense", "CC0-1.0"]

[bans]
multiple-versions = "warn"   # the git gpui stack pulls dup versions (objc2 0.5+0.6) — deny is unworkable
wildcards = "deny"
deny = [{ crate = "openssl", reason = "prefer rustls/ring; avoid the OpenSSL CVE surface" }]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
# All SIX git origins, not two: Zed pins its own forks of font-kit/reqwest/scap/wgpu, pulled
# transitively by the gpui stack. Re-derive with `grep 'git+' Cargo.lock` after every bump-gpui.
allow-git = ["https://github.com/zed-industries/zed",
             "https://github.com/zed-industries/font-kit",
             "https://github.com/zed-industries/reqwest",
             "https://github.com/zed-industries/scap",
             "https://github.com/zed-industries/wgpu",
             "https://github.com/longbridge/gpui-component"]
```

**Why for an agent.** Three agent/maintenance failure modes, gated at once: (a) pulling in a crate
with a known RUSTSEC advisory, (b) adding a dep from a random git fork (the `[sources]` allow-list
permits only the known git origins — currently the six Zed/longbridge ones the gpui stack pulls — so
a typo-squat or malicious fork fails CI), (c) license drift in a copyleft project. For a 🔐 wallet this is table stakes; even for Deck it's
cheap insurance.

**Tradeoff.** The license `allow` list must be **seeded locally first** (`cargo deny check
licenses`), and `multiple-versions` must stay `warn` because the git gpui tree legitimately
duplicates crates. We therefore land the CI job **non-blocking** and promote it after one green run
(see Rollout).

**Advisory policy — what gates a PR vs what only informs (#82).** Split the gate by *determinism*.
`bans` / `licenses` / `sources` are deterministic — they only change when *we* change dependencies —
so they stay **required, blocking PR gates**. The advisory check is different: it re-reads the live
RustSec database and crates.io yank status on every run, so a dependency yank or a freshly-published
CVE can turn a PR red with **no change on our side**, blocking work that never touched that
dependency (this is what bit PR #81). We handle that by *kind*:

- **A real RUSTSEC advisory still blocks** — on PRs, at release, and in the daily scan. A known
  vulnerability stops the line until it is fixed or given a justified `ignore`.
- **A bare crates.io yank is warn-level** (`yanked = "warn"`). A yank is usually a pulled publish,
  not a vulnerability; if it *is* a security pull it also carries a RUSTSEC advisory and still
  blocks. So yanks stay visible without gating unrelated PRs.

The safety net is three things, not the PR gate: continuous **daily detection**
(`.github/workflows/audit.yml`, which opens a tracking issue on failure so a new advisory is loud,
not just an email), a **hard gate at the release boundary** (the reusable `ci.yml` runs the advisory
check blocking when invoked from `release.yml`, so a known-vulnerable tree can never ship), and the
`ignore` list as the **deliberate, reviewed** escape hatch for transitive advisories with no in-tree
fix. This is the mainstream Rust-OSS posture (schedule the non-deterministic check, gate the
deterministic ones), tuned so "we take security seriously" means *never ship a vulnerable build*,
not *block every contributor on outside-world drift*.

### 5. Close the CI gaps

**What.** Our CI currently runs only `cargo build`, `cargo build --features tray`, and
`cargo clippy --all-targets --features tray -- -D warnings`. That means: **format is never checked,
tests never run, and clippy never lints the default (`--features`-less) build that we actually
ship.** Add (split across the existing macOS/Linux jobs to respect the macOS minute multiplier):

```yaml
# linux job (formatting is OS-independent, so check it once on the cheap runner):
- run: cargo fmt --all --check
- run: cargo clippy --locked --all-targets -- -D warnings   # the DEFAULT feature config CI never linted
- run: cargo test --locked --workspace

# macOS job:
- run: cargo clippy --locked --all-targets -- -D warnings
- run: cargo test --locked --workspace
```

> Add `--locked` to **all** `cargo build`/`clippy`/`test` invocations (not shown above per-line):
> reproducibility lives entirely in the committed `Cargo.lock` (it pins the exact git gpui commits),
> so `--locked` makes a stale lockfile a CI failure. `just bump-gpui` rewrites and commits the lock,
> so this never fights the bump workflow.

Plus the `cargo-deny` job (non-blocking first):

```yaml
cargo-deny:
  runs-on: ubuntu-latest
  # Informational on first land. Promote to a required check after one green run AND after seeding
  # deny.toml [licenses].allow from a local `cargo deny check licenses`.
  continue-on-error: true
  steps:
    - uses: actions/checkout@v4
    - uses: EmbarkStudios/cargo-deny-action@v2
      with: { command: check advisories bans sources licenses }
```

**Why for an agent.** This is principle #2 made real. `deckard-core`'s keystore/eth tests exist but
**never run in CI today** — an agent can break the encrypted-vault round-trip and CI stays green.
Running `cargo test` and the default-feature clippy closes the "green CI but actually broken" hole
that lets an agent honestly believe it's done.

**Tradeoff.** Slightly longer CI; the macOS runner is 10× billed on *private* repos (we're public,
so free) — hence formatting/deny run only on Linux.

### 6. Unsafe policy — forbid the core, deny the app

**What.**

```rust
// crates/deckard-core/src/lib.rs — first item, after the //! module docs
#![forbid(unsafe_code)]   // the engine is pure-safe; make it a compile-time guarantee
```

```toml
# root Cargo.toml [workspace.lints.rust] — applies to the app crate
unsafe_code = "deny"      # deny, NOT forbid — see below
```

**Why for an agent.** `deckard-core` (the part that touches keys, crypto, and untrusted bytes) has
zero `unsafe` — `forbid` locks that in so no agent can ever sneak an `unsafe` block into the
security core. The app crate's only Apple-FFI code (the 🔐/tray dock-hiding via `objc2`) currently
uses *safe* wrappers, so `unsafe_code = "deny"` compiles today for both the default and
`--features tray` builds.

**Tradeoff — why `deny` not `forbid` at the root:** `forbid` cannot be locally overridden. If a
future `objc2` bump reintroduces a raw `unsafe {}` block in the tray path, `forbid` would brick the
build with no escape hatch. `deny` lets you add a single, reviewed, `// SAFETY:`-commented
`#[allow(unsafe_code)]` at that exact site. (Note: `[workspace.lints]` governs first-party code
only — `unsafe` inside dependencies is unaffected, which is correct.)

### 7. Tell the agent the rules — `CLAUDE.md` + `AGENTS.md`

**What.** Our `CLAUDE.md` only covers the design system. Add an "Engineering & verification"
section, and create an `AGENTS.md` (Codex and other agents read that filename) mirroring it. The
essentials: the fast iteration command, an explicit **definition of done**, and the code
constraints (flagging what's lint-enforced today vs what's still convention).

```markdown
## Engineering & verification
Iterate fast: `cargo check -p deckard-core` (the engine is GPUI-free — checks in seconds).
Definition of done (ALL must hold; paste the command output as evidence):
1. `cargo fmt --all --check` clean
2. clippy `-D warnings` green on BOTH the default and `--features tray` configs
3. `cargo test --workspace` green
4. No new/changed deps (Cargo.toml or Cargo.lock) unless explicitly approved

## Code constraints
Workspace: `todo!`/`dbg!` denied; `unused_must_use` denied; `deckard-core` is `#![forbid(unsafe_code)]`
+ app crate `unsafe_code = "deny"`; `mem::forget`/`thread_rng` denied. deckard-core additionally
`#![deny(...)]`s unwrap/expect/panic/indexing_slicing in non-test code (untrusted bytes go through the
bounded `Reader`; genuine fatal boundaries use a scoped `#[allow]` + reason).
🔐 Always: never log or `Debug`-print a seed, key, or passphrase; secrets stay in `Zeroizing`.
```

**Why for an agent.** An explicit "definition of done with evidence" is the single most effective
guardrail against an agent declaring victory while red. The constraints duplicate what the lints
enforce, in prose, so the agent internalizes them *before* writing rather than learning from a
failed build.

**Tradeoff.** Two files to keep in sync (`CLAUDE.md` and `AGENTS.md`). Keep them short and point
both at this doc for the rationale.

---

## Deliberately NOT recommended (so we don't over-rotate)

| Tempting | Why we skip it |
|---|---|
| `#![forbid(unsafe_code)]` at the **workspace root** | No escape hatch if an `objc2` bump needs raw `unsafe` in the tray path. Use root `deny` + `forbid` only on `deckard-core`. |
| `clippy::pedantic` / `clippy::nursery` globally | Too noisy for a small UI app; nursery lints are unstable → a toolchain bump can surface new warnings and break `-D warnings`. Cherry-pick instead. |
| The full `clippy::restriction` group | Clippy's own docs say never enable it wholesale (it contains mutually contradictory lints). Pick individual ones. |
| `indexing_slicing = "deny"` | 🔐 The keystore's binary-format `Reader` does *bounds-checked* raw indexing by design; `deny` forces a rewrite or `#[allow]` litter for no safety gain. |
| `let_underscore_must_use = "deny"` | Breaks 6 intentional `let _ = <fallible>()` discards (e.g. best-effort `sync_all()`, `reply.send()` on a closed channel). |
| Edition 2024 bump | We pin 1.95 in lockstep with gpui's git HEAD; an edition bump is orthogonal churn that risks the gpui pairing. |
| Nightly rustfmt keys / nightly build flags (`-Zthreads`, cranelift, `-Zshare-generics`) | The 1.95 stable pin is **mandatory** for the git gpui build — nightly flags in a committed config brick `cargo build` for everyone. |
| `panic = "abort"` | A wallet wants unwinding so a panic mid-keystore-write doesn't `abort()`; also the tests assert rejection/`is_err()` paths. |
| `multiple-versions = "deny"` in deny.toml | The git gpui stack pulls duplicate versions (objc2 0.5+0.6); permanently red. Keep `warn`. |
| mold/lld linker config | On 1.95, `rust-lld` is already the Linux default and Apple `ld-prime` the macOS default (lld on macOS is a measured *regression*). Do nothing. |

---

## Rollout order (stays green at every step)

1. **Baseline format.** `cargo fmt --all`, commit. Add `rustfmt.toml`. → `--check` now passes.
2. **Lints.** Add `[workspace.lints]` + `[lints] workspace = true` in both manifests + `clippy.toml`.
   Run clippy (both feature configs); fix any wave; the deny-level items here are pre-verified clean.
3. **Unsafe policy.** `#![forbid(unsafe_code)]` on `deckard-core`; `unsafe_code = "deny"` at root.
4. **CI gaps.** Add fmt-check, default-feature clippy, and `cargo test`. Land, confirm green, mark required.
5. **deny.toml.** Seed `[licenses].allow` from a local `cargo deny check licenses` run *first*; land
   the `cargo-deny` job non-blocking; promote to required after one green run.
6. **Docs.** Update `CLAUDE.md`, add `AGENTS.md`.
7. **Tier 2** (see below) incrementally.

The invariant: **every new lint enters as `warn`; every new CI job enters non-blocking. Confirm
green, then tighten.** `main` is never red because of a hardening change.

---

## Provenance

Derived from a multi-agent research sweep of paradigm Rust OSS repos (reth, alloy, Zed, tokio,
ripgrep, Embark) plus a feasibility pass that verified every recommendation against this repo's
actual source — which caught and corrected several plausible-but-wrong suggestions (e.g. the
keystore's `unwrap`s are all test-only; its `Reader` legitimately indexes raw bytes). Tier 2 / Tier
3 (nextest, profile tuning, typed errors, `cargo-machete`, doctests, proptest, MSRV job, release
automation) are tracked separately.
