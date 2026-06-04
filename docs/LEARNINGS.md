# LEARNINGS — building a native Mac app on GPUI

Everything worked out while extracting a fork-ready macOS Deck from Zed's GPUI and
gpui-component. This is the "why" behind every decision in the repo, and the direct answers to:
*how nice can the theme get? what icons can I use? how do I save preferences? can I have a
menu-bar icon, a dock icon, a tray-first app? how do app icons work? how much should I bundle?*

All API claims below were verified against the actual crate sources that this Deck compiles
against: `gpui 0.2.2` and `gpui-component 0.5.1` (the published crates.io versions), plus the Zed
source tree for context. File/line refs point into those.

---

## 1. The big picture: three layers

```
┌─────────────────────────────────────────────┐
│ your app  (src/*.rs)                          │  ~700 lines, a few files
├─────────────────────────────────────────────┤
│ gpui-component 0.5   buttons, inputs, theme, │  shadcn-style kit
│                      TitleBar, Root, icons   │
├─────────────────────────────────────────────┤
│ gpui 0.2   windows, elements, flexbox,       │  Zed's UI framework
│            Metal renderer, actions, menus    │  (longbridge fork)
└─────────────────────────────────────────────┘
```

GPUI is Zed's retained-mode, GPU-accelerated UI framework: a React-ish model (`Render` views that
hold state and re-render on `cx.notify()`), a flexbox API that reads like Tailwind
(`div().flex().gap_2().p_4()`), a Metal renderer on macOS, an async executor, and native plumbing
(windows, menu bar, key dispatch). gpui-component adds the *look*: a themeable component library.

The hard parts (native window, GPU text, event loop, retained UI) are **inherited**. What a Deck
**adds** is the chrome that makes it feel like an app: a window that looks native, a menu bar,
shortcuts, a theme that isn't harsh, a place to store preferences, an icon, and a `.app`.

---

## 2. The dependency decision (the most important learning) {#dependencies}

There are **three** different GPUI lineages, and mixing them does not compile. Picking the right one
is the whole game for a clean `cargo run` fork.

| Crate | Source | Renderer | Use it when |
|---|---|---|---|
| `gpui` (git) | `github.com/zed-industries/zed` | Metal/Vulkan | You vendor Zed; heavy first build, no crates.io |
| **`gpui` 0.2.x** | **crates.io (longbridge fork)** | **Metal (macOS) + Vulkan (Linux)** | **This Deck — matches gpui-component** |
| `gpui-unofficial` 0.231 | crates.io (Zed wgpu fork) | wgpu | Only for headless servers / SwiftShader fallback; needs vendored gpui-component |

The trap: **`gpui-component 0.5.1` on crates.io is built against `gpui` 0.2.x**, *not*
`gpui-unofficial`. If you mix `gpui-unofficial` with crates.io `gpui-component`, both halves compile
but your `Render` view satisfies one crate's `Render` trait while `open_window`/`Root::new` want the
other's — `E0277` trait-mismatch errors at *your* call sites (the dep crates compile fine, which is
what makes it confusing).

**Decision: pure crates.io, matched pair** — `gpui = "0.2"` + `gpui-component = "0.5"` +
`gpui-component-assets = "0.5"`. No git deps, no submodules, no vendoring. This same pair is
**cross-platform**: `gpui` 0.2 ships a `platform/linux/` backend and the `blade` renderer, so it
renders with **Metal on macOS** and **Vulkan on Linux** (X11 + Wayland are both default features). The
`gpui-unofficial` + wgpu fork is a *different* lineage you only need for the narrow headless-server /
SwiftShader case (a GPU-less box where Vulkan can't init) — not for normal desktop Linux.

What the Deck does to stay portable: the only macOS-only code is the tray's dock-hiding
(`setActivationPolicy`), which is `#[cfg(target_os = "macos")]` and whose `objc2` deps are
**target-gated** in `Cargo.toml` (`[target.'cfg(target_os = "macos")'.dependencies]`) so a Linux
`--features tray` build never tries to compile Apple crates. Shortcuts use `secondary` (= ⌘ on macOS,
Ctrl elsewhere) instead of `cmd`, and key-hint chips swap ⌘ for Ctrl via `cfg`. `tray-icon`,
`directories` (XDG vs App Support), and gpui-component's `TitleBar` (traffic lights vs window-control
buttons) all adapt per-OS on their own.

---

## 3. Window chrome & the Linear look

Default `WindowOptions` gives a **standard** titled macOS window
(`titlebar: Some(TitlebarOptions { appears_transparent: false, .. })`). For the modern
frameless/unified look (content under the traffic lights), opt in with the recipe gpui-component
provides:

```rust
titlebar: Some(gpui_component::TitleBar::title_bar_options()),
// → { appears_transparent: true, traffic_light_position: Some(point(9., 9.)), title: None }
```

Then render `TitleBar::new().child(..)` as the **first** child of your root `flex_col`. That's the
whole trick. **Two rules that both panic if violated:** `Root::new(view, ..)` must be the literal top
layer of the window, and `gpui_component::init(cx)` must run before you open any window (it installs
the `Theme` global; otherwise the first `cx.theme()` panics).

---

## 4. Theme — making it not-harsh, with a live accent {#theme}

> *"The default theme is very harsh black and white. What's the common pattern to make it nicer?"*

gpui-component's stock dark theme is near pure-black-on-white. The common pattern (Linear, GitHub,
Zed) is: a **soft** near-black (not `#000`) with slightly-elevated surfaces, **muted** secondary
text, and one saturated **accent** that carries the brand. Three things, really:

1. `background` → a soft near-black with a faint cool cast (`#0C0D11`), not `#000`.
2. `foreground` → a soft white (`#E6E7EB`), not `#FFF`; secondary text via `muted_foreground`.
3. `primary` → a real accent color, not white. This is what makes it feel designed.

### How to apply it (the toggle-safe way)

`Theme::change(mode, ..)` re-applies a `ThemeConfig` on every switch, so don't poke at
`cx.theme().colors` directly — your edits get wiped on the next toggle. Instead **clone the built-in
`ThemeConfig`, override the tokens you care about, and set it as the dark/light config** (`theme.rs`):

```rust
let mut dark = (**ThemeRegistry::global(cx).default_dark_theme()).clone();
dark.colors.background = Some("#0C0D11".into());
dark.colors.foreground = Some("#E6E7EB".into());
dark.colors.primary    = Some(accent_hex.into());      // the brand color
dark.colors.ring       = Some(accent_hex.into());
// …~15 more tokens…
Theme::global_mut(cx).dark_theme = Rc::new(dark);
Theme::change(ThemeMode::Dark, None, cx);              // now re-applies your config
```

`ThemeConfigColors` is ~80 `Option<SharedString>` hex fields (`schema.rs`); unset ones keep the
built-in value. Note the `accent` *token* is a subtle hover **surface**, not the brand — the brand is
`primary`. In a view you read tokens via `cx.theme().<token>` (the `Theme` `Deref`s to its
`ThemeColor`), all `Hsla` you drop straight into `.bg(..)` / `.text_color(..)`.

### Live accent

Because the palette is just a function of one accent color, the settings page can offer a swatch
picker that rebuilds the configs and calls `Theme::change` — the whole app (logo, buttons, focus
rings, even the tray icon) recolors instantly. That's `Shell::set_accent` → `theme::install`.

### The other option: theme JSON files

gpui-component can also hot-load `.theme` JSON files via
`ThemeRegistry::watch_dir(dir, cx, on_load)` (it ships a JSON schema). Great if you want
user-editable / downloadable themes. The Deck builds the palette in code instead because it's
self-contained and lets the accent picker mutate it live — but the JSON path is there when you want
it. **Omakase pick: dark by default, indigo accent, in-code palette.**

---

## 5. Settings & preference storage {#settings}

> *"Ship a basic preference-saving infrastructure. What's the common, mainstream way? What does Zed
> do? What's the most Rust path?"*

**The mainstream Rust path, and what this Deck ships:** a `serde`-derived struct written as JSON
into the OS config directory, found with the [`directories`](https://crates.io/crates/directories)
crate. No database, no framework. The entire layer is ~40 lines (`settings.rs`):

```rust
#[derive(Serialize, Deserialize)]
#[serde(default)]                  // ← missing fields fall back to Default: forward-compatible
struct Settings { theme_mode: ThemeModePref, accent: Accent, display_name: String, /* … */ }

fn path() -> Option<PathBuf> {
    ProjectDirs::from("com", "Example", "Deck")     // reverse-DNS, matches the bundle id
        .map(|d| d.config_dir().join("settings.json"))
}
// load(): read_to_string + serde_json::from_str, fall back to Default on missing/corrupt
// save(): create_dir_all + serde_json::to_string_pretty + write   (best-effort)
```

On macOS that resolves to `~/Library/Application Support/com.Example.Deck/settings.json`. Load
once at startup (so the theme reflects saved prefs before the first paint); save on every change.
The two non-obvious bits: **`#[serde(default)]`** so old config files survive you adding fields, and
**using the platform config dir** (not `~/.myapp`) so you're a good macOS citizen.

### The spectrum of options

| Approach | What it is | When |
|---|---|---|
| **`directories` + `serde_json`** (this Deck) | ~40 lines you own, fully visible | Most apps. Mainstream, zero magic. |
| [`confy`](https://crates.io/crates/confy) | One-liner wrapper over exactly the above (`confy::load`/`store`, TOML by default) | You want it in two lines and don't care where the file is. |
| **Zed's settings system** | Layered **default + user** JSON, file-watched, hot-reloaded, schema-validated, merged into typed structs | A large app with a settings *file* users hand-edit. Overkill for a starter — it lives across several Zed crates, not in gpui. |
| `rusqlite` / a KV store | A real database | Lots of rows (history, documents), queries, migrations. |

**What Zed does**, concretely: it has a dedicated `settings` crate that loads a bundled
`default.json`, merges a user `settings.json` on top, watches both for changes, and deserializes into
typed `Settings` structs via `serde` + `schemars` (the JSON Schema powers editor autocomplete). It's
excellent and it's a lot — the right altitude for an editor users configure by hand, the wrong one
for a fork-and-hack starter. **Omakase pick: `directories` + `serde_json`**, with `confy` as the
two-line alternative noted in the code.

---

## 6. Icons — Lucide, already bundled, ISC-licensed {#icons}

> *"Is there a common icon pack we can include with the right license to use freely?"*

**Yes — and it's already here.** gpui-component's `IconName` enum *is*
[Lucide](https://lucide.dev), and `gpui-component-assets` bundles the SVGs (verified: the files are
`arrow-right.svg`, `book-open.svg`, `bot.svg`, … — verbatim Lucide names). **Lucide is ISC-licensed**
— a permissive, MIT-equivalent license, free for personal and commercial use; just keep the
copyright notice if you redistribute the icons (this repo's [NOTICE](../NOTICE) does). So you get a
clean, modern, ~80-icon set for free, today.

To make them render you must register the asset source once — `IconName::*` is blank otherwise:

```rust
Application::new().with_assets(gpui_component_assets::Assets).run(..)   // one line, in main.rs
```

Then `Button::new("x").icon(IconName::ArrowRight)` or `Icon::new(IconName::Settings)`.

### Adding your own icons

The bundled set is the subset `IconName` names. For anything else, embed your own SVG and point an
`Icon` at it by path:

```rust
// embed your SVGs in your own AssetSource (rust_embed) under `icons/…`, then:
Icon::empty().path("icons/my-glyph.svg").size_4().text_color(cx.theme().foreground)
```

For the **full** Lucide set (1000+), download the SVGs you want into your assets folder and reference
them the same way. Note: gpui-component bundles icons but **no font** — the UI font is the system
font; to ship a custom typeface you embed the `.ttf` and set `Theme.font_family`.

---

## 7. The app menu bar — native, you just declare it {#menu}

> *"Can we have a menu bar?"* — Yes, fully native, and one of the nicest things you inherit.

Build it declaratively and hand it over with `cx.set_menus(Vec<Menu>)`:

```rust
cx.set_menus(vec![
    Menu { name: "Deck".into(), items: vec![
        MenuItem::action("About Deck", About),
        MenuItem::separator(),
        MenuItem::action("Settings…", OpenSettings),     // shows ⌘, automatically
        MenuItem::action("Quit Deck", Quit),         // ⌘Q
    ]},
    Menu { name: "Edit".into(), items: vec![
        MenuItem::os_action("Copy",  NewItem, OsAction::Copy),   // native editing selectors
        MenuItem::os_action("Paste", NewItem, OsAction::Paste),
    ]},
]);
```

The shortcut shown next to an item is derived from your `KeyBinding` for that action. macOS
auto-injects **Services**, **Hide**, and the **Window** items. (`OsAction::{Cut,Copy,Paste,SelectAll}`
wire the native editing selectors; `Undo/Redo` are routed oddly in some GPUI builds — handle them
yourself if needed.)

---

## 8. The dock icon, the tray icon, and tray-first apps {#tray}

> *"Can we have a dock icon? A menu-bar (tray) icon? A tray-first, no-dock app? Does that fit GPUI —
> the tray won't be GPUI-rendered, but I don't want a second rendering system."*

**Your instinct is exactly right.** Here's the full picture:

### Dock icon — automatic

Every GPUI app is a `Regular` app (`gpui-0.2.2/src/platform/mac/platform.rs:1390` hardcodes
`NSApplicationActivationPolicyRegular`), so it gets a dock icon for free; `cx.activate(true)`
foregrounds it. You **cannot set the dock icon image from Rust** (there's no `setApplicationIconImage`
binding) — the image + label come from the app bundle's `Info.plist`. So `cargo run` shows a generic
icon + the binary name; `cargo bundle` shows yours. Develop with `run`, ship with `bundle`.

### Tray icon — a native status item, **no second renderer**

A macOS menu-bar item (`NSStatusItem`) is, like the dock icon, **just a native image plus a native
menu**. There is nothing to "render" with a UI framework — so adding one introduces **no second
rendering system**. The tray icon is drawn by AppKit; every *window* you show stays 100% GPUI. This
is the key insight that makes it fit cleanly.

GPUI has **no** status-item API itself (neither does Zed), so it's an opt-in. This Deck ships it
behind `--features tray` using the [`tray-icon`](https://crates.io/crates/tray-icon) crate (`tray.rs`).
The integration works because **GPUI's run loop *is* the standard AppKit `NSApplication.run` loop**:

```rust
// 1. Build the native status item (image + native menu) on the main thread, in app.run.
let tray = TrayIconBuilder::new().with_icon(brand_icon(accent)).with_menu(..).build()?;
cx.set_global(TrayState { tray });          // keep it alive + reachable for live restyle

// 2. tray-icon posts click/menu events to a global channel. Drain it on GPUI's own
//    executor and act on the main thread — no separate event loop:
cx.spawn(async move |cx| loop {
    cx.background_executor().timer(Duration::from_millis(120)).await;
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        if ev.id == quit_id { cx.update(|cx| cx.quit())?; }
        if ev.id == show_id { cx.update(|cx| cx.activate(true))?; }
    }
}).detach();
```

Because the icon is a function of the accent, the settings picker restyles it live
(`tray::set_accent` → `TrayIcon::set_icon`). Two rules: create it **on the main thread** inside
`app.run`, and **keep the handle alive** (drop it and the icon vanishes — we stash it in a GPUI
global).

### Tray-first / no-dock apps

To hide the dock icon (a true menu-bar-only app), flip the activation policy to `Accessory` — GPUI
hardcodes `Regular`, so you override it at runtime via objc2 (already in the tree):

```rust
let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);   // no dock icon, no ⌘-Tab
```

That's the whole `--features tray` story: a native status item + accent-synced icon + an
event bridge + one activation-policy call. The window stays GPUI. (For a *true* tray-first app you'd
also skip opening the window at launch and open it on "Show" — a small extension of `tray.rs`.)

---

## 9. App icons — one PNG in, a `.icns` out

macOS app icons are `.icns` bundles. The pipeline keeps a single source of truth:

```
assets/icon.svg  ──cairosvg/qlmanage──▶  assets/icon.png (1024²)  ──sips+iconutil──▶  assets/icon.icns
```

`just icon` runs the whole chain (uses macOS built-ins `sips` + `iconutil`). And `cargo bundle` will
itself turn a single `icon.png` into the `.icns` at build time — so the *minimum* a forker does is
**replace `assets/icon.png`**. Unlike iOS, macOS does **not** auto-mask icons into the squircle, so
`icon.svg` draws the rounded tile itself (inset in the 1024 canvas, ~180px corner radius).

---

## 10. Bundling — batteries included, not option soup {#bundling}

> *"Is batteries-included right, or should we overload everything with options?"* — **Batteries
> included. Opinionated defaults, escape hatches documented, no option soup.**

The whole bundling story is **one block** in `Cargo.toml`, read by
[`cargo-bundle`](https://github.com/burtonageo/cargo-bundle):

```toml
[package.metadata.bundle]
name = "Deck"
identifier = "com.example.deck"
icon = ["assets/icon.png"]
category = "public.app-category.productivity"
osx_minimum_system_version = "11.0"
```

`cargo install cargo-bundle` then `just bundle` → a double-clickable `Deck.app`. **Left as opt-in
(documented, not automated)** because they need *your* credentials or aren't needed for local use:
code signing / notarization (a paid Apple Developer ID; `codesign` + `xcrun notarytool`), a custom
`Info.plist`, DMG/installer. Zed uses a big hand-rolled `script/bundle-mac` because it ships to
millions — the wrong altitude for a starter. cargo-bundle's declarative block is the right one.

---

## 11. Keyboard shortcuts & actions

Three lines per shortcut:

```rust
gpui::actions!(deck, [NewItem]);                      // 1. declare (namespace is a bare ident)
cx.bind_keys([KeyBinding::new("cmd-n", NewItem, None)]);  // 2. bind a key
div().on_action(cx.listener(|this, _: &NewItem, _w, cx| { this.count += 1; cx.notify(); }))  // 3. handle
```

Keystroke syntax: `cmd`, `ctrl`, `alt`, `shift`, joined with `-`. Gotchas: **`KeyBinding::new`
panics on a malformed string** (keep them literal); the namespace is a **bare identifier**; **no JSON
keymap loader** exists (you bind in code — typo-checked at compile time); and for a key to reach a
view's `on_action`, the view must be in the focus path (the Deck `track_focus`es its root and
focuses it in `Shell::new`).

---

## 12. Components & assets — what's in the box

`gpui-component 0.5.1` modules: `accordion, alert, avatar, badge, breadcrumb, button, chart,
checkbox, … dialog, divider, dock, form, input, kbd, label, link, list, switch, tab, table, tag,
text, tooltip, tree`. Sharp edges that bite forkers:

- **No `Modal`** — the modal primitive is **`Dialog`** (+ a `Sheet` drawer), and overlay layers
  (Dialog/Sheet/Notification) are invisible unless your top view is `Root::new` *and* you emit
  `Root::render_dialog_layer/…`. The Deck's pages are plain views to avoid this until you need it.
- **`Input` is stateful** — `Input::new(&entity)` needs an `InputState` *entity* you own and keep
  alive (`cx.new(|cx| InputState::new(window, cx))`); subscribe to `InputEvent::Change` to react.
  (The settings page does exactly this for the display-name field.)
- **Icons need the asset source** (§6); **no bundled font** — UI font is the system font.

---

## 13. Inherited vs. added — the crisp summary

| Inherited from Zed/GPUI + gpui-component (free) | Added by this Deck |
|---|---|
| Native window, Metal renderer, GPU text, async executor | Frameless title bar wiring; app bootstrap order |
| Dock icon (automatic); the **menu bar API** | The system menu bar; keyboard shortcuts → actions |
| Theme engine + ~80 tokens; component library | **Refined palette + live accent picker** |
| `actions!` / `KeyBinding` / focus dispatch | **Settings page + JSON preference persistence** |
| Lucide icon set (ISC) | App-icon pipeline + `cargo bundle` config |
| async executor + AppKit run loop | Optional **native tray icon + dock hiding** (`--features tray`) |

---

## 14. Gotcha digest

1. Match the GPUI lineage to gpui-component: **`gpui` 0.2 + `gpui-component` 0.5** (§2).
2. `gpui_component::init(cx)` **before** opening a window, or `cx.theme()` panics.
3. `Root::new(view, ..)` must be the **literal top layer** of the window.
4. Pass `TitleBar::title_bar_options()` for the frameless look (the default titlebar is opaque).
5. `.with_assets(gpui_component_assets::Assets)` or your icons are blank.
6. Don't poke `cx.theme().colors` directly — override the `ThemeConfig` so it survives toggles (§4).
7. Dock icon/label is **bundle-only**; `cargo run` shows the binary name. Ship via `cargo bundle`.
8. Tray icon: create on the main thread in `app.run` and **keep the handle alive** (§8).
9. `KeyBinding::new` panics on bad strings; the `actions!` namespace is a bare ident.
10. No `Modal` (use `Dialog`); `Input` is stateful (`InputState` entity); no bundled font.

---

## 15. Omakase decisions (chosen → rejected)

- **Pure crates.io `gpui` 0.2 + `gpui-component` 0.5** → rejected git-vendoring Zed and the
  gpui-unofficial/wgpu fork (heavier, Linux-only upside).
- **Soft near-black palette + one live accent** → rejected the harsh stock theme, and rejected a full
  user-editable theme-file system (available via `watch_dir` when you want it).
- **`directories` + `serde_json` for settings** → rejected `confy` (less visible) and a Zed-style
  layered/hot-reloaded settings system (overkill for a starter); rejected a database (no rows yet).
- **Lucide (already bundled, ISC)** → rejected pulling a second icon pack.
- **`cargo bundle` block + a single `icon.png`** → rejected a Zed-style signing/notarizing script.
- **Tray as an opt-in feature, window stays GPUI** → rejected any second rendering system; the status
  item is a native image, nothing more.
- **A few small files, ~700 lines** → rejected a sprawling, over-engineered monorepo. Read it in ten minutes,
  then start deleting.
