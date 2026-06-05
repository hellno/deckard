# Deck — task runner. Install `just`: brew install just
# (Everything here is plain cargo + macOS built-ins; you can run the commands by hand too.)

# List available recipes.
default:
    @just --list

# Run the app (debug). This is the one you'll use 99% of the time.
run:
    cargo run

# Run optimized.
run-release:
    cargo run --release

# Run as a menu-bar / tray app (no dock icon).
run-tray:
    cargo run --features tray

# Format + lint (both feature configurations).
fmt:
    cargo fmt
check:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --features tray -- -D warnings

# Bump the git GPUI stack to the latest upstream commits, then rebuild.
# Reproducibility lives in Cargo.lock — commit it (and rust-toolchain.toml if you
# bumped it) after this succeeds. If the build fails on an unstable-feature error,
# match rust-toolchain.toml to Zed's: https://github.com/zed-industries/zed/blob/main/rust-toolchain.toml
# Full procedure + the crates.io fallback channel: docs/UPGRADING.md
bump-gpui:
    cargo update -p gpui -p gpui_platform -p gpui-component -p gpui-component-assets
    cargo build
    @echo "→ Bumped. Run the app to smoke-test, then commit Cargo.lock (+ rust-toolchain.toml if changed)."

# Build a distributable Deckard.app (needs: cargo install cargo-bundle).
# Output: target/release/bundle/osx/Deckard.app
bundle:
    cargo bundle --release
    @echo "→ target/release/bundle/osx/Deckard.app"

# Open the bundled app.
open: bundle
    open "target/release/bundle/osx/Deckard.app"

# Regenerate assets/icon.png + assets/icon.icns from assets/icon.svg.
# Needs cairosvg (pip install cairosvg); falls back to qlmanage if missing.
# Uses only macOS built-ins (sips, iconutil) for the .icns step.
icon:
    #!/usr/bin/env bash
    set -euo pipefail
    cd assets
    if command -v cairosvg >/dev/null; then
        cairosvg icon.svg -o icon.png -W 1024 -H 1024
    else
        qlmanage -t -s 1024 -o . icon.svg >/dev/null && mv icon.svg.png icon.png
    fi
    rm -rf icon.iconset && mkdir icon.iconset
    for sz in 16 32 64 128 256 512; do
        sips -z $sz $sz       icon.png --out icon.iconset/icon_${sz}x${sz}.png   >/dev/null
        sips -z $((sz*2)) $((sz*2)) icon.png --out icon.iconset/icon_${sz}x${sz}@2x.png >/dev/null
    done
    sips -z 1024 1024 icon.png --out icon.iconset/icon_512x512@2x.png >/dev/null
    iconutil -c icns icon.iconset -o icon.icns
    rm -rf icon.iconset
    echo "→ assets/icon.png + assets/icon.icns regenerated"
