#!/usr/bin/env bash
#
# deckard-drive.sh — drive the running Deckard GPUI app on macOS and capture
# per-window screenshots, so any GUI change can produce before/after evidence.
#
# Deckard is a native GPUI app (no headless browser). This wrapper bundles the
# only combination that works reliably on macOS:
#   • window finder  — Swift CGWindowListCopyWindowInfo → the app's own window id
#                      + on-screen bounds (largest layer-0 window owned by "deckard").
#   • capture        — `screencapture -x -o -l<id>` grabs THAT window's bitmap
#                      regardless of z-order, so Conductor/Zed/Claude sitting on top
#                      never occlude the shot (region capture reads back the wrong app).
#   • input          — `cliclick` for clicks/typing at window-relative coords;
#                      `osascript keystroke` for registered-action shortcuts (⌘, / ⌘⇧D).
#
# One-time grants (System Settings › Privacy & Security):
#   • Accessibility  → the controlling terminal app (e.g. Conductor / Terminal / iTerm)
#                      — required for cliclick + osascript synthetic input.
#   • Screen Recording → the same app — required for screencapture.
#
# Full recipe, honest caveats, and the demo launch story: docs/dev/driving-the-app.md
# Linux / CI (headless Xvfb + software Vulkan) path:      docs/dev/headless-gui-screenshots.md
#
# Usage:
#   scripts/deckard-drive.sh <command> [args]
#
#   win                     print "<id> <x> <y> <w> <h>" for the Deckard window
#   raise                   bring the Deckard app to the front
#   shot <name>             capture the window to $DECKARD_SHOT_DIR/<name>.png
#   click <fx> <fy>         click at window-relative FRACTIONs (0..1), e.g. 0.5 0.7
#   clickpt <px> <py>       click at window-relative POINTs from the top-left
#   type "<text>"           type text into the focused field
#   key <char> [mods...]    registered-action shortcut, mods ∈ {cmd shift opt ctrl}
#                           e.g.  key , cmd     (Settings)     key d cmd shift  (⌘⇧D)
#   unlock [passphrase]     type the passphrase + CLICK Unlock (default: deckard-demo),
#                           then screenshot to $DECKARD_SHOT_DIR/unlocked.png
#   deps                    verify swift + cliclick + screencapture are present
#
# Env:
#   DECKARD_WINDOW_OWNER    owner-name substring to match (default: deckard)
#   DECKARD_SHOT_DIR        screenshot output dir (default: .context/shots)
#   DECKARD_UNLOCK_FRAC     "fx,fy" of the Unlock button (default: 0.5,0.65)
#
# IMPORTANT — synthetic input needs the app to be the ACTIVE (key) app. `raise`
# uses NSRunningApplication.activate, but on macOS 14+ an app cannot steal focus
# from another *active* app, and a GPUI app launched in the background (e.g. this
# repo's `just demo` under an agent/CI shell) may refuse activation entirely
# (`isActive` stays false; it exposes no AXWindow). Capture (`screencapture -l`)
# works regardless of focus; clicks/keys do NOT. See docs/dev/driving-the-app.md
# → "Honest caveats" for when this bites and what to do (physically click; or
# verify logic via `cargo test`, not screenshots).
#
set -euo pipefail

OWNER="${DECKARD_WINDOW_OWNER:-deckard}"
SHOT_DIR="${DECKARD_SHOT_DIR:-.context/shots}"
UNLOCK_FRAC="${DECKARD_UNLOCK_FRAC:-0.5,0.65}"

die() { printf 'deckard-drive: %s\n' "$*" >&2; exit 1; }

# ── window finder ─────────────────────────────────────────────────────────────
# Print "<id> <x> <y> <w> <h>" for the largest on-screen layer-0 window whose
# owner name contains $OWNER. Coordinates are top-left-origin logical points —
# the same space cliclick and screencapture use, so no conversion is needed.
find_window() {
  swift - "$OWNER" <<'SWIFT'
import CoreGraphics
import Foundation

let needle = (CommandLine.arguments.dropFirst().first ?? "deckard").lowercased()
guard let list = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
      ) as? [[String: Any]] else { exit(1) }

var best: (id: Int, x: Int, y: Int, w: Int, h: Int, area: Double)?
for win in list {
  let owner = (win[kCGWindowOwnerName as String] as? String ?? "").lowercased()
  guard owner.contains(needle) else { continue }
  // Layer 0 == a normal app window; title-bar/shadow helpers live on other layers.
  guard (win[kCGWindowLayer as String] as? Int ?? -1) == 0 else { continue }
  let b = win[kCGWindowBounds as String] as? [String: Any] ?? [:]
  let x = Int(b["X"] as? Double ?? 0), y = Int(b["Y"] as? Double ?? 0)
  let w = Int(b["Width"] as? Double ?? 0), h = Int(b["Height"] as? Double ?? 0)
  let area = Double(w * h)
  let id = win[kCGWindowNumber as String] as? Int ?? -1
  if best == nil || area > best!.area { best = (id, x, y, w, h, area) }
}
guard let b = best else {
  FileHandle.standardError.write(
    "no on-screen layer-0 window owned by '\(needle)' — is the app running?\n".data(using: .utf8)!)
  exit(2)
}
print("\(b.id) \(b.x) \(b.y) \(b.w) \(b.h)")
SWIFT
}

# Raise the app so synthetic input reaches it. Uses the modern activation API
# (NSRunningApplication.activate) — the right primitive when the app CAN be
# activated. Best-effort: on macOS 14+ this cannot steal focus from another
# active app, and a background-launched GPUI app may not activate at all (see the
# header note + docs/dev/driving-the-app.md). Prints "isActive=<bool>" so a caller
# can tell whether input will actually land.
raise_app() {
  local pid
  pid="$(pgrep -x "$OWNER" | head -1)"
  [ -n "$pid" ] || { echo "isActive=false (no '$OWNER' process)"; return 0; }
  swift - "$pid" <<'SWIFT' 2>/dev/null || true
import AppKit
let pid = Int32(CommandLine.arguments.dropFirst().first ?? "0") ?? 0
guard let app = NSRunningApplication(processIdentifier: pid) else { print("isActive=false"); exit(0) }
app.activate()
usleep(400_000)
print("isActive=\(app.isActive)")
SWIFT
}

# ── commands ──────────────────────────────────────────────────────────────────
cmd_win() { find_window; }

cmd_raise() { raise_app; echo "raised $OWNER"; }

cmd_shot() {
  local name="${1:-shot}"
  mkdir -p "$SHOT_DIR"
  read -r id _ _ _ _ < <(find_window)
  local out="$SHOT_DIR/${name}.png"
  screencapture -x -o -l"$id" "$out"
  echo "$out"
}

# Click at window-relative fractions (0..1 of width/height).
cmd_click() {
  [ $# -eq 2 ] || die "click needs <fx> <fy> (fractions 0..1)"
  raise_app
  read -r _ x y w h < <(find_window)
  local sx sy
  sx=$(awk -v x="$x" -v w="$w" -v f="$1" 'BEGIN{printf "%d", x + f*w}')
  sy=$(awk -v y="$y" -v h="$h" -v f="$2" 'BEGIN{printf "%d", y + f*h}')
  cliclick "c:${sx},${sy}"
  echo "clicked frac($1,$2) -> screen($sx,$sy)"
}

# Click at window-relative points (from the top-left corner of the window).
cmd_clickpt() {
  [ $# -eq 2 ] || die "clickpt needs <px> <py> (points from window top-left)"
  raise_app
  read -r _ x y _ _ < <(find_window)
  cliclick "c:$((x + $1)),$((y + $2))"
  echo "clicked pt($1,$2) -> screen($((x + $1)),$((y + $2)))"
}

cmd_type() {
  [ $# -ge 1 ] || die "type needs \"<text>\""
  raise_app
  osascript -e "tell application \"System Events\" to keystroke \"$1\""
  echo "typed: $1"
}

# Registered-action shortcut, e.g. `key , cmd` or `key d cmd shift`.
cmd_key() {
  [ $# -ge 1 ] || die "key needs <char> [mods...]"
  local char="$1"; shift
  raise_app
  if [ $# -eq 0 ]; then
    osascript -e "tell application \"System Events\" to keystroke \"$char\""
  else
    local clause="" m
    for m in "$@"; do
      case "$m" in
        cmd)  clause+="command down, " ;;
        shift) clause+="shift down, " ;;
        opt)  clause+="option down, " ;;
        ctrl) clause+="control down, " ;;
        *) die "unknown modifier '$m' (use cmd|shift|opt|ctrl)" ;;
      esac
    done
    clause="${clause%, }"
    osascript -e "tell application \"System Events\" to keystroke \"$char\" using {$clause}"
  fi
  echo "key: $char ${*:-}"
}

# Unlock the demo/QA vault: the passphrase field auto-focuses, so type it and then
# CLICK the Unlock button — a synthetic Return does NOT submit (GPUI InputState eats
# it). Argon2 + the daemon round-trip take ~1s (PRODUCTION KDF) before the app lands.
cmd_unlock() {
  local pass="${1:-deckard-demo}"
  mkdir -p "$SHOT_DIR"
  local active; active="$(raise_app)"; echo "$active"
  case "$active" in
    *isActive=false*)
      echo "deckard-drive: WARNING — Deckard is not the active app, so the passphrase and" >&2
      echo "  Unlock click will NOT land (you'll get an empty field). Give the window focus" >&2
      echo "  first — run it from a foreground Terminal, or physically click the app once —" >&2
      echo "  then re-run. See docs/dev/driving-the-app.md → Honest caveats." >&2 ;;
  esac
  osascript -e "tell application \"System Events\" to keystroke \"$pass\"" >/dev/null
  local fx="${UNLOCK_FRAC%,*}" fy="${UNLOCK_FRAC#*,}"
  read -r _ x y w h < <(find_window)
  local sx sy
  sx=$(awk -v x="$x" -v w="$w" -v f="$fx" 'BEGIN{printf "%d", x + f*w}')
  sy=$(awk -v y="$y" -v h="$h" -v f="$fy" 'BEGIN{printf "%d", y + f*h}')
  cliclick "c:${sx},${sy}"
  # Give the KDF + daemon unlock a moment before capturing the unlocked screen.
  osascript -e 'delay 2' >/dev/null 2>&1 || true
  local out="$SHOT_DIR/unlocked.png"
  read -r id _ _ _ _ < <(find_window)
  screencapture -x -o -l"$id" "$out"
  echo "typed passphrase, clicked Unlock at frac($fx,$fy) -> screen($sx,$sy); shot: $out"
}

cmd_deps() {
  local ok=1
  for bin in swift cliclick screencapture osascript; do
    if command -v "$bin" >/dev/null 2>&1; then
      echo "ok   $bin -> $(command -v "$bin")"
    else
      echo "MISS $bin"; ok=0
    fi
  done
  [ $ok -eq 1 ] || die "install missing tools (cliclick: brew install cliclick; swift ships with Xcode CLT)"
}

main() {
  [ $# -ge 1 ] || { grep -E '^#( |$)' "$0" | sed -E 's/^# ?//'; exit 0; }
  local cmd="$1"; shift
  case "$cmd" in
    win|find)   cmd_win "$@" ;;
    raise)      cmd_raise "$@" ;;
    shot)       cmd_shot "$@" ;;
    click)      cmd_click "$@" ;;
    clickpt)    cmd_clickpt "$@" ;;
    type)       cmd_type "$@" ;;
    key)        cmd_key "$@" ;;
    unlock)     cmd_unlock "$@" ;;
    deps)       cmd_deps "$@" ;;
    -h|--help|help) grep -E '^#( |$)' "$0" | sed -E 's/^# ?//' ;;
    *) die "unknown command '$cmd' (try: help)" ;;
  esac
}

main "$@"
