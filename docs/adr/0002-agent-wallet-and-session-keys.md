# ADR 0002 — Agent wallet and session keys

- **Status:** Proposed (2026-06-14), then **Deferred (2026-06-15, `/plan-ceo-review`)** — not before the
  first agent demo runs and an audit. See [`docs/agent-authorization-map.md`](../agent-authorization-map.md)
  → Current focus. The positioning below stands; the timing does not.
- **Deciders:** @hellno (maintainer)
- **Context inputs:** [`docs/agent-authorization-map.md`](../agent-authorization-map.md),
  [`docs/research/05-agentic-wallets.md`](../research/05-agentic-wallets.md), `THREAT-MODEL.md`,
  `MANIFESTO.md`, `DESIGN.md`, `docs/build/30-mcp-shape.md`, issue
  [`#33`](https://github.com/hellno/deckard/issues/33) (EIP-7702 session keys),
  [`#31`](https://github.com/hellno/deckard/issues/31) (wire evolution).
- **Supersedes / relates:** records the positioning behind the "agent wallet" goal. The executable
  work lives as GitHub issues (`#33`, `#31`, and a sizing spike drafted alongside this ADR).
  Convention: work items live in issues; decisions + research live in `docs/`.

## Decision drivers (agreed with the maintainer)

Captured via a requirements pass (2026-06-14):

1. **A distinct agent-controlled wallet is a real product goal**, not just framing. The motivating
   capability is the agent having its **own address** and its **own funded float**, with
   **blast-radius isolation**: a runaway or compromised agent can drain only the float, never the
   user's main balance.
2. **The mechanism is EIP-7702 session keys** (`#33`): the user's wallet funds and authorizes a
   **scoped grant** (per-tx cap, expiry, allowlist) to the agent's key. The agent wallet is not a
   second independent EOA the user tops up by hand; it is an authority the user's wallet delegates.
3. **v1 is software-enforced, consistent with the rest of v1.** Per the v1 honesty decision
   (2026-06-14), v1 limits are enforced by the daemon's policy gate plus human approval, not by the
   chain. Chain-enforcement is the post-audit upgrade, not a v1 claim.
4. **Staging is left open, to be settled by a spike.** Before committing to "software agent wallet
   now" vs "jump straight to `#33`", size `#33` (backend selection, audited-contract dependency, the
   `#31` prerequisite, effort). The spike decides.

Plus the standing constraint from `SECURITY.md`: Deckard is `0.0.1-alpha`, unaudited, single-maintainer,
testnet-keys-only. Any new signing authority is sequenced behind an audit, and `MANIFESTO.md` already
commits to this ("Hands-off autonomy comes after a security audit ... not before").

## Context

Today Deckard runs **model 1**: a single human key lives in `deckard-signerd`; the agent is a
**key-less proposer** that submits typed `Intent`s over MCP; the daemon's policy gate decides; the
human approves via a native hold-to-confirm card. There is no agent key, no agent address, no float.
The trust spine that makes this safe is built and merged (`#4`, `#45` resolver authentication, `#28`
deny vocabulary, `#27` agent surface).

An "agent wallet" is **model 2**: a second, lesser key with its own scoped authority. This is the
single largest item in the backlog and currently exists only as plan (`#33`). It depends on `#31`
(capability discovery on the frozen wire), an on-chain delegation contract that must be on a hardcoded
allowlist of audited deployments, and a backend choice (MetaMask Delegation Framework vs Porto/Ithaca).

## Decision

1. **Adopt the agent wallet as a goal, delivered through `#33` session keys.** The agent gains a
   distinct address and a scoped, revocable float granted by the user's wallet.
2. **v1 enforcement is software.** The daemon holds the agent key (a second keystore class) and
   refuses to sign anything outside the granted scope. On mainnet, the existing guardrail
   (`THREAT-MODEL.md`: chain-1 auto-Allow downgrades to `NeedsApproval`) still applies, so hands-free
   autonomy is real on testnet/fork now and gated on mainnet until chain-enforcement lands.
3. **Chain-enforcement is the post-audit upgrade.** EIP-7702 delegation to an audited contract makes
   the scope physically unbreakable. Same agent address; software caps swap for chain-enforced ones.
4. **Staging is decided by a spike, not now.** Run a sizing spike for `#33` (see the drafted issue).
   Its report chooses between "software agent wallet first" and "jump to `#33`", and records the
   backend selection and the `#31` dependency.

## Invariants this must respect (non-negotiable, from existing locked decisions)

- **The thinking agent stays key-less.** The MCP/proposer layer never holds either key. Only the
  daemon holds keys. The agent wallet is a *daemon-held* key the agent can request signatures from
  within scope, never a key in the agent process. (`THREAT-MODEL.md`, `30-mcp-shape.md`.)
- **Resolver authentication is unchanged.** Approvals are honored only on the app's capability channel
  (`#45`). A second key class does not add a second approval path.
- **STOP zeroizes everything.** `RevokeAll` zeroizes both the human key and the agent session key and
  denies in-flight approvals. Revoke of a session grant zeroizes the local agent key.
- **Wire changes are additive.** New request kinds ship as capabilities under `#31`'s `Hello`
  mechanism, preserving byte-stable round-trip (`#28` rules). Never sign `chain_id = 0`; the delegate
  contract must be on a hardcoded versioned allowlist of audited deployments.
- **Two-signal model holds.** The agent wallet is cyan (agent class); approvals and "where you are"
  stay amber (`DESIGN.md`).

## Consequences

- **Positive:** isolates blast radius; gives the agent a real identity for attribution and receiving;
  delivers the manifesto's "autonomy is the point" on an honest, audit-gated path.
- **Cost:** the policy gate becomes per-account (the agent account carries its own float and scope), a
  modest but real extension of `Policy`. A second keystore class adds key-lifecycle surface
  (grant, rotate, revoke, expiry).
- **Deferred:** chain-enforcement, multi-account agent fleets, and any agent-to-agent delegation are
  out of scope here. So is exporting a grant to an external agent runtime
  (`smart-account-autonomous` mode), per `#33`.

## Status / next step

**Deferred (2026-06-15, `/plan-ceo-review`).** The first agent demo (auto-shield, LLM in the loop) uses
the authorization that already exists (software policy + approval + STOP), so an agent wallet is not needed
yet. The earlier "spike `#33` first" plan is reversed. Revisit after that demo runs and an audit. When
picked back up: run the `#33` sizing spike, settle the staging decision, then promote this ADR to Accepted.
