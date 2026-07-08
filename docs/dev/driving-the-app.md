# Driving the app + capturing screenshots (macOS)

Deckard is a native GPUI app dogfooded on macOS — there is no headless browser to point a
test runner at. When you touch a `crates/deckard-app` view you owe the PR a **before/after
screenshot** (`CLAUDE.md` → Visual verification; the v4 epic #179 DoD). This is the committed,
reusable recipe for driving the running app and capturing per-window screenshots, so every GUI
issue can produce that evidence.

The helper is [`scripts/deckard-drive.sh`](../../scripts/deckard-drive.sh). For a headless
Linux / CI session (no display, no GPU) use the sibling recipe instead:
[`docs/dev/headless-gui-screenshots.md`](./headless-gui-screenshots.md).

## Read the caveats first

Synthetic input on this GPUI app is **partial**, and — this is the one that costs hours —
**capture works even when input doesn't.** `screencapture -l<id>` reads the window's own bitmap
regardless of focus or z-order, so a clean screenshot does **not** prove your clicks/keys landed.
Always confirm state from the *next* screenshot, never from the fact that a command exited 0.

- **Synthetic input only reaches the app while it is the ACTIVE (key) app.** Mouse events route
  by screen location, but a click on an *inactive* app is swallowed as an activation click, and
  keystrokes always go to whatever app is active. `deckard-drive.sh raise` calls
  `NSRunningApplication.activate` and prints `isActive=<bool>` — **if it prints `isActive=false`,
  stop: nothing you type or click will land.** On macOS 14+ an app cannot steal focus from another
  active app, and a GPUI app launched in the **background** (e.g. `just demo` under an agent/CI
  shell, as opposed to a Terminal you're sitting in front of) may refuse activation entirely — it
  renders and captures fine but never becomes key. In that situation you must physically click the
  app once (or launch it from a foreground Terminal), or drive the headless paths below and verify
  logic with tests. See [Honest caveats](#honest-caveats-read-before-you-script-a-flow).
- **Unlock is a CLICK, never Return.** Once the app IS active, the passphrase field auto-focuses,
  but a synthetic `Return` does not submit — GPUI's `InputState` eats it. Type the passphrase,
  then click the button.
- **You can reliably capture SCREENS** (unlock, compose, review, activity) by clicking to
  navigate and screenshotting — provided the app is active per the point above.
- **You cannot reliably fire the hand-rolled key handlers** — the `⌘↵` / click *confirm*, the
  ⌘K query field, Activity `j`/`k`/`x`/`Esc`. Synthetic key events don't land the way the app's
  `on_key_down` expects. Do **not** try to drive a real broadcast from the keyboard.
- **Registered-action shortcuts DO work** — `⌘,` (Settings), `⌘⇧D` (dev toggle) — because they
  dispatch through GPUI's action system, not a per-view key listener.
- To drive a **real value move** end-to-end, use the headless agent (`just demo-deposit` +
  `just demo-agent`), not synthetic clicks. Verify **logic** with `cargo test`, not screenshots.

## One-time setup (macOS permissions)

The controlling terminal app (Conductor, Terminal, iTerm, …) needs two grants. System Settings ›
Privacy & Security ›

- **Accessibility** → enable your terminal app — required for `cliclick` clicks and `osascript`
  keystrokes to reach the app.
- **Screen Recording** → enable the same app — required for `screencapture`.

Confirm the tools are installed:

```bash
scripts/deckard-drive.sh deps        # swift, cliclick, screencapture, osascript
# cliclick missing? -> brew install cliclick   (swift ships with the Xcode Command Line Tools)
```

## 1. Launch the app

**Option A — the funded demo (preferred for evidence).** `just demo` boots a local anvil fork of
Sepolia (pinned block) plus the app + daemon, seals a throwaway vault for anvil account 0, and
lands straight on the **Unlock** screen. It needs an upstream Sepolia **archive** RPC to fork
from (this is anvil's `--fork-url`; no Deckard binary reads it). A free archive endpoint works:

```bash
export RPC_URL_SEPOLIA=https://sepolia.drpc.org     # or Alchemy/Infura Sepolia
just demo                                           # unlock passphrase: deckard-demo
```

`just demo` runs in the foreground and tears the anvil fork down on exit, so start it in its own
terminal (or background it and kill it when done). The demo world is fixed:

| | |
|---|---|
| config dir | `~/.deckard/demo` |
| socket | `~/.deckard/demo/signerd.sock` |
| chain | `11155111` (Sepolia fork on anvil `:8545`) |
| wallet | anvil account 0 — `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` |
| **passphrase** | **`deckard-demo`** |

Then, from another terminal (the app must be **unlocked** first):

```bash
just demo-deposit          # send a real 0.02 ETH inbound transfer (a positive delta)
just demo-deposit 0.15     # over the per-tx cap → drives the human-approval beat
just demo-agent            # run the headless watch-and-shield agent loop
just demo-fund             # (optional) top the wallet up with 10 ETH
```

**Option B — `just run`** launches the everyday app against your real config (no fork). **Option
C — `just qa`** seals a throwaway anvil vault under `/tmp/deckard-qa` (chain `31337`, passphrase
`deckard-qa`) and boots straight to Unlock — the lightest way to reach an unlocked wallet when
you don't need the funded fork; start `anvil --chain-id 31337` first for live balances.

## 2. Unlock and capture (the quick path)

Once the app is on the Unlock screen:

```bash
scripts/deckard-drive.sh unlock                 # types `deckard-demo`, CLICKS Unlock, shots
scripts/deckard-drive.sh shot home              # -> .context/shots/home.png
```

`unlock` takes the passphrase as an optional arg (`unlock deckard-qa` for the QA vault). Screenshots
land in `.context/shots/` (the gitignored scratch dir) by default — override with
`DECKARD_SHOT_DIR`. Attach the labelled PNG to the PR.

## 3. Navigate and capture other screens

```bash
scripts/deckard-drive.sh win                    # "<id> <x> <y> <w> <h>" of the Deckard window
scripts/deckard-drive.sh click 0.5 0.7          # click at window-relative FRACTIONs (0..1)
scripts/deckard-drive.sh clickpt 120 400        # ...or window-relative POINTs from top-left
scripts/deckard-drive.sh type "0.02"            # type into the focused field
scripts/deckard-drive.sh key , cmd              # ⌘,  (Settings — a registered action, works)
scripts/deckard-drive.sh shot review            # capture the current screen
```

Clicks use **window-relative** coordinates, so they survive the window moving between calls (the
finder re-reads the live bounds each time). Compute a fraction from a screenshot: the button at
pixel `(x, y)` in an `W×H` capture is fraction `(x/W, y/H)`. To place a button precisely, `shot`
first, read the fraction off the image, then `click`.

## How it works (rebuild it without the script)

Three macOS primitives, which is all the helper wraps:

**Find the window** with a Swift `CGWindowListCopyWindowInfo` snippet — the largest on-screen
layer-0 window whose owner name contains `deckard`. It prints the window id + top-left-origin
bounds in logical points (the same space `cliclick` and `screencapture` use, so no conversion):

```swift
import CoreGraphics; import Foundation
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements],
                                      kCGNullWindowID) as? [[String: Any]] ?? []
for w in list where (w[kCGWindowOwnerName as String] as? String ?? "").lowercased().contains("deckard") {
  guard (w[kCGWindowLayer as String] as? Int ?? -1) == 0 else { continue }   // 0 == a real window
  let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
  print(w[kCGWindowNumber as String] as? Int ?? -1,
        Int(b["X"] as? Double ?? 0), Int(b["Y"] as? Double ?? 0),
        Int(b["Width"] as? Double ?? 0), Int(b["Height"] as? Double ?? 0))
}
```

**Capture that window** by id — this dodges occlusion. Region capture (`screencapture -R x,y,w,h`)
reads back whatever is *composited* at those screen coordinates, so Conductor/Zed/Claude sitting
on top give you the wrong app's pixels. `-l<id>` grabs the Deckard window's own bitmap regardless
of z-order:

```bash
screencapture -x -o -l<CGWindowID> out.png       # -x no sound, -o no shadow
```

**Send input** with `cliclick` (clicks/typing at screen points) and `osascript` (keystrokes for
registered actions). Screen point for a window-relative fraction `(fx, fy)` is
`(win_x + fx·win_w, win_y + fy·win_h)`.

## Honest caveats (read before you script a flow)

Two things make synthetic input uneven here: the app must be the **active/key** app for *any*
input to land (macOS activation), and GPUI **hand-rolls per-view key handling** with focus quirks
(so even when active, some keys don't behave). What actually happens:

| Interaction | Synthetic input? | Do this instead |
|---|---|---|
| **App is active/key** (prerequisite for ALL input) | ⚠️ only if `raise` reports `isActive=true`; a background/agent-launched app on macOS 14+ may never activate | run from a foreground Terminal, or physically click the app once; capture still works regardless |
| **Unlock submit** | ❌ `Return` doesn't submit (`InputState` eats it) | **Click** the Unlock button (`unlock` does this — once the app is active) |
| **Compose / review SCREENS** | ✅ reachable by clicking to navigate | click through, then `shot` |
| **`⌘↵` / click confirm** (arm-delay send) | ❌ doesn't reliably fire the confirm | never drive a real send from the keyboard |
| **⌘K palette query** | ❌ the query field is flaky under synthetic keys | screenshot the palette open; don't script typed search |
| **Activity `j`/`k`/`x`/`Esc`** | ❌ hand-rolled `on_key_down`, doesn't land | verify selection/stop **logic** with `cargo test` |
| **`⌘,` Settings, `⌘⇧D` dev toggle** | ✅ registered actions dispatch fine | `key , cmd` / `key d cmd shift` |
| **Real value move** (deposit → shield) | n/a — don't fake it | `just demo-deposit` + `just demo-agent` (headless agent) |

The rule of thumb: **capture SCREENS via clicks; drive real BROADCASTS through the headless
agent; verify LOGIC via tests, not screenshots.** A screenshot proves a screen renders; it does
not prove the confirm/keyboard path works — that's what the workspace test suite is for.

Other gotchas:

- **`raise` before a click batch, and heed its `isActive`.** A stray `cargo run` / terminal /
  chat app grabbing focus makes input vanish. The helper calls `raise` (→
  `NSRunningApplication.activate`) before every click/type; if input isn't landing, run `raise`
  on its own and read the `isActive=` line — `false` means you're driving a window that can't take
  focus, so fix that first (foreground Terminal / physical click) instead of retrying blindly.
- **Never `pkill -f 'target/debug/deckard'`.** The pattern matches your own shell's command line
  and can kill the session. Kill the app by explicit PID, or `pkill -x deckard` (note: process
  names truncate at 15 chars, so `deckard-signerd` won't match `-x` — kill it by PID).
- Clicks are in **logical points** (Retina-safe): `CGWindowBounds` and `cliclick` agree, so a
  fraction of the window maps correctly on any display scale.

## Attaching a screenshot to a PR

`gh` **cannot** upload an image into GitHub markdown — that path is the web UI's private upload
endpoint, so a comment/PR body built from the CLI can only *reference* an image by URL. GitHub
renders `![](url)` for any URL that returns image bytes, and this is a **public** repo, so it
serves its own files at `raw.githubusercontent.com`. [`scripts/pr-image.sh`](../../scripts/pr-image.sh)
parks the PNG on a dedicated orphan **`assets`** branch (never merged, never in a PR diff, never in
your working tree — it is built with git plumbing) and prints paste-ready markdown:

```bash
scripts/pr-image.sh --prefix 198 .context/shots/BEFORE-you-are-sending.png \
                                 .context/shots/AFTER-dapp-requests.png
# → ![198-BEFORE…](https://raw.githubusercontent.com/<owner>/<repo>/assets/198-BEFORE….png)
#   ![198-AFTER…](https://raw.githubusercontent.com/<owner>/<repo>/assets/198-AFTER….png)
```

It **appends** to the `assets` branch (old images survive) and replaces any same-named file. It
refuses to run against a private repo, whose raw URLs won't render for viewers — drag-drop into the
web UI instead there.

## Linux / CI

For a headless Linux or Claude-on-the-web session, the mechanics are different (virtual X display
+ software Vulkan, `xdotool` for input, `import -window` for capture). That reproducible recipe
lives in [`docs/dev/headless-gui-screenshots.md`](./headless-gui-screenshots.md).
