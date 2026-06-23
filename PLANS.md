# PLANS.md

## Purpose

This document defines how execution plans are created, maintained, and completed in Deckard.

Execution plans are working artifacts for agents. They are used to make non-trivial work legible
across Codex, Claude Code, humans, and future sessions. A good plan should let another agent resume
the task without reconstructing the problem from chat history.

Plans serve as:

- design notes for the task at hand
- execution checklists
- validation checklists
- decision and status logs
- a way to keep security and trust-boundary assumptions explicit

Plans are not a replacement for GitHub issues, ADRs, research docs, or permanent product docs.

## Plan Location

All execution plans live in:

```text
execplans/
```

Start from `execplans/TEMPLATE.md`. Use lowercase, hyphenated filenames that include the issue
number when one exists:

```text
execplans/issue-111-keystore-feature-gate.md
execplans/walletbeat-local-signature-lane.md
execplans/debug-signerd-stop-latency.md
```

Keep the directory flat until it becomes noisy. If it grows large, split it into `active/` and
`completed/` in a separate cleanup change.

Use `scripts/review-execplans.sh` to list stale or incomplete plans during local cleanup.

## Relationship To Issues And Docs

- GitHub issues are the backlog and product/task tracker.
- `execplans/` files are task-local execution memory.
- `docs/` contains durable research, specs, and operational documentation.
- `docs/adr/` contains durable architectural decisions.

If a plan uncovers knowledge that should outlive the task, move that knowledge into the appropriate
durable doc and link to it from the plan. Do not let an execution plan become the only copy of a
security invariant, architecture decision, workflow rule, or user-facing product decision.

## Plan Ownership

Plans are written and maintained by agents.

The user may seed direction, correct misunderstandings, or approve scope changes, but the agent doing
the work is responsible for:

- creating the plan when required
- updating the plan when reality changes
- keeping status notes accurate
- recording validation evidence
- marking completion honestly

When two agents are working nearby, the plan must make ownership boundaries clear enough to avoid
conflicting edits.

## When A Plan Is Required

Create or update an execution plan before editing when the task involves any of the following:

- cross-crate changes
- `deckard-core`, `deckard-contract`, `deckard-signerd`, key management, signing, policy, or wallet RPCs
- private keys, seed phrases, passphrases, encrypted keystore behavior, or `Zeroizing`
- wire contract changes or backward/forward compatibility concerns
- browser connector, MCP, agent authorization, or dapp-provider behavior
- UI/visual work in `crates/deckard-app`
- CI, release, audit, or dependency policy changes
- security hardening or threat-model work
- debugging where the cause is not immediately obvious
- work expected to span multiple steps, sessions, or agents
- performance work or reliability work

If the task is security-sensitive, create a plan. If unsure, create a short plan.

## When A Plan Is Not Required

A plan is usually unnecessary for:

- typo fixes
- one-line comments or copy edits
- mechanical formatting
- small README updates that do not change workflow, behavior, security posture, or product claims
- pure issue triage with no repo edits

Even for small work, create a plan if the task touches secrets, signing, trust boundaries, or user
funds.

## Required Plan Structure

Every plan should use this structure. Sections may be brief, but do not omit the security and
validation sections for implementation work.

### 1. Title

Short and specific. Include the GitHub issue number if one exists.

### 2. Context

- What problem are we solving?
- Why does it matter?
- What part of Deckard does it affect?
- Is this product work, security work, CI/workflow work, or discovery?

### 3. Source Of Truth

List the references used, such as:

- user instructions
- GitHub issue or PR links
- `AGENTS.md`
- `CLAUDE.md`
- `DESIGN.md`
- relevant `docs/` or `docs/adr/` files
- relevant crate files
- external standards or EIPs, if used

If sources conflict:

- user instructions win for scope and priority
- `docs/adr/` wins for accepted architecture decisions
- code and tests win for current behavior
- security invariants win over convenience

### 4. Current State Analysis

- What exists today?
- What is incorrect, missing, risky, or unclear?
- What assumptions are unresolved?
- What branch, PR, or issue state matters?

### 5. Target State

- What should be true after the work?
- What behavior should change?
- What should not change?
- What compatibility promises must hold?

### 6. Security And Trust Invariants

Explicitly state the invariants that must remain true. Include any that apply:

- private keys, seed phrases, passphrases, and decrypted keystore material are never logged,
  `Debug`-printed, copied into screenshots, or written to test artifacts
- secrets remain in `Zeroizing` or an equally justified boundary
- the signer daemon remains the only process that owns key material when the task depends on that
  isolation
- key-less crates and binaries do not gain decrypt/sign capability accidentally
- wallet RPCs preserve permission, origin, chain, and human-confirmation boundaries
- agent actions remain observable and stoppable where the product requires it
- wire-contract changes are additive or explicitly versioned
- unverified reads are never displayed as verified
- real-value chains fail closed unless the task explicitly changes that rule

If no security invariant applies, say why. Do not leave this section blank.

### 7. UX And Design Constraints

Required for UI work; optional otherwise.

- What `DESIGN.md` constraints apply?
- What command-palette entry is required?
- What before/after screenshots or visual proof must be attached to the PR?
- What user-facing language needs to stay plain and non-jargony?

### 8. Execution Plan

List concrete steps in order. Keep steps small enough that another agent can resume from the middle.

For multi-agent work, identify disjoint ownership boundaries by file, crate, or behavior.

### 9. Validation Criteria

Define how correctness is proven. Include exact commands and any scenario checks.

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Also include any task-specific validation:

- focused unit or integration tests
- Anvil/fork tests
- browser extension or WalletBeat QA lanes
- screenshots for GUI changes
- wire-format round-trip tests
- security regression tests
- dependency or feature-closure checks

If a required validation command cannot be run, explain why and record the residual risk.

### 10. Failure Signals

Define what indicates the work is wrong, incomplete, or unsafe. Examples:

- a key-less binary links keystore/decrypt code
- an unverified chain renders as verified
- a GUI action is not reachable from the command palette
- a STOP path fails to revoke or deny in-flight work
- a wire-contract round trip breaks
- CI passes only because a required check is missing

### 11. Risks And Tradeoffs

- What could go wrong?
- What assumptions might be false?
- What is intentionally deferred?
- What would require user approval before continuing?

### 12. Out Of Scope

State what this plan does not include. This keeps agents from silently expanding the task.

### 13. Status Notes

Append timestamped notes as work progresses. Include what changed, what was verified, what failed,
what assumptions changed, and what remains.

Use concise entries, for example:

```text
- 2026-06-23: Created plan from #111 after inspecting `deckard-core/src/lib.rs` and ADR 0003.
- 2026-06-23: Implemented feature gate; `cargo fmt --all --check` and `just core` passed.
- 2026-06-23: Full `cargo test --workspace` not run locally because ...; PR must run CI before merge.
```

## Execution Rules

- Follow the plan; update it when reality changes.
- Stop and revise the plan if core assumptions break.
- Do not treat a stale plan as authoritative.
- Do not store secrets, raw private data, or unsafe screenshots in a plan.
- Keep plans tied to the task; move durable conclusions into `docs/` or `docs/adr/`.
- Keep GitHub issue state and plan status consistent when both exist.

## Completion Criteria

A plan is complete when execution steps are finished or intentionally deferred, validation criteria are
satisfied or explicitly reported as not run, security and trust invariants still hold, status notes
reflect the final state, and durable knowledge has been moved out of the plan into the correct doc.

## Philosophy

Deckard is a self-custodial wallet. Agent speed is useful only when the repo remains legible,
verifiable, and conservative around keys, user funds, and trust boundaries.

Plans should make the work smaller, safer, and easier to resume. If a plan becomes ceremony, shorten
it. If an agent has to rediscover context from chat, issues, or memory, improve the plan.
