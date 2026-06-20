# Design System — Deckard

> Source of truth for every visual and UI decision. Read this before building any view.
> v2 (2026-06-20): re-grounded the visual language (editorial, not card-heavy), made the
> foundation **enforceable** (bundled fonts + a shared widget vocabulary + a visual definition
> of done), redesigned the trust-critical confirm, and defined the agent interaction model.
>
> **Golden references (the pixel ground-truth agents build against):**
> - `~/.gstack/projects/hellno-deckard/designs/deckard-editorial-v3.html` — home, send confirm,
>   swap compose, swap review, activity (the editorial language, light + dark).
> - `~/.gstack/projects/hellno-deckard/designs/deckard-agent-v4.html` — the agent surface, the
>   compact agent presence on the home, and the redesigned transaction-as-hero confirm (this
>   confirm supersedes the v3 send confirm).
> When this doc and a reference disagree, the doc wins on rules and the reference wins on pixels;
> fix whichever is stale.

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
  amount) is the biggest thing: ~64-72px mono on the wallet home, ~44px on a confirm. Money is the
  load-bearing object; make it unmistakable.
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

### Light (first-class equal, refined; must not wash out)
`bg.base #F6F5F1` · `rail #EEEDE6` · `raise #FFFFFF` · `hover #ECEBE4` · `hairline #DDDBD2` ·
`border.strong #CFCCC2` · `text.primary #17191E` · `text.secondary #52585F` · `text.muted #6B7280`
(>=4.5:1 on base) · `accent #A8650C` · `agent #0C7E75` · `success #2F8F47` · `error #C23B40`.

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

---

## Typography
- **UI / display:** **Schibsted Grotesk** (bundled, OFL-1.1). Sentence case; no ALL-CAPS except tiny section
  labels. Hierarchy from **weight + size + color, in that order** (the old build leaned on color
  alone, which read flat).
- **Money / numbers / addresses / hashes:** **JetBrains Mono** (bundled), tabular figures, full
  precision, never abbreviated in a ledger.
- **Scale (px):** balance hero 64-72 mono (home) / transaction hero 44 mono (confirm) · screen-title
  H1 20-22/600 · section 14/600 · body 13/400 · label 10-11 uppercase +0.07em tracking.
- **Weights:** 400 / 500 / 600. Never heavier than 600.
- **Mono-for-money rules** (all via `money.rs`): dimmed decimals (integer `primary`, decimals+ticker
  `muted`, **color only**, no size step); every USD figure carries `$` or a `… USD` column header;
  one precision/abbreviation rule per context; zero renders `$0`, never `$0.0k`; reserve a fixed
  sign slot so decimals align across signed/unsigned.

---

## Spacing, radii, motion
- **Spacing:** 4px grid. Scale 2 · 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48. Hoist named mark sizes
  (16 / 20 / 30 px, radius 4-7) so squares never drift; 34px is off-grid, use 32.
- **Grouping:** tiny uppercase section labels + **whitespace**, not a hairline between every row.
  Default to no surface; see §Visual language.
- **Borders:** always a 6-12% step from the background; never harsh.
- **Radii:** 4px inputs/buttons · 6-7px rows/marks/chips (no fully-rounded pills) · 9-11px confirm
  buttons / modals / palette · full only for the round human identicon.
- **Motion:** minimal-functional, 120 / 160 / 220ms, ease-out enter / ease-in exit. The **one**
  ambient motion is a slow ~1.6s breathing pulse on an agent **currently acting**. No spinners, no
  skeleton confetti, no celebratory animation on money movement.

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
- **Page header** — one anatomy: identity mark/glyph + H1 (`text.primary`, 20-22/600) + a muted
  one-line subtitle.
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
- **Arm-delay:** the confirm is inert for ~400-600ms after the screen opens (key-caps dimmed), so a
  queued or held keypress from the previous screen can't carry through and fire. A one-line note
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
Everything here is expressible in `theme.rs` (token override, light + dark) + gpui-component +
`widgets.rs`. Fonts bundle via GPUI assets (`add_fonts` in `main.rs`); no web-font CDN. The two
signal colors are app-level helpers (`theme::amber`/`agent`). `div()` is `display:block` in GPUI, so
a child's `flex_1`/`justify_center` is inert unless the parent is `v_flex`/`h_flex`.

---

## Visual definition of done (QA + /code-review enforce this)
A GUI change is not done until ALL hold (paste screenshots as evidence):
- [ ] Renders in **Schibsted Grotesk + JetBrains Mono** (fonts actually bundled), not the system font.
- [ ] Money + addresses are **mono**, dimmed-decimals, via `money.rs`; addresses via `short_addr`
      (6+4) + identicon.
- [ ] No raw hex colors in the view; only `theme.*` + `theme::amber/agent`.
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
- [ ] Matches the golden-reference HTML in layout and hierarchy.

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
