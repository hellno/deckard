# Headless GUI screenshots (Linux / Claude-on-the-web)

Deckard is a GPUI app dogfooded on macOS (`just run`). When you touch a `crates/deckard-app`
view from a **headless Linux session** (no display, no GPU) you still owe the PR a screenshot
(`CLAUDE.md` → Visual verification). This is the reproducible recipe — it brings the app up
under a virtual display with a software GPU, drives it, and captures real pixels.

It is fiddly and environment-specific. Budget ~15 min the first time. Everything below is a
**local, throwaway** setup — none of it is committed.

## 1. System packages (apt)

```bash
# Software Vulkan (Mesa llvmpipe/lavapipe) — GPUI's Linux backend (Blade) needs a Vulkan device.
sudo apt-get install -y mesa-vulkan-drivers libvulkan1 vulkan-tools
# Virtual X display + capture + input injection.
sudo apt-get install -y xvfb x11-apps imagemagick xdotool
# X11/xcb dev libs — needed to LINK the x11 windowing backend (see step 2).
sudo apt-get install -y libxcb1-dev libx11-dev libx11-xcb-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libxcb-randr0-dev libxcb-xfixes0-dev libxcb-shape0-dev \
  libxcb-render0-dev libxcb-render-util0-dev libxcb-cursor-dev libxcb-xkb-dev \
  libxcb-icccm4-dev libxcb-keysyms1-dev libxcb-shm0-dev libxcb-image0-dev libxcb-util-dev
```

Confirm the software device exists:

```bash
export DISPLAY=:99 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
Xvfb :99 -screen 0 1280x900x24 &
vulkaninfo --summary | grep deviceName   # -> llvmpipe (LLVM ...)
```

## 2. Enable the X11 windowing backend (TEMPORARY — do not commit)

The app's `gpui_platform` dependency enables only `font-kit`, so `gpui_linux` ships with **no
windowing backend** on Linux. Meanwhile `gpui`'s own `x11` feature is on, so
`guess_compositor()` picks `"X11"` from `$DISPLAY` — but the matching backend arm in
`gpui_linux::current_platform` is compiled out, and the app panics with
`internal error: entered unreachable code` at startup. Fix it for the local build only:

```toml
# crates/deckard-app/Cargo.toml — add "x11" to the gpui_platform features
gpui_platform = { git = "...", features = ["font-kit", "x11"] }
```

```bash
# GOTCHA: cargo reuses a cached gpui_linux built WITHOUT x11, so the feature change alone is a
# no-op. Force the recompile:
cargo clean -p gpui_linux
cargo build -p deckard-app -p deckard-signerd
```

**Revert `crates/deckard-app/Cargo.toml` and `Cargo.lock` before committing** — the default
build targets the macOS/dogfood path and must stay unchanged.

## 3. Get past onboarding: seed a vault

The Send (and most) screens live behind unlock. Instead of driving the multi-step create flow
blind, seal a vault so the app lands on the Unlock screen. Throwaway example
(`crates/deckard-core/examples/seed_vault.rs`, delete after):

```rust
use deckard_core::{KdfParams, Vault};
fn main() {
    let dir = std::env::var("DECKARD_CONFIG_DIR").expect("set DECKARD_CONFIG_DIR");
    std::fs::create_dir_all(&dir).unwrap();
    let mnemonic = "test test test test test test test test test test test junk"; // anvil acct 0
    let vault = Vault::import_mnemonic(mnemonic, "smoke-pass", KdfParams::PRODUCTION).unwrap();
    vault.write_atomic(std::path::Path::new(&dir).join("vault.bin").as_path()).unwrap();
}
```

```bash
export DECKARD_CONFIG_DIR=/tmp/deckard-smoke
cargo run -p deckard-core --example seed_vault   # -> sealed .../vault.bin addr=0xf39f...2266
```

## 4. Launch

The GUI/Vulkan/X11 syscalls must run **unsandboxed** — in Claude Code the Bash sandbox kills
the process silently. Use `dangerouslyDisableSandbox: true` for the launch + drive commands.

```bash
export DISPLAY=:99 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export DECKARD_CONFIG_DIR=/tmp/deckard-smoke DECKARD_SOCKET_PATH=/tmp/deckard-smoke/signerd.sock
export DECKARD_CHAIN_ID=31337 DECKARD_RPC_URL=http://127.0.0.1:8545   # dummy local; reads just fail
export DECKARD_SIGNERD_BIN="$PWD/target/debug/deckard-signerd"
nohup ./target/debug/deckard >/tmp/app.log 2>&1 &
```

## 5. Drive + capture

```bash
WIN=$(xdotool search --name "Deckard" | head -1)   # the 880x620 app window
xdotool windowmap "$WIN"; xdotool windowactivate "$WIN"
# type into a focused field: click it first, then type
xdotool mousemove --window "$WIN" 440 316 click 1; xdotool type --window "$WIN" "smoke-pass"
xdotool key --window "$WIN" Return                 # unlock (PRODUCTION Argon2 ~1s)
# ...navigate (click buttons by approx coords), then:
import -window "$WIN" /tmp/shot.png                # capture real pixels
```

Gotchas learned the hard way:

- **Never `pkill -f 'target/debug/deckard'`** — the pattern matches *your own shell command's*
  cmdline and kills the session (shows up as a mysterious `exit 144`). Kill by explicit PID, or
  by exact short name with `pkill -x deckard` (note: process names truncate at 15 chars, so
  `deckard-signerd` won't match `-x` — kill it by PID).
- `propose` works **offline** (policy-only, no RPC), so compose → review renders against a dummy
  RPC. Stop before **Hold to send** unless you have a funded local chain — it broadcasts.
- `libEGL warning: DRI3 error` lines are harmless (software path).
- Capture the **window** (`import -window $WIN`), not the root — the root isn't composited and
  reads back blank.

## 6. Clean up

Kill the app/daemon/Xvfb (by PID), `rm -rf $DECKARD_CONFIG_DIR`, delete the throwaway example,
and `git checkout -- crates/deckard-app/Cargo.toml Cargo.lock`. Confirm `git status` is clean
before you commit your actual change.
