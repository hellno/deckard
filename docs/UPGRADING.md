# UPGRADING — keeping GPUI current

Deckard ships on the **git channel** by default: `gpui` + `gpui_platform` from Zed's repo and
`gpui-component` (+ `gpui-component-assets`) from Longbridge's, all pulled from git. This is the only
way to pair *fresh* gpui with the component kit — Longbridge develops gpui-component against Zed's gpui
**HEAD**, while the crates.io releases pin an ~8-month-old gpui snapshot. Background:
[LEARNINGS §2](LEARNINGS.md#dependencies).

**Reproducibility** comes from the committed `Cargo.lock`: it pins the exact upstream commits, so every
clone (and CI) builds identical bytes until *you* choose to bump. You don't track every commit — you bump
on a cadence (monthly is plenty).

## Toolchain

The git channel needs a **recent Rust stable**, pinned in `rust-toolchain.toml` (currently `1.95.0`) to
match Zed's. Zed's gpui uses freshly-stabilized `std` APIs (e.g. `std::hint::cold_path`, stable as of
1.95.0), so an older stable fails with `use of unstable library feature …`. **When you bump gpui, also
check Zed's toolchain and match it:** <https://github.com/zed-industries/zed/blob/main/rust-toolchain.toml>

## The monthly bump (default git channel)

1. **`just bump-gpui`** — updates the four git crates to the latest upstream commits and rebuilds.
2. **Unstable-feature error?** Bump `rust-toolchain.toml`'s `channel` to match Zed's (link above), rebuild.
3. **Fix any API drift.** It surfaces only in Deckard's small UI surface:
   - `src/main.rs` — the bootstrap (`gpui_platform::application()`, `with_assets`, `init`, menus, window)
   - `src/shell.rs` / `onboarding.rs` / `welcome.rs` / `settings_view.rs` / `palette.rs` / `receive.rs` — the views (`Button`, `Input`, `Switch`, flex, actions, focus)
   - `src/theme.rs` — the theme tokens
   - `src/tray.rs` — the `--features tray` status item
   > For reference, the Oct-2025 → mid-2026 jump needed only four kinds of one-line edits:
   > `Application::new()` → `gpui_platform::application()`, a new `Menu { disabled: false, .. }` field,
   > a `cx` argument on `window.focus(&handle, cx)`, and dropping `let _ =` on the now-infallible
   > `cx.update(..)` in the tray.
4. **Smoke-test:** `cargo run`, then `cargo run --features tray`. Click the theme toggle, accent picker,
   settings page, and menu bar — runtime contracts (init order, `Root` as top layer) aren't caught by `cargo build`.
5. **Commit** `Cargo.lock` (+ `rust-toolchain.toml` if you bumped it). Note what drifted — it speeds up next time.

## Pinning to a specific commit (optional)

Float-deps + committed `Cargo.lock` already pins exact commits. If you want the pin visible in
`Cargo.toml` too, add `rev = "<sha>"` to each git dep — but keep `gpui`'s rev compatible with the
`gpui-component` commit (read the Zed sha out of gpui-component's own `Cargo.lock` at your chosen
gpui-component commit). Floating + lockfile is simpler and what we ship.

## Fallback: the crates.io channel (plain-stable, zero git deps, but stale)

Prefer simple builds on plain stable and don't mind an old gpui? Swap the GPUI block in `Cargo.toml`
back to the published pair:

```toml
gpui = "0.2"
gpui-component = "0.5"
gpui-component-assets = "0.5"
```

Then: remove the `gpui_platform` dependency; in `src/main.rs` revert the bootstrap
(`gpui_platform::application()` → `Application::new()`, re-add `Application` to the `gpui` import) and
drop the `disabled: false` menu fields; in `src/shell.rs` drop the `cx` arg
(`window.focus(&focus_handle)`); re-add `let _ =` on the tray's `cx.update(..)` calls in `src/tray.rs`;
and set `rust-toolchain.toml` back to `channel = "stable"`. This builds on stable but **freezes gpui at
the Oct-2025 snapshot** (no wgpu Linux renderer, etc.) until Zed publishes a new release. Bumping then is
`cargo update` within the `0.x` ranges; cross a minor (`0.5` → `0.6`) by editing the version and reading
the [release notes](https://github.com/longbridge/gpui-component/releases).

## If a bump goes bad

Revert and you're back on the last good lockfile:

```
git checkout -- Cargo.toml Cargo.lock rust-toolchain.toml src/
```

gpui-component yanks ~1 in 5 releases and gpui HEAD is pre-1.0, so an occasional bad bump is normal —
stay on the last green commit and try again next cycle.
