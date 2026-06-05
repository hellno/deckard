# Design System — Deckard

> Source of truth for every visual and UI decision. Read this before building any view.
> Evolved 2026-06-05 from the v0 single-screen system into a multi-surface foundation.
> Interactive reference (dogfoods every token, light + dark, all surfaces):
> `~/.gstack/projects/hellno-deckard/designs/deckard-foundation-preview.html`

## The memorable thing
**Your money on autopilot — and you can see and stop everything.** Calm sovereign control
plus total auditability. Linear's calm precision and Conductor's parallel-agent cockpit, with
a terminal's honesty and a thin Blade Runner edge (deep near-black, warm light for the human,
cold light for the machine). Every decision below serves that feeling. Not a casino.

## Product context
- **What:** Deckard, a native desktop (macOS + Linux) self-custodial Ethereum wallet, built in
  Rust + GPUI 0.2 + gpui-component 0.5. Offline-first, privacy-focused, open source (AGPL-3.0).
- **Model:** **human-sovereign, agent-delegated.** The human owns the keys and sets policy;
  AI agents act on the human's behalf within budget/scope/policy bounds (the "sovereign autopilot").
  The human is the principal; agents are constrained delegates.
- **Who:** onchain operators who live in crypto and refuse custodians; later, anyone delegating
  money-movement to agents.
- **Peers in feel:** Linear, Conductor, Splits, Superhuman, Stripe. Dense, calm, keyboard-fast,
  premium. NOT consumer-crypto casino UIs.

## How this system was grounded (read this)
The first drafts of this system were built from **prose descriptions** of Linear/Conductor — and
read as generic AI slop. The fix was looking at the **actual product screenshots** of Linear
(settings, board, dense list), Conductor (project rail), and Splits (subaccount sidebar), and
re-deriving the language from pixels. **Rule for all future design work: ground every decision in
real reference screenshots, never in remembered descriptions.** Use the `gemini` vision skill or
read the images directly. The reference shots live in `.context/attachments/` for this session.

---

## Information architecture
**Two-pane shell** (sidebar + main + a thin top breadcrumb bar + a bottom status strip). No third
inspector pane — neither Splits nor Conductor has one; detail is contextual or a right slide-over
drawer when genuinely needed.

**Sidebar = a single-column Conductor-style tree.** Top-level entities are **Projects** (spaces /
contexts, e.g. "Personal", "DAO Treasury", "Trading"). Each project expands to two groups:
- **Wallets** — Splits-style subaccount rows: identity square + name + truncated mono address +
  right-aligned balance.
- **Agents** — the delegates scoped to that project's wallets: cyan squircle glyph + name +
  right-aligned spend magnitude + status glyph.

**Views are contextual to selection, not top-level nav:**
- Select a **wallet** → its home (Splits-style: header, balance hero, allocation bar, Send/Receive/
  Swap actions, holdings, recent activity).
- Select a **project** → aggregate balance across its wallets + its wallets and agents.
- Select an **agent** → its **policy card** (the agent is defined purely by policy, no job-type copy)
  + controls + recent activity.
- **Settings** lives at a bottom gear (global). **Send / Receive / Swap** are actions on the
  selected wallet, not nav destinations. **Clear-signing is the confirm step inside Send/Swap**, and
  the same review component renders for an agent's proposal awaiting approval.

> Open question (do not hard-code yet): how agents are *registered* in the UX. The foundation
> places them under projects beside wallets, but their lifecycle/identity model is still being
> decided. Keep agent surfaces buildable from policy data alone.

---

## The actor model (human vs agent)
A machine spends the human's money, so every attributable row must answer "who did this — me or
which agent?" in under a second. The answer is a **two-signal axis plus shape**, never a rainbow:

- **amber = the human / "where you are" / caution.** **cyan = the agent class** (a machine actor).
  This is the Blade-Runner warm/cold duality made load-bearing. The two never mean the same thing,
  so they never compete. This is a disciplined 2-color system, not categorical coloring.
- **Shape is the accessibility backup:** the human principal uses a **full-round** identicon; an
  agent uses a **cyan squircle** monogram. Survives grayscale and dark mode.
- **Identity, not state:** cyan marks agent identity (the squircle glyph + status); it is **never**
  used to tint a page title, body text, or a link. Page titles are always `text.primary`.
- **Accountability chain is rendered, not flattened:** an agent-proposed + human-approved action
  shows both actors with a connector ("Atlas proposed → You approved").

---

## Color
~95% grayscale. The two signal colors are spent sparingly; semantics are kept distinct from both.

### Dark (primary)
| token | hex | use |
|---|---|---|
| `bg.base` | `#0A0B0D` | app canvas |
| `bg.rail` | `#0B0C0F` | sidebar / status strip / top bar |
| `bg.raise` | `#121419` | active/selected lift, cards, inputs |
| `bg.raise2` | `#161922` | primary-button fill, popovers, palette |
| `bg.hover` | `#14161B` | hover lift |
| `border.hairline` | `#1B1E25` | dividers (a ~8% brightness step from base) |
| `border.strong` | `#262A33` | input outlines, stronger separation |
| `text.primary` | `#E7E9EC` | headings, values (never pure white) |
| `text.secondary` | `#9298A2` | labels, body |
| `text.muted` | `#646A73` | metadata, addresses, dimmed decimals |
| `accent` (amber) | `#F2A43B` | hover `#FFB454`, tint `rgba(242,164,59,.14)` |
| `agent` (cyan) | `#3CC9BC` | tint `rgba(60,201,188,.12)` |
| `success` | `#4FB463` · `error` | `#E5565B` (tint `rgba(229,86,91,.12)`) |

### Light (first-class equal, refined — must not wash out)
`bg.base #F6F5F1` · `rail #EEEDE6` · `raise #FFFFFF` · `hover #ECEBE4` · `hairline #DDDBD2`
(a ~9% step, stronger than dark's proportion so panes still separate) · `border.strong #CFCCC2` ·
`text.primary #17191E` · `text.secondary #52585F` · `text.muted #6B7280` (>=4.5:1 on base —
do not go lighter) · `accent #A8650C` (deepened for AA) · `agent #0C7E75`.

### Color discipline (rules the design review settled)
1. **Amber is the one warm light, <1% of pixels.** It means only: the human acting, "where you
   are" (active step, status "awaiting you"), caution, and the brief **hold-to-confirm** gesture
   sweep. It is **NOT** a primary-button color, **NOT** a routine toggle, **NOT** a chart segment,
   **NOT** a logo-plus-button-plus-meter stack on one screen.
2. **Primary buttons are neutral** (`bg.raise2` fill, `text.primary`, weight 600). Amber appears
   only as the fill sweep during a hold gesture. (Linear's primaries are neutral high-contrast.)
3. **Cyan is only the agent class**, low-chroma, on the squircle glyph + agent status. Never title
   or link text.
4. **Identity colors** (project/wallet squares) are **desaturated** tinted-neutral chips. They must
   avoid the warm/amber band entirely, and identity-green must sit off the semantic `success` hue
   (use teal-leaning greens for identity, reserve `#4FB463` for status only). Token swatches follow
   the same rule (rETH etc. must be cool, never gold).
5. **Allocation/category bars** use neutral/low-chroma tonal steps, never amber as a category.
6. **Danger stays loud red, early.** Even though the app is otherwise near-colorless, unlimited
   approvals, unknown contracts, fresh-address sends, and over-cap states surface in `error`.
7. **Caution banners** = neutral surface + a 2px amber left keyline + amber icon/text. Not a filled
   warm block.
8. **Budget/utilization bars** = thin (4px), neutral track, neutral/cyan fill at rest; amber only
   ≥90%, red at ≥100%. Never a saturated amber slab.

---

## Typography
- **UI / display:** **General Sans** (Fontshare, free, bundleable). Sentence case everywhere; no
  ALL-CAPS shouting except tiny section labels. Hierarchy from weight + color, not size.
- **Money / numbers / addresses / hashes:** **JetBrains Mono**, tabular figures, full precision,
  never abbreviated in a ledger. Bundle both fonts; no web-font CDN (offline-first).
- **Scale (px):** screen-title/H1 19–24 · section 14 · body 13 · label 10–11 (uppercase, +.6–.8
  letter-spacing) · balance hero 30–40 mono.
- **Weights:** General Sans 400/500/600. Never heavier than 600.
- **Mono-for-money rules:**
  - **Dimmed decimals:** integer in `text.primary`, the decimal part + ticker one tier quieter
    (`text.muted`). Reduce the decimals via **color only**, not also a size step (avoid the
    superscript look).
  - **$ discipline:** every USD figure carries `$` or its column is labelled `… USD`; never mix
    `$39,200.55` (with) and `3,141.40` (without) in one table.
  - **One precision + abbreviation rule per context;** zero renders `$0`, never `$0.0k`.
  - **Tabular sign column:** reserve a fixed slot for +/- so decimal points align across signed and
    unsigned amounts.

---

## Spacing, radii, motion
- **Spacing:** 4px grid. Scale 2 · 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48. Density compact-comfortable;
  Settings is deliberately more spacious (~52px rows, section headers + whitespace).
- **Grouping:** tiny uppercase section labels + **whitespace**. **Not** a hairline between every
  row. Cards never have interior cross-rules; one faint frame at most.
- **Borders:** always a 6–12% brightness step from the background. Never harsh.
- **Radii:** 4px inputs/buttons · 6–7px rows/cards/chips (no fully-rounded pills anywhere) · 10–11px
  modals + palette · full only for round identicons.
- **Motion:** minimal-functional, 120 / 160 / 220ms, ease-out enter / ease-in exit. The one ambient
  motion allowed is a slow ~1.2s breathing pulse on an agent that is *currently acting*; everywhere
  else renders instantly from local cache. No spinners, no skeleton confetti, no celebratory animation
  on money movement.

---

## Components (spec + states)
Every interactive component must define **rest / hover / focus-visible / selected / disabled**, and
data components must define **empty / loading / error**. Defaults:
- **hover** = ~5% brightness lift (`bg.hover`). **selected/active** = a brightness lift
  (`bg.raise`), **never a colored keyline**. **focus-visible** = 1px amber ring (the sanctioned
  accent focus). **disabled** = `bg` one step below base, `text.muted`, no hover, not actionable.

- **Sidebar tree** — `PROJECTS` header (label + filter + new). Project row: chevron (toggles
  expand) + identity square + name + hover-revealed `•••` (context menu: Rename / New wallet / New
  agent / Project settings / Remove project[red]) and `+`. The **current** project (containing the
  selection) reads brighter; the selected row gets the `bg.raise` lift. Children: hover reveals a
  drag handle; wallet rows show a dimmed right-aligned balance; agent rows show a dimmed mono spend
  magnitude + status glyph. Inline rename on double-click.
- **Breadcrumb top bar** — `[identity square] Project › current`. Right: network pill, search/⌘K,
  theme toggle. Not two competing chips.
- **Page header** — one anatomy for every surface: H1 (`text.primary`, 19–24/600) + optional muted
  one-line subtitle (+ identity glyph for agents), then content. Wallet uses wallet-name H1 +
  address subtitle above the balance hero, matching project/agent.
- **Balance hero + allocation bar** — big mono balance (dimmed decimals) + a meta line
  (change · synced). Below it, a thin Splits-style allocation bar (tonal, no amber) + a small
  legend. Enforce a min visible width (~3px) for any non-zero segment.
- **Holdings table** — tight rows, hairline row separators only, hover lift, clickable → Swap.
  Columns: Asset (desaturated token square + name + dimmed ticker) · Balance · Price · Value · 24h.
  All USD with `$`; deltas desaturated success/error.
- **Activity row + status glyphs** — one schema everywhere: `[icon] [subject verb object · context]
  …… [signed amount] [status glyph] [time]`. Subject is "You" or the agent name. **State is a small
  circular status glyph**, not a pill/dot: `success` filled check = confirmed, amber clock ring =
  pending, `error` x-ring = failed. Day-grouped header bands.
- **Amount input** — one component for Send (asset fixed) and Swap (asset selectable): mono amount
  left, asset chip right, dimmed USD-equivalent inline. Focus = amber ring.
- **Clear-signing review card** (the shared trust engine, rendered for self-send, swap, and an
  agent's proposal): a plain-language **headline**, then **one** canonical key/value list grouped by
  whitespace (no per-row hairlines, no triple-repeat of the same fact). Danger surfaces at the top
  in red. Confirm is a **deliberate hold**, never a tap. For agent proposals, a header band names
  the requesting agent + the policy/boundary it cites.
- **Policy card** (agent) — 2-column label/value pairs grouped by whitespace inside one faint frame;
  **no interior grid lines**, 16px+ cell padding so values never clip. States: per-tx cap, period
  budget + reset, allowed assets, allowlist, session-key expiry countdown, the one autonomy line
  ("act < $X · ask above"). Followed by a thin threshold-colored budget gauge and
  Pause / Revoke key / Rotate / Adjust limits controls.
- **Buttons** — primary (neutral fill, weight 600; amber hold-sweep only on irreversible confirms),
  ghost (transparent + hairline), danger (error text). Routine forward CTAs (Continue, Open Deckard,
  Confirm) are neutral, never amber.
- **Toggle / segmented / dropdown** — toggle "on" is neutral/low-chroma for routine prefs (amber
  only for security-relevant switches). Segmented selected state must have a clear (>=15% luminance)
  gap from unselected in BOTH modes, and bind to real state.
- **Context menu** — popover with hover-highlighted items, consistent group gaps, destructive items
  in red, keyboard hints right-aligned where shortcuts exist.
- **Command palette (⌘K)** — the universal control plane across every surface (send, swap, switch
  project, pause agent, approve, jump to asset, lock). Opens <50ms, fuzzy, shows shortcuts.

### Required states the build must include (flagged missing in review)
- **Disabled** primary action when input is incomplete/invalid (e.g. empty amount, unanswered seed
  verify). Not holdable, `text.muted`.
- **Input error** (amount > balance): error border + helper text replacing the "Max" link.
- **Pending / failed** transactions in Activity (not only "Confirmed"): pending = amber clock ring,
  failed = red x-ring.
- **Loading**: 3 skeleton rows at `bg.raise` (subtle), never a spinner.
- **Empty**: one muted line ("No assets yet" / "No activity"), no illustration.

---

## Trust & safety affordances (this holds funds)
- **Addresses** always mono, middle-truncated, paired with identicon + ENS; raw hex one action
  away; one-click **copy** with an inline "Copied ✓". Show enough of each address (first 6 + last 4)
  that two addresses are visually distinguishable; warn if a recipient resolves to the active wallet.
- **Clear-signing** as above: plain language first, exact mono figures, danger early, deliberate
  hold. The "three facts" framing (what leaves / where / how to stop) is the spirit; do not let it
  become a third restatement of the same data.
- **Seed reveal** — blurred by default, **hold-to-reveal**, auto-hides after a few seconds, a
  "make sure nobody is watching" caution, **never auto-copied** (and Copy is visually demoted below
  Hold-to-reveal). The index numbers stay legible so the grid reads as "present but hidden."
- **Network warning** on Receive — the one caution moment; neutral surface + amber keyline, the risk
  word emphasized, not the network chip.
- **Kill switch / revocation** — Pause / Revoke / Rotate always one deliberate action away on any
  agent; a master "Pause all agents" belongs in Settings (agent governance), styled deliberate.

## Onboarding flow
A stepped, calm, full-bleed flow: **Welcome** (Create / Import) → **Secure** (passphrase + strength;
Argon2id + XChaCha20-Poly1305, "we can't reset it") → **Back up** (recovery phrase, hold-to-reveal,
nobody-watching, never-auto-copied) → **Verify** (confirm words by position; Confirm disabled until
correct) → **Ready**. Import path: 12/24-word grid. A standalone re-reveal is reachable from the
lock affordance and the Settings keystore danger zone. Amber appears only on the active step, the
caution banner, and the focus ring — primary CTAs are neutral.

---

## SAFE vs RISK
- **SAFE (category baseline):** dark-first near-black + cool neutrals, one accent discipline, grotesk
  + mono pairing, minimal motion, keyboard-first + ⌘K, clear-signing, status-as-glyph, the
  Conductor/Splits sidebar tree.
- **RISK (Deckard's face):** the **two-signal axis** (amber human / cyan agent) as the core semantic;
  monospace for ALL money + addresses; the agent layer as first-class (roster/policy/approvals);
  Portfolio framed as control + composition, not price-chart casino; the breathing "currently acting"
  pulse as the one ambient motion.

## Build notes (GPUI)
Everything here is expressible in `theme.rs` (token override model, light + dark) + gpui-component.
The two-signal axis adds `agent.*` tokens alongside the existing accent. The agent squircle = the
existing identicon language with a stroked rounded-rect; the budget gauge + breathing pulse fit the
existing motion budget. Fonts bundle via GPUI assets. No web-font CDN.

## Decisions log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-06-05 | Initial v0 system | /design-consultation. Amber-on-near-black, General Sans + JetBrains Mono, mono-for-money, clear-signing. |
| 2026-06-05 | Reframe to human-sovereign / agent-delegated; memorable thing = "autopilot you can see and stop" | The product is a wallet for agents that doesn't suck; the human stays principal. |
| 2026-06-05 | Two-signal axis: amber=human, cyan=agent | Makes "who acted" instant; turns the stated Blade-Runner duality into the core semantic; stays a 2-color discipline (cyan = class, not per-agent). |
| 2026-06-05 | IA: Conductor sidebar tree (Projects → Wallets + Agents), 2-pane, contextual views, Splits-style wallet rows | Grounded in real Conductor + Splits product shots; gives agents a home without a separate console. |
| 2026-06-05 | Re-grounded the whole language in REAL Linear/Conductor/Splits screenshots | First drafts were rumor-mill slop; active=brightness-lift, whitespace grouping, circular status glyphs, Lucide icons, amber <1% all come from the pixels. |
| 2026-06-05 | Design-review pass applied | Neutral primary buttons (amber only for caution/hold/where-you-are/focus); thin threshold budget bars; agent title white (identity via glyph); no interior card grid-lines; collapsed clear-signing; desaturated identity/token colors off the amber + ok-green bands; light-mode contrast + segmented-state fixes; money $/precision/zero rules; required empty/loading/error/pending/failed/disabled states. |
