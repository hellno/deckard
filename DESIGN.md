# Design System — Deckard

> Source of truth for every visual and UI decision. Read this before building any view.
> v2 (2026-06-20): re-grounded the visual language (editorial, not card-heavy), made the
> foundation **enforceable** (bundled fonts + a shared widget vocabulary + a visual definition
> of done), redesigned the trust-critical confirm, and defined the agent interaction model.
> v3 (2026-07-01): closed the doc-vs-code enforcement gap. Named every scale as a **token**
> (spacing, type, radii, motion, elevation, opacity, stroke, icon) so the code can carry a const
> instead of a magic number; gave the semantic roles their own names (the one warm light still
> serves them all, but on purpose now); promoted the light theme to a full token table; and
> versioned the golden references in-repo. The token names below are the contract the crate's
> `theme.rs` / `tokens` module implement — a view that hardcodes a value the token layer already
> names is a review failure.
>
> **Golden references (the pixel ground-truth agents build against), versioned in-repo under `designs/`:**
> - `designs/deckard-editorial-v3.html` — home, send confirm, swap compose, swap review, activity
>   (the editorial language, light + dark).
> - `designs/deckard-agent-v4.html` — the agent surface, the compact agent presence on the home, and
>   the redesigned transaction-as-hero confirm (this confirm supersedes the v3 send confirm).
> When this doc and a reference disagree, the doc wins on rules and the reference wins on pixels;
> fix whichever is stale. (These were unversioned under `~/.gstack/...` through v2; v3 checked them
> into the repo so the "matches golden reference" definition-of-done item is actually verifiable.)

## The memorable thing
**Your money on autopilot, and you can see and stop everything.** Calm sovereign control plus total
auditability. Linear's calm precision and Conductor's parallel-agent cockpit, with a terminal's
honesty and a thin Blade Runner edge (deep near-black, warm light for the human, cold light for the
machine). Every decision below serves that feeling. Not a casino.

## Product context
- **What:** Deckard, a native desktop (macOS + Linux) self-custodial Ethereum wallet, built in
  Rust + GPUI 0.2 + gpui-component 0.5. Offline-first, privacy-focused, open source (AGPL-3.0).
- **Model:** **human-sovereign, agent-delegated.** The human owns the keys and sets policy; AI
  agents act on the human's behalf within budget/scope/policy bounds. The agent is a **key-less
  proposer**: it never holds the key, the daemon does, and every move passes a policy gate the human
  controls (see §The agent interaction model).
- **Who:** onchain operators who live in crypto and refuse custodians; later, anyone delegating
  money-movement to agents.
- **Peers in feel:** Linear, Superhuman, Stripe, Splits. Dense, calm, keyboard-fast, premium.
  NOT consumer-crypto casino UIs.

## How this system is grounded (read this)
The first drafts read as generic AI slop because they were built from **prose descriptions** of
Linear/Conductor. The fix is to re-derive from **real product pixels**. The v2 visual language was
chosen by generating three elevation directions (Instrument / Editorial / Vault) and picking
**Editorial** against real Linear/Superhuman/Stripe/Splits reference. **Rule for all future design
work: ground every decision in real reference screenshots and the golden-reference HTML, never in
remembered descriptions.**

---

## Enforcement model (the heart of v2)
The old DESIGN.md was good prose, and the app still drifted into slop. Root cause, found by a
source audit: the foundation (theme tokens, money rendering, the compose→review engine, the palette)
is disciplined, but the **leaf widgets were hand-rolled per file** (3-4 divergent copies of every
helper), and the **brand fonts were never bundled** so the app ran in the OS system font. Prose
cannot fix that. Three things make this foundation enforceable:

### 1. Fonts are bundled, not aspirational
Schibsted Grotesk (UI) and JetBrains Mono (money/addresses) are both OFL-1.1 (free to redistribute,
even in a public repo) and **must be embedded** via `cx.text_system().add_fonts(...)` in `main.rs` with the files in
`crates/deckard-app/assets/fonts/`. Until they are, every typography rule below is fiction (the app
falls back to the system font). This is the single highest-leverage visual fix.

### 2. One shared widget vocabulary (`crates/deckard-app/src/widgets.rs`)
Every view composes from these primitives. Re-rolling any of them inline is a review failure.
A primitive bakes in the correct DESIGN values so a screen **cannot** drift.

| primitive | what it is | the rule it enforces |
|---|---|---|
| `identity_mark(seed, size)` | project/wallet square or human round, with a **deterministic monogram/identicon** | never a blank fill; round = human principal, square = project/wallet, cyan squircle = agent |
| `agent_squircle(...)` | the cyan agent glyph (exists, keep) | cyan = agent class only; static, pulse only when acting |
| `section_label(text)` | 10-11px uppercase, +0.07em tracking, `text.muted` | one label treatment everywhere (kills the 3 divergent copies) |
| `short_addr(s)` | middle-truncate **first-6 + last-4** | one truncation rule (kills `short_mid`/`short_tx` variants) |
| `truncated_address(addr)` | identicon + `short_addr` + ENS, self-send warning | every address surface is distinguishable + identifiable |
| `caution_line(severity, text)` | Lucide `TriangleAlert` icon + inline text, **no box** | one caution language (kills the `⚠` emoji); amber = caution, red = danger |
| `kv_row(label, value)` | label-left muted / mono-value-right, `min_w_0`+truncate | one key/value row (clear-signing, policy, holdings) |
| `money(...)` / `usd(...)` | mono dimmed-decimals (exists in `money.rs`, keep) | integer `primary`, decimals+ticker `muted` by color only |
| `amount_field(...)` | mono input + asset chip + USD-equiv + Max + over-balance gate | the compose input always knows the balance |
| `budget_gauge(spent, cap)` | thin 4px track, neutral→amber≥90%→red≥100% | over-cap can never render calm |
| `confirm_button(label, keys)` | the `⌘↵` key-cap confirm with arm-delay (see §Confirm) | no hold-to-confirm; spam-proof |
| `key_hint_chip(keys)` | a bordered key-cap | shortcuts live in chips/menus, never as on-canvas filler rows |
| `status_glyph(state)` | circular check/x-ring/clock-ring/minus | one status vocabulary across feed + chips |
| `nav_row(...)` | sidebar/footer row: rest/hover/selected lift, one resting color | one row anatomy, one resting treatment |
| `page_header(mark, title, subtitle)` *(v3)* | mark/glyph + H1 at `text-h1` + muted one-line subtitle | one page-header anatomy at **one** title size (kills the 6 hand-rolled copies at 3 sizes) |
| `divider()` *(v3)* | a 1px `border.hairline` full-width rule | one hairline (kills the 6 inline `div().h(px(1.))` copies) |
| `skeleton_row(shape)` *(v3)* | a money-/row-shaped loading placeholder | one loading vocabulary (never a spinner, never a bare `—`) |
| `stop_brake(state)` *(v3)* | the STOP control: amber-idle → danger-armed, `⌘↵` to fire | one kill-switch treatment across Activity + the agent surface |

### 3. A visual definition of done (see the checklist at the end)
QA and `/code-review` enforce it. A GUI PR that fails any item is not done.

---

## Visual language (editorial)
**Hierarchy comes from type, weight, whitespace, and hairlines. Not from cards.** This is the
Linear/Superhuman/Stripe discipline and the v2 decision: defaulting to elevated/floating cards is the
AI-coded-design tell, and we reject it.

- **Surfaces/cards are rare and purposeful.** Default to whitespace + a tiny section label for
  grouping; a single hairline only where a list needs row separation. At most **one faint frame**
  for a genuine contained object, and never an elevation/shadow stack of cards.
- **The hero is the largest object on screen.** The balance (and, on a confirm, the transaction
  amount) is the biggest thing: `text-hero` (64) mono on the wallet home, `text-tx-hero` (44) on a
  confirm. Money is the load-bearing object; make it unmistakable.
- **Cockpit layout.** Main surfaces are **left-anchored, full-bleed, hairline-ruled columns** with a
  **right-edge metadata rail** (Linear's row anatomy: lead glyph · subject · object on the left,
  signed-amount · status-glyph · time pinned right). Every row clamps: left cluster `min_w_0` +
  truncate, right rail `flex-none`, so content can never run off either edge (this is also the fix
  for the historical horizontal-overflow bug).
- **Density is compact-comfortable**, calm not crowded; Settings is more spacious.

---

## Information architecture
**Two-pane shell** (sidebar + main + a thin top breadcrumb bar + a bottom status strip). No third
inspector pane; detail is contextual or a right slide-over when genuinely needed.

**Sidebar = a single-column Conductor-style tree.** Top-level entities are **Projects**. Each
project expands to two first-class groups:
- **Wallets** — Splits-style rows: identity mark + name + truncated mono address + right-aligned
  balance.
- **Agents** — **first-class, standalone** (this is a v2 change; agents were previously buried in
  the wallet home). Row: cyan squircle + name + right-aligned spend magnitude + status glyph.

**Views are contextual to selection:**
- Select a **wallet** → its home (hero balance, allocation, actions, holdings, and a **compact
  agent presence** row, not the full policy).
- Select a **project** → aggregate across its wallets + its wallets and agents.
- Select an **agent** → the **agent surface** (§The agent interaction model).
- **Activity** is a destination (the see-and-stop log). **Settings** is a bottom gear (global).
  **Send / Receive / Swap / Shield** are actions on the selected wallet. Clear-signing is the
  confirm step inside them, and the same review renders for an agent's proposal awaiting approval.

---

## The actor model (human vs agent)
A machine spends the human's money, so every attributable row must answer "who did this, me or which
agent?" in under a second. A **two-signal axis plus shape**, never a rainbow:

- **amber = the human / "where you are" / caution.** **cyan = the agent class.** The Blade-Runner
  warm/cold duality made load-bearing. Disciplined 2-color system, not categorical coloring.
- **Shape is the accessibility backup:** human principal = **full-round** identicon; agent = **cyan
  squircle** monogram; project/wallet = rounded **square** mark. Survives grayscale.
- **Identity, not state:** cyan marks agent identity (squircle + status); it is **never** a page
  title, body, or link color. Page titles are always `text.primary`.
- **Accountability chain is rendered:** an agent-proposed + human-approved action shows both actors
  ("Atlas proposed → You approved").

---

## Color
~95% grayscale. Two signal colors spent sparingly; semantics kept distinct from both.

### Dark (primary)
| token | hex | use |
|---|---|---|
| `bg.base` | `#0A0B0D` | app canvas |
| `bg.rail` | `#0B0C0F` | sidebar / status strip / top bar |
| `bg.raise` | `#121419` | active/selected lift, the rare surface, inputs |
| `bg.raise2` | `#161922` | primary-button fill, popovers, palette |
| `bg.hover` | `#14161B` | hover lift |
| `border.hairline` | `#1B1E25` | dividers (~8% step from base) |
| `border.strong` | `#262A33` | input outlines, key-caps, stronger separation |
| `text.primary` | `#E7E9EC` | headings, values (never pure white) |
| `text.secondary` | `#9298A2` | labels, body |
| `text.muted` | `#646A73` | metadata, addresses, dimmed decimals |
| `accent` (amber) | `#F2A43B` | hover `#FFB454`, tint `rgba(242,164,59,.14)` |
| `agent` (cyan) | `#3CC9BC` | tint `rgba(60,201,188,.12)` |
| `success` | `#4FB463` · `error` | `#E5565B` (tint `rgba(229,86,91,.12)`) |
| `identity` slate | `#3A4250` | desaturated identity-mark fill (off amber + success) |
| `shield` slate | `#33424C` | private/shielded tone (off the actor axis) |
| `bg.overlay` | `rgba(0,0,0,.5)` | the one dimming scrim under a modal/palette/slide-over; no shadow stacks |
| `border.focus` | = `accent` | the 1px focus ring — the sole amber-colored border on the app |

### Light (first-class equal, refined; must not wash out)
Same token names, same roles — only the values change. Light has **one fewer surface level**:
`raise` and `raise2` collapse to `#FFFFFF` (there is no second lift on a white surface), so a
primary button reads as a hairline-framed white, not a darker fill.

| token | hex | use |
|---|---|---|
| `bg.base` | `#F6F5F1` | app canvas |
| `bg.rail` | `#EEEDE6` | sidebar / status strip / top bar |
| `bg.raise` | `#FFFFFF` | active/selected lift, the rare surface, inputs |
| `bg.raise2` | `#FFFFFF` | primary-button fill, popovers, palette (collapses to `raise` in light) |
| `bg.hover` | `#ECEBE4` | hover lift |
| `border.hairline` | `#DDDBD2` | dividers |
| `border.strong` | `#CFCCC2` | input outlines, key-caps |
| `text.primary` | `#17191E` | headings, values |
| `text.secondary` | `#52585F` | labels, body |
| `text.muted` | `#6B7280` | metadata, addresses (≥4.5:1 on base) |
| `accent` (amber) | `#A8650C` | deepened for AA on light; tint `rgba(168,101,12,.14)` |
| `agent` (cyan) | `#0C7E75` | tint `rgba(12,126,117,.12)` |
| `success` | `#2F8F47` | · `error` `#C23B40` (tint `.12`) |
| `identity` slate | `#A7AEBA` | identity-mark fill |
| `shield` slate | `#94A2AC` | private/shielded tone |
| `bg.overlay` | `rgba(0,0,0,.45)` | scrim (a touch lighter than dark's `.5`) |
| `border.focus` | = `accent` | the amber focus ring |

### Color discipline
1. **Amber is the one warm light, <1% of pixels.** It means only: the human acting, "where you are"
   (active step, "awaiting you"), caution, the armed-confirm key-cap, the focus ring. **Not** a
   primary-button fill, **not** a routine toggle, **not** a chart segment.
2. **Primary buttons are neutral** (`bg.raise2` fill, `text.primary`, weight 600).
3. **Cyan is only the agent class**, low-chroma, on the squircle + agent status. Never title/link.
4. **Identity/token marks** are desaturated tinted-neutral, off the warm/amber band and off the
   `success` hue. Two different wallets must be visually distinguishable (deterministic mark).
5. **Allocation/category bars** use neutral/low-chroma tonal steps, never amber.
6. **Danger stays loud red, early.** Unlimited approvals, unknown contracts, fresh-address sends,
   over-cap, irreversible-loss surface in `error`.
7. **Caution = an amber `TriangleAlert` icon + the risk text, inline. No box, no left keyline.**
   (Always via `caution_line`.)
8. **Budget/utilization bars** = thin 4px, neutral track, neutral/cyan fill at rest; amber ≥90%,
   red ≥100%. Never a saturated slab.
9. **Verified vs unverified is a real signal**: a verified mainnet read reads `success`; the
   downgrade reads the loud `NOT VERIFIED` amber tag (§Per-chain trust tiers). Never fake verified.

### Semantic roles (what each color *means*, not just its hex)
The base palette above is the raw material; these are the **roles** a view actually reaches for. The
point of naming them: the single warm light (`accent`) does five jobs, and that overload is a
**deliberate** decision (one warm light, <1% of pixels) — not an accident to be "cleaned up" by
tinting each role differently. Naming the roles makes the overload visible and lets exactly one of
them peel off to its own token if it ever must, without touching the other four.

| role | token today | meaning |
|---|---|---|
| `signal.human` | `accent` (amber) | the human acting / "where you are" / the active step / "awaiting you" |
| `signal.agent` | `agent` (cyan) | the agent class — squircle + agent status only |
| `state.caution` | `accent` (amber) | a recoverable risk (first-time recipient, slippage) — via `caution_line` |
| `state.danger` | `error` (red) | irreversible / loss-bearing (public+permanent, unlimited approval, over-cap) |
| `state.success` | `success` (green) | verified read, confirmed tx |
| `focus.ring` | `border.focus` = `accent` | the 1px focus-visible ring |
| `armed.confirm` | `accent` (amber) | the `⌘↵` key-caps once armed |
| `overlay.scrim` | `bg.overlay` | the dimming layer under a modal/palette |

Rule: a view names the **role**, never re-derives the hex. `state.danger` may equal `error` today,
but a screen asks for "danger," so a future palette change moves one token, not fifty call sites.

---

## Typography
- **UI / display:** **Schibsted Grotesk** (bundled, OFL-1.1). Sentence case; no ALL-CAPS except tiny section
  labels. Hierarchy from **weight + size + color, in that order** (the old build leaned on color
  alone, which read flat).
- **Money / numbers / addresses / hashes:** **JetBrains Mono** (bundled), tabular figures, full
  precision, never abbreviated in a ledger.
- **Type scale (named tokens — one value each, no ranges):**

  | token | px | weight | use |
  |---|---|---|---|
  | `text-hero` | 64 | 500 | balance hero (wallet home) |
  | `text-tx-hero` | 44 | 500 | **transaction hero** — the oversized amount on a clear-signing confirm (Send / Shield / Approve). Was a 40px send hero |
  | `text-h1` | 20 | 600 | screen title |
  | `text-section` | 14 | 600 | section heading |
  | `text-body` | 13 | 400 | body / values |
  | `text-label` | 10 | 500 | tiny uppercase group label, `tracking-label` |

  The old spec gave ranges (hero 64-72, confirm 44, H1 20-22, label 10-11); v3 collapses each to one
  value so a screen carries a token, not a judgement call. The balance hero (`text-hero`, home) and
  the transaction hero (`text-tx-hero`, confirm) are the only two display sizes and are distinct on
  purpose — a confirm is not the home. In code: `text-hero` / `text-tx-hero` / `text-body` /
  `text-label` are `tokens::TEXT_*` consts (gpui has no utility at those sizes); `text-h1` and
  `text-section` are gpui's `.text_xl` (20) and `.text_sm` (14), a compact 12px size is `.text_xs`,
  and a swap **compose** amount (a step below the confirm hero) is `.text_3xl` (30).
- **Leading:** `leading-tight` 1.15 (hero + headings) · `leading-normal` 1.4 (body + labels) ·
  `leading-mono` 1.0 (money / addresses / hashes — tabular figures set tight so columns align).
- **Tracking:** `tracking-label` +0.07em is the **only** non-zero tracking (the tiny uppercase
  label). Everything else is 0 — Schibsted Grotesk is drawn for editorial text at its natural spacing.
- **Fallback stacks** (GPUI silently drops to the system font if a family isn't registered, so name
  the fallback explicitly): UI = `Schibsted Grotesk, system-ui, sans-serif`; mono =
  `JetBrains Mono, ui-monospace, monospace`.
- **Weights:** 400 / 500 / 600. Never heavier than 600.
- **Mono-for-money rules** (all via `money.rs`): dimmed decimals (integer `primary`, decimals+ticker
  `muted`, **color only**, no size step); every USD figure carries `$` or a `… USD` column header;
  one precision/abbreviation rule per context; zero renders `$0`, never `$0.0k`; reserve a fixed
  sign slot so decimals align across signed/unsigned.

---

## Spacing, sizing, radii, motion — the token layer
Everything here is a **named value**, and a view carries the name — never a raw `px(...)` that
duplicates one. Two naming vehicles, by design:
- **gpui's own utilities** already name two of the scales, so they stay the idiom: **spacing** is
  `.gap_N` / `.p_N` / `.m_N` (gpui's 4px grid — `N` units = `N×4`px), and the **h1 / section / small**
  type steps are `.text_xl` (20) / `.text_sm` (14) / `.text_xs` (12).
- **the `tokens` module** (`crates/deckard-app/src/tokens.rs`, see §Build notes) carries a
  `Pixels` / `Duration` const for everything gpui can't name — the display type sizes, 13px body,
  10px label, the exact radii, the object-size ladder, the chrome dimensions, and the arm-delay.

A raw `text_size(px(..))` is always a review failure (the `no_raw_text_size_px` test fails the
build); a raw `px(..)` elsewhere that duplicates a named value below is one too. Bespoke one-off
layout dimensions with no token (a specific column width) stay a literal — they name nothing.

### Spacing (the 4px grid — governs gaps, padding, margins)
`space-2` 2 · `space-4` 4 · `space-8` 8 · `space-12` 12 · `space-16` 16 · `space-20` 20 ·
`space-24` 24 · `space-32` 32 · `space-48` 48. `space-2` is the sole sub-grid half-step (for
hairline-adjacent gaps); every other value is a 4px multiple. Off-scale spacing is a bug: 34 → 32.
**Grouping** still comes from whitespace + a tiny section label, not a hairline between every row
(see §Visual language) — the scale is what that whitespace is *made of*.

### Object sizes (a SEPARATE ladder — marks, gauges, chrome)
Object sizes are tuned to glyph legibility and chrome ergonomics, **not** to the spacing grid, so
they get their own named ladder. This is the fix for the old "polices 34 but bakes in 30" tension:
object sizes are allowed off the grid, on purpose, and named — the grid governs *space between*
things, this ladder governs *the size of* things.
| token | px | use |
|---|---|---|
| `mark-sm` | 16 | inline identity mark (rows, chips) |
| `mark-md` | 20 | sidebar / breadcrumb mark |
| `mark-lg` | 30 | page-header mark |
| `track` | 4 | budget / utilization gauge thickness |
| `sidebar-w` | 248 | the sidebar column |
| `breadcrumb-h` | 44 | the top breadcrumb bar |
| `status-h` | 25 | the bottom status strip |
| `content-max-w` | 760 | the reading column on a main surface |
| `confirm-w` | 460 | the centered clear-signing / confirm card |

(Control heights — button / input / row — are not yet one token; standardizing them is a code-side
follow-up. Until then compose from the spacing scale, e.g. `space-8` vertical padding on `text-body`.)

### Radii
`radius-input` 4 (inputs, buttons, key-caps) · `radius-row` 6 (rows, marks, chips — never a
fully-rounded pill) · `radius-modal` 10 (confirm buttons, modals, palette, slide-over) ·
`radius-full` (the round human identicon only). Borders are always a 6-12% step from their
background; never harsh.

### Stroke widths
`stroke-hairline` 1 (dividers, input outlines, the focus ring) · `stroke-track` 4 (the gauge). There
are no other stroke widths.

### Elevation / layering (brightness + one scrim, never a shadow stack)
The editorial language rejects shadow-stacked cards, so elevation is **not** a shadow scale — it is
(1) background brightness (`bg.base` < `bg.hover` < `bg.raise` < `bg.raise2`) and (2) exactly one
dimming `bg.overlay` scrim under a floating surface. Stacking order, low → high:
`canvas → content → popover/menu → ⌘K palette → slide-over → modal (+ scrim) → toast`. A surface
higher in this order dims what's below it with the scrim; it does not cast a shadow onto it.

### Opacity / tint alpha
One alpha ladder, so tints stop being one-off rgba literals:
`alpha-hairline` .06 (faint fills) · `alpha-tint` .12 (the standard signal tint — cyan, red) ·
`alpha-tint-warm` .14 (amber only — it reads weaker at equal alpha, so it gets a hair more) ·
`alpha-scrim` .5 dark / .45 light (`bg.overlay`) · `alpha-disabled` .4 (a disabled control where the
`text.muted` step-down alone isn't enough).

### Icons
**Lucide**, hairline-weight, monochrome, inheriting `currentColor` — never a second accent, never a
filled/duotone style. Sizes: `icon-sm` 14 (inline in a `caution_line`) · `icon-md` 16 (status glyphs,
row leads) · `icon-lg` 20 (page-header glyph).

### Motion
`motion-fast` 120ms · `motion-base` 160ms · `motion-slow` 220ms; ease-out on enter, ease-in on exit.
Two fixed timings beyond those: `arm-delay` 450ms (the confirm's inert window, formerly "400-600ms")
and `pulse` 1600ms (the one ambient motion — a slow breathing pulse on an agent **currently
acting**, formerly "~1.6s"). No spinners, no skeleton confetti, no celebratory animation on money.

### Contrast targets (AA)
Text tokens meet WCAG AA on their background: `text.primary` / `text.secondary` and `text.muted`
≥ 4.5:1 (already noted for light `text.muted` on base). The signal colors are **graphical** objects
(marks, rings, glyphs), held to ≥ 3:1 against their background — amber and cyan clear this on both
themes. Never drop a text token below AA to fit a layout.

---

## Component primitives (spec + states)
Every interactive component defines **rest / hover / focus-visible / selected / disabled**, and data
components define **empty / loading / error**. Defaults: hover = ~5% lift (`bg.hover`);
selected/active = a brightness lift (`bg.raise`), **never a colored keyline**; focus-visible = 1px
amber ring; disabled = a step below base, `text.muted`, no hover.

- **Sidebar tree** — `PROJECTS` header. Project row: chevron + identity mark + name + hover `•••` +
  `+`. Children: **Wallets** (mark + name + mono address + right balance) and **Agents** (squircle +
  name + spend magnitude + status). Current selection gets the `bg.raise` lift. All rows through one
  `nav_row` with a single resting color.
- **Breadcrumb top bar** — `[identity mark] Project › current`, where `current` names the selected
  entity (the wallet/agent name, never the literal word "Wallet"). Right: network pill, ⌘K, theme,
  mask toggle. Network name appears once (not also restated in the status strip).
- **Page header** — one anatomy via `page_header`: identity mark/glyph + H1 (`text.primary`,
  `text-h1` 20/600) + a muted one-line subtitle.
- **Balance hero + allocation** — big mono balance (dimmed decimals) + a meta line
  (USD · synced · verified). Below it a thin tonal allocation bar (no amber) + a small legend.
  Loading shows a money-shaped skeleton bar, never a bare `—`.
- **Holdings ledger** — hairline rows, hover lift, clickable → Swap. Columns: Asset (desaturated
  token mark + name + dimmed ticker) · Balance · 24h · Value, all USD with `$`, Stripe-aligned so
  columns scan vertically. Empty = one muted line ("No assets yet"), **no box**.
- **Activity row + status glyphs** — one schema: `[glyph] [subject verb object · context] …… [signed
  amount] [status glyph] [time]`. Subject is "You" (amber) or the agent name. State is a small
  circular `status_glyph` (filled check = confirmed, amber clock-ring = pending, red x-ring =
  failed/declined). Day-grouped bands.
- **Amount input** (`amount_field`) — mono amount, asset chip right, dimmed USD-equiv, a Max link,
  and a live over-balance error border that disables the forward action. One component for Send and
  Swap.

### The confirm pattern (replaces hold-to-confirm)
Hold-to-confirm is an anti-pattern; we do not use it. Confirmation is **keyboard-first with a visible
key-cap affordance** (like Linear/Raycast "Create ↵"), made spam-proof by tiering and an arm-delay:
- **Routine forward steps** (Continue, Review, Next): the primary shows `↵`; Enter advances.
- **Irreversible money moves** (Send, Swap, Shield, Approve an agent proposal, Revoke): the primary
  shows the **`⌘↵` chord** (a chord can't be fat-fingered like Enter). The key-caps render amber when
  armed.
- **Arm-delay:** the confirm is inert for `arm-delay` (450ms) after the screen opens (key-caps
  dimmed), so a queued or held keypress from the previous screen can't carry through and fire. A one-line note
  states this: "Press ⌘↵ to send. It arms a moment after this screen opens, so a stray keypress
  can't approve it."
- **Highest-risk** (fresh address over a threshold, unlimited approval, agent over-cap): the first
  `⌘↵` **arms** ("Press ⌘↵ again to send") and the second sends. No modal, no checkbox.
- Mouse users get an equivalent click on the same button; the button is never a hold target.

### Clear-signing review (the shared trust engine; transaction-as-hero)
Rendered for self-send, swap, shield, and an agent proposal. It is a **statement, not a form**:
- The **transaction is the hero**: a tiny `SENDING`/`SWAPPING` label, then the **amount big in
  mono**, then the USD-equiv, then `TO` with a **prominent identicon + ENS + full-ish address**
  (via `truncated_address`). What can lose money (how much, to whom) dominates.
- **Danger first, in red** (`caution_line` danger): "Public on Ethereum and can't be undone."
  Then amber caution lines (first-time recipient, slippage). Never a gray box.
- **Quiet supporting facts** demoted below a hairline: From, network fee, route/slippage. State each
  fact **once** (no triple-restatement).
- The `⌘↵` confirm button + the arm note + an Edit link.

### Policy / agent surface, budget gauge, kill switch
See §The agent interaction model. The `budget_gauge` and the Pause/Revoke/Rotate/Adjust controls are
mandatory there.

### Command palette (⌘K)
The universal control plane. Every user-facing action registers a `Command`
(CLAUDE.md §Command palette reachability). Fuzzy + frecency, matched chars lift by weight not hue,
selected row is a brightness lift, shortcuts shown right-aligned **inside** the rows. Shortcuts live
here and in menus, **never as an on-canvas hint-chip row** (that was slop; it is deleted).

### Required states the build must include
Disabled primary when input is invalid; input error (amount > balance) with border + helper
replacing Max; pending/failed in Activity; loading = skeleton rows (never a spinner, never a bare
`—`); empty = one muted line (no box, no illustration).

---

## The agent interaction model
The product's reason to exist, and the part to get right and lean.

### What an agent is (v1 reality, do not over-build)
Three authority models, kept distinct (`docs/agent-authorization-map.md`):
1. **Key-less proposer — ALL of v1.** The agent proposes typed intents via the MCP sidecar; the
   human approves; the daemon signs with the human's key only after the policy gate. Limits are
   **software-enforced, not chain-enforced** (never claim "cannot exceed"). STOP zeroizes the key.
2. **Agent wallet / session keys** — a distinct agent-controlled address, EIP-7702. **Deferred**
   (ADR-0002).
3. **Per-origin dapp grant** — a different principal. **Deferred** (ADR-0001).

So a v1 agent **is its policy + its activity**, nothing more.

### The expandability contract (lean now, expandable later)
**An agent is rendered entirely from its policy data and its activity feed.** Adding a capability is
a new field on the fence + a new verb in the feed, never a redesign. Multiple agents = more rows +
more surfaces. The deferred model-2 (agent wallet/session keys) becomes new fence fields + a second
identity on the agent surface. The feed already attributes by actor, so multi-agent works for free.
Build for one agent (Atlas) today; change nothing structural to reach N.

### Where agents live
- **First-class and standalone in the sidebar** under the project, beside Wallets.
- **The agent surface** (select an agent): identity (squircle + name `text.primary` + live status
  with the acting-pulse) + a one-line **plain-language autonomy statement** ("Atlas acts on its own
  under $0.20 per move and asks you above that. It can shield ETH only. It never holds your key, and
  it cannot send to a new address.") + **editable** Limits (per-tx cap, daily budget + the
  `budget_gauge`) and Scope (allowed actions, allowed assets, session-key expiry) edited **in-app,
  not policy.json** + the controls **Pause / Rotate key / Adjust limits / Revoke (and STOP)**, kill
  switch always one deliberate action away + **what this agent did** (its slice of the feed). This
  surface owns the policy; the wallet home does not.
- **The wallet home shows only a compact agent presence**: one `nav_row`-style row per agent
  (squircle + name + status + a thin `budget_gauge` + a chevron) that links to the agent surface.
  The home answers "is an agent running and how close to its budget" at a glance; detail is one
  click away. The old read-only policy dump on the home is removed.

---

## Activity (see-and-stop)
**User headspace:** "My money is on autopilot. What just happened, is anything waiting on me, can I
stop it right now?" Vigilance plus reassurance.

### Lean scope (build now)
- **An audit log**: append-only, newest first, day-grouped, status glyphs, what the agent and you
  did, attributed by actor. Calm zero-state ("All clear, nothing needs you").
- **STOP**: always top-right; arms to `⌘↵`; revokes + zeroizes the key.
- Live streaming as the agent loop runs; the acting-pulse on the agent glyph.

### Expandable target (documented, build later)
A **triage inbox ("Needs you")** stacked above the log: pending approvals as actionable rows (open →
the clear-signing review → Approve `⌘↵` / Deny / Inspect), keyboard nav (j/k, Enter), drill-in
receipts (tx hash, the cited limit, the policy check, explorer link), and filter by actor / status /
wallet, with repetitive within-cap actions collapsing into a group so exceptions float up. The
no-blind-approve invariant holds (approve resolves only the still-pending reviewed record).

---

## Trust & safety affordances (this holds funds)
- **Addresses** always mono, middle-truncated **first-6 + last-4** (one rule, via `short_addr`),
  paired with identicon + ENS (via `truncated_address`), one-click copy with inline "Copied ✓", and
  a warning if a recipient resolves to the active wallet.
- **Clear-signing** as above: plain language, exact mono figures, danger early in red, caution as an
  amber icon inline (no box), the `⌘↵` confirm.
- **Seed reveal** — blurred, **hold-to-reveal**, auto-hides after a few seconds, a "make sure nobody
  is watching" caution, **never auto-copied**, Copy demoted below reveal, index numbers legible.
  (Hold-to-reveal is fine; it is hold-to-*see*, not hold-to-*approve*.)
- **Network warning** on Receive — one amber `caution_line`, the risk word emphasized.
- **Kill switch** — Pause / Revoke / Rotate always one deliberate action away on the agent surface;
  a master "Pause all agents" in Settings. STOP on Activity zeroizes the key.

## Per-chain trust tiers
Verified reads are mainnet-only (embedded Helios). Every other chain reads from a trusted RPC. We
show those chains in the one wallet and never let an unverified number wear the verified look.
- **Tier 1 — mainnet:** Helios-checked. `Verified` on a fresh head, `Unsynced` otherwise.
- **Tier 2 — verified L2 (future):** OP-stack via helios-opstack is sequencer-trusted, renders
  `Degraded`, never `Verified` (#77).
- **Tier 3 — raw-RPC (Arbitrum, Tempo, most L2s):** no light client, every read is `Unsynced` /
  "NOT VERIFIED". Reuse that affordance; a Tier-3 balance never gets the `Verified` row treatment.
- **Never fake it:** a non-mainnet read never reaches `Verified`. The downgrade is loud, not hidden.
- **No native asset (Tempo):** gas in a stablecoin, no native balance. Deferred until the portfolio
  can show "no native asset" instead of a placeholder dressed as money.
- **Guardrail is per-chain:** the human-approval brake fires on every real-value chain (#76).
- A transport/RPC failure shows a **calm humanized line** ("Couldn't reach the network. Retrying."),
  never the raw provider string in the status strip.

## Onboarding flow
A stepped, calm, full-bleed flow: **Welcome** (lead with the promise, not "Welcome to Deckard")
→ **Secure** (passphrase + a live **strength meter** + the consequence: "if you forget it, no one
can reset it"; algorithm names demoted to a details affordance) → **Back up** (recovery phrase,
hold-to-reveal, nobody-watching, never-auto-copied) → **Verify** (a **separate step**; the grid is
**hidden**, confirm by position, the primary disabled until correct) → **Ready** (a real screen).
Amber only on the active step, the caution, and the focus ring; primary CTAs neutral.

---

## SAFE vs RISK
- **SAFE (category baseline):** dark-first near-black + cool neutrals, one accent discipline,
  grotesk + mono pairing, minimal motion, keyboard-first + ⌘K, clear-signing, status-as-glyph, the
  Conductor/Splits sidebar tree.
- **RISK (Deckard's face):** the **two-signal axis** (amber human / cyan agent); monospace for ALL
  money + addresses; the **agent layer as first-class** (standalone in the sidebar, its own surface,
  rendered from policy); Portfolio as control + composition, not a price-chart casino; the editorial
  type-driven, card-free composition; the breathing "currently acting" pulse as the one ambient
  motion; the `⌘↵` key-cap confirm as the signature trust gesture.

## Build notes (GPUI)
The token names above map to Rust consts, not prose:
- **Color** lives in `theme.rs` — `refine()` overrides the gpui-component `ThemeConfig` slots (light +
  dark). The two signal colors + their tints and the identity/shield neutrals resolve through the
  theme (`amber`/`agent`/`amber_tint`/`agent_tint`/`identity_square`/`shield`), so a widget reads them
  from `cx.theme()` rather than taking them as arguments.
- **Spacing / type / radii / stroke / motion** live in a `tokens` module as `const Pixels` /
  `Duration` (`space_8`, `text_body`, `radius_modal`, `motion_base`, …). A view uses the const; a raw
  `px(...)` that duplicates a named token is a review failure and the magic-number lint flags it.
- **Fonts** bundle via GPUI assets (`add_fonts` in `main.rs`); no web-font CDN. Name the fallback
  stacks (§Typography) — GPUI silently drops to the system font if a family isn't registered.
- `div()` is `display:block` in GPUI, so a child's `flex_1`/`justify_center` is inert unless the
  parent is `v_flex`/`h_flex`. GPUI's `Styled` has no letter-spacing setter, so `tracking-label` is
  approximated with size + uppercase on the tiny label (the only tracked text).

---

## Visual definition of done (QA + /code-review enforce this)
A GUI change is not done until ALL hold (paste screenshots as evidence):
- [ ] Renders in **Schibsted Grotesk + JetBrains Mono** (fonts actually bundled), not the system font.
- [ ] Money + addresses are **mono**, dimmed-decimals, via `money.rs`; addresses via `short_addr`
      (6+4) + identicon.
- [ ] No raw hex colors in the view; only `theme.*` + the theme's `amber`/`agent`.
- [ ] No magic-number `px()` for a value the token layer names — spacing via `space-*`, type/object
      sizes via their tokens, radii via `radius-*`, motion via `motion-*` (the lint flags raw dupes).
- [ ] **No card unless purposeful**; grouping is whitespace + hairlines + section labels.
- [ ] **No `⚠` emoji** anywhere; caution/danger via `caution_line` (icon, no box).
- [ ] Confirm is the `⌘↵` key-cap (or `↵` for routine), **never hold-to-confirm**; arm-delay present.
- [ ] Section labels via `section_label` (10-11px uppercase tracked); no divergent copies.
- [ ] Rows clamp (`min_w_0` + truncate left, `flex-none` right); nothing overflows the pane.
- [ ] Empty/loading/error/pending/failed/disabled states present; loading is a skeleton, not `—`.
- [ ] Amber ≤1% of pixels and only on human/caution/where-you-are/focus/armed-confirm.
- [ ] Every new action has a ⌘K `Command`.
- [ ] No leftover starter slop (keyboard-hint rows, "Welcome to Deckard", dead settings, leaked
      build-flag or provider strings, orphan `—`).
- [ ] Matches the golden-reference HTML (`designs/deckard-editorial-v3.html` /
      `designs/deckard-agent-v4.html`) in layout and hierarchy.

## Decisions log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-06-05 | Initial v0 system | /design-consultation. Amber-on-near-black, General Sans + JetBrains Mono, mono-for-money, clear-signing. |
| 2026-06-05 | Human-sovereign / agent-delegated; memorable thing = "autopilot you can see and stop" | The product is a wallet for agents that doesn't suck; the human stays principal. |
| 2026-06-05 | Two-signal axis: amber=human, cyan=agent | Makes "who acted" instant; the Blade-Runner duality as core semantic. |
| 2026-06-05 | IA: Conductor sidebar tree (Projects → Wallets + Agents), 2-pane, contextual views | Grounded in real Conductor + Splits shots. |
| 2026-06-05 | Re-grounded in REAL Linear/Conductor/Splits screenshots | First drafts were rumor-mill slop. |
| 2026-06-20 | Multi-chain = one wallet with a loud honest downgrade; mainnet stays the only `Verified` tier | Verified-or-honestly-labeled reads are the moat. Scope: `docs/research/multichain-scope.md`. |
| 2026-06-20 | **v2 foundation overhaul.** Made the foundation **enforceable**: bundle the fonts (they were never embedded, the app ran in the system font), one shared `widgets.rs` vocabulary, a visual definition of done. | An 8-agent source audit found the bones disciplined but the leaf widgets hand-rolled per file (3-4 divergent copies of every helper) and fonts unbundled. Prose could not fix per-file copy-paste. |
| 2026-06-20 | **Editorial visual language; reject card-default.** Hierarchy from type + whitespace + hairlines; surfaces/cards are rare and purposeful; oversized mono hero; cockpit row layout. | Chose Editorial over Instrument/Vault against real references. Defaulting to elevated cards is the AI-coded-design tell; Linear/Superhuman don't do it. |
| 2026-06-20 | **`⌘↵` key-cap confirm replaces hold-to-confirm.** Tiered (`↵` routine / `⌘↵` irreversible / double-`⌘↵` highest-risk) + arm-delay. | Hold-to-confirm is an anti-pattern; the key-cap is keyboard-first and the chord + arm-delay make it spam-proof. |
| 2026-06-20 | **Transaction-as-hero clear-signing.** Amount + recipient are the heroes; details demoted; danger red first; caution amber inline (no box). | The old flat key/value form read amateurish; what can lose money must dominate. |
| 2026-06-20 | **Agent interaction model.** Agents first-class standalone in the sidebar; a dedicated agent surface owns editable policy + controls + its own activity; the home shows a compact agent presence; expandability contract (agent = policy + activity, UX renders from data). | The read-only policy dump on the home was awkward; an agent that spends your money deserves a first-class surface; lean for one agent, expandable to N and to model-2 without redesign. |
| 2026-06-20 | **Activity lean now = audit log + STOP**; triage inbox / keyboard nav / drill-in receipts / filtering deferred and documented. | Build the see-and-stop log first; layer the inbox interactions once the loop is real. |
| 2026-06-20 | **UI/display face: General Sans → Schibsted Grotesk** (JetBrains Mono unchanged). | General Sans is Fontshare/ITF proprietary — its EULA forbids redistributing the raw files / public-server hosting, which the public repo violated once #114 committed them. Schibsted Grotesk is OFL-1.1, a structural drop-in (same 400/500/600 weights), and built for editorial publishing — it fits the locked Editorial direction. Chosen over Hanken Grotesk (safer/quieter) and IBM Plex Sans (more recognizable) via /design-consultation. |
| 2026-07-01 | **v3 token layer + versioned references.** Named every scale (spacing, object sizes, type, radii, stroke, elevation, opacity, icon, motion) as a token → Rust const; gave the semantic roles their own names over the single warm light (deliberate overload, not accidental); promoted light to a full token table; added elevation/opacity/AA sub-specs; unified the transaction hero to one `text-tx-hero` (was 40/32); declared object sizes a separate ladder (resolving the 30px-off-grid tension); checked the golden HTML references into `designs/`. | A design-system audit found the doctrine strong but enforcement partial: color + fonts were centralized, but everything below (127 raw `px()` across ~43 values, 11 scattered `text_size(px())`, duplicated leaf widgets) was hand-rolled, and "matches golden reference" pointed at an out-of-repo, unversioned, stale file. Prose can't enforce a spacing grid; a `const` the compiler + a lint check can. |
