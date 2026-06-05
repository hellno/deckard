# Deckard — Autopilot (v1 Plan, draft v0.1)

> Working name: **Deckard** (umbrella product) · v1 face: **Autopilot**
> Status: pre-build plan, written for CEO/eng/design review.
> Date: 2026-06-04

---

## 0. One-liner

**Vision (the flag):** a native, self-custodial money autopilot. You set policies only you
can change; a local AI proposes actions and an on-chain policy engine enforces them, so your
finances run themselves and no bank, company, or state can freeze them.

**The soul is the autonomy (decided 2026-06-04 after an outside-voice challenge).** "Money that
runs itself" is the reason to exist; an advisory-only version is just a wallet with suggestions.
So the project commits to hands-off autonomy as the destination and is **resourced for it**: a
security audit as a hard gate before mainnet autonomy, a security-minded co-maintainer, the
audit funded as an explicit grant line item, and a timeline longer than a solo sprint.
**Advisory-first ships first** as the trust-building, demo-able, grant-attracting milestone on
the committed path, not as the ceiling. This is a funded, audited, multi-contributor public
good, not a strictly-solo project.

Tagline candidates: *"Your money, on your rules, that no one can quietly switch off."* /
*"Set the rules once. It runs itself. You hold the keys."*

---

## 1. Thesis — why it exists, why now

**The flag:** Ethereum as *sanctuary technology* (EF Mandate, Mar 2026; Vitalik's
"sanctuary technology" note, May 2026). CROPS = Censorship/capture-resistance, Open
source, Privacy, Security. The EF is shrinking, narrowing, and explicitly stepping back
from the application layer — they want the ecosystem to build the humane apps. This is
an invitation.

**The differentiation (vs Proton, the incumbent "privacy daily driver"):** Proton now
ships a full suite + Lumo AI, but it's *jurisdictional* privacy — trust EU law + trust the
Proton company. Proton is a single point of capture (acquirable, compellable, breachable).
Deckard's whole identity is the one thing Proton structurally can't be: **trustless** —
your keys, your device, credibly-neutral rails, no company in the loop. *"Proton asks you
to trust a company. Deckard asks you to trust no one — just math and your keys."*

**The load-bearing role of AI (kills the "AI slop" objection):** People don't self-custody
because of *terror and tedium*, not tech. The local AI is the **trust/usability layer that
finally makes self-custody safe and effortless.** Crypto is load-bearing: the thing being
made usable *is* self-custody. Remove crypto → no product. Remove AI → self-custody stays
terrifying. The AI is the mechanism, not the pitch. This is a **security product**, not an
"AI app."

**Why now (three curves crossing):**
- EU **Technological Sovereignty Package** adopted 2026-06-03, "open-source first," aiming
  to cut >80% non-EU dependence. Political + funding tailwind, this week.
- CROPS gives Ethereum the credibly-neutral substrate + targeted EF/ESP funding.
- Local AI is finally good enough to run on-device (Apple Silicon / llama.cpp / MLX).
- Account abstraction (ERC-4337 / EIP-7702 + session keys) makes **policy-bounded
  autonomy** real for the first time.

---

## 2. v1 wedge — ruthless scope

**The discipline:** "sovereign daily driver" is a decade. Ship ONE organ that stands alone
and implies the whole body. v1 = the **Autopilot** organ only.

**Primary user (Milestone 1):** onchain operators and crypto-native earners — people paid in
crypto who live onchain and refuse custodians (DAO contributors, crypto founders, onchain
freelancers). Recurring pain: manually juggling runway, conversions, savings, and recurring
payments across wallets and chains, plus exposure to CEX freezes / debanking. Reached via the
founder's Farcaster + build-in-public megaphone; their respect is the founder's impact goal.

**IN (v1, advisory-first, 4–8 months):**
- Native **sovereign control plane** app (macOS-first, see open questions).
- A **smart account** per user (ERC-4337 or EIP-7702).
- **Policy engine as a proposal generator** — a small, high-value policy set. The engine
  watches state and *proposes* the move; **the user approves with one tap.** No hands-off
  execution in v1. Starting policies:
  1. **Runway floor** — keep N months of expenses in a chosen sovereign asset (see §4.2); flag excess.
  2. **Auto-pay** — surface recurring payments (rent, subs) for one-tap approval on schedule.
  3. **Custody guardrails** — alert when > X sits on flagged custodial addresses; propose a withdraw.
  4. **Rebalance** — propose trades to hold a target allocation within bands.
- **Local AI** — proposes/explains each move, drafts policies from natural language, narrates
  what it recommends and why. Runs on-device; no cloud sees your data.
- **Radical transparency UI** — plain-language policy authoring + a "here's what I recommend,
  here's why, approve / decline" log. Trust is earned through visibility before autonomy.

**THE DESTINATION (the soul, gated by an audit):**
- **Bounded hands-off autonomy** via narrowly-scoped, revocable session keys delegated to a
  keeper, so approved policy classes execute while you sleep. This is the reason the product
  exists. It ships *after* advisory-first has earned trust AND a third-party security audit is
  done AND a security collaborator is on board. Advisory-first is Milestone 1; this is the
  point of the whole thing.

**OUT (later organs / later versions):**
- Messaging, social, the Vault/Companion/Passport organs, mobile, fiat on/off-ramp,
  complex DeFi strategies, multi-user. All explicitly deferred.

---

## 3. Trust architecture — the actual innovation

- **AI proposes; deterministic policy enforces.** The AI literally *cannot* move funds
  outside policy bounds — enforcement is cryptographic (smart-account validation + session-key
  scopes), not "the AI promised." This is the whole game.
- **Root key never leaves the Secure Enclave.** Session keys are narrowly scoped (allowed
  targets, amounts, rates, time windows) and instantly revocable by the root key.
- **Local + open-source + auditable.** No server custodies anything.
- **"Unfreezable" = rules live in the smart account, not a company.** Execution can be done by
  any keeper (self-run, opportunistic, or a decentralized keeper network) — no single keeper is
  load-bearing.
- This is the most security-critical *and* most fundable part (security + open source + privacy
  = CROPS bullseye).

---

## 4. Hard problems & de-risking

1. **Autonomous funds safety (the #1 risk) — managed by sequencing + hard gates, not avoided.**
   Because autonomy is the committed destination (decision 2026-06-04), the safety work is
   non-negotiable, not optional: advisory-first ships first to keep early blast radius near
   zero; mainnet autonomy is gated behind a third-party security audit AND a security-minded
   co-maintainer. Plus conservative defaults, simulate-before-execute, per-policy + global spend
   caps, kill switch, testnet + tiny mainnet caps. The AI is *never* the enforcement boundary.
   The audit is a funded grant line item, not a someday.
2. **The "unfreezable" / stablecoin contradiction — RESOLVED (2026-06-04): tiered, sovereign by
   default.** Deckard defaults to assets that cannot be unilaterally frozen or blacklisted
   (decentralized / over-collateralized stables, ETH / LSTs). A selected centralized stable
   (e.g. USDC) is available only as an explicit, clearly-labeled opt-in the user swaps into
   themselves, never a silent default. The "unfreezable" promise holds for the default tier; the
   convenience tier is the user's informed choice. Shapes the runway-floor policy and the asset
   picker UI.
3. **Keeper liveness / censorship.** Multiple keepers; manual fallback; path to decentralized keeper.
4. **Local AI reliability.** AI only proposes/explains; determinism lives in the policy engine.
5. **Key loss / recovery.** Sovereign social recovery on the smart account (not custodial).
6. **Regulatory surface (autonomous money movement).** Non-custodial, user-controlled; get a
   read; EU-sovereignty framing helps.

---

## 5. Tech stack (proposed — for eng review)

- **Native:** macOS, Swift/SwiftUI; Secure Enclave for the root key. (Desktop-first because
  Autopilot wants an always-available control plane + enclave; mobile companion later.)
- **Account abstraction:** EIP-7702 / ERC-4337 smart accounts; session-key permissions
  (ERC-7715 / 7710-style) for bounded delegation.
- **Chain:** an Ethereum L2 with low fees + strong neutrality story; choose assets per §4.2.
- **Policy engine:** deterministic local module + an on-chain validation module on the smart account.
- **Local AI:** small on-device model (llama.cpp / MLX on Apple Silicon) for proposal,
  explanation, NL→policy drafting.
- **License:** copyleft (commons-funding narrative) vs permissive (adoption) — decide (§9).

---

## 6. Funding plan (dual rail, sequenced)

Same product, two loglines:
- **Crypto/CROPS rail:** "credibly-neutral, self-custodial, CROPS-pure security primitive."
- **EU sovereign-OSS rail:** "open-source software that reduces dependence on US Big Tech and
  custodial US finance."

- **Phase 0 (build MVP, testnet):** self-funded + apply to **NLnet/NGI Zero** (EU OSS,
  rolling — near-perfect fit) and **Gitcoin** (privacy/security domain).
- **Phase 1 (working demo + audience):** **EF ESP** (security + self-custody UX + open source),
  **Octant**, account-abstraction / L2 ecosystem grants.
- **Phase 2 (traction):** retroactive PGF, **Sovereign Tech Agency** (Germany), EU Tech
  Sovereignty open-source funding, GitHub Sponsors.

---

## 7. Audience / build-in-public

- Narrative is a magnet: "an unfreezable bank account that runs itself." Ship loud.
- Open-source repo from day one = the public good that funds itself.
- Demo videos of policy → on-chain execution; an open writeup of the trust model (great
  technical-credibility artifact, plausibly EF/privacy-community amplified).
- Engage EF / privacy / account-abstraction communities; this is where the user grows impact.

---

## 8. Roadmap (≈8 months, rough)

- **M1** — Resolve the asset-integrity decision (§4.2); spec + trust architecture + threat
  model; testnet smart account + 1 policy (runway floor) proposing a move end-to-end.
- **M2–3** — Policy engine as proposal generator + one-tap approval flow; local AI
  proposing/explaining; native macOS control-plane shell.
- **M4** — 3–4 core policies; NL policy authoring; transparent recommend/approve log; testnet demo.
- **M5** — Hardening + recovery; first grant apps (NLnet / Gitcoin) with the advisory demo.
- **M6** — Limited mainnet, advisory-first (every move human-approved); build-in-public launch;
  EF ESP application.
- **M7–8** — Polish, expand policies, broaden funding; write the v2 autonomy spec + line up an
  audit so hands-off execution can ship next.
- **v2+ (post-audit)** — Bounded hands-off autonomy (session keys + keeper). The original
  Autopilot promise, shipped once the advisory product has trust and an audit.

---

## 9. Open questions (for the reviews)

1. **Platform:** macOS-first vs cross-platform desktop (Tauri) vs mobile companion?
2. **Assets:** RESOLVED — tiered, sovereign-default (non-blockable assets), with explicit user
   opt-in to a selected centralized stable. See §4.2.
3. **Keeper:** pragmatic single-keeper v1 vs decentralized from the start?
4. **License:** copyleft (commons narrative) vs permissive (adoption)?
5. **Name:** keep "Deckard / Autopilot" or rename?
6. **Solo-maintainability:** partly resolved. Advisory-first shrinks the security surface to
   solo-ownable size; the v2 autonomy engine still needs a collaborator or a grant-funded audit.

---

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | complete | Pressure-tested 3 risks; asset integrity resolved; v1 posture decided then revisited. |
| Outside Voice | Codex (GPT-5) | Independent strategy challenge | 1 | issues_raised | Called the direction grant-shaped + "doc-beautiful," flagged imaginary user + no distribution, and "advisory-first guts the premise." Digested below. |
| Eng Review | `/plan-eng-review` | Architecture & tests | 0 | — | not run (no code yet) |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | — | not run |

**Decision history:**
- **D1 (CEO review):** reduce v1 to advisory-first for solo-safety.
- **Outside-voice digestion (Codex):** kept — advisory-only may gut the premise; must name a
  real user with a sharp daily pain; build from conviction not to impress EF; drop the reckless
  "no one can freeze" tagline; money-handling is heavy trust for a solo dev. Discarded as
  wrong-game — "pivot to a Farcaster utility," mass-market demand, asset-stability-for-normies,
  grant-treadmill-as-failure.
- **Framing lock:** Farcaster is the megaphone (amplification + build-in-public), NOT the
  product surface. The product stays cypherpunk / decentralized, not on the Farcaster protocol.
- **D3 (supersedes D1):** autonomy is the soul. Keep hands-off autonomy as the destination;
  resource it (audit gate + security co-maintainer + funded audit + longer timeline). Advisory-
  first becomes Milestone 1, not the ceiling. This is a funded, multi-contributor public good.

- **RESOLVED:** primary user = onchain operators / crypto-native earners (see §2).
- **OPEN:** (1) write the Milestone 1 build spec, (2) recruit a security co-maintainer + a
  fund-the-audit plan, (3) run `/plan-eng-review` once there is an architecture.
- **VERDICT:** Direction is fully specified and high-conviction. Strategy phase complete. Next is
  shaping Milestone 1 into a build spec.
