# Walletbeat Compatibility & Requirements

> What Deckard must do to be **compatible with — and pass — the Walletbeat rubric**
> (`beta.walletbeat.eth.limo`), attribute by attribute, with Deckard's current status and the
> concrete work required for each. This is the **action-oriented companion** to the neutral research
> in [`research/07-wallet-rankings.md`](research/07-wallet-rankings.md) — that file explains *what
> Walletbeat is and how it scores*; this file says *what Deckard needs to build*.
>
> Snapshot date: **2026-06-14**. The live software-wallet card is the source of truth for the
> attribute set and grouping; the underlying PASS/PARTIAL rules are cited from Walletbeat's schema
> source via file 07. Walletbeat is designed to **raise the bar over time**, so re-check the live
> card before acting on any single rule.

## How Walletbeat scores (the short version)

- **5 attribute groups** for software wallets — **Security, Privacy, Self-sovereignty,
  Transparency, Ecosystem** (a 6th, Maintenance, applies only to hardware/embedded wallets).
- Each attribute gets a mostly **PASS / PARTIAL / FAIL** rating by an *objectively measurable* rule.
  Numerically `FAIL=0.0`, `UNRATED=-0.5`, `EXEMPT=null`; `PASS`/`PARTIAL` are **verifiability-weighted**
  (a `PASS` is 1.0 when self-evident, 0.7 if independently audited, **0.1 if unverifiable**). Group
  scores are weighted averages; the overall score runs **-0.5 → 1.0**.
- **Implication for us:** an undocumented, unverifiable claim scores almost like a FAIL. Passing is as
  much about **publishing verifiable evidence** (audit reports, reproducible-build instructions,
  funding disclosure) as it is about shipping code.
- **Stages ladder** (L2BEAT-style): **Stage 0** = public source code (qualify for evaluation);
  **Stage 1** = recent audit + multi-vendor hardware + chain verification + private-by-default
  transfers + account portability + own-node + FOSS license + ENS + browser-integration standards;
  **Stage 2** = funded bug bounty + address non-correlation + permissionless L2→L1 withdrawal +
  custom RPC everywhere + funding disclosure + fee transparency + chain-specific address resolution +
  account abstraction + atomic batching.

## Decisions locked (2026-06-14)

Recorded so the rubric below reflects deliberate product choices, not unexamined gaps:

- **Funding model — self-funded, no revenue.** Deckard is self-funded and takes **no protocol fee**
  (the 0.25% on shields is **Railgun's**, not Deckard's). This is the verifiable content for a future
  `FUNDING.md` (#21); the file is **not yet written** (no quick-win work started this round).
- **Security audit — budget undecided.** The audit (#1) stays a **decision spike**, not an actionable
  issue, until a vendor/budget is set. It remains the highest-leverage item and the main Stage-1 gate.
- **Out of scope for v0 (deliberate):** **social/guardian recovery (#6)** — conflicts with the pure-EOA,
  no-third-party model — and **L2 force-withdrawal / transaction inclusion (#15)** — only meaningful once
  L2s are supported. Both are documented trade-offs, accepted as unrated, not chased.
- **Walletbeat ambition — harvest, don't climb.** Target the **maximum score on vision-aligned
  attributes**; we explicitly do **not** chase Walletbeat's Stage ladder. The differentiator (an
  agent that can't move funds without policy) is something Walletbeat doesn't even score; the rubric
  is a checklist to harvest where it aligns, not a ladder to climb.
- **Hardware-wallet signing (#4) — out of scope.** Deckard is keyboard-first, native, single-binary;
  external hardware signing is a deliberate non-goal. (A *secure-enclave* unlock path, #5, is separate
  and still in scope.)
- **Connection surface — agent-only (for now).** The **MCP agent is Deckard's connection surface**;
  there is no dapp/browser integration. **#25 (browser integration)** is **deferred** (not hard
  out-of-scope — the dapp-surface question is under separate exploration), and **#11 (app isolation)**
  is **reframed** as per-agent / per-session policy isolation.
- **Account abstraction (#26) — 7702 spike.** Research-spike **EIP-7702** as a path to enforce
  agent-policy **on-chain** (scoped session keys), which would also unlock atomic batching (#27).
  Decide adoption after the spike; weigh the new delegation-contract trust surface against the current
  `accountUnruggability` PASS. Until then #26/#27 stay FAIL by choice.
- **Multi-identity (#8/#9) — a goal (post-v0).** Support multiple, non-correlated accounts/identities
  (standard BIP-44 + non-correlation UX). Privacy-aligned; not a v0 blocker.
- **Process:** decisions are folded into this doc; **no GitHub issues opened** yet.

## Where Deckard stands today

| Group | PASS | PARTIAL | MISSING | Notes |
|---|---|---|---|---|
| **Security** (7) | 0 | 5 | 2 | Strong crypto hygiene; gated on external audit + clear-signing + hardware |
| **Privacy** (5) | 2 | 3 | 0 | Railgun shields + no-telemetry are real strengths |
| **Self-sovereignty** (6) | 4 | 2 | 0 | **Deckard's strongest group** — Helios + on-device keys + BIP-39 |
| **Transparency** (5) | 3 | 1 | 1 | FOSS + public; needs reproducible builds + funding disclosure |
| **Ecosystem** (4) | 0 | 1 | 3 | **Weakest group** — desktop-native, EOA-only, no dapp surface |
| **Total** (27) | **9** | **12** | **6** | |

These statuses are Deckard's self-assessment against the rules below; Walletbeat would rate
independently. The biggest single lever is the **external security audit** (#1). Per the decisions
above, several attributes are **deliberate non-goals** and are not counted as actionable gaps:
#4 hardware signing, #25 browser integration (deferred), #6 social recovery, #15 L2 force-withdrawal —
plus #26/#27 (account abstraction / batching) gated behind a 7702 spike. We optimize the
**vision-aligned** attributes, not Walletbeat's Stage ladder.

---

## Security

| # | Attribute | What Walletbeat wants (PASS) | Deckard now | Requirement to pass |
|---|---|---|---|---|
| 1 | **Security audits** | Audited within last 365 days, all medium+ findings fixed; stale/unresolved = PARTIAL | **MISSING** — "no third-party audit yet" (`SECURITY.md`) | **Fund + complete an external audit** of the signer/policy/keystore surface; publish the report and remediation. Highest-leverage single item (also a Stage-1 gate). **Decision spike — budget/vendor undecided (2026-06-14).** |
| 2 | **Scam prevention** | Warn users about scams (malicious address / approval / phishing / simulation) | **PARTIAL** — network-warning on Receive only (`receive.rs`, `DESIGN.md`) | Add malicious-address / known-scam checks and **simulate-before-sign warnings** in the clear-signing card; surface risk before approval. |
| 3 | **Transaction legibility** (clear-signing) | Display basic, human-readable tx details before signing; decode EIP-712 | **PARTIAL** — clear-signing card wired for shield/swap (`shield_view.rs`); EIP-712 machinery exists (`cow_types.rs`) but not user-facing for arbitrary tx | Extend the clear-signing card to **all writes** (esp. the gated Send), and render decoded **EIP-712** structured data in plain language. |
| 4 | **Hardware wallet support** | Sign via external hardware wallets, ideally 3+ vendors (Stage-1 wants multi-vendor) | **OUT OF SCOPE (2026-06-14)** — no Ledger/Trezor path | **Deliberate non-goal** — keyboard-first, native, single-binary. (Secure-enclave unlock, #5, is separate and in scope.) |
| 5 | **Security best practices** | Key storage in secure enclave/HSM = PASS; **standardized-KDF-encrypted / OS-sandboxed = PARTIAL**; on-device keygen with OS CSPRNG required | **PARTIAL** — Argon2id + XChaCha20-Poly1305 at rest, on-device keygen, OS CSPRNG (`keystore.rs`) | Already a solid PARTIAL. To reach **PASS**, add a **secure-enclave / OS-keychain** key path (Touch ID / Secure Enclave on macOS — already a Phase-2 line item). |
| 6 | **Account recovery** | Credits **only guardian/social recovery** — seed backup does **not** count; needs 3+ independent shares, no single party can recover, reconstituted on user device | **OUT OF SCOPE (2026-06-14)** — BIP-39 seed backup only | **Deliberately out of scope for v0.** Guardian/social recovery conflicts with the pure-EOA, no-third-party model; accepted as unrated and documented as a trade-off. |
| 7 | **Duress resistance** | Protect against physical coercion / unauthorized access (decoy, duress unlock, panic) | **PARTIAL** — STOP panic brake zeroizes the key (`deckard_revoke_all`, `daemon.rs`); explicit lock | Add **idle auto-lock** (already noted missing) and consider a **decoy/duress unlock**. The panic brake is a genuine partial credit. |

## Privacy

| # | Attribute | What Walletbeat wants (PASS) | Deckard now | Requirement to pass |
|---|---|---|---|---|
| 8 | **Wallet address privacy** | Address not linkable to sensitive personal info | **PARTIAL** — single on-chain receive address | Pair with shields (below) and stealth/rotating receive addresses to reduce linkability. |
| 9 | **Multi-address privacy** | Multiple addresses not correlatable with each other (Stage-2) | **PARTIAL** — standard BIP-44 derivation can yield multiple addresses; identity switching not a product surface | **Goal (2026-06-14, post-v0):** ship **multi-account / identity switching** with non-correlation UX; avoid reusing one address across contexts. |
| 10 | **Private token transfers** | Transfers/balances private by default | **PASS** — key-less **Railgun** shield, app-reachable, black-box tested (`shield.rs`, `STATUS.md`) | Maintain; extend to private **send/unshield** to strengthen "by default". This is a flagship strength. |
| 11 | **App isolation** | Distinct accounts per app to limit data correlation | **REFRAMED (2026-06-14)** — no dapp surface; agent is the connection surface | **Reframed as per-agent / per-session policy isolation** (Deckard has no dapps). Tie to the policy gate so each agent session is scoped/correlatable-by-choice. |
| 12 | **Privacy hygiene** | Collect no more data than a default web browser; no telemetry | **PASS** — no telemetry (`onboarding.rs`); Helios light-client reads; RPC URLs redacted from reasons (`helios.rs`, `THREAT-MODEL.md`) | Maintain; keep a written **no-data-collection** statement public as the verifiable evidence. |

## Self-sovereignty — *Deckard's strongest group*

| # | Attribute | What Walletbeat wants (PASS) | Deckard now | Requirement to pass |
|---|---|---|---|---|
| 13 | **L1 provider independence** | Self-hosted node configurable **before any request hits a default RPC**; all basic ops work through it | **PARTIAL — bar not yet met** — RPC overridable (`eth.rs`, `settings.rs`) but the provider spawns with `DEFAULT_RPC` at `Shell::new()` *before* the auth gate; custom RPC only applies on settings-blur, so the first read hits the default unless `DECKARD_RPC_URL` is set | Add an **onboarding RPC step or lazy provider spawn** so a custom RPC binds before the first request (`YES_BEFORE_ANY_REQUEST`). **Re-tiered to `L` (app-flow change), not a quick win (2026-06-14).** |
| 14 | **Account portability** | Standards-compliant **BIP-39 + BIP-32 + BIP-44** with exportable seed/key | **PASS** — BIP-39 keystore, hold-to-reveal seed export (`keystore.rs`, `DESIGN.md`) | Maintain. Confirm full BIP-32/44 derivation path is standard and documented. |
| 15 | **Transaction inclusion** (censorship resistance) | Withdraw L2 funds to L1 without intermediaries (force-withdraw); mempool independence | **OUT OF SCOPE (2026-06-14)** — L1-focused, no L2 force-exit | **Deliberately out of scope for v0** (no L2 roadmap yet). Stage-2 item; revisit if/when L2s are supported via **permissionless L2→L1 withdrawal**. |
| 16 | **Chain verification** | Verify integrity of the Ethereum chain (light client) | **PASS** — **Helios** light-client verified reads with a Verified/Unsynced badge (`helios.rs`, `shell_chrome.rs`) | Maintain. A standout strength most wallets fail. |
| 17 | **Account unruggability** | No external party can take over/reconstruct the account; on-device key control; no provider-held seed backup | **PASS** — process-isolated daemon gates writes, on-device encrypted seed, no remote upgrade path (`SECURITY.md`, `daemon.rs`) | Maintain. Keep any future cloud/agent component **key-less** to preserve this. |
| 18 | **Permissions management** | Inspect and **revoke ERC-20/721/1155 approvals** | **PARTIAL** — `deckard_revoke_all` panic brake + policy caps/allowlist (`policy.rs`); no per-token approval UI | Add a **token-approval viewer + revoke** UI (per-allowance revoke), not just the global STOP. |

## Transparency

| # | Attribute | What Walletbeat wants (PASS) | Deckard now | Requirement to pass |
|---|---|---|---|---|
| 19 | **Source code license** | OSI-FOSS (MIT/Apache/BSD/GPL); delayed-FOSS = PARTIAL; unlicensed = FAIL | **PASS** — AGPL-3.0-or-later (`LICENSE`) | Maintain. Ensure **every component/file carries a FOSS license** (unlicensed bits are conservatively treated as NOT_FOSS). |
| 20 | **Source visibility** | All repos publicly viewable | **PASS** — `github.com/hellno/deckard`, all crates public | Maintain; keep any future sidecar/repo public too. |
| 21 | **Funding** | Funding sources and revenue model public and transparent (Stage-2) | **MISSING** — no `FUNDING.md` yet; **content decided (2026-06-14)** | Write **`FUNDING.md`** stating: **self-funded, no protocol revenue** (the 0.25% shield fee is Railgun's, not Deckard's). Content locked; file not yet written. Cheap, verifiable, Stage-2 gate. |
| 22 | **Fee transparency** | Fees shown to the user at all times; no hidden fees | **PASS** — Railgun 0.25% shield fee shown on the review card (`shield_view.rs`) | Maintain; show any future swap/relay fees with the same explicitness. |
| 23 | **Release process** | Release process follows safety best practices (reproducible/verifiable builds, signed releases, changelog) | **PARTIAL** — `CHANGELOG.md`, pinned toolchain, committed `Cargo.lock`; **no reproducible-build instructions** | Publish **reproducible-build instructions + signed release artifacts** (also satisfies WalletScrutiny). Document the verify-from-source path in `RELEASING.md`. |

## Ecosystem — *Deckard's weakest group*

| # | Attribute | What Walletbeat wants (PASS) | Deckard now | Requirement to pass |
|---|---|---|---|---|
| 24 | **Address resolution** | Send to human-readable names (ENS); ideally chain-specific (ERC-7828/7831) | **PARTIAL** — ENS resolver imported in engine (`eth.rs`) but not wired to UI | **Wire ENS resolution into the Send/Receive UI** (resolve + reverse-resolve, with the verified-read badge). Engine work is mostly done. |
| 25 | **Browser integration** | Comply with browser dapp-integration standards (EIP-6963 / WalletConnect) | **DEFERRED (2026-06-14)** — desktop-native, agent-only connection surface | **Deferred, under separate exploration.** Deckard's connection surface is the **MCP agent**, not dapps; no WalletConnect/EIP-6963 planned. Accepts a FAIL here by choice unless the separate dapp-surface discussion changes it. |
| 26 | **Account abstraction** | Be AA-ready (ERC-4337 and/or **EIP-7702** smart accounts) | **MISSING → 7702 spike (2026-06-14)** — EOA-only | **Research-spike EIP-7702** to enforce agent-policy **on-chain** (scoped session keys); address-preserving, Rust tooling exists (`research/02`). Decide adoption after weighing the new delegation-contract trust surface vs. the `accountUnruggability` PASS. |
| 27 | **Transaction batching** | Support **atomic batched transactions** (EIP-5792 `wallet_sendCalls`) | **MISSING** — single-tx only | **Contingent on the #26 7702 spike.** If 7702 is adopted, implement **EIP-5792 `wallet_sendCalls`** atomic batching. |

---

## Architecture fit & core-expansion map

Validates the **code-fit layer**: where each item lands in the crate boundary, whether the seam already
exists or the engine must be **expanded**, rough size, and risk. From a four-cluster read of the actual
code (2026-06-14). Altitude only — no line-level diffs. This is the answer to "did we read our own
infra correctly, and does the core need expanding?"

**Headline: `deckard-core` was built with the right seams.** Most items need *new typed surfaces*, not
*new crypto* — multi-account derivation already exists, the keystore header already reserves an
enclave/biometric flag, the shield builder is key-less (reusable for simulate), and the signerd
broadcast path is alloy/7702-ready and builder-agnostic. Two findings correct the first survey:

- **Re-tiered UP — #13 "custom RPC before first request" is NOT a quick win (now L).** The provider is
  spawned at `Shell::new()` with `DEFAULT_RPC` *before the auth gate*; a custom RPC only applies on
  settings-blur, so the first read after unlock hits the default RPC unless `DECKARD_RPC_URL` is set.
  Passing the literal bar needs an onboarding RPC step or lazy provider spawn (an app-flow change).
- **Re-tiered DOWN — #8/#9 multi-identity is cheaper (no core crypto).** `deckard-core` already derives
  any account (`account_signer(index)`/`account_address(index)`, Railgun per-index too); the daemon
  hard-wires index 0. It's a daemon `active_account` + a `SelectAccount` wire request + a switcher UI.

| Item | Crate(s) | Seam vs. expansion | Size | Risk |
|---|---|---|---|---|
| **Un-gate Send** (linchpin) | app (daemon/contract done) | **Seam exists** — `IntentKind::Send` fully handled + tested (`daemon_e2e`/`anvil_e2e`/`parity`); only `send_view.rs` missing | **S** | none |
| **#3** clear-sign all + EIP-712 decode | app + core | Partial (shield card only); **core expansion** — generic EIP-712 decoder (`cow_types` machinery is CoW-specific) | M–L | med — decoder scope (start CoW/Send/ERC-20-Permit, leave extension point) |
| **#2** simulate + scam warnings | core + app (+contract maybe) | **Expansion** — key-less `simulate_intent` (eth_call) app-side; heuristic address checks | L | high — racy + detection is heuristic, not cryptographic; UX must be honest |
| **#18** approvals viewer + revoke | core + app + contract + signerd | Partial (`PendingPayloadView::Approve` exists); **expansion** — `enumerate_approvals` + likely new `IntentKind::RevokeApproval` | M | med — ERC-20 has no `allApprovals`; curated-spender list is fast but incomplete |
| **#5** enclave / Touch ID unlock | core + app | **Seam reserved** (keystore header `flags` bit); **expansion** — pluggable `UnlockBackend` (passphrase-only today); enclave wraps the DEK, not the secp256k1 key | M | med — secret-handling edge |
| **#7** idle auto-lock | signerd + app | **Seam exists** — add TTL to daemon + app keepalive | S | low — pure state |
| **#8/#9** multi-identity | signerd + contract + app (**core ready**) | Core seam exists; expansion in daemon (`active_account`) + `SelectAccount` wire request + UI | L | med-low — no new secrets (index is public) |
| **#13** RPC-before-first-request | app + core | Partial (respawn-on-blur); **expansion** — onboarding RPC step or lazy provider spawn | **L** | high — currently **not met** (first read hits `DEFAULT_RPC`) |
| **#24** ENS forward resolve | core + app | **Seam exists** but isolated to the watch-address field; surface in Send/Receive | M | low |
| **#24** ENS reverse + verified badge | core + app + contract | **No seam** — no `ReverseResolveName`; name results aren't wrapped in `Read<>` for the badge | M | low-med |
| **#26** EIP-7702 on-chain policy | signerd + core (+app); contract can stay policy-blind | **Seam** (alloy `TransactionBuilder7702`, builder-agnostic broadcast); **expansion** — session-key derivation + `broadcast_7702` | L (spike) | med — new on-chain delegation trust surface |
| **#27** atomic batching | signerd + contract + core | **Expansion** — `Intent::Batch` + bundle policy semantics; practically gated on 7702 | M–L | med — bundle evaluation soundness |
| **#11** per-agent isolation | contract + signerd + mcp | **Expansion** — policy is global/immutable today (one `policy.json`, no `SetPolicy`); needs session-token in the wire contract | M | med — token leak = agent impersonation |

**Cross-cutting findings:**

1. **Send is a UI-only unlock and the dependency hub.** The whole Intent→Policy→Decision→sign→broadcast
   path already handles Send (tested); un-gating it (an `S` app surface) is what enables #2, #3, #18, and
   the Send half of #24. Do this first.
2. **The frozen wire contract (`deckard-contract`) is the higher-ceremony expansion point.** Three items
   want to touch it — `RevokeApproval` (#18), `Intent::Batch` (#27), session-token fields (#11). **Batch
   those contract decisions together**; don't let them sprawl across separate changes.
3. **Simulation & scam detection are app-side, key-less, off the daemon mutex, non-blocking, and
   heuristic** — the UX must say "double-check this address," never "this is safe."
4. **`specs/strategy.md` already anticipates v2 session keys**, so the 7702 direction aligns with existing
   strategy — keep the two docs cross-linked, not divergent.

### EIP-7702 spike brief (#26 → #27)

Scoped go/no-go, **testnet-only, non-autonomous, ~4–6 dev-days (Rust only).** 7702 would **complement,
not replace** the local policy gate (defense-in-depth: local gate = usability + incident prevention;
on-chain scope = blast-radius containment).

- **Build:** derive a scoped session key from the master seed; build a 7702 authorization delegating the
  EOA to the audited `Simple7702Account`, scoped by cap / target / block-height expiry; construct + sign +
  broadcast via alloy `TransactionBuilder7702`; verify on-chain that **over-cap / expired / out-of-scope
  are rejected**; document file deltas + the delegation-signing UX; update `THREAT-MODEL.md` for the new
  delegation surface.
- **Go/no-go questions the spike must answer:** (1) session-key source — master-seed-derived vs.
  fresh/enclave-backed; (2) revocation atomicity — can a pre-revocation session tx still land? (implies
  block-height TTL); (3) per-chain vs. all-chain delegation scope; (4) is the local `Policy` mirrored
  on-chain, or is on-chain just auth/expiry/scope; (5) is per-agent isolation (#11) v1 or v2.

## Recommended sequencing

Grouped by leverage vs. effort, updated with the architecture-fit sizes above. **Suggested ordering for
discussion, not a committed roadmap.**

### Tier 0 — the linchpin
0. **Un-gate Send** (`S`, UI-only — daemon path is production-ready and tested). Unblocks #2/#3/#18 and the Send half of #24.

### Quick wins (low effort, no core expansion)
1. **`FUNDING.md`** — funding/revenue disclosure (#21). **Content locked (2026-06-14): self-funded, no protocol revenue.** Hours; not started.
2. **Idle auto-lock** (#7, `S`) — daemon TTL + app keepalive; flagged missing in `STATUS.md`.
3. **Reproducible-build instructions + signed releases** in `RELEASING.md` (#23) — also satisfies WalletScrutiny.
4. **ENS forward-resolve in Send/Receive** (#24, `M`) — engine has forward resolve; surface it.

### Core product work (medium effort; some core/contract expansion)
5. **Clear-signing for all writes + generic EIP-712 decoder** (#3, `M–L`) — rides on un-gated Send.
6. **Multi-account / identity switching** (#8/#9, `L` — **core already derives accounts**, no crypto work).
7. **Token-approval viewer + per-allowance revoke** (#18, `M`) — core `enumerate_approvals` + likely a new `RevokeApproval` wire kind.
8. **Secure-enclave / Touch ID unlock** (#5, `M`) — pluggable `UnlockBackend`; pushes security-best-practices toward PASS.
9. **ENS reverse resolve + verified badge** (#24, `M`).
10. **Custom RPC before first request** (#13, **`L`** — re-tiered: onboarding step or lazy provider spawn).
11. **Simulate-before-sign + scam warnings** (#2, `L`) — app-side key-less simulate; honest heuristic UX.

### Strategic / large (sets the ceiling)
12. **External security audit + funded bug bounty** (#1, and Stage-2 bug bounty) — *the* highest-leverage item. **Decision spike — budget/vendor undecided (2026-06-14); not yet an actionable issue.**
13. **EIP-7702 spike** (#26, ~4–6 dev-days) → if adopted, unlocks **atomic batching** (#27) and on-chain scoped agent permissions. See the spike brief above. Batch its wire-contract changes with #18/#27/#11.
14. **Per-agent isolation** (#11, `M`) → session-token in the wire contract; policy is global/immutable today.

### Out of scope / deferred for v0 (decided 2026-06-14 — acknowledge, don't chase)
- **Hardware-wallet signing** (#4) — deliberate non-goal (keyboard-first, native, single-binary).
- **Browser integration / WalletConnect** (#25) — deferred; the MCP agent is the only connection surface (under separate exploration).
- **L2 force-withdrawal / transaction inclusion** (#15) — only meaningful once L2s are supported; no L2 roadmap yet.
- **Guardian/social recovery** (#6) — conflicts with the pure-EOA, no-third-party model; deliberate trade-off, accepted as unrated.

## The Stages ladder (reference only — we are not climbing it)

Per the **"harvest, don't climb"** decision (2026-06-14), Deckard does **not** target Walletbeat's
Stages. Two Stage-1 gates — **multi-vendor hardware (#4)** and **browser integration (#25)** — are
deliberate non-goals, so **full Stage 1 is structurally unreachable by choice**, and that's accepted.
The ladder is kept here only to show which vision-aligned items happen to overlap:

- **Stage 0** — *cleared*: public source code (#20).
- **Stage 1 (not targeted)** — gated by **hardware (#4, out)** and **browser integration (#25, deferred)**.
  Deckard nonetheless satisfies the vision-aligned Stage-1 items: **chain verification** ✅ (#16),
  **private-by-default transfers** ✅ (#10), **account portability** ✅ (#14), **FOSS license** ✅ (#19),
  with **own-node** (#13, near-PASS), **ENS** (#24), and **audit** (#1) in flight.
- **Stage 2 (not targeted)** — vision-aligned overlaps worth doing anyway: **address & multi-address
  non-correlation** (#8/#9), **custom RPC everywhere** (#13), **funding disclosure** (#21),
  **fee transparency** ✅ (#22). The rest (**L2→L1 withdrawal** #15, **AA** #26, **batching** #27) are
  out-of-scope or spike-gated.

## See also

- [`research/07-wallet-rankings.md`](research/07-wallet-rankings.md) — the verified deep-dive on
  Walletbeat's rubric, scoring math, Stages, and WalletScrutiny (with source-file citations).
- [`research/09-deckard-relevance.md`](research/09-deckard-relevance.md) — cross-cutting synthesis.
- [`../THREAT-MODEL.md`](../THREAT-MODEL.md), [`../SECURITY.md`](../SECURITY.md) — the security claims
  several Security/Self-sovereignty attributes lean on.
- Live card: `beta.walletbeat.eth.limo` — re-check before acting; the rubric tightens over time.
