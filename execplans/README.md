# Execution Plans

Execution plans are repo-local working notes for non-trivial agent work. They are not backlog items;
GitHub issues remain the backlog and task tracker.

Use this directory when a task needs more durable structure than chat:

- cross-crate implementation
- signing, keystore, wallet RPC, policy, MCP, browser connector, or agent authorization work
- UI work that needs design and screenshot evidence
- CI, release, audit, or dependency-policy changes
- debugging where the cause is not immediately obvious
- multi-session or multi-agent work

Start from [TEMPLATE.md](TEMPLATE.md), then name the plan with the issue number when one exists:

```text
issue-111-keystore-feature-gate.md
issue-093-walletbeat-local-signature-lane.md
debug-signerd-stop-latency.md
```

Keep active plans updated as reality changes. When work finishes, leave the final status notes in the
plan, but move durable architecture, security, workflow, or product decisions into `docs/` or
`docs/adr/`.

To review stale or incomplete plans locally:

```sh
scripts/review-execplans.sh
```
