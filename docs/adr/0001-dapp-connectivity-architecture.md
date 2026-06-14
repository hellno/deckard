# ADR 0001 — Dapp connectivity architecture

- **Status:** Proposed (2026-06-14)
- **Deciders:** @hellno (maintainer)
- **Context inputs:** [`docs/research/10-dapp-connectivity.md`](../research/10-dapp-connectivity.md)
  (cited evidence), `THREAT-MODEL.md`, `SECURITY.md`, `DESIGN.md`, `docs/build/30-mcp-shape.md`.
- **Supersedes / relates:** establishes the umbrella decision; the executable work items live as
  GitHub issues under **epic [#44](https://github.com/hellno/deckard/issues/44)** — PRD-01 → #45,
  PRD-02 → #46, PRD-03 → #47, PRD-05 → #48, spike → #49, PRD-04 → #50. (Convention: work items/epics
  live in issues; decisions + research live in `docs/`.)

## Decision drivers (agreed with the maintainer)

Captured via a requirements pass before research:

1. **Mobile horizon — "don't compromise desktop for it."** Optimize for macOS/Linux; keep mobile
   *possible* but don't let it dictate the desktop design.
2. **Dapp openness — "curated/allowlisted first," but universal reach is an accepted goal.** Start with
   a vetted set (swaps, bridges); on-brand "calm sovereign control, not a casino"; per-origin scope.
   Users *will* want arbitrary dapps, so the architecture must reach the long tail eventually.
3. **Browser engine — "avoid; keep wallet minimal."** Do not bundle a browser engine.
4. **Own the connection — "no external bad stuff."** A second-pass requirement: Deckard does **not**
   build on a bloated third-party transport with bad UX. No WalletConnect relay, no dependence on a
   browser-store as a trust anchor. Universal reach is delivered through a **bridge Deckard owns
   end-to-end** (our wire, our UX, local-first, no third-party relay). WalletConnect is recorded as a
   rejected alternative, not a roadmap item.
5. **Deliverable — "ADR first, then multiple PRDs"** with per-workstream Definitions of Done.

Plus the standing constraint from `SECURITY.md`: Deckard is `0.0.1-alpha`, unaudited, single-maintainer,
testnet-keys-only. Any new attack surface is sequenced *behind* an audit.

## Context

Deckard's security model (`THREAT-MODEL.md`) is a hub: **the key lives in `deckard-signerd`; every
other process is an untrusted *proposer* that submits a typed `Intent`; the daemon's policy gate
decides; the human approves via a native clear-signing card.** `deckard-mcp` is the reference proposer
(key-less; no `propose`-arbitrary, no `resolve`). The contract is frozen in `deckard-contract`
(`Intent`/`Decision`/`Policy`/`SignerRequest`).

Two facts in the current code shape this decision:

- **`crates/deckard-signerd/src/auth.rs` gates only on `same_uid()`.** Per the threat model's residual
  risk #1, `Resolve{approved:true}` is honored from *any* same-uid caller. This is fine for today's
  small, human-controlled proposer set; it is **not** fine once a browser-reachable proposer exists.
  The research confirms this is the textbook confused-deputy gap (`research §22–24`).
- **Message-signing scaffolding already exists** — `SwapOrder`/`SignOrder` (EIP-712),
  `PendingPayloadView::Approve{token,spender,amount}`, the shaped-approve gate. Dapp signing *extends*
  this, it doesn't start from zero.

The user's two original options map to: **A = embedded in-app webview**, **B = external browser
extension**. Research surfaced a third transport, **WalletConnect v2** (relay; desktop + mobile; no
extension), and a crux beneath all three (the resolver-authentication boundary).

## Decision

**1. Reject the embedded in-app browser (Option A). Deckard does not bundle a browser engine.**
Rationale (`research §16–21`): `wry` means three engines (WebView2/WKWebView/WebKitGTK) to track;
WebKitGTK on Linux patches via distro lag with default-*off* renderer sandboxing and a live 2025
ACE-class CVE stream; Tauri's own docs say "don't load untrusted remote content" and have shipped an
IPC-bypass CVE for exactly that. It imports the largest, worst-patched TCB next to the keys to
replicate a *mobile-only* pattern that exists because phones lack the extension/external-browser
options desktop already has. This contradicts decision driver #3 and the `deckard-core`
`#![forbid(unsafe_code)]` minimal-core ethos.

**2. Adopt a phased model that reuses the existing key-less-proposer → policy-gating-daemon → native
clear-signing pattern. No new trust boundary is invented; we extend the one we have.**

- **Phase 0 — Curated native integrations (no web transport).** Realize "curated dapps first"
  (driver #2) as *native* Deckard surfaces that build typed `Intent`/`SwapOrder`s directly in Rust —
  exactly how Shield (Railgun) and the CoW swap path already work. No dapp connection, no browser, no
  relay, no new external proposer. Highest security, lowest new surface, fully on-brand. → **PRD-03**.
- **Phase 1 — Foundational hardening (prerequisite for *any* external proposer).**
  - **1a. Resolver authentication.** The daemon hands the GPUI app it already spawns
    (`supervise.rs`) an **unforgeable approval capability** (a `socketpair()` end / `SCM_RIGHTS`-passed
    fd); `Resolve` is accepted *only* on that channel; the public proposer socket can no longer
    self-approve. Closes residual-risk #1 (`research §22–28`). → **PRD-01**.
  - **1b. Clear-signing v2 + message-signing intents.** Extend the `Intent` surface to off-chain
    signatures (`personal_sign`, EIP-712) with decode, domain binding, permit/Permit2/Seaport
    recognition, EIP-7702 handling, and ERC-7730 consumption with a safe blind-sign fallback
    (`research §30–36`). Phase 0's swap already needs Permit2/EIP-712, so this is not gated on Phase 2.
    → **PRD-02**.
- **Phase 2 — Universal reach via a Deckard-native bridge, when warranted (post-audit).** Deliver
  "connect to any dapp" through a transport Deckard **fully owns**: a first-party, key-less connector
  that injects a *standard* EIP-1193 + EIP-6963 provider into the user's own browser (so every dapp
  works — they already speak EIP-1193) and forwards requests over a **Deckard-owned local wire** (native
  messaging preferred over a localhost port, `research §2–5`) to a new key-less proposer process
  mirroring `deckard-mcp`. No third-party relay, no embedded engine, no store-as-trust-anchor. The UX
  is native cards, local and instant — the explicit antidote to WalletConnect's relay-latency/QR-dance
  UX. → **PRD-04** (the bridge) + **PRD-05** (per-origin permissions, scope & registry).
  - **WalletConnect v2 is rejected and shelved** (see Alternatives; rationale recorded in epic
    [#44](https://github.com/hellno/deckard/issues/44)). It would import a centralized relay +
    Project-ID dependency (privacy/offline-first tension) and a UX the maintainer explicitly does not
    want.
  - The embedded webview stays rejected.
  - **Honest cost:** a browser-side connector is desktop-only. Rejecting WalletConnect (the
    mobile-capable transport) leaves *mobile* universal-reach unsolved; this is consistent with driver
    #1 ("don't compromise desktop for it") but is the deliberate trade and an open question for a future
    mobile effort, not something this ADR solves.
  - **Supply-chain containment:** the connector is a recurring class of attack target (`research §6`),
    so its trust is *bounded by design*, not by the store: it is key-less, cannot `resolve` (PRD-01),
    submits only typed intents through PRD-02 builders, and every effect is clear-signed. A fully
    compromised connector can propose garbage — it can never sign, self-approve, or exfiltrate a key.

**3. Cross-cutting invariants every phase inherits (non-negotiable):**
- New proposers are **key-less**, reuse `deckard-contract`, and **cannot** `resolve` or submit a raw
  arbitrary `Intent` that bypasses a typed builder (the `deckard-mcp` rule).
- **Origin is displayed as attacker-controllable** (unverified unless independently attested); the card
  always clear-signs *actual effects*, never a claimed dapp name (`research §29`).
- Every new user action registers a ⌘K `Command` (CLAUDE.md §Command palette reachability).
- The mainnet guardrail and STOP semantics (`THREAT-MODEL.md`) apply unchanged to every new write path.

## Consequences

**Positive**
- The security boundary is *strengthened* before any surface is added (PRD-01 fixes a standing
  accepted risk regardless of connectivity).
- The minimal-core, offline-first, privacy identity is preserved *all the way through* — the owned
  bridge has **no third-party relay**, so no IP/metadata leaks to a Reown-style intermediary; everything
  stays local. This is a genuine privacy win over WalletConnect, not just parity.
- We own the UX end-to-end: native cards, local and instant, no QR pairing or relay round-trip — the
  deliberate answer to "WalletConnect UX sucks."
- Universal reach is still achieved (any EIP-1193 dapp) without becoming a browser or depending on a
  relay/store.
- Phasing matches the alpha → audit → 1.0 trajectory: ship the safest, smallest thing first.

**Negative / costs**
- **Mobile universal-reach is unsolved.** A browser-side connector is desktop-only; we gave up the one
  mobile-capable transport (WalletConnect). Accepted under driver #1, but flagged as a future open
  problem.
- **We carry a first-party connector** (a browser artifact + a native-messaging host) — engineering and
  a perpetual supply-chain target to maintain. Contained by the key-less/bounded-boundary design, but
  it is real surface we now own.
- **Building our own wire** instead of adopting WalletConnect is more invention; we don't get dapps'
  existing WC support for free (mitigated: dapps speak EIP-1193 already, so the *injected provider*
  path needs no per-dapp work).
- Declining the embedded browser means Deckard never offers a fully in-app "open any dapp" surface; the
  curated-native + owned-bridge combination is the deliberate substitute.

## Alternatives considered (and rejected)

- **Embedded webview (Option A as clarified):** rejected — see Decision §1.
- **WalletConnect v2 as the generic transport:** **rejected and shelved** (full rationale recorded in
  epic [#44](https://github.com/hellno/deckard/issues/44)). Summary: a centralized
  Reown relay + Project-ID dependency that leaks IP/metadata (against offline-first/privacy,
  `research §10–11`), a QR/relay UX the maintainer explicitly rejects, and no maintained Rust
  wallet-side SDK (`research §14`). Its one advantage — mobile reach — does not outweigh owning the
  transport and UX. Recorded so the decision isn't re-litigated.
- **Generic browser extension over a Frame-style localhost RPC bridge (`ws://127.0.0.1:1248`):** the
  *localhost* wire is rejected — web-reachable (DNS-rebinding / cross-site WebSocket hijacking), defended
  only at the UI layer (`research §2`). The owned bridge (PRD-04) prefers **native messaging** (stdio
  host, not web-reachable) for exactly this reason. A first-party connector is *kept* (it is the bridge's
  browser end), but its trust is bounded by the key-less/clear-signing boundary, not by the store.
- **In-process plugins / dylibs loaded into the app or daemon:** rejected outright — same-uid code
  with arbitrary execution lands on the *trusted* side of the boundary and can self-approve; this is
  the maximal form of residual-risk #1. (Extensibility, if wanted, must be sandboxed, key-less,
  intent-only — never in-process native code.)

## Open questions (resolved inside the PRDs)

- PRD-01: `socketpair` inheritance vs `SCM_RIGHTS` fd-pass vs `0600` cookie — pick the most portable
  capability for macOS + Linux given `supervise.rs` already spawns the daemon (note the
  daemon-spawns-app vs app-spawns-daemon direction inversion).
  - **Resolved → `socketpair` inheritance (app is the parent).** The app already `fork+exec`s the
    daemon (`supervise.rs`), so it mints an `AF_UNIX` `socketpair` pre-spawn, hands the child one
    end by fd inheritance (its number in `DECKARD_RESOLVE_FD`), and keeps the other as the
    `ControlChannel`; the daemon honours `Resolve` only there (`daemon.rs::Channel`,
    `server.rs::serve_control`). Portable on macOS + Linux with no peer-cred dependence for the
    role decision — the fd *is* the unforgeable role proof that same-uid peer-cred (`SO_PEERCRED`
    on Linux; `LOCAL_PEERCRED`, no pid, on macOS) cannot give. **`SCM_RIGHTS`** was rejected: it
    fits a daemon-spawns-app or already-authenticated peer, but here authentication *is* the
    problem (the daemon would hand the fd back over the very public socket any same-uid client can
    open). **`0600` cookie** was rejected: a uid-readable secret doesn't split the same-uid
    boundary and would need a wire field, breaking "no wire change." The one unavoidable `unsafe`
    (`from_raw_fd`) lives daemon-side under a scoped, documented `#[allow]`; `deckard-core`'s
    `#![forbid(unsafe_code)]` is untouched.
- PRD-02: which `IntentKind`s to add (`SignMessage`, `SignTypedData`, and whether EIP-7702 `Delegate`
  is supported-behind-allowlist or refused outright).
- PRD-04: the browser-reach mechanism for the owned bridge — native-messaging stdio host (preferred,
  not web-reachable) vs a hardened localhost wire; how the first-party connector is distributed and
  signed without leaning on a store as a trust anchor; and how/whether mobile reach is ever addressed
  given the connector is desktop-only.
- PRD-05: registry format (adopt ERC-7730 descriptors directly?) and how the curated allowlist ships
  and updates offline-first.
