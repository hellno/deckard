# ADR 0001 — Dapp connectivity architecture

- **Status:** Proposed (2026-06-14)
- **Deciders:** @hellno (maintainer)
- **Context inputs:** [`docs/research/10-dapp-connectivity.md`](../research/10-dapp-connectivity.md)
  (cited evidence), `THREAT-MODEL.md`, `SECURITY.md`, `DESIGN.md`, `docs/build/30-mcp-shape.md`.
- **Supersedes / relates:** establishes the umbrella decision the `docs/prd/*` series executes.

## Decision drivers (agreed with the maintainer)

Captured via a requirements pass before research:

1. **Mobile horizon — "don't compromise desktop for it."** Optimize for macOS/Linux; keep mobile
   *possible* but don't let it dictate the desktop design. → favors transports that can survive onto
   mobile over desktop-only ones, without over-investing now.
2. **Dapp openness — "curated/allowlisted first."** Start with a vetted set (swaps, bridges); on-brand
   "calm sovereign control, not a casino"; per-origin scope; expand later.
3. **Browser engine — "avoid; keep wallet minimal."** Connect to the user's own browser and/or a relay
   rather than bundling an engine. Preserves the hardened minimal-core identity.
4. **Deliverable — "ADR first, then multiple PRDs"** with per-workstream Definitions of Done.

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
- **Phase 2 — Generic dapp connectivity, when warranted (post-audit).** **WalletConnect v2 is the
  primary generic transport**, implemented as a new key-less proposer process mirroring `deckard-mcp`,
  with CAIP-25 scope negotiation, Verify-API origin attestation, per-origin policy, a curated
  registry + anti-phishing blocklist, and explicit relay-privacy mitigations. → **PRD-04** (transport)
  + **PRD-05** (per-origin permissions & registry).
  - **A browser extension (Option B) is a secondary, optional desktop-convenience add-on**, not the
    primary path: it is desktop-only (doesn't reach mobile, driver #1) and the extension artifact is a
    recurring drainer/supply-chain target (`research §6`). If ever built, it is a thin EIP-6963
    proposer that speaks the *same* WalletConnect-or-native bridge — never the daemon wire directly,
    and never with `resolve` capability. Tracked as a future PRD, not in this batch.
  - The embedded webview stays rejected.

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
- The minimal-core identity and offline-first posture are preserved (no engine; Phase 0/1 need no
  network relay at all).
- Phasing matches the alpha → audit → 1.0 trajectory: ship the safest, smallest thing first.
- WalletConnect gives a single transport that reaches desktop today and mobile later, so the eventual
  mobile app doesn't force an architecture rewrite (honors driver #1 cheaply).

**Negative / costs**
- WalletConnect imports a **centralized relay + Project-ID dependency** (privacy/offline-first tension,
  `research §10–11`) and a **non-trivial in-house Rust Sign-protocol build** (no maintained Rust
  wallet SDK, `research §14`). PRD-04 must address relay-egress privacy and the build/borrow decision.
- Declining the embedded browser means Deckard never offers a fully in-app "open any dapp" experience;
  the curated-native + WalletConnect combination is the deliberate substitute.
- The extension being deprioritized means desktop-browser dapps that only speak injected
  `window.ethereum` (not WalletConnect) aren't reachable until/unless the optional extension ships.

## Alternatives considered (and rejected)

- **Embedded webview (Option A as clarified):** rejected — see Decision §1.
- **Browser extension as the *primary* transport (Option B):** rejected as primary — desktop-only and
  a supply-chain liability; retained only as an optional secondary add-on.
- **Frame-style localhost RPC bridge (`ws://127.0.0.1:1248`):** rejected — web-reachable
  (DNS-rebinding / cross-site WebSocket hijacking), defended only at the UI layer (`research §2`).
- **In-process plugins / dylibs loaded into the app or daemon:** rejected outright — same-uid code
  with arbitrary execution lands on the *trusted* side of the boundary and can self-approve; this is
  the maximal form of residual-risk #1. (Extensibility, if wanted, must be sandboxed, key-less,
  intent-only — never in-process native code.)

## Open questions (resolved inside the PRDs)

- PRD-01: `socketpair` inheritance vs `SCM_RIGHTS` fd-pass vs `0600` cookie — pick the most portable
  capability for macOS + Linux given `supervise.rs` already spawns the daemon (note the
  daemon-spawns-app vs app-spawns-daemon direction inversion).
- PRD-02: which `IntentKind`s to add (`SignMessage`, `SignTypedData`, and whether EIP-7702 `Delegate`
  is supported-behind-allowlist or refused outright).
- PRD-04: build the WC Sign layer in Rust vs run a sidecar (a key-less WC↔daemon translator akin to
  `deckard-mcp`); relay privacy egress; whether a custom/self-hosted relay URL is exposed.
- PRD-05: registry format (adopt ERC-7730 descriptors directly?) and how the curated allowlist ships
  and updates offline-first.
