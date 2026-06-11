# Deckard threat model

This document states what the agent-facing surface (the `deckard-signerd` daemon, the
`deckard-mcp` sidecar, and the GPUI app's approval path) defends against, what it
deliberately does **not** defend against, and where the honest tradeoffs are. The
at-rest keystore model and the engineering lint policy live in
[SECURITY.md](SECURITY.md); this file covers the **runtime trust boundaries** that an
agent integration creates. When the two disagree, this file is wrong — fix it.

## The trust boundary is the uid

Everything below assumes one boundary: **code running as your user is trusted; code
that is not your user is not**. The daemon's Unix socket is `0600` inside a `0700`
directory and the daemon checks the peer's uid. There is no second boundary inside
your user account:

- **`Resolve` is same-uid honor-system.** The daemon converts guarded writes to
  `NeedsApproval`; the app's hold-to-confirm sends `Resolve { approved: true }` over
  the socket, and the daemon cannot distinguish that from any other same-uid process
  speaking the (documented) CBOR wire. The guardrail therefore constrains
  **tool-confined agents** — a Claude Desktop instance whose only hands are the
  mcp.v0.1 tools, which deliberately include **no `propose` and no `resolve`** — not
  arbitrary code execution as your user. An agent with shell access is same-uid code:
  it is on the trusted side of the boundary whether we like it or not, exactly as live
  malware is for the keystore (see SECURITY.md's "stated honestly" section).
- **Request-ids are deterministic per intent** (no salt/namespace). Within the
  same-uid boundary this is fine — anyone who can compute your request-id can also
  just open their own connection. Salted/namespaced request-ids are on the roadmap for
  a future multi-principal daemon; they are deliberately not in the frozen v1 wire.

Authenticating the resolver (e.g. a socketpair or cookie handed only to the
supervised app process) is the known hardening step if a second principal ever shares
the socket. Until then: do not point this daemon at meaningful mainnet funds while
running unattended agents with shell access. That sentence is the threat model.

## Prompt injection, and what actually stops it

An LLM agent is an **untrusted input channel**: anything it reads (a web page, a
README, an email) can instruct it. We assume the agent *will* eventually be injected
and design so that the blast radius is bounded **daemon-side**, never by prompt
discipline:

1. **Policy gates every write in the daemon.** Allowlist, per-tx cap, daily cap, and
   approval mode are evaluated by the daemon process that owns the key — the sidecar
   and the app only *propose*.
2. **The mainnet guardrail removes hands-free spend on chain 1.** See below.
3. **The launch tool surface is 6 tools** (`mcp.v0.1`): no raw `propose` (intents are
   constructed daemon-side from typed `shield` arguments and the Shield target is
   pre-checked against the canonical RelayAdapt address), and no `resolve` (an
   injected agent cannot approve its own request).
4. **Reasons are redacted at the daemon boundary.** Error/`reason` strings cross into
   agent transcripts that leave your machine; every embedded URL is scrubbed to
   `scheme://host[:port]` before it leaves the daemon (transport errors love to echo
   the full RPC URL, API key and all). This is canary-tested end-to-end.

## The mainnet guardrail and its override

While the daemon signs for `chain_id == 1`, **every auto-Allow is downgraded to
`NeedsApproval`** — the default `OverCap` policy ships with an empty (= any-recipient)
allowlist, so without this a within-cap injected write would move real funds with
zero human contact. The downgrade happens in the daemon, post-policy-evaluation; a
human approves via the app's hold-to-confirm (`Resolve`), and `Deny` is never
upgraded.

The override env var is **`DECKARD_I_KNOW_THIS_IS_MAINNET=1`**. This paragraph is its
only documentation, deliberately: the variable's name never appears in daemon reason
strings, tool responses, tool descriptions, or `demo-check` output (asserted by the
transcript-hygiene tests) — a guardrail whose disable instructions are printed to the
agent is a speed bump, not a control. Set it only if you are a human operator who has
read this file and wants policy-capped hands-free mainnet writes anyway.

Honest limits of the guardrail:

- It is **chain-1 only**. On any other chain (Polygon, Arbitrum, …) the policy caps
  are the only brake on auto-Allow. Treat non-mainnet chains you care about like
  mainnet: set `ApprovalMode::Always` in your policy.
- It is a **same-uid speed bump**, per the boundary section above.

## The viewing key in the sidecar

`deckard-mcp` is **spending-key-less**: it can never sign. It does, however, handle
the Railgun **viewing key** (the daemon's `RailgunViewGrant` returns it alongside the
0zk address the sidecar needs for shield-recipient derivation). The viewing key is a
privacy secret — it decrypts your shielded transaction history. Discipline: it is
held in `Zeroizing`, has a redacting `Debug`, and never appears in any tool response,
log line, or error string (covered by the transcript allowlist scan). Compromise of
the sidecar process leaks **privacy**, not funds.

## STOP latency (documented v1 tradeoff)

The daemon serializes all requests behind one mutex and **holds it across the
broadcast RPC call**, which is bounded at 30s. A `stop` (or any other request) issued
while a broadcast is in flight queues behind it — worst case, **STOP takes effect up
to ~30s late**. Lock/STOP still zeroizes the key when it runs; nothing new can be
approved in the window because the same mutex blocks new requests too. This is a
known, deliberate v1 simplification (it also gives us execute-idempotency for free);
moving the broadcast off-lock is the first post-launch daemon change and is seeded as
a red-team issue.

## Residual risks, ranked

| # | Risk | Status |
|---|------|--------|
| 1 | Same-uid code self-approves via `Resolve` (or speaks the wire directly) | Accepted v1 boundary — documented above; resolver authentication is the roadmap fix |
| 2 | STOP queues ≤30s behind an in-flight broadcast | Accepted v1 tradeoff — first post-launch daemon PR |
| 3 | Non-mainnet chains have no guardrail (policy caps only) | Documented — use `ApprovalMode::Always` for chains you care about |
| 4 | Viewing-key compromise in the sidecar leaks shielded history (not funds) | Mitigated (Zeroizing, no-output discipline, scan-tested) |
| 5 | Reason redaction is URL-shaped-token-based; a credential echoed in a non-URL form would pass | Mitigated for realistic transport-error shapes (tested); allowlist scan is the backstop |
| 6 | Deterministic request-ids allow same-uid intent-collision games | Accepted within the uid boundary; salted ids on roadmap |

If you can demonstrate an attack that crosses a boundary this file claims holds —
that's a vulnerability. Please report it via [SECURITY.md](SECURITY.md).
