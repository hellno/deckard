# Deckard — Agent Authorization (the thinking)

> The *why* and the *shape* of how Deckard's agent stays bounded, and what we are building first.
> This doc holds the thinking, not the task list. Work items live in **GitHub issues**, decisions live in
> **`docs/adr/`**, build state lives in **`STATUS.md`**. When those move, this doc does not need to.
>
> Last reconciled: **2026-06-24** (policy model — ADR-0005).

## What we want (the thesis)

Your money on autopilot, and you can see and stop everything. The agent reads, proposes, and explains.
Only the signer moves money, and every move passes a policy gate you control. So the agent is a **key-less
proposer**: it never holds the key, the daemon does, and you approve by hand. That spine is built
(process-isolated signer, policy gate, capability-gated approval, STOP).

## Three authority models (don't conflate them)

The threads that sound tangled ("agent permissions", "agent wallet", "session keys", "the activity
screen") are three different things wearing similar words:

1. **Key-less proposer (today).** The agent proposes typed intents, you approve, the daemon signs with
   *your* key. Limits are software-enforced (the policy gate) plus your approval. **This is all of v1.**
2. **Agent wallet / session keys.** A distinct agent-controlled address with its own float and
   blast-radius isolation, granted scoped authority via EIP-7702 session keys. Software-enforced first,
   chain-enforced after audit. Positioning and rationale: **ADR-0002**.
3. **Per-origin grant.** Scoped permission for a *dapp origin*, not the agent. Part of dapp connectivity.
   Decision: **ADR-0001**.

"Agent wallet" and "session keys" are the same future thing (model 2). Model 3 is about dapps, a different
principal. Keep them apart.

## Current focus — what we are building first

We have never watched an agent do a real task here, so the backlog was inverted: autonomy got ticketed,
the see-and-stop loop did not. The fix is to run one concrete task on the authorization that already
exists, learn, then decide what is next.

**First demo: the agent keeps your incoming funds private.** Claude watches the balance, shields new funds
within your cap, and you see and stop it in an activity view. The LLM is in the loop on purpose, so we
learn how the agent plus approval loop actually feels.

- Activity feed + see-and-stop view: **#60**
- Agent loop (poll balance, propose shield): **#61**

It uses only what exists: the policy gate (`auto_shield_min`, `OverCap`), the auto-approval guardrail, STOP. No
new authority.

## What is deferred (and why)

Everything that adds agent *authority* or *reach* waits until the first demo runs and teaches us
something. Building authorization for money we have never moved is premature.

- **Agent wallet / session keys** — ADR-0002 (#33).
- **Machine payments (x402)** — #32, #34.
- **Dapp connectivity / owned bridge** — ADR-0001 (epic #44). Also audit-gated.

This reverses an earlier "spike #33 first" call, on purpose.

## How the agent's Rules are shaped (ADR-0005)

The policy gate above is moving off a single flat struct onto a small, default-deny, **per-action rule
list** — the user-facing **"Rules."** Decision and rationale: **ADR-0005**.

- **Default-deny, per action.** Each rule names an action (send / shield / swap …) and its limits (cap,
  recipients, approval mode). No matching rule ⇒ denied. Adding a capability is one rule, not a new
  field on a god-struct.
- **Caps are native `uint256`.** No general policy engine (Cedar, OPA, Biscuit) enforces the spend caps:
  all are 64-bit-limited and cannot represent wei (verified 2026-06-23). We enforce in Rust and keep
  `evaluate` one pure function (the mock⇄daemon parity contract).
- **Vocabulary, not architecture (PEP/PDP/PAP/PIP).** The gate is the PEP, `evaluate` the PDP, authoring
  the PAP (deferred), balances/`spent_today`/sim the PIP — borrowed from NIST SP 800-162 for naming only.
- **One vocabulary, two enforcers.** Models 2 and 3 above (session keys, per-origin grants) reuse the
  Rules *vocabulary* (cap / allowlist / expiry / mode) but stay **distinct enforcers** — software
  (daemon) vs chain (7702). Never collapse them; software is not "cannot exceed."

## Invariants any future agent work must respect

Grounded in `THREAT-MODEL.md` and `DESIGN.md`. Non-negotiable:

- **The thinking agent stays key-less.** Only the daemon holds keys.
- **One approval path.** Approvals are honored only on the app's capability channel (resolver auth).
- **v1 limits are software-enforced, not chain-enforced.** Do not claim "cannot exceed" until the chain
  does.
- **STOP zeroizes the key and denies in-flight work.** Always reachable.
- **Two-signal model:** amber is human, cyan is agent. Identity is a glyph, never the only signal.
