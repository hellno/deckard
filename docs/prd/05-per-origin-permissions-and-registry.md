# PRD-05 — Per-origin permissions, dapp registry & anti-phishing

> Phase 2 of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). The policy + trust layer that
> makes the Deckard-native bridge (PRD-04) safe: scope each connected origin (accounts × chains ×
> methods), ship a curated registry + anti-phishing blocklist, and treat origin as
> attacker-controllable. Pairs with PRD-04; depends on PRD-02.

## Why this exists

Once a dapp can connect (PRD-04), the wallet must answer "what may *this* origin see and do?" — not
"what may any caller do?". Research (`research §37`) shows the standards: EIP-2255
(`wallet_requestPermissions`/`getPermissions`, the `{invoker, parentCapability, caveats}` object) and
CAIP-25 session scoping (per-scope accounts × chains × methods). And `research §13, 29` shows origin is
**self-asserted and spoofable** — so scoping must be paired with attestation-as-a-hint + a shipped
blocklist, and the human must always approve on decoded effects, not a claimed name.

## Goals

- A **per-origin permission model**: a connected origin is granted a scoped session (which accounts,
  which chains, which methods, optional caps) that the daemon enforces — distinct from the global
  `Policy`. Persisted, revocable, and visible in the UI.
- A **curated dapp registry** (the "allowlist first" posture for generic connectivity): known-good
  origins with metadata (name, verified domain, ERC-7730 descriptor references). Offline-first and
  **signed** if updatable without a release.
- An **anti-phishing blocklist** (known-malicious domains), shipped and updatable, mirroring
  `eth-phishing-detect` (`research §37`).
- Wire-level guards: `wallet_addEthereumChain`/`switchEthereumChain` per `research §36` (sign only with
  user-submitted chainId; confirm requester + target chain).

## Non-goals

- The transport (PRD-04) and the signing decode (PRD-02).
- Building/curating ERC-7730 descriptors upstream — we *reference and consume* them (PRD-02 renders).
- Open access to arbitrary origins as a default — default posture is curated-allowlist; unknown origins
  are allowed only with an explicit, friction-ful "unverified origin" confirmation (never silent).

## Design

### Per-origin policy (`crates/deckard-contract` + `deckard-signerd`)

- Extend the policy layer with an **origin-scoped permission** type: `OriginGrant { origin,
  accounts, chains, methods, caps?, expiry }`. Keep the global `Policy` as the outer fence — an origin
  grant can only ever be *narrower* than the global policy (defense-in-depth; a permissive grant can't
  widen the global caps/allowlist).
- The daemon evaluates an incoming dapp proposal against **both** the origin grant and the global
  policy; the stricter wins. Add a pure `evaluate_origin(&proposal, &OriginGrant, &Policy)` next to the
  existing decision functions (preserve the mock⇄daemon parity charter).
- Grants are created at connect-time (the human approves the requested scope, EIP-2255/CAIP-25 style)
  and are **revocable** — surface revoke in the agent/governance settings alongside the existing
  "Pause all agents" kill switch (`DESIGN.md`).

### Origin attestation (treat as a hint, `research §29`)

- The daemon receives an origin string from the (untrusted) connector/proposer. It is displayed as
  **unverified** unless corroborated by (a) a curated-registry match and/or (b) any first-party
  attestation the bridge can establish (PRD-04). There is no third-party relay attestation (we shelved
  WalletConnect's Verify API along with WalletConnect) — so origin trust leans on the registry +
  blocklist + the always-clear-signed effects, never a remote checkmark.
- The card shows attestation state (verified-domain / unverified / mismatch / known-scam) using
  `DESIGN.md` caution affordances — but the **decoded effects** remain the ground truth the human holds
  to confirm. Never gate solely on a green checkmark.

### Curated registry & blocklist (offline-first, signed)

- **Registry**: in-repo signed JSON of curated origins → metadata + ERC-7730 descriptor refs. Ships
  with the build; if updatable out-of-band, verify a maintainer signature before trust (mirror the
  `policy_store.rs` loud-fallback discipline — a tampered/unsigned list is refused loudly, never
  silently widening trust).
- **Blocklist**: known-malicious domains; a match is a hard, loud block (red), not a soft warning.
  Updatable on the same signed-config mechanism. Default-deny on a blocklist match even if the user
  tries to proceed (this is the one place we override user intent — a known drainer domain).

### ERC-7730 descriptor sourcing

- The registry references which ERC-7730 descriptors apply to a curated origin's contracts so PRD-02's
  card can render human-readable intent; uncovered contracts fall back to PRD-02's explicit blind-sign
  warning.

## Acceptance tests

- `origin_grant_cannot_widen_global`: an origin grant requesting more than the global `Policy` allows is
  clamped to the global fence (stricter wins); assert the effective decision.
- `out_of_scope_method_denied`: a method/chain/account outside the origin grant is denied.
- `blocklist_hard_blocks`: a blocklisted origin is refused even on explicit user "proceed".
- `unsigned_registry_refused`: a tampered/unsigned registry/blocklist is rejected loudly; the wallet
  falls back to the safe built-in (curated-empty = nothing auto-trusted).
- `unverified_origin_requires_friction`: an unknown (not-curated, not-blocked) origin connect requires
  the explicit unverified-origin confirmation path, not a silent allow.
- `addchain_uses_user_chainid`: a `wallet_addEthereumChain` flow signs only with the user-submitted
  chainId; an RPC-returned chainId is never trusted.
- Parity: `evaluate_origin` identical in mock + daemon.

## Definition of Done

PRD-series DoD **plus**: revoke-origin and list-connected-origins are ⌘K commands and appear in
settings governance; registry/blocklist trust model documented in `SECURITY.md`; the
"origin-is-untrusted, effects-are-ground-truth" rule documented in `THREAT-MODEL.md`; UI matches
`DESIGN.md` (caution affordances, danger-early, no green-check-as-safety).

## Risks & fallbacks

- **Curated registry maintenance burden.** Start tiny (the same protocols PRD-03 integrates natively);
  grow by reviewed PR. The registry being small is fine — it's the safe default.
- **Blocklist staleness** (`research §37` counts are directional). It's a backstop, not the primary
  defense; the clear-signing card is. Update cadence documented; signed.
- **Origin spoofing** (`research §13`): contained by never substituting a name for decoded effects, by
  attestation-as-hint, and by the blocklist.

## Sources

`docs/research/10-dapp-connectivity.md §13, 29, 36–37`; eips.ethereum.org eip-2255, eip-3085/3326;
chainagnostic.org CAIP-25; github.com/MetaMask/eth-phishing-detect; existing `policy.rs`,
`policy_store.rs` (loud-fallback discipline).
