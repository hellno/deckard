# Bundled fonts (offline-first — no web-font CDN)

DESIGN.md mandates two bundled families. They are **not committed** here because
they are licensed binaries a human must place; the build stays green without them
(the theme sets the family names and GPUI silently falls back to the system font
until the files exist).

Drop these files into this directory, then **uncomment the `add_fonts(...)` block
in `crates/deckard-app/src/main.rs`** (search `TODO(fonts)`):

| File | Family / weight | Source | License |
|------|-----------------|--------|---------|
| `GeneralSans-Regular.otf`  | General Sans 400 | https://www.fontshare.com/fonts/general-sans | Fontshare (free) |
| `GeneralSans-Medium.otf`   | General Sans 500 | same | same |
| `GeneralSans-Semibold.otf` | General Sans 600 | same | same |
| `JetBrainsMono-Regular.ttf`| JetBrains Mono 400 | https://www.jetbrains.com/lega/font / GitHub `JetBrains/JetBrainsMono` | OFL 1.1 |
| `JetBrainsMono-Medium.ttf` | JetBrains Mono 500 | same | OFL 1.1 |

Notes:
- DESIGN caps weight at **600** — do not bundle Bold (700+).
- The family-name strings in `theme.rs` (`"General Sans"` / `"JetBrains Mono"`)
  must match the font files' internal name table, or GPUI silently falls back.
  Verify by launching the app and confirming money renders in mono after dropping
  the files in.
