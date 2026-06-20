# Bundled fonts (offline-first — no web-font CDN)

DESIGN.md mandates two bundled families. They are **committed here and embedded at
build time** via `cx.text_system().add_fonts(...)` in `crates/deckard-app/src/main.rs`.
Both are licensed under the **SIL Open Font License 1.1**, which explicitly permits
redistributing the raw font files in a public repo, embedding them in a binary, and
commercial use — so checking them in is safe.

| File | Family / weight | Source | License |
|------|-----------------|--------|---------|
| `SchibstedGrotesk-Regular.otf`  | Schibsted Grotesk 400 | https://github.com/schibsted/schibsted-grotesk | OFL 1.1 |
| `SchibstedGrotesk-Medium.otf`   | Schibsted Grotesk 500 | same | OFL 1.1 |
| `SchibstedGrotesk-SemiBold.otf` | Schibsted Grotesk 600 | same | OFL 1.1 |
| `JetBrainsMono-Regular.ttf`     | JetBrains Mono 400 | https://github.com/JetBrains/JetBrainsMono | OFL 1.1 |
| `JetBrainsMono-Medium.ttf`      | JetBrains Mono 500 | same | OFL 1.1 |

The full license text ships beside each family (`SchibstedGrotesk-OFL.txt`,
`JetBrainsMono-OFL.txt`) as OFL 1.1 requires.

Notes:
- DESIGN caps weight at **600** — do not bundle Bold (700+).
- Schibsted Grotesk replaced General Sans (2026-06-20): General Sans ships under the
  Fontshare/Indian Type Foundry proprietary EULA, which forbids redistributing the raw
  font files / hosting them on a public server. Since this repo is public, that was a
  licensing violation. Schibsted Grotesk is OFL 1.1 and a structural drop-in (same
  Regular/Medium/SemiBold weights at usWeightClass 400/500/600).
- The family-name strings in `theme.rs` (`"Schibsted Grotesk"` / `"JetBrains Mono"`)
  must match the font files' internal name table (typographic family, name ID 16), or
  GPUI silently falls back to the system font. Verify by launching the app.
