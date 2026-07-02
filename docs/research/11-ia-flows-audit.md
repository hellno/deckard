# Grounded flow audit — the funded demo, as-built (feeds #170)

> Discovery audit for the UI/UX overhaul (#170, step 1). The app was run on a **funded
> `just demo`** (anvil fork of Sepolia @ block 10822990, chain 11155111, verified-reads off) on
> macOS and driven live: onboarding → unlock → home → send / swap / shield / receive →
> clear-signing → agent propose → approve → activity → STOP → settings. Every flow was
> screenshotted as-built and graded against the north star (`DESIGN.md` "Your money on autopilot,
> and you can see and stop everything") and the versioned golden references
> (`designs/deckard-editorial-v3.html`, `designs/deckard-agent-v4.html`).
>
> Evidence: `docs/screenshots/audit-170/`. Method + caveats at the bottom (a few observations are
> **fork-conditioned** or **synthetic-driving artifacts**, called out explicitly so they don't get
> mistaken for product bugs).

## Headline

The bones are **strong and on-brand**. The editorial language is real in the running app — bundled
Schibsted Grotesk + JetBrains Mono, the oversized mono hero, hairline-ruled cockpit rows, the
two-signal actor axis (amber human / cyan agent), the `⌘↵` transaction-as-hero confirm, and the
see-and-stop Activity loop with a genuine NEEDS-YOU approval beat all render as designed. This is
**not** a rebuild-from-scratch situation.

The overhaul should be a **consistency + honesty + naming pass**, not a redesign. The friction
clusters in five places: (1) **identity/naming** — an unnamed wallet reads as "Personal / Wallet"
everywhere; (2) **two different clear-signing treatments** (transaction-as-hero for Send vs a
boxed key/value card for the agent approval) instead of the one shared review DESIGN promises;
(3) the **agent surface is a read-only policy dump** (no per-row editing, dimmed controls, no
"what Atlas did" feed) — the exact thing v2 said it removed from the home; (4) **money loses its
USD/verified context** below the hero (holdings show bare integers, no `$` column); and (5) a
**trust-relevant cap mismatch** — the agent advertises a "0.1 ETH per move" cap but auto-shielded
0.15 ETH.

Verdict per flow (● good · ◐ drifts from spec · ○ gap):

| Flow | Grade | One-line |
|---|---|---|
| Onboarding — Welcome | ● | Promise-led ("your new favorite wallet"), full-bleed, AGPL/no-telemetry footer |
| Onboarding — Secure | ● | Live strength meter (Strong/Fair), "no one can reset it" consequence, min-8 |
| Unlock | ● | Clean; amber focus ring; neutral primary |
| Wallet home (hero/alloc/actions) | ● | 64px mono hero, dimmed decimals, Shield-primary action row, honest private-balance line |
| Holdings ledger | ○ | Bare integer balances, **no `$` value column, no 24h** (golden ref has 4 cols) |
| Balance meta line | ◐ | **No USD-equiv / "synced · verified" line** under the hero (golden ref has it) |
| Compact agent presence | ● | `Atlas · acting · 0.15 / 0.5 today · 30%` gauge + chevron — exactly the spec |
| Command palette (⌘K) | ● | Search + wallet-context + all commands w/ shortcuts; brightness-lift selection |
| Send — compose | ◐ | Correct fields + status-strip downgrade banner; but centered, blank header mark |
| Send — clear-signing | ◐ | Excellent hero; but **both warnings red** (no amber caution tier), **no fee/From row** |
| Shield — compose | ● | Plain-language, 0zk auto-filled, gated CTA |
| Swap — compose | ◐ | CoW copy good; token pickers are a sprawling chip grid, bare `—`, blank marks |
| Receive | ● | QR + full EIP-55 addr + amber network caution + copy |
| Agent surface | ○ | Autonomy statement + limits + gauge + Revoke good, but **read-only** (see #3) |
| Activity — empty | ● | "All clear", amber idle STOP |
| Activity — feed + NEEDS YOU | ● | Best screen in the app: amber NEEDS-YOU band, cited reason, day-grouped log, glyphs |
| Agent approval — review | ◐ | Cites reason + no-blind-approve keys; but **boxed card, not the shared hero review** |
| STOP | ● | Idle amber → (arm-to-`⌘↵` documented; armed state not re-captured) |
| Settings | ◐ | Well-labelled; but bordered cards per section (editorial language is card-free) |

---

## Findings, ranked

### P1 — trust + identity (fix first; these undercut the north star)

**1. Wallet identity collapses to "Personal / Wallet" for an unnamed wallet.**
The breadcrumb reads `Personal › Wallet` (literal "Wallet"), and the page-header H1 is the *project*
name "Personal" with the address beneath — because `display_name` is blank (Settings → Profile).
DESIGN is explicit: the breadcrumb `current` "names the selected entity (the wallet/agent name,
**never the literal word 'Wallet'**)", and the golden ref shows `Personal › Main`. A machine spends
"Personal"'s money and the row that should say *which wallet* says nothing. Fix: derive a real
default wallet name (ENS → short address → "Wallet 1"), never the literal word, and never fall the
page title back to the project name. Evidence: `02-home-dark`, `07-send-clearsigning`.

**2. Two different clear-signing treatments break the "one shared review" contract.**
Send renders the transaction-as-hero review (tiny `SENDING` → 44px mono amount → full untruncated
address → red danger → armed `⌘↵`) — this is beautiful and correct (`07`). But the **agent-proposal
approval** renders a *different* screen: "Review request" with a **bordered key/value box**
(Amount / To / Breached limit) and no oversized hero (`14`). DESIGN says "the same review renders
for an agent's proposal awaiting approval" and clear-signing is "a statement, not a form… never a
gray box." The highest-trust moment in the product (approving a machine's spend) uses the *weaker*
of the two layouts. Fix: render the agent approval through the same transaction-as-hero engine, with
the cited limit as the danger/caution line.

**3. The agent surface is a read-only policy dump — the thing v2 said it deleted.**
The surface (`11`) has the good bones: cyan "Atlas · acting", a plain-language autonomy statement,
a LIMITS list, the budget gauge, and a red "Revoke & STOP". But: the limits have **no per-row Edit
affordances**; the `Pause / Rotate key / Adjust limits` controls render **dimmed/disabled**; there
is **no "What Atlas did" feed** (the golden `deckard-agent-v4.html` puts the agent's slice of
activity here); and Scope (allowed actions/assets, session-key expiry) is folded into one flat
LIMITS list rather than its own section. DESIGN's whole agent-model pivot was "a dedicated agent
surface **owns editable policy + controls + its own activity**… the old read-only policy dump on
the home is removed." Today the dump moved to the agent surface rather than becoming editable. This
is the single biggest gap between the built product and the design intent.

**4. The agent advertises a per-move cap it doesn't enforce on shields.**
The agent surface and autonomy statement say "Per-transaction cap **0.1 ETH**" / "acts on its own
under 0.1 ETH per move and asks you above that." But a **0.15 ETH** auto-shield **auto-approved and
broadcast** (agent log: "auto-approved within cap · broadcast ✓"); only a 0.8 ETH shield (over the
0.5 **daily** cap) triggered the approval beat. The demo `policy.demo.json` shield rule has no
`per_tx_cap_wei`, so shields are gated only by the daily cap — but the UI states a 0.1 per-move cap
that shields ignore. For a wallet whose entire pitch is "software-enforced limits you can trust,"
the displayed cap must match the enforced cap. Fix: either enforce a per-tx cap on shields or make
the surface show the *actual* gate per action ("Shields: capped by daily budget"). Evidence: agent
run log + `11` + `13`.

### P2 — money loses its context (portfolio reads unfinished)

**5. Holdings ledger has no USD value column and no 24h delta.**
Rows render `Ethereum / ETH …… 10,000` — a bare right-aligned integer, no `$` value, no 24h, no
dimmed decimals (`03`). The golden ref holdings row is 4 columns (asset · balance · 24h · $value,
Stripe-aligned). Test tokens show bare `2` / `8`. Partly fork-conditioned (no price feed on a
Sepolia fork), but the ledger *design* always carries a `$` column and DESIGN requires "every USD
figure carries `$`". At minimum the ledger needs the value column with an honest empty treatment
when price is unavailable, not a naked integer.

**6. The balance hero has no USD-equiv / "synced · verified" meta line.**
Under `10,000.001 ETH` the golden ref shows `$24,180.55 · synced 4s ago · verified on mainnet`.
The built home jumps straight to "Total" + the allocation bar with no meta line (`02`). The
per-chain honesty *is* present elsewhere (the status strip's `⚠ Demo fork · NOT VERIFIED` amber
banner is great — `06`/`07`), but the hero itself is missing the freshness + verified signal that
makes the number trustworthy at a glance. (USD is fork-conditioned; "synced Xs ago" is not.)

### P3 — consistency + polish

**7. Action composes are center-floated, not left-anchored.** Send/Shield/Swap/Receive center their
form in the pane (`06`,`08`,`09`,`10`) while the home is left-anchored full-bleed (`02`). DESIGN's
cockpit language is "left-anchored, full-bleed, hairline-ruled columns." Pick one; the editorial
direction says left-anchor.

**8. Blank-fill identity marks on action headers + swap token chips.** The Send/Shield/Swap page
headers show an empty gray rounded square where a glyph should be (`06`,`08`,`24`); swap token chips
(WETH/COW/USDC/USDT/GNO) also render blank-fill marks. DESIGN: `identity_mark` is "**never a blank
fill**". Give each a monogram/glyph or drop the mark.

**9. Send's first-time-recipient warning is red, not amber.** The Send review shows *two* red danger
lines — "public on Ethereum" **and** "Double-check the destination… funds are lost" (`07`). The
golden ref uses red for the irreversible-public danger and **amber** for the first-time-recipient
caution. DESIGN reserves red for irreversible/loss and amber for recoverable caution; collapsing
both to red flattens the danger/caution tier the whole trust model leans on.

**10. Send clear-signing omits the network fee + From.** The review shows amount + recipient +
dangers + confirm, but no "Network fee" and no "From" (`07`). The golden `deckard-agent-v4.html`
confirm demotes both below a hairline. A user approving a send can't see what gas will cost.

**11. Swap token selection is a sprawling inline chip grid.** Both sell and buy show all five tokens
as a chip row (`24`), versus the golden ref's compact asset-chip-with-chevron dropdown. It eats
vertical space and reads less premium. Also the "receive at least" shows a bare `—` placeholder
(DESIGN: "loading is a skeleton, not a bare —").

**12. Settings uses a bordered card per section.** APPEARANCE / PRIVACY / NETWORK / PROFILE each sit
in a bordered box (`15`). DESIGN allows Settings to be "more spacious" but the editorial language is
card-free (whitespace + hairlines + section labels). Minor; flagged for consistency.

**13. Agent status reads "acting" at rest.** On the home presence row, the sidebar, and the agent
surface, Atlas shows cyan "acting" even when the loop is idle and nothing is in flight (`03`,`11`).
The acting-pulse should mean *currently working*; a resting agent should read "idle"/"watching".

**14. Every action ⌘K-reachable, but the agent surface is not.** The palette (`05`) has all 16
commands, but there is **no command to open an agent** (`portfolio/send/receive/shield/swap/settings/
copy/theme/mask/lock/approvals/activity/approve-selected/deny-selected/revoke-all/refresh`). The
agent surface is reachable only by clicking the sidebar row. CLAUDE.md: "every user-facing action
must be reachable from ⌘K." Add an "Open agent / Atlas" command.

**15. Receive copy has no "Copied ✓" feedback.** The Receive "Copy address" writes to clipboard
silently (`10`); DESIGN calls for inline "Copied ✓". Minor.

---

## Fork-conditioned observations (NOT product bugs — don't fix blindly)

- No USD figures anywhere (hero, holdings, review) — the Sepolia fork has no price feed. The *design
  gap* is the missing `$` column/row treatment; the empty values themselves are expected on a fork.
- Test tokens "USD Coin (Sepolia test) / GNO" with integer balances — inherited fork state.
- `NOT VERIFIED` / "Demo fork: not mainnet" everywhere — correct, honest behavior (verified reads
  are mainnet-only). This is a feature, and it renders well.

## Driving artifacts (measurement noise — explicitly excluded from findings)

These are limitations of synthetic macOS input (cliclick / System Events) against GPUI, **not**
product defects — a human with real keyboard/mouse focus does not hit them:
- The Send/Swap `⌘↵` and mouse-click confirm did not broadcast under synthetic input; real
  broadcasts were driven through the headless agent path instead (which is why the Activity feed and
  balances are real). The review *screens* render correctly.
- The ⌘K palette query, Activity `j/k/x/Enter/Esc`, and onboarding `Continue` don't receive
  synthetic key events (hand-rolled `on_key_down` handlers + focus quirks); registered-action
  shortcuts like `⌘,` (Settings) and `⌘⇧D` (theme) *do* work. Palette *navigation* and the
  onboarding Back-up/Verify/Ready sub-steps were therefore not captured live; their structure is
  known from the code + DESIGN.

## Method

macOS, live app on `just demo` (RPC via a free Sepolia archive). Window driven with a throwaway
harness (`.context/drive.sh`: Swift `CGWindowListCopyWindowInfo` finder + `cliclick` +
`screencapture -l<windowID>` to dodge occlusion). Onboarding captured from a second app instance on
a fresh vault-less config dir. Full 40-shot capture set in `.context/shots/`; the 17 curated in
`docs/screenshots/audit-170/` are the evidence cited above.

## Recommended direction for #170

Frame the overhaul as **"make the built product match its own design system, and make the agent
surface earn its name"** — a consistency/trust pass, not a redesign. Suggested epic shape (draft;
confirm before filing):

1. **Identity & naming** (P1 #1) — default wallet name, breadcrumb never says "Wallet", page title
   never falls back to the project.
2. **One shared clear-signing engine** (P1 #2) — route the agent approval through the
   transaction-as-hero review; kill the boxed variant.
3. **Editable agent surface** (P1 #3) — per-row Edit, wire Pause/Rotate/Adjust, add the "What Atlas
   did" feed, split Scope from Limits.
4. **Cap honesty** (P1 #4) — displayed cap == enforced cap, per action.
5. **Money keeps its context** (P2 #5/#6) — `$` column + hero meta line with honest fork/price
   fallbacks.
6. **Consistency sweep** (P3 #7–#15) — left-anchor composes, kill blank marks, restore the
   danger/caution color tier, fee row on Send, swap picker, ⌘K agent command, copy feedback.

Do #1–#4 first; they are where the "autopilot you can trust and stop" promise currently leaks.
