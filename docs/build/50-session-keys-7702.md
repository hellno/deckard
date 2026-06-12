# EIP-7702 Session Keys — Chain-Enforced Agent Limits, In Public

> The roadmap's minimal-7702 NOW item, specified: the user's EOA reversibly delegates to an
> audited contract so a scoped agent key has **chain-enforced** caps/expiry/allowlists — and the
> work is done *in public* (spec, revocation demo beat, Walletbeat contribution) because the
> verified white space says nobody in the EF orbit has shipped this · status (spec, backend
> decision deferred per 2026-06-11 review) · researched 2026-06-11. Part of the Deckard build docs.

## Why this exists (2-4 sentences, concrete)

Today every limit the agent operates under is enforced by the daemon's policy gate — software on
the same machine. Decision ①B (roadmap) requires the honest claim to become "a compromised agent
**cannot exceed its on-chain limits**", which needs the chain, not the daemon, to be the fence.
The UX bar is set by Tempo's protocol-native access keys ("authorize once; matching transfers
sign without prompts; expiry; per-token limits") — this doc specifies the same experience on
Ethereum via EIP-7702, **without a 4337 stack or bundler** (verified feasible, two independent
stacks). Verified white space: Kohaku has no 7702/session-key work, eth-infinitism ships no
scoped-key contract, and Walletbeat marks scoped-permission rating as future work — first-mover
room, with a clock running (MetaMask's Advanced Permissions + "Agent Wallet" entered Early Access
2026-06-08).

## Where it sits — Depends on / Unblocks

**Depends on**
- `40-contract-evolution.md` — `SessionGrant`/`SessionRevoke` ship as capability
  `session-keys-7702`, additive on the frozen wire.
- `30-mcp-shape.md` — the approval-card flow; the grant card is this spec's central amber moment.
- The policy gate (`deckard-signerd`) — local policy remains the *first* fence; the chain becomes
  the second. Both fences carry the same numbers (see invariants).

**Unblocks**
- The `smart-account-autonomous` autonomy mode (roadmap ④) — the only mode whose safety claim
  survives daemon compromise.
- `60-x402.md` later phases — x402's exact/EVM scheme lists **ERC-7710 delegation** as a
  settlement method, so the delegation stack chosen here can carry machine payments too.
- The public-proof artifacts: this spec and the on-camera revocation beat. (A Walletbeat upstream
  contribution proposing the session-key attribute the rubric lacks was considered and **cut
  2026-06-12** — revisit once the beat is shipped evidence.)

## Architecture / approach

```
        amber (human, once)                              cyan (agent, repeatedly)
┌──────────────────────────────┐              ┌─────────────────────────────────────────┐
│ 1. EOA signs 7702 auth →     │              │ 3. agent asks daemon to act; daemon     │
│    audited delegate contract │              │    policy gate = fence #1 (unchanged)   │
│ 2. EOA signs a GRANT for the │              │ 4. session key sends a PLAIN, gas-paid  │
│    session key: caps, expiry,│              │    tx; the delegate contract checks     │
│    target allowlist          │              │    caps/expiry/allowlist = fence #2     │
└──────────────────────────────┘              │    (holds even if fence #1 is gone)     │
        revoke: one on-chain tx               └─────────────────────────────────────────┘
        un-delegate: 7702 auth → address(0)       no EntryPoint, no UserOp, no bundler
```

**Backend-abstract by decision (2026-06-11):** the spec freezes grant/revoke/execute *semantics*;
two verified stacks conform, and the implementation issue picks one after deeper due diligence:

| | MetaMask Delegation Framework (ERC-7710) | Porto / IthacaAccount |
|---|---|---|
| Delegate contract | `EIP7702StatelessDeleGator` (stateless; EOA key stays root) | `IthacaAccount` (key registry in account storage) |
| Grant | **off-chain EIP-712 delegation** + caveat enforcers (38 shipped: ERC-20 amount/period/streaming, native streaming, timestamp, allowed targets/methods/calldata, value-LTE, limited-calls, nonce…) | **on-chain** `authorize(Key)` + `setCanExecute` + `setSpendLimit` (periods Minute…Forever) |
| No-bundler execution | `DelegationManager.redeemDelegations` — permissionless, caveat-checked, session EOA pays gas (verified in source + docs) | ERC-7821 `execute` third path: `opData = nonce ++ wrapped sig`, guarded by `canExecute` + spend limits (verified in source) |
| Revoke | `disableDelegation` (delegator-only, on-chain) | `revoke(keyHash)` (self-call by super-admin key) |
| Assurance | **12 firm audit reports** (Cyfrin ×7, ConsenSys Diligence ×5, 2024→2026-05) | 2-week individual-researcher review; bug bounty ≤ 5 ETH; **no firm report found** ⚠ |
| State legibility | wallet must track signed delegation blobs | keys/limits/expiry are readable chain state the app can mirror |
| Standards | ERC-7710/7715 (Draft; 7715's 2026-01 revision converges on 7710 redemption) | ERC-7821 + bespoke |

Selection criteria frozen now: (1) assurance evidence at impl time (a Porto firm audit would
reopen the question), (2) deployment availability on Sepolia + mainnet, (3) grant legibility in
the amber card, (4) reuse for x402's 7710 settlement path.

## Concrete interface (additive, capability `session-keys-7702`)

```rust
// deckard-contract — new requests (wire details final at impl; semantics frozen here)
SessionGrant {
    scope: SessionScope,                  // the human-readable, card-rendered grant
}                                         // -> Decision (ALWAYS NeedsApproval; never auto)
SessionRevoke { key_id: KeyId },          // -> Ack (on-chain revoke + local kill)

struct SessionScope {
    per_tx_cap_wei:  U256,                 // the ONE spend cap in v1 (decision 2026-06-12)
    expiry_unix:     u64,                  // REQUIRED; no unbounded grants, ever
    allow_to:        Vec<Address>,         // REQUIRED non-empty; no any-target grants in v1
    token:           Option<Address>,      // None = native; Some = single ERC-20
}
// Deferred from v1 (cut 2026-06-12, both backends support them — pure additions later):
// period caps (e.g. 0.2 ETH / Day) and per-method (selector) granularity. expiry + allow_to are
// NOT caps and NOT cuttable: a grant without them is unbounded authority, the anti-pattern.
```

- The **session key is generated and held by the daemon** in v1 (a second, lesser keystore
  class). Local policy stays fence #1; the chain grant is fence #2 — defense in depth with one
  set of numbers. Exporting a grant to an *external* runtime (the key leaves the machine, only
  fence #2 holds) is the explicit LATER that unlocks `smart-account-autonomous` mode — out of
  scope here, but the semantics above are written so it needs no wire change.
- **Delegation is pinned:** the daemon signs a 7702 authorization only for delegate addresses on
  a hardcoded, versioned allowlist (the chosen stack's audited deployment). User-supplied
  delegate addresses are rejected unconditionally — the dominant real-world attack is
  malicious-delegate phishing (Wintermute: >97% of early delegations were sweeper contracts).
- The grant card renders: delegate contract identity + version, every `SessionScope` field, and
  "revoke costs one transaction" — the legibility requirement from the roadmap ("what am I
  delegating to").

## Invariants (frozen here)

- **Never sign a 7702 authorization with `chain_id = 0`** (valid on all chains by spec). Each
  authorization is chain-pinned; the mainnet guardrail applies to grants like any write.
- Authorization nonce semantics: self-sponsored delegation signs `nonce + 1` (the tx consumes the
  current nonce). Captured authorizations are includable by anyone — the daemon treats a signed
  authorization as live until landed or invalidated.
- **Un-delegation (`address(0)`) clears code, not storage.** Re-delegating to a *different*
  implementation is refused unless the implementations are storage-compatible (ERC-7201
  namespacing) — checked against the pinned allowlist metadata, not user judgment.
- `SessionGrant` is `NeedsApproval` always — no policy mode auto-grants authority. `RevokeAll`
  (STOP) additionally queues the on-chain revoke for every live grant.
- Local fence #1 is never widened to match fence #2: the daemon enforces
  `min(local policy, session scope)`.
- Grant/revoke/expiry events are append-only audit-log entries.

## In-public deliverables (the point of this doc)

1. This spec, public in-repo (done by merging it).
2. The **revocation beat**: on the demo fork, the agent's session key transacts within the cap →
   over-cap attempt **reverts on-chain** → human revokes (one tx) → next attempt reverts —
   recorded as the session-keys sibling of the Helios walk-away demo.
3. KB correction (file `01-landscape-2026.md`): MetaMask Advanced Permissions did **not** ship
   2026-04-06; Early Access opened **2026-06-08** (alongside "MetaMask Agent Wallet"), GA targeted
   summer 2026.

(Cut 2026-06-12: the Walletbeat rubric contribution — revisit with shipped evidence.)

## Acceptance test (agent-runnable asserts, anvil Sepolia fork)

```
Scenario "chain-enforced leash" (chosen backend deployed on the fork):
  S1 SessionGrant{cap 0.05 ETH, expiry 24h, allowlist [A]}
                                  assert: Decision == NeedsApproval; card lists delegate
                                          identity + every scope field; approve → grant live
  S2 session key → plain EIP-1559 tx within the cap to A (no bundler process anywhere)
                                  assert: lands; gas paid by session EOA
  S3 over-cap attempt             assert: REVERTS on-chain (fence #2, not a daemon Deny)
  S4 target B ∉ allowlist         assert: reverts on-chain
  S5 warp past expiry; retry      assert: reverts on-chain
  S6 revoke (one tx); retry S2    assert: reverts on-chain; local key zeroized
  S7 un-delegate → address(0)     assert: EOA code empty; plain sends work as before
  S8 grant request for a delegate NOT on the pinned allowlist
                                  assert: Deny (frozen-vocabulary reason), nothing signed
  S9 authorization request with chain_id=0
                                  assert: unconditionally refused, nothing signed
  S10 transcript scan (S1..S9)    assert: no key bytes, no passphrase (the 30-mcp T9 gate)
```

## Risks & fallbacks

- **Backend ambiguity costs time at impl.** *Mitigation:* selection criteria frozen above; the
  semantics are backend-neutral so the spec doesn't churn with the choice.
- **DTK singleton dependency / Porto audit gap** — the two failure modes are different (supply
  chain vs assurance). *Fallback:* criteria (1) explicitly reopens on new audit evidence.
- **Sepolia deployment availability** at impl time. *Fallback:* deploy the chosen stack's
  contracts to the demo fork from source (both are open source); mainnet waits for canonical
  deployments regardless.
- **Gas UX for the session EOA** (it pays its own gas; "low-gas pre-flight" applies to it too).
  *Fallback:* the daemon funds the session EOA at grant time from the grant card (visible,
  capped).
- **7702 ecosystem churn** (7715 was rewritten 2026-01). *Mitigation:* we consume contracts, not
  the RPC drafts; 7715's request shape is prior art for `SessionScope`, not a dependency.

## Open questions

- Funding the session EOA: at grant time (simple, visible) or lazily per low-gas pre-flight?
- Should `RevokeAll`'s queued on-chain revokes broadcast even while the vault is locked
  (pre-signed revoke txs), or is "revoke on next unlock" honest enough for v1?

## Sources

- EIP-7702 (revocation via `address(0)` clears code not storage; chain_id 0 universal validity;
  nonce semantics; authorizations not tx-bound; ERC-7201 recommendation) —
  https://eips.ethereum.org/EIPS/eip-7702 — (spec, high)
- MetaMask Delegation Framework (DelegationManager `redeemDelegations` permissionless — verified
  in source; 38 enforcers; `EIP7702StatelessDeleGator`; audits dir: cyfrin ×7 + diligence ×5;
  `disableDelegation`) — https://github.com/MetaMask/delegation-framework — (github, high);
  EOA-delegate plain-tx redemption confirmed in docs —
  https://raw.githubusercontent.com/MetaMask/metamask-docs/main/smart-accounts-kit/guides/advanced-permissions/execute-on-metamask-users-behalf.md — (docs, high)
- Porto / IthacaAccount (key registry: `Key{expiry,keyType,isSuperAdmin}`; GuardedExecutor
  `canExecute` + `setSpendLimit` periods Minute…Forever; ERC-7821 external-caller path
  `opData = nonce ++ sig` — verified in source; `revoke(keyHash)`; no 4337 anywhere; audit =
  named individual researchers, no firm report found ⚠) —
  https://github.com/ithacaxyz/account — (github, high); relay (Rust, MIT/Apache, optional) —
  https://github.com/ithacaxyz/relay — (github, high)
- Non-conforming for the no-bundler path (EntryPoint-gated execution): Simple7702Account —
  https://raw.githubusercontent.com/eth-infinitism/account-abstraction/develop/contracts/accounts/Simple7702Account.sol ;
  Kernel v3 — https://github.com/zerodevapp/kernel ; Nexus — https://github.com/bcnmy/nexus — (github, high)
- ERC-7710 / ERC-7715 (both Draft; 7715 revised 2026-01-16: `wallet_requestExecutionPermissions`,
  BasePermission+rules, response carries `delegationManager`) —
  https://eips.ethereum.org/EIPS/eip-7710 · https://eips.ethereum.org/EIPS/eip-7715 — (spec, high)
- Footgun evidence: Wintermute "CrimeEnjoyor" (>97% of early delegations malicious sweepers) —
  https://www.coindesk.com/tech/2025/06/02/post-pectra-upgrade-malicious-ethereum-contracts-are-trying-to-drain-wallets-but-to-no-avail-wintermute — (news, medium);
  Inferno Drainer 7702 batch-execute phish — secondary reporting ⚠; init front-running —
  https://www.fireblocks.com/blog/security-first-approach-to-eip-7702 — (vendor, medium);
  OpenZeppelin EOA-delegation notes + `EIP7702Utils` —
  https://docs.openzeppelin.com/contracts/5.x/eoa-delegation — (docs, high)
- White space: Kohaku packages (no 7702/session keys) — https://github.com/ethereum/kohaku —
  (github, high); Walletbeat `permissions-management` ("complex account permissions" = future
  work) —
  https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/self-sovereignty/permissions-management.ts — (github, high)
- Clock: MetaMask Advanced Permissions + Agent Wallet EAP opened 2026-06-08 (GA summer 2026) —
  https://metamask.io/news/introducing-advanced-permissions — (blog, medium ⚠ via search excerpts;
  site bot-blocked)
- Tempo access-key UX bar — `docs/research/10-tempo-accounts.md` (this repo)
