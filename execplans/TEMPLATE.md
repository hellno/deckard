# Title

## 1. Title

Short and specific. Include the GitHub issue number if one exists.

## 2. Context

- What problem are we solving?
- Why does it matter?
- What part of Deckard does it affect?
- Is this product work, security work, CI/workflow work, or discovery?

## 3. Source Of Truth

- User instructions:
- GitHub issue / PR:
- Repo guidance: `AGENTS.md`, `PLANS.md`
- Design guidance, if UI: `DESIGN.md`
- ADRs / docs:
- Relevant code files:
- External standards:

If sources conflict, record the conflict and which source wins.

## 4. Current State Analysis

- What exists today?
- What is incorrect, missing, risky, or unclear?
- What assumptions are unresolved?
- What branch, PR, or issue state matters?

## 5. Target State

- What should be true after the work?
- What behavior should change?
- What should not change?
- What compatibility promises must hold?

## 6. Security And Trust Invariants

- Private keys, seed phrases, passphrases, and decrypted keystore material are never logged,
  `Debug`-printed, copied into screenshots, or written to test artifacts.
- Secrets remain in `Zeroizing` or an equally justified boundary.
- Signer/key ownership boundaries remain intact.
- Wallet RPCs preserve permission, origin, chain, and human-confirmation boundaries.
- Unverified reads are never displayed as verified.
- Real-value chains fail closed unless this plan explicitly changes that rule.

If no security invariant applies, explain why.

## 7. UX And Design Constraints

- `DESIGN.md` constraints:
- Command palette entry:
- Screenshot / visual proof:
- Plain-language copy constraints:

## 8. Execution Plan

1.
2.
3.

For multi-agent work, identify disjoint ownership boundaries by file, crate, or behavior.

## 9. Validation Criteria

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Task-specific checks:

-

If a required validation command cannot be run, explain why and record the residual risk.

## 10. Failure Signals

-

## 11. Risks And Tradeoffs

-

## 12. Out Of Scope

-

## 13. Status Notes

- YYYY-MM-DD: Created plan.
