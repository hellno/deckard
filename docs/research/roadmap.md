<!-- /autoplan restore point: /Users/hellno/.gstack/projects/hellno-deckard/hellno-wallet-research-autoplan-restore-20260605-143304.md -->
# Deckard Product Roadmap — Now / Later / Never

> Operator-first prioritization derived from the research KB in this directory (`README.md` + files
> 01–09) and **pressure-tested via `/autoplan`** (CEO · Eng · DX · codex dual voices, 2026-06-05).
> Citations `[NN]` point to the KB file that grounds the item. The `/autoplan` consensus + decision
> log are at the end of this file.

## What Deckard is (fixed constraints)

- A **native, cross-platform (macOS + Linux) desktop** Ethereum wallet, written in **Rust on GPUI**.
- **Self-custodial**, **local-first**. Keys live on the user's device.
- Today (v0): a single alloy-generated **secp256k1 EOA**, persisted as **plaintext hex** in the OS config dir.
- North-star: an **"operator wallet"** — an LLM that manages the wallet semi-autonomously, running
  locally or wired to the user's chosen AI, **under limits it cannot exceed**.

## Decisions applied (from the `/autoplan` review)

The review found the v1 draft **engineering-correct but strategically inverted**: it led with a
conventional security floor and enforced the operator's limits in a *same-process software gate* that
all four voices judged unsafe to call "the agent never sees the seed." The decisions below re-spine it.

1. **Safety boundary (①B):** the signer runs in an **isolated process** (key + policy inside it; the
   AI gets a key-less client), **and** a **minimal EIP-7702 session-key** path is pulled into NOW so
   limits are **chain-enforced**, not just software-checked. The honest claim becomes: *"a compromised
   agent cannot exfiltrate the key or exceed its on-chain limits — only request actions the policy and
   the chain permit."*
2. **Operator-first spine (②A):** the security floor is **cost-of-admission, done fast**; the operator
   is the headline; target a **6-week demo**: *"the agent safely pays/swaps/monitors under revocable
   limits."*
3. **Embedded Helios stays in NOW (③B):** kept as a differentiator (with the trusted-checkpoint +
   visible-fallback invariants below).
4. **Operator-experience pieces promoted to NOW (④A):** STOP/override, typed refusals, native approval
   surface, low-gas pre-flight, autonomy modes.
5. **Dapp connectivity allowed (⑤):** native-desktop form factor, **but** WalletConnect/dapp
   connections are supported — only the *browser-extension form factor* is excluded.

## How to read this

Each item: **capability · KB ref · why · gate · build signal (S/M/L)**.
NOW = reachable on a 7702-capable EOA in Rust today + on the critical path to the demo.
LATER = gated on a further prerequisite. NEVER = excluded by positioning.

---

## NOW

### 0 · Ship-floor (cost of admission — do fast, then stop polishing)

| Capability | KB | Why | Build |
|---|---|---|---|
| **Encrypted keystore** (Argon2id + XChaCha20-Poly1305) replacing the v0 plaintext key | `[08]` | v0 plaintext key is **below the universal field floor** | M |
| **BIP-39 seed backup + key export** | `[07][08]` | Real cross-wallet recovery; Walletbeat `accountPortability` = PASS | S |
| **Simulate-before-sign + clear-signing (ERC-7730)** — **fail-closed**; returns machine-readable asset deltas (for the agent) *and* the human card | `[01][08]` | EOA-compatible safety baseline; the input the operator reasons over | M |

### 1 · Operator core (the headline / the wedge)

| Capability | KB | Why | Build |
|---|---|---|---|
| **Process-isolated signer daemon** — holds the decrypted key + runs the policy; exposes only a `sign(intent)` RPC over an authenticated local socket; **no "sign arbitrary bytes"**; single-instance lock; audit log | `[05][08]` | The real trust boundary; the AI process never holds the key (Decision ①) | L |
| **Policy gate (inside the daemon)** — caps, allowlists, expiry, sim-on-deviation; **decodes calldata** (approvals/permits/7702 SetCode); **default-deny** unrecognized; returns **typed allow/deny/needs-approval + machine-readable reason + remediation** | `[05]` | Limits the agent can't bypass in-process; typed refusals stop agent flailing | L |
| **Minimal EIP-7702 session keys** — reversible, address-preserving delegation to an audited target (`Simple7702Account`/session-key validator); **chain-enforced** caps/expiry/allowlist; legible "what am I delegating to" UX | `[01][02]` | Makes the operator's limits **unbreakable**, not just local (Decision ①B) | L |
| **Local MCP sidecar** — key-less client of the daemon; `read`/`simulate`/`draft`/`scoped-execute` tools; **refuses flag-based secrets**; stdio or authed-localhost-HTTP with documented auth + revocation | `[04][05]` | The converged integration surface (Splits' one-binary CLI+MCP) | M |
| **Native approval surface** — desktop modal/tray showing intent + asset delta + counterparty + sim source + "why the agent wants this"; **deny / approve-once / approve-rule / pause-agent** | `[05]` | The most-seen operator interaction; Deckard's native edge over browser MCP (Decision ④) | M |
| **Owner-key override / STOP / revoke-all-agent-authority** | `[05]` | The operator panic button (Decision ④) | S |
| **Autonomy modes** — observe-only / human-confirm / local-autonomous (within limits) / smart-account-autonomous (7702-enforced); the risk boundary is **visible** | `[05]` | Resolves the agent-speed vs human-approval tension; sets honest expectations (Decision ④) | S |
| **Low-gas / insufficient-funds pre-flight** — structured refusal + funding affordance | `[01]` | A bare EOA needs ETH per tx; prevents silent mid-task stalls (Decision ④) | S |
| **🎯 First-autonomous-action demo (6 weeks)** — "monitor balance + pay an allowlisted address ≤ $X/day, simulated-then-approved, under revocable limits"; an **operator quickstart golden path** (testnet default) | `[04][05]` | The product proof the whole NOW set exists to deliver (Decision ②) | — |

### 2 · Operational privacy + signing hardening

| Capability | KB | Why | Build |
|---|---|---|---|
| **Private/proxied RPC by default** | `[06][07]` | Cheap, immediate; Walletbeat `l1ProviderIndependence`; stops IP+address leak | S–M |
| **Embedded Helios light client** (trust-minimized reads) — *kept in NOW per Decision ③* | `[06]` | Verifiable local reads, Rust-native; **invariants:** trusted-checkpoint policy + **visible** fallback when unsynced + prototype-and-measure sync cost | M–L |
| **Hardware-wallet signing (Ledger/Trezor)** — *separate from Touch ID*; protects the **user-driven** path (mutually exclusive with unattended agent signing) | `[08]` | Strongest single-key-risk reduction for the human path | M |
| **Touch ID** gates the keystore **unlock secret** (cold state only — orthogonal to per-tx operator auth) | `[08]` | At-rest protection; not per-action agent gating | S |

## LATER (gated)

| Capability | KB | Gate |
|---|---|---|
| **Full smart-account substrate** (7579 Kernel/Nexus/Safe) + full ERC-7715/7710 beyond minimal 7702 | `[01][05]` | Beyond the NOW minimal-7702 path; cross-chain module portability unsolved |
| **Gas abstraction** — paymasters, sponsored gas, pay-gas-in-token | `[01][02]` | Smart-account/paymaster infra; removes the gas-babysitting trap |
| **x402 payments** — *note: EOA-reachable today, gated only on prioritization* (unbundled from AP2/8004) | `[05]` | Product priority — a cheap early way for the operator to pay for data/compute |
| **AP2 Mandates / ERC-8004 identity** | `[05]` | Frontier; identity only when transacting with other agents/services |
| **On-chain passkey signer** (RIP-7212 / EIP-7951) | `[08]` | Smart-account-only |
| **Privacy upgrades** — stealth addresses; shielded pools (Railgun via Kohaku's Rust crate); EIP-8182 if it makes Hegota (EOA-compatible) | `[06]` | Kohaku Rust-crate consumability; EIP-8182 fork inclusion |
| **Splits integration** — register as a signer; call distribution contracts | `[04]` | API token + ERC-1271/UserOp signing |
| **Social recovery / guardians** | `[07][08]` | Smart-account-only; Walletbeat `accountRecovery` |
| **Independent audit + funded bug bounty** | `[07]` | Funding (a NOW threat-model review precedes MCP signing — see invariants) |

## NEVER (not by positioning — revisit only if positioning changes)

| Excluded | KB | Why |
|---|---|---|
| Custodial / WaaS / **MPC-as-a-service** custody | `[05][08]` | Breaks self-custody |
| Operating a **hosted relayer/bundler/treasury/fiat** service | `[04][05]` | Breaks local-first; *calling/renting* is fine |
| **Browser-extension form factor** (the *form*, not connectivity — see below) | `[03][07]` | Deckard is native desktop |
| Any **cloud component that can move funds** without on-device key control | `[07]` | Breaks Walletbeat `accountUnruggability` |
| The agent obtaining **unbounded signing authority** (or the raw seed) | `[05]` | The corrected safety axiom — bounded, revocable authority only |

> **Allowed (Decision ⑤):** dapp connectivity via **WalletConnect / companion surfaces**. Native-desktop
> is the form factor; web/dapp interaction is not banned (a wallet that can't touch apps loses on utility).

## Build invariants (non-negotiable — applied from the Eng review, not optional)

- **Atomic keystore writes** (temp + fsync + rename; never overwrite in place; never `let _ = fs::write`); **versioned self-describing header** (KDF/params/AEAD/nonce); **decrypt-after-encrypt round-trip verify** before deleting plaintext; `Zeroizing` on every decrypted buffer incl. error paths.
- **v0 migration hazard:** the v0 key is `PrivateKeySigner::random()` with **no mnemonic** — migration must encrypt-in-place, tell the user this key has no seed phrase, and offer a fresh BIP-39 wallet. Never fake a mnemonic.
- **Signer daemon:** authenticated caller (peer-cred/token), single-instance lock, replay protection, no raw-byte signing, append-only audit log.
- **Simulation = risk signal, not authorization:** fail-closed; treat the third-party simulator as untrusted + a privacy leak; re-check invariants close to broadcast; ERC-7730 descriptors from an untrusted registry → verify provenance, raw-hash fallback.
- **NOW threat-model / security-design review** before MCP signing ships (full audit reserved for funded release).
- **Tests:** key round-trip; migration crash-injection; policy-gate calldata-decode (approval/permit/7702); sidecar redaction; fail-closed simulation.

## Sequencing (rationale, not a fixed timeline)

`encrypted keystore → signer daemon (process boundary) → policy gate (inside daemon) + minimal 7702
session keys → MCP sidecar (key-less client) → simulate (feeds the gate) → native approval surface +
STOP/override + autonomy modes → 🎯 demo`. Private RPC and the Helios/HW-wallet hardening run in
parallel. The floor (keystore/BIP-39/simulate) is done fast and quietly; the operator core is the loud,
demoable spine.

---

## /autoplan Review Report

**Scope reviewed:** this roadmap. **Voices:** Claude subagents (CEO/Eng/DX, independent) + codex (gpt-5.5,
cross-model). **Design phase:** skipped (no UI scope). **Date:** 2026-06-05.

### Consensus — the unanimous finding

All four voices independently flagged the same **critical** issue: a **same-process software policy gate
on a hot EOA key is not a security boundary**. "The agent never sees the seed" was true only literally —
the agent/tool layer could obtain *unbounded signing authority*. Codex's "the one thing this most gets
wrong": *"it treats 'agent never sees the seed' as the safety boundary, when the real boundary is whether
the agent/tool layer can obtain an unbounded signing capability over the EOA."* → **resolved by Decision ①B.**

### Dual-voice verdicts (pre-revision)

| Voice | Headline | Verdict |
|---|---|---|
| CEO (Claude) | Engineering-correct but strategically inverted; defers the moat | NO ×4 / PARTIAL premises |
| Eng (Claude) | "Agent never sees the seed" false as architected; isolate the signer; missing error paths + tests | 1 YES / 3 NO / 2 PARTIAL |
| DX (Claude) | Sound prioritization, incomplete operator-experience spec | NO/PARTIAL ×5 |
| Codex (gpt-5.5) | Real boundary is unbounded-signing-capability; pull 7702 forward; define first action | critical ×4 |

### Decision log

| # | Decision | Choice | Principle / source |
|---|----------|--------|--------------------|
| ① | Operator safety boundary | **B** — isolated signer daemon NOW + minimal EIP-7702 chain-enforced limits NOW | User Challenge (all 4 voices); user-confirmed |
| ② | Roadmap spine | **A** — operator-first; floor as cost-of-admission; 6-week demo target | CEO+DX+codex; user-confirmed |
| ③ | Embedded Helios | **B** — keep in NOW (with trusted-checkpoint + visible-fallback invariants) | user override of the demote recommendation |
| ④ | Operator-experience pieces | **A** — promote all (STOP/override, typed refusals, native approval, gas pre-flight, autonomy modes) | DX+Eng+codex; user-confirmed |
| ⑤ | Dapp connectivity | **Allow** WalletConnect/dapp connections; native-desktop form only | codex; applied by default |
| — | Eng build invariants | **Applied** as non-negotiable requirements (atomic writes, fail-closed sim, migration hazard, tests, NOW threat-model) | Eng review |
| — | x402 | Noted **EOA-reachable**, gated only on prioritization (unbundled from AP2/8004) | Eng review |

**Status: APPROVED with revisions applied.** Next step when you're ready to build: `/spec` the first NOW
item (the **process-isolated signer daemon**, the critical-path dependency), or `/ship` once changes land.
