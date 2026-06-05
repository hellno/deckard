# Design System — Deckard

> Source of truth for every visual and UI decision. Read this before building any view.
> Created by /design-consultation, 2026-06-05. Place at the repo root when forking `deck`.

## The memorable thing
**A precision instrument for your sovereign money, not a casino.** Linear's calm precision
plus a terminal's honesty, with a thin Blade Runner edge (deep near-black, one warm signal
light). Every decision below serves that feeling.

## Product Context
- **What:** Deckard, a native desktop (macOS + Linux) self-custodial Ethereum wallet.
- **Who:** onchain operators (paid in crypto, live onchain, refuse custodians).
- **Space:** crypto wallets / dev-grade desktop tools. Peers in feel: Linear, Superhuman, Raycast,
  a terminal. NOT consumer-crypto casino UIs.
- **Type:** APP UI (dense, calm, keyboard-first), not a marketing site.
- **Build constraint:** Rust + GPUI 0.2 + gpui-component 0.5. `theme.rs` already does accent +
  light/dark; fonts bundle with the app. Everything here is expressible in that theme.

## Aesthetic Direction
- **Direction:** Industrial-minimal with noir-tech restraint.
- **Decoration:** minimal. Typography, hairlines, and one accent do the work. No decorative cards,
  no gradients, no blobs.
- **Mood:** calm, precise, serious, trustworthy. Dark-first (operators live in dark).
- **Dark-first:** design dark as primary, light as a first-class equal, not an afterthought.

## Color
Warm amber signal accent on cool near-black neutrals. The warm-on-cold duality is the Blade
Runner move; it reads as money/gold without going gaudy and avoids the crypto defaults (blue,
degen-green, AI-slop purple). The accent is rare and meaningful: primary actions, focus, selection,
and caution. Never wallpaper.

**Dark (primary):**
- `bg.base` `#0A0B0D` · `bg.surface` `#14161A` · `bg.elevated` `#1B1E24` (palette, modals)
- `border.hairline` `#23272E` · `border.strong` `#2E333C`
- `text.primary` `#E6E8EB` · `text.secondary` `#A7AEB7` · `text.muted` `#878E97`
- `accent` `#F2A43B` · `accent.hover` `#FFB454` · `accent.tint` `rgba(242,164,59,0.12)`

**Light:**
- `bg.base` `#F7F7F4` (a hint warm, to pair with amber) · `bg.surface` `#FFFFFF`
- `border.hairline` `#E6E5E0`
- `text.primary` `#16181D` · `text.secondary` `#4A4F57` · `text.muted` `#6B7280`
- `accent` `#B9700F` (deepened for AA contrast on light) · `accent.tint` `rgba(185,112,15,0.10)`

**Semantic (kept distinct from the accent so warnings never blur with brand):**
- `success` `#3FB950` · `error` `#E5484D` · `info` `#4493F8`
- Caution states (unlimited approval, unknown contract, fresh address) use `error` when dangerous
  and the amber `accent` + a warning icon when "look closely." Danger is loud and early, never buried.
- Contrast: all body/UI text must hold >= 4.5:1 against its background in both modes.

## Typography
A precise grotesk for UI, plus a real monospace for every number, address, and hash. Mono-for-money
is both a craft signal and a safety affordance: you can read a hex address character by character.
- **UI / display:** **General Sans** (Fontshare, free, bundleable). Precise, clean, not Inter.
- **Data / numbers / addresses / hashes:** **JetBrains Mono** (OFL, free, bundleable; legible 0/O,
  1/l; tabular figures). Premium upgrade option later: Berkeley Mono.
- **Loading:** bundle both font files with the app and register via GPUI assets. Both licenses are
  AGPL-compatible. No web font CDN (offline-first, sovereign).
- **Scale (px):** display 32 · screen-title 24 · section 18 · body 14 (deliberate desktop density,
  Linear-class) · primary-reading 15 · caption/label 12. Balance hero (mono) 28-32 tabular.
- **Weights:** General Sans 400/500/600. Avoid going heavier than 600 (calm, not shouty).

## Spacing
- **Base:** 4px. **Density:** compact-comfortable (operator daily driver).
- **Scale:** 2 · 4 · 8 · 12 · 16 · 24 · 32 · 48 · 64.

## Layout
- **Approach:** grid-disciplined, calm surface hierarchy. Primary workspace + a thin nav/rail +
  secondary context. One accent. No dashboard-card mosaics.
- **Border radius:** restrained, not bubbly. 4px inputs/buttons · 6px rows/cards · 10px modals +
  command palette · full for pills/identicons.
- **Chrome:** hairline borders over fills; lean on `bg.surface` vs `bg.base` for grouping, not boxes.

## Motion
- **Approach:** minimal-functional. Motion clarifies state, never decorates (local-first, zero-spinner).
- **Duration:** micro 120ms · short 160ms · medium 220ms. **Easing:** enter ease-out, exit ease-in.
- **Command palette:** opens 160ms, scale 0.98 to 1 + fade. No bounce. The one allowed loading
  moment app-wide is the Helios first-sync; everywhere else renders instantly from cache.

## Command Palette + Keyboard Model (the signature)
- **cmd-K** opens the palette: fuzzy-filtered (nucleo) over every action (go to Portfolio/Send/
  Receive/Swap/Settings, send, swap, switch account, copy address, lock, toggle theme). Recent and
  context-relevant actions rank first. Rows: icon + label + right-aligned shortcut hint.
- **Keys:** arrow/enter/esc semantics; visible focus ring in `accent`; every primary flow completable
  with no mouse. Reserve `secondary-k` (Cmd/Ctrl-K) for palette, `secondary-,` settings, plus
  account/route quick-switches. `secondary` = Cmd on macOS, Ctrl on Linux (matches the deck starter).
- **Speed:** palette opens < 50ms; results filter per keystroke. Speed IS the brand.

## Trust / Clear-Signing Affordances (this holds funds)
- **Addresses:** always mono, middle-truncated with full-on-expand, paired with an identicon
  (jazzicon/blockies) + ENS name when resolvable. The raw hex is always one action away.
- **Signing screens:** a calm plain-language panel: "You are sending X to Y", "You are approving
  UNLIMITED USDC to <unknown contract>". Dangerous parts (unlimited approval, unknown/unverified
  contract, never-seen address) in `error`, surfaced not buried. Amounts + addresses in mono.
  Deliberate confirm, not a tiny button.
- **Seed reveal:** blurred by default, hold-to-reveal with auto-hide timeout, never silently copied
  to clipboard, with a "make sure nobody is watching" full-screen affordance.

## SAFE vs RISK (for future you)
- **SAFE:** dark-first near-black + cool neutrals, one accent, grotesk + mono pairing, minimal motion.
- **RISK (Deckard's face):** amber-gold accent (not crypto blue/green/purple); monospace for ALL
  money + addresses; noir-tech restraint that reads as "instrument."

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-06-05 | Initial design system created | /design-consultation. Amber-on-near-black, General Sans + JetBrains Mono, mono-for-money, clear-signing affordances. Serves "precision instrument, not a casino." |
