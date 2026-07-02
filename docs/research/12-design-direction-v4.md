# Design direction v4 — the request-origin wallet (feeds #170)

> Output of a `/design-consultation` run that followed the grounded flow audit
> (`docs/research/11-ia-flows-audit.md`). Method: real-reference competitive research (browsed
> Linear, Raycast, Superhuman, Stripe, Rabby, Rainbow) + two independent outside-voice design
> directions (Codex GPT-5 + a Claude designer) + synthesis, then requirements locked with the user
> across three clarification rounds. The visual system (DESIGN.md v3) is **kept**; this is an
> IA / flow / interaction pass. Pixel ground-truth: `designs/deckard-v4.html`.

## The one line

Deckard is a **desktop wallet where every request to move your money — from you, from a dapp, or
from an agent — lands in one honest review, and you keep your hand on a brake.** The category's core
verb is *sign*; Deckard's is *see who's asking, and approve or stop*.

The audit found the bones strong and on-brand; all three design voices independently agreed the
energy belongs on the same thing: the **request → review → approve/stop** loop, generalized across
*who is asking*. The agent is one requester, not the whole product.

## Competitive research (grounded in real reference captures)

- **Linear** (`linear.app`, captured 2026-07-02) — now literally "the product development system for
  teams **and agents**." Load-bearing patterns: activity attributed by actor ("created the issue **on
  behalf of** karri"), a live **watch-the-agent-work** panel, and a **three-pane** nav · content ·
  **right metadata rail**. Its select→act model (hotkey / command menu / right-click all dispatch the
  same action) is the blueprint for Deckard's dual-modality.
- **Raycast** (`raycast.com`) — "your shortcut to everything." The **⌘K launcher + contextual Action
  Panel** (grouped actions, a shortcut shown per row) is the reference for Deckard's palette and its
  per-object action model.
- **Superhuman** — command palette that **teaches its own shortcuts** by showing them every use;
  **Split Inbox** (auto-categorized focused streams so the actionable is never buried). The "Waiting
  on you" home reflex is this mechanic.
- **Stripe** — mono for IDs/amounts + humanist sans for chrome; object-detail with an event timeline.
  Validates the mono-for-money discipline and the left-aligned-content / right-aligned-numbers grid.
- **Rabby** — its **product** is the domain gold: **simulate-before-sign**, showing the balance-change
  diff and risk/allowance flags on one canonical pre-sign screen whether the request came from a dapp
  or from you. Deckard's shared Review adopts this. Its **marketing** (purple hero, blob mascot,
  centered) is the generic-crypto slop Deckard rightly defines against — validates the editorial,
  card-free, two-signal direction.

**First-principles read (where we break from the category):** every other wallet treats "review this
transaction" as a per-transaction modal interrupt from a single source. Deckard's reality is *many
request origins over time* (you, dapps, agents, later plugins/session-keys) against *one policy and
one key*. So the differentiator is not a prettier sign-modal — it's a **unified request model**: one
review surface, one attributed feed, one Rules vocabulary, spanning every origin. This is already how
the architecture is built (`docs/agent-authorization-map.md`: one approval path, one Rules vocabulary,
three principals) — the UI should make it visible.

## Two outside voices (kept in full for the record)

Both landed on the same core independently — strong convergence.

- **Codex ("trading-desk terminal"):** keep the fonts + the two-signal axis; split the palette into
  disjoint **actor** vs **state** color registers and lock danger/warning/success/ring as
  non-skinnable; **one shared Review** (source changes the header rail, never the trust surface); a
  **"Waiting on you"** queue as the home reflex with a designed empty state; the agent surface is a
  **feed, not a policy dump**; the **cap is a live enforced ledger** and the UI must *never display an
  enforcement claim the engine doesn't back* (struck-through "unenforced" tag until fixed); a
  dual-modality model where **selection / verb / STOP are the same command id** no matter how you
  reach them. Two departures: no cards ever; trust-loudly-revoke-fast.
- **Claude ("The Watchfloor"):** home foregrounds live supervision; a drivable "scrub the future"
  time-scrubber on the confirm; the autonomy **sentence** as the policy editor; warm/cold made
  temporal; a mono-money odometer. (User rejected the scrubber, the temperature shift, and the
  odometer — see decisions.)

Full transcripts of both are preserved in the session task outputs.

## Locked v4 decisions (from the clarification rounds)

**Scope — go deep on:** the everyday desktop wallet + the one shared Review + the origin-attributed
"Waiting on you" / Activity surface. **Light on agent internals** (the agent↔wallet interaction is
MCP-only today with no in-wallet model; needs more thought later — documented as an expandable slot,
not redesigned now).

1. **Shell → three-pane.** Sidebar · main · a **collapsible right metadata rail** showing contextual
   detail for the focused object (wallet → holdings/status; a pending request → its clear-signing
   detail; an activity row → its receipt). This departs from v3's "no third inspector pane" — adopted
   deliberately (the Linear model). Collapsible so casual use stays two-pane.
2. **The request-origin model is the spine.** Requests come from **you**, **dapps** (browser bridge
   today, plugins later), and **agents** (MCP). One shared Review renders for all; the origin is a
   **header rail + identity**, never a third signal color. Amber = human, cyan = agent stay sacred; a
   **dapp/external origin is a neutral identity** (favicon + domain) **+ a trust badge that borrows the
   state colors** (verified = success, first-seen = amber caution, flagged = danger).
3. **Requester handles, not a persona.** "Atlas" was a placeholder and is retired. Non-human sessions
   get an **auto-assigned, human-renamable handle** (rotating codename/city list, e.g. Kyoto) shown
   alongside the **underlying origin** (MCP client id / dapp domain). Distinguishable sessions, no
   invented mascot.
4. **One shared clear-signing Review** — the **static, Rabby-style balance-diff** (you pay / you
   receive, ± per asset). *(The drivable scrubber was rejected as bad UX.)* Plus recipient
   known/whitelist/**unknown** badge, the **authorizing rule + remaining cap after this move**, and the
   verified-read meta. Danger-first red, then amber caution, then quiet facts. `⌘↵` arm-delay confirm.
5. **Approval is policy / per-origin, not one global posture.** In-policy moves proceed per the policy
   the user set; **"approve every move" is itself a valid policy**; the browser extension can carry
   per-domain settings. Full autopilot is *not* a smart default (on-chain moves are irreversible; STOP
   halts future action, it can't claw back). Editing approval models is a later feature.
6. **Everyday-wallet excellence.** Identity **masthead** (real wallet name + deterministic mark; the
   literal word "Wallet" never appears anywhere as a label); holdings with a **mono `$` value column +
   24h**; a **hero USD + synced/verified meta line** (honest fork/price fallbacks); **left-aligned**
   action composes.
7. **Watchfloor-when-active home.** Wallet-forward by default; the "Waiting on you" region expands
   above the portfolio when a request/agent is live, origin-attributed with countdowns. Empty state:
   one quiet line ("Nothing waiting · your agents are within policy").
8. **Honest cap ledger.** Shown as `$X of $Y remaining` and as cap-after on the Review; enforced on
   **every** value path including shields; if the engine doesn't enforce it, render it struck-through
   with an **unenforced** danger tag rather than fake it. (Fixes audit gap #4.)
9. **Dual-modality, one command id.** Every object is one focused selection (locked, always-visible
   ring); `j/k` in lists, `h/l` across panes; ⌘K teleport + a contextual action panel that *is* the
   right-click menu *is* the hover cluster; STOP persistent, global-hotkey-even-inside-modals, top ⌘K
   entry. Invariant: every verb is one command id all surfaces dispatch (QA-assertable both ways).

**Kept exactly as v3:** Schibsted Grotesk + JetBrains Mono; ~95% grayscale on near-black; the
amber/cyan two-signal actor axis; editorial card-free composition; ⌘K; the `⌘↵` arm-delay confirm;
current multi-chain UX (network pill + status-strip honesty banner); and v3 motion (the acting-pulse
only — **no** temperature-shift, **no** odometer/number-roll, **no** scrubber).

**Deferred / documented slots (don't build deep now):** agent internals + an in-wallet agent
interaction model; deep dapp Connections editing (ADR-0001 / epic #44 — the sidebar reserves a
`CONNECTIONS` group and the origin-approval model is represented, but editing is later); session-key
principals (ADR-0002); multi-chain portfolio expansion.

## Maps to the audit's P1 gaps (#174)

The four load-bearing gaps this direction closes: **#1 identity** → the named masthead (§6); **#2 two
clear-signing treatments** → the one shared Review across origins (§4); **#3 read-only agent surface**
→ reframed as origin-attributed feeds + editable policy (documented, kept light this iteration per
scope); **#4 unenforced cap** → the honest cap ledger (§8).

## Deliverables

- `designs/deckard-v4.html` — the new pixel ground-truth (five views: wallet-forward home,
  watchfloor-when-active home, the shared Review for a dapp origin, Waiting-on-you + Activity, and the
  three-pane rail expanded), built on the exact v3 system.
- DESIGN.md → v4 edits (three-pane + rail, the request-origin model, the generalized shared Review,
  identity rules, money-context, requester handles, honest cap, decisions log) — applied after sign-off
  on the golden ref.
