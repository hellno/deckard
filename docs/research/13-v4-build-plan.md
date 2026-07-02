# v4 build plan — the request-origin wallet (implementation backlog for #170)

> The executable, agent-proof breakdown for building the codified v4 design in the GPUI app. Sources
> of truth: `DESIGN.md` v4, `designs/deckard-v4.html` (pixel ground-truth, five views), the audit
> (`docs/research/11-ia-flows-audit.md`, gaps #170/#174), the direction (`docs/research/12-...`).
> **DRAFT backlog for review — no GitHub issues filed yet.** On the go-ahead: an epic + 8 children
> under #170, each linking the Implementer Contract below.

---

## Implementer Contract (every child links here — read before writing code)

**Reuse, never hand-roll** (convention; `/code-review` + the per-view fidelity checklist enforce it).
Compose every view from `widgets.rs`. If a primitive is missing, add it to `widgets.rs` (see E1) —
do NOT inline a one-off copy. Existing: `short_addr`, `caution_line`, `error_line`, `section_label`,
`divider`, `identity_mark`, `budget_gauge`, `truncated_address` (+ `money.rs`). v4-added (E1):
`key_cap`, `origin_header`, `action_tag`, the `meta_rail`, `balance_diff`. Colors via `theme.*` +
`amber`/`agent` only (no raw hex); sizes/spacing/radii/motion via `tokens` (no magic `px()`); money +
addresses via `money.rs` (mono, tabular, dimmed decimals).

**Never regress (trust invariants — a PR that touches these must prove they still hold):**
- Clear-signing renders before every value move; the `⌘↵` arm-delay confirm (never hold-to-confirm).
- **No blind approve:** an approval resolves ONLY the still-pending record the human actually reviewed.
- **STOP** is always reachable, zeroizes the key, and denies in-flight work.
- **Cap enforcement is real:** the UI never shows an enforcement claim the engine doesn't back.

**Definition of Done — ALL issues:**
1. `cargo fmt --all --check` clean · `just check` green (clippy `-D warnings`, both feature configs) ·
   `cargo test --workspace` green.
2. No new/changed deps (`Cargo.toml`/`Cargo.lock`) without explicit approval.
3. Every new user-facing action has a ⌘K `Command` (`palette_commands.rs` + handled in
   `run_palette_command`).
4. GUI issues: annotated **before/after screenshots** off the funded `just demo` (use E0's recipe),
   AND the per-view **fidelity checklist** (below) checked off in the PR.
5. A short **"do not touch"** honored (per child).

**Definition of Done — trust-critical (E5 only), in addition:**
6. Regression tests for the two invariants at stake: **cap-enforced-on-shields** (a shield over the
   stated per-move cap ASKS, never auto-broadcasts) and **no-blind-approve** (approve resolves only
   the still-pending reviewed record). Paste the passing test output.
7. A **codex adversarial review** (GPT-5 xhigh, cross-model) of the FIX diff, looped until 0 findings
   (per `docs/AGENTIC-ENGINEERING.md` + the "verify the fixes, not just features" practice).

**Golden-ref match rule:** the target is the matching view in `designs/deckard-v4.html` (open it,
switch views via the top bar). Fidelity = the per-view checklist passes AND the before/after
screenshot reads like that view in layout + hierarchy. The HTML is web; the app is GPUI — there is no
pixel diff, so the checklist is the gate.

---

## Per-view fidelity checklists (the golden-ref gate)

Each references a `designs/deckard-v4.html` view. Check all in the PR; anything unchecked blocks merge.

**Wallet home** (`data-s="home"`)
- [ ] Sidebar top group is `WALLETS` (no `PROJECTS`); Agents show a handle + status; Connections show favicon + domain.
- [ ] Breadcrumb is the entity name only (e.g. `Meridian`) — no `Personal ›`, no literal "Wallet".
- [ ] Identity masthead: name + `identity_mark` above the mono hero.
- [ ] Hero mono, dimmed decimals; meta line `$… · synced … · verified on mainnet` (honest fallback off mainnet / no price).
- [ ] Actions left-anchored (Shield primary; Send/Receive/Swap ghost).
- [ ] Waiting strip is ONE line (`N waiting for you · Review →` amber, else `Nothing waiting for you.`) — not a stacked band.
- [ ] Holdings: Asset · Balance · 24h · **$ Value**, mono, decimals **aligned on the point** across rows.
- [ ] Right rail "This wallet": sync/verified/network + cap ledger + Connections/Agents kv rows.

**Shared Review** (`data-s="review"`)
- [ ] ONE transaction-as-hero surface; origin header rail (`You are sending` / `<handle> proposes` / `<domain> requests`) = identity + a state-color trust badge, never a third signal color.
- [ ] Amount stated once (hero + one USD line); no duplicate "you pay / you receive" for a simple swap.
- [ ] `TO` = identicon + full address + known/unknown badge.
- [ ] One danger line `This can't be undone.`; no speculative site-trust; no arm-delay prose explainer.
- [ ] Quiet facts once: From · Route · Network fee · **Allowed by: `<rule>` · $X of $Y daily left after this**.
- [ ] Confirm via the `key_cap` `⌘↵` (platform-aware, armed amber) + Edit link.

**Activity** (`data-s="activity"`)
- [ ] Plain `ACTIVITY` header (no hero title, no explainer subtitle).
- [ ] `Stop all agents` emergency control, shown only when an agent is active.
- [ ] `NEEDS YOU` + a count badge; day-grouped log.
- [ ] Rows scannable: `[origin] [ACTION tag] [mono amount] [→ dest] · [warning tag] · [hash link] [glyph] [time]` — no "wants to / proposes to" prose.
- [ ] Attributed across you / dapp / agent.

**Transaction** (`data-s="tx"`)
- [ ] Header origin + verb + hash; green `Confirmed` chip; read-only hero + USD.
- [ ] Rows: Status · Hash (copy + explorer link) · From · To · Amount · gas · Block · Time · **Authorized by** (rule/origin).
- [ ] Rail mirrors status/links; explorer link + copy work.

**Rail** (`data-s="rail"`)
- [ ] Right rail always present (not collapsible), ~300px, hairline-left; content contextual to focus.
- [ ] No "Nothing selected." while a row is selected.

---

## Verified current state (grounded 2026-07-02)
- Routing: `shell.rs` `enum Surface` (line 86); layout in `shell.rs` (~3149-3239) + `shell_chrome.rs`.
- Literal "Wallet": `shell_chrome.rs:73` (`Selection::Wallet => "Wallet"`), `palette.rs:205`. Breadcrumb `Personal ›` at `shell_chrome.rs:284-287`.
- `widgets.rs` has 8 primitives (above); the v4 five do not exist.
- Shared review = `commit_view.rs:263`; the divergent boxed agent card = `activity_view.rs:830`.
- Cap bug: `policy.demo.json` shield rule has no `per_tx_cap_wei`; a 0.15 shield auto-broadcast under a stated 0.1 cap. Enforcement: `deckard-core` `evaluate` + the signerd shield path.

## Dependency graph + sequencing
```
E0 driving ·(prereq for every GUI issue's evidence)
E1 widgets ─┬─> E2 identity
            ├─> E4 money
            ├─> E3 rail ─┬─> E6 activity/waiting
            │            └─> E7 transaction
            └─> E5 🔴 shared review + enforced cap ──> E7
   E2..E7 ─────────────────────────────────────────> E8 ⌘K
```
E0 first (so evidence is possible). E1 next (all reuse it). E2/E4 small early wins. E3 structural
(E6/E7 rail content needs it). **E5 is the trust-critical center** (enforcement + the review that
displays it, merged, one adversarial review). E8 wires ⌘K last.

---

## Epic: Build the v4 request-origin IA
One wallet, one shared Review, N request origins (you / dapp / agent). Visual system unchanged;
IA/flows rebuilt. Each child ~1-3 days, independently shippable, links the Implementer Contract.

### E0 — App-driving + screenshot recipe (prerequisite)
Commit a documented, reusable way to drive the running app and capture per-window screenshots, so
every GUI issue can produce before/after evidence.
- macOS: `just demo` (funded fork) or `just run`; a helper for window-scoped capture
  (`screencapture -l<CGWindowID>`, id via a Swift `CGWindowListCopyWindowInfo` snippet) + `cliclick`
  for clicks/typing; unlock via click (synthetic Enter won't submit).
- **Honest caveats (save agents hours):** synthetic input does NOT reliably fire the `⌘↵`/click
  confirm, the ⌘K query, or Activity `j/k/x/Esc` (GPUI hand-rolled `on_key_down` + focus quirks);
  registered-action shortcuts (`⌘,`, `⌘⇧D`) DO. So: capture compose/review SCREENS via clicks; drive
  real broadcasts through the headless agent (`just demo-agent` / `just demo-deposit`); verify
  LOGIC via tests, not screenshots. Link `docs/dev/headless-gui-screenshots.md` for Linux/CI.
**Do not touch:** app code. **AC:** a committed `docs/dev/*` recipe + a helper script (not gitignored);
another agent can follow it to unlock the demo and capture a labelled screenshot.
**Files:** `docs/dev/`, `scripts/` (or `just` targets).

### E1 — v4 widget foundation (`widgets.rs`)
Add `key_cap(keys, armed)` (platform-aware `⌘`/`Ctrl` via `std::env::consts::OS`, chord as one cap),
`origin_header(origin)`, `action_tag(kind)`, the `meta_rail` scaffold, `balance_diff`. Audit + land
any DESIGN-referenced primitive not yet built (`page_header`, `kv_row`, `status_glyph`, `stop_brake`).
**Do not touch:** view files (that's E2-E7). **AC:** primitives compile, unit-tested where pure
(assert `key_cap` yields `Ctrl` on a forced-Linux path, `⌘` on macOS); each used by ≥1 view; `just
check` green. **Files:** `widgets.rs`, `tokens.rs`.

### E2 — Identity & naming (fixes P1 #1)
Kill the literal "Wallet" (`shell_chrome.rs:73`, `palette.rs:205`); breadcrumb names the entity, drop
`Personal ›` (`shell_chrome.rs:284-287`); wallet-home masthead; deterministic default wallet name;
auto-assigned renamable agent handles (retire "Atlas").
**Do not touch:** the review/activity/tx flows. **AC:** reflective test asserts "Wallet" appears
nowhere as a UI label; breadcrumb + masthead render; handles generated + renamable; **home checklist**
passes; screenshots. **Files:** `shell_chrome.rs`, `palette.rs`, wallet-home view, a handle generator,
`widgets.rs`.

### E3 — Three-pane shell + always-on right rail
Add the always-on ~300px right `meta_rail` (contextual to the focused `Surface`); drop the Projects
layer; sidebar groups → Wallets · Agents · Connections (Connections = reserved slot, list only).
**Do not touch:** deep Connections editing (deferred #44). **AC:** three-pane renders at app width, no
overflow, rows clamp; rail updates on selection; **rail checklist** passes; screenshots (home/activity/tx).
**Files:** `shell.rs` (layout), `shell_chrome.rs`, `meta_rail` (E1).

### E4 — Money keeps its context (fixes P2)
Holdings `$` value column + 24h, decimal-point aligned; hero USD/synced/verified meta line with honest
fallbacks (explicit "unverified"/"—" off mainnet or with no price, never a fake number); left-anchor
the action composes.
**Do not touch:** multi-chain portfolio (deferred). **AC:** holdings `$`+24h decimals aligned; hero
meta honest on the fork; composes left-anchored; **home checklist** passes; screenshots mainnet AND fork.
**Files:** wallet-home + compose views, `money.rs`.

### E5 — 🔴 The ONE shared Review + honest enforced cap (fixes P1 #2 + #4)  ·  TRUST-CRITICAL
Merged because the review's "Allowed by: cap after this move" is only truthful if the engine enforces
the cap. Ships + gets adversarially reviewed together.
- **Engine:** enforce the per-tx cap on **every** value path including shields (`deckard-core`
  `evaluate` + signerd shield path; `policy.demo.json` gains a shield cap). If a path is genuinely
  unenforced, the UI renders it struck-through with an "unenforced" danger tag.
- **UI:** route the agent-approval (`activity_view.rs:830`, delete the boxed card) AND the dapp path
  through the ONE transaction-as-hero review (`commit_view.rs:263`); add the `origin_header` rail and
  the **Allowed by** authority line (rule + cap-after from `evaluate`); danger copy `This can't be
  undone.`; drop the speculative site-trust + the arm-delay prose.
**Do not touch:** agent internals beyond routing the proposal into the shared review (deferred).
**AC (Contract §6/§7 apply):** one review renders for send/swap/shield/agent/dapp, header-rail-only
difference; **cap-enforced-on-shields** + **no-blind-approve** regression tests pass (paste output);
codex adversarial review looped to 0 findings; **review checklist** passes; screenshots of all origins.
**Files:** `commit_view.rs`, `activity_view.rs:830`, `shell.rs` review handlers (~1732/1944/2251),
`deckard-core` `evaluate` + policy schema, signerd shield path, `policy.demo.json`.

### E6 — Origin-attributed Activity + "Waiting on you"
Scannable tag rows (identity + `action_tag` + mono amount + warning tag + hash/glyph/time); `NEEDS
YOU` + count badge; day-grouped log; origin attribution; the compact home waiting strip; STOP → "Stop
all agents" (only when an agent is active).
**Do not touch:** the no-blind-approve/STOP-zeroize logic (keep intact). **AC:** rows scannable (tags,
not prose); **activity checklist** passes; STOP reframed; screenshots empty + populated (driven by
`just demo-agent`). **Files:** `activity_view.rs` (302, 452, 495), wallet-home strip, `shell.rs`.

### E7 — Transaction detail view (new)
New `Surface` variant + `transaction_view.rs`: read-only receipt (reuse the E5 review structure) +
Status/Hash(copy+explorer)/From/To/Amount/gas/Block/Time/**Authorized by**; rail mirrors it; reachable
by clicking a tx AND via ⌘K.
**Do not touch:** the shared review component's shape (reuse read-only, don't fork it). **AC:** click a
tx → detail; explorer + copy work; Authorized-by shows rule/origin; **transaction checklist** passes;
screenshots. **Files:** `shell.rs` (`Surface` + routing + `run_palette_command`), new
`transaction_view.rs`, `palette_commands.rs`, `activity_view.rs`.

### E8 — ⌘K coverage + agent command
Add the missing commands (open agent — none today; open Connections; open a transaction); enforce
every v4 verb is one command id all surfaces dispatch.
**Do not touch:** existing command ids/shortcuts (add, don't rename). **AC:** every v4
destination/action ⌘K-reachable; a reflective test asserts registry ↔ handler coverage; palette
screenshots. **Files:** `palette_commands.rs`, `shell.rs` `run_palette_command`.

## Out of scope (documented slots — deferred)
Agent internals / in-wallet agent-interaction model; deep dapp Connections editing (ADR-0001 / #44);
session keys (ADR-0002 / #33); multi-chain portfolio expansion.

## Effort (rough, CC-assisted)
E0 ~0.5d · E1 ~1d · E2 ~1d · E3 ~2d · E4 ~1d · **E5 ~3-4d (engine + UI + tests + adversarial review)** ·
E6 ~2d · E7 ~1-2d · E8 ~1d.
