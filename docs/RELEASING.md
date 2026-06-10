# Releasing Deckard — maintainer runbook

A repeatable checklist for cutting a Deckard release. Written for `0.0.1-alpha`; the same steps
apply to every future cut (substitute the version string throughout).

> **Status of what you are shipping.** Deckard is a native, self-custodial Ethereum wallet
> (GPUI + Rust; macOS + Linux). **`0.0.1-alpha` is ALPHA, pre-1.0, EXPERIMENTAL software.**
> It is **not production-ready**, has had **no third-party security audit**, and must **never** be
> used with real funds or real mainnet keys — **testnet / throwaway keys only**. Carry that framing
> into the release title, the notes, and the GIF caption. Do not imply production-readiness anywhere.
>
> What is actually built vs. gated is the ground truth in [`STATUS.md`](../STATUS.md). At
> `0.0.1-alpha`: encrypted BIP-39 keystore + onboarding, live on-chain balances (Multicall3),
> receive (address + QR), the command palette, the amber-on-near-black design system, Helios-verified
> reads, the process-isolated signer daemon, and the shield hero (wired + black-box tested on an anvil
> fork) all work. **Send is gated ("next release"), Swap is TODO, the agent/MCP surface
> (`deckard-mcp`) is not built, and the receive-watcher auto-detect is TODO.** Do not claim any of
> those four are finished in release notes.

This runbook touches **only** docs, tags, and GitHub — it does not change engine behaviour. Releasing
is a deliberate act: take it slowly and do not skip the pre-flight.

---

## Conventions (read once)

- **Version string.** The Cargo manifest `version` **is** the full pre-release string —
  e.g. `0.0.1-alpha` (SemVer pre-release syntax), not a separate `0.0.1` + a `-alpha` suffix bolted
  on later. The **git tag mirrors it verbatim with a leading `v`**: manifest `0.0.1-alpha`
  → tag `v0.0.1-alpha`. Keep these two in lockstep for every release.
- **The four crates** (virtual Cargo workspace, all under `crates/`): `deckard-app` (the GPUI app,
  binary `deckard`), `deckard-core` (headless engine), `deckard-contract` (frozen wire contract),
  `deckard-signerd` (signer daemon). All carry `license = "AGPL-3.0-or-later"`.
- **License.** AGPL-3.0-or-later — see [`LICENSE`](../LICENSE) and [`NOTICE`](../NOTICE). Deckard was
  forked from the [`deck`](https://github.com/hellno/deck) GPUI starter (originally 0BSD, which
  permits relicensing). Do not strip `NOTICE`; it carries the upstream + bundled-asset attributions.
- **Repo:** <https://github.com/hellno/deckard> · **Security contact:** GitHub private vulnerability reporting (Security → Report a vulnerability)
- **Toolchain** is pinned in [`rust-toolchain.toml`](../rust-toolchain.toml) (currently `1.95.0`) — use
  it; do not build a release on an ad-hoc toolchain.

---

## 1. Pre-flight — Definition of Done all green, tree clean

Do **not** start a release while any check is red. Run all four and paste the output into the release
PR / cut notes as evidence. These are verbatim the project's Definition of Done (all must hold):

1. `cargo fmt --all --check` clean
2. `just check` green (clippy `-D warnings` on **both** the default and `--features tray` configs)
3. `cargo test --workspace` green
4. No new/changed dependencies in `Cargo.toml` / `Cargo.lock` unless explicitly approved

```bash
cargo fmt --all --check
just check
cargo test --workspace
git diff --stat -- Cargo.toml Cargo.lock   # expect: no unexpected dep churn
```

> **Test caveats to remember (do not treat as failures).** Some integration tests are `#[ignore]`
> and need `anvil` + an archive RPC (the shield e2e), and `anvil_e2e` silently skips if `anvil`
> isn't installed. These are documented in [`STATUS.md`](../STATUS.md) — a green `cargo test
> --workspace` is the bar for the cut; the deeper anvil/archive runs are a separate, manual
> confidence pass.

Then confirm the working tree is clean and you are on the intended commit:

```bash
git status --porcelain   # expect: no output (clean tree)
git log --oneline -1     # the commit you are about to tag
```

Decide the release branch now: cut from `main` (or the merge commit that lands the release PR). Do not
tag a feature branch.

---

## 2. Version bump — all 4 crate manifests + sync the lockfile

Set the **same** `version` in every crate manifest. Note that at the first cut these may be
out of sync (some crates carried `0.1.0`); bringing them all to the release string is part of the bump.

Edit the `version = "…"` line under `[package]` in each of:

- `crates/deckard-app/Cargo.toml`
- `crates/deckard-core/Cargo.toml`
- `crates/deckard-contract/Cargo.toml`
- `crates/deckard-signerd/Cargo.toml`

```toml
[package]
version = "0.0.1-alpha"
```

Then sync `Cargo.lock` so the recorded crate versions match (this is the *only* dependency-graph
change allowed during a routine cut — it is a version sync, not new deps):

```bash
cargo update --workspace --offline   # re-resolves the four workspace crates to the new version
# (a plain `cargo build`/`cargo check` also rewrites Cargo.lock; either is fine)
git diff -- Cargo.lock                # sanity-check: only the deckard-* versions changed
```

> Do **not** hand-edit the GPUI git pins in `Cargo.lock`; those are bumped only via `just bump-gpui`
> (see [`docs/UPGRADING.md`](UPGRADING.md)) and are out of scope for a version cut.

Verify all four moved together:

```bash
git grep -nE '^version = ' -- 'crates/*/Cargo.toml'
```

---

## 3. Update `CHANGELOG.md`

Move the accumulated `Unreleased` notes into a dated section for the new version.

- If `CHANGELOG.md` does not exist yet (first release), create it at the repo root in
  [Keep a Changelog](https://keepachangelog.com) style.
- For each subsequent release, you only do the *move*: rename `Unreleased` to the version + today's
  date, then open a fresh empty `Unreleased` block at the top for the next cycle.

```markdown
# Changelog

All notable changes to Deckard are documented here. Format: Keep a Changelog; versioning: SemVer
(pre-release tags like `-alpha` per the convention in docs/RELEASING.md).

## [Unreleased]

## [0.0.1-alpha] — 2026-06-10
### Added
- First public alpha. Encrypted BIP-39 keystore + onboarding; live on-chain balances (Multicall3);
  receive (address + QR); command palette; Helios-verified reads (no third-party RPC trusted by
  default); process-isolated signer daemon (`deckard-signerd`); the shield hero (auto-private via
  Railgun) — wired and black-box tested on an anvil fork.

### Known limitations (alpha)
- **Not production-ready. No third-party audit. Testnet / throwaway keys only — never real funds.**
- Send UI is gated ("next release"); Swap is TODO; the agent/MCP surface (`deckard-mcp`) is not built;
  receive-watcher auto-detect is TODO.
```

Keep the changelog honest — it is the user-facing record. Mirror the "alpha / not for real funds"
framing here too; do not list Send / Swap / MCP / receive-watcher under "Added".

---

## 4. Record the demo GIF → `docs/demo.gif`

The GIF should show the **final** UI of the release you are cutting (record after the version bump,
against a clean build). Drive a real flow — e.g. onboarding → funded portfolio → the shield hero
making a balance private. Use testnet / throwaway keys on camera, **never** a real seed.

**macOS prerequisites (one-time):** grant **Screen Recording** permission (System Settings →
Privacy & Security → Screen Recording) to whatever captures the screen, and **Accessibility**
permission if you script the click-through. Without both, the capture is black or the synthetic input
is dropped.

1. **Build + run the final app:**

   ```bash
   just run        # builds deckard-signerd, then runs the app (the 99%-of-the-time command)
   ```

   (For a perf-true capture you can use `just run-release` instead.)

2. **Screen-record the flow.** Use macOS screen recording (`⇧⌘5`, or your preferred recorder) to
   capture the app window through the demo flow. Record at a steady pace; keep it short (a tight
   ~10–20s loop reads better and stays small).

3. **Convert the recording to a clean, small GIF.** Two-pass `palettegen`/`paletteuse` gives a sharp
   palette at a small size. Target **< 8 MB**, **~12 fps**, **width ~1000px**:

   ```bash
   # 1) Generate an optimized palette from the source recording.
   ffmpeg -i recording.mov \
     -vf "fps=12,scale=1000:-1:flags=lanczos,palettegen=stats_mode=diff" \
     -y docs/.demo-palette.png

   # 2) Apply the palette to produce the GIF.
   ffmpeg -i recording.mov -i docs/.demo-palette.png \
     -lavfi "fps=12,scale=1000:-1:flags=lanczos[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=5" \
     -y docs/demo.gif

   # 3) Clean up the scratch palette and check the size.
   rm docs/.demo-palette.png
   ls -lh docs/demo.gif
   ```

   If it lands over ~8 MB, trim the recording, drop to `fps=10`, or narrow to `scale=900:-1`. Confirm
   the GIF renders correctly (open it / preview the README) before committing.

4. Reference `docs/demo.gif` from the README if it isn't already, and commit it with the release.

---

## 5. Tag + GitHub release

Commit the version bump + changelog (+ GIF) first, on the release commit. Then create an **annotated**
tag whose message becomes the release body, push it, and publish a **pre-release** on GitHub.

```bash
# Commit the cut (version bump, CHANGELOG, demo.gif).
git add crates/*/Cargo.toml Cargo.lock CHANGELOG.md docs/demo.gif
git commit -m "release: 0.0.1-alpha"

# Annotated tag — mirrors the manifest version with a leading v.
# The annotation text is reused as the release notes via --notes-from-tag below.
git tag -a v0.0.1-alpha -m "Deckard 0.0.1-alpha — first public alpha

ALPHA / EXPERIMENTAL / pre-1.0. Not production-ready, no third-party security audit.
Testnet or throwaway keys only — do NOT use with real funds or real mainnet keys.

Works: encrypted BIP-39 keystore + onboarding, live on-chain balances, receive (address + QR),
command palette, Helios-verified reads, process-isolated signer daemon, the shield hero (wired +
black-box tested on an anvil fork).
Not yet: Send (gated, next release), Swap (TODO), agent/MCP surface (not built), receive-watcher (TODO).

License: AGPL-3.0-or-later. Security: report privately via GitHub Security Advisories."

# Push the commit and the tag.
git push origin main          # or the release branch you cut from
git push origin v0.0.1-alpha

# Publish a PRE-RELEASE on GitHub, reusing the tag annotation as the notes.
gh release create v0.0.1-alpha \
  --prerelease \
  --title "Deckard 0.0.1-alpha (experimental)" \
  --notes-from-tag
```

`--prerelease` is **required** for an alpha — it keeps GitHub from marking it "Latest" and signals to
visitors this is not a stable build.

### Optionally attach the macOS app bundle

If you want a downloadable build on the release, produce the `.app` and attach it. Make clear in the
asset description / notes that it is unsigned, experimental, and testnet-only.

```bash
just bundle    # → target/release/bundle/osx/Deckard.app  (needs: cargo install cargo-bundle)

# Zip the .app (GitHub release assets must be single files), then upload it to the release.
ditto -c -k --sequesterRsrc --keepParent \
  target/release/bundle/osx/Deckard.app Deckard-0.0.1-alpha-macos.app.zip
gh release upload v0.0.1-alpha Deckard-0.0.1-alpha-macos.app.zip
```

> The bundle is **not code-signed or notarized**; on first launch macOS Gatekeeper will warn. That is
> expected for an experimental alpha. Mention it in the release notes so users aren't surprised.

---

## 6. Visibility — flipping private → public is a deliberate decision

Making the repository public is a **conscious maintainer choice, not a routine step of cutting a
release.** A version can be tagged and a (pre-)release published while the repo stays private. Before
flipping `hellno/deckard` from private to public, deliberately confirm:

- The security framing is unambiguous everywhere a newcomer lands (README, release notes, this
  runbook): **alpha, unaudited, testnet/throwaway keys only, never real funds.**
- `LICENSE` (AGPL-3.0-or-later) and `NOTICE` (upstream `deck` + bundled-asset attributions) are
  present and accurate.
- No secrets, seeds, or private keys are anywhere in the history. Nothing in `Zeroizing`-protected
  paths leaked into a fixture, log, or commit.
- GitHub private vulnerability reporting is enabled (Security → Report a vulnerability) for responsible disclosure.

Only when all of that holds should a maintainer flip visibility. Treat it as its own reviewed action.

---

## Quick reference

| Step | Command(s) |
|---|---|
| Pre-flight DoD | `cargo fmt --all --check` · `just check` · `cargo test --workspace` · clean `git status` |
| Bump version | edit `version` in all 4 `crates/*/Cargo.toml`, then sync `Cargo.lock` |
| Changelog | move `Unreleased` → `[0.0.1-alpha] — <date>` in `CHANGELOG.md` |
| Demo GIF | `just run` → record → `ffmpeg` palettegen/paletteuse → `docs/demo.gif` |
| Tag + release | `git tag -a v0.0.1-alpha …` · `git push origin v0.0.1-alpha` · `gh release create … --prerelease --notes-from-tag` |
| Bundle (optional) | `just bundle` → zip → `gh release upload` |
| Go public | deliberate maintainer decision, **not** part of the routine cut |
