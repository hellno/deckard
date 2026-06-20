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

- **`Resolve` is gated by an unforgeable capability (PRD-01), not the honor system.**
  The daemon converts guarded writes to `NeedsApproval`; approval
  (`Resolve { approved: true }`) is honoured **only** on the private channel the daemon
  inherits from the app that supervises it — a `socketpair` end passed at spawn
  (`supervise.rs` mints it, `server.rs` serves it, `daemon.rs` gates on it). A `Resolve`
  on the public proposer socket — any *other* same-uid process speaking the documented
  CBOR wire — is refused with `resolve_not_authorized`. So a same-uid proposer (including
  a fully compromised MCP sidecar) can propose and read, but it **cannot self-approve**;
  the mcp.v0.1 tool surface still deliberately includes **no `propose` and no `resolve`**
  (defense in depth). The honest residual: an agent with arbitrary **shell** access is
  same-uid code that can reach the app's inherited fd anyway (ptrace, `/proc/<pid>/fd`) —
  it is on the trusted side of the boundary, exactly as live malware is for the keystore
  (see SECURITY.md's "stated honestly" section). The capability closes the *cheap,
  wire-level* self-approve that any same-uid process could previously do; it does not
  fence off arbitrary code execution as your user.
- **Request-ids are deterministic per intent** (no salt/namespace). Within the
  same-uid boundary this is fine — anyone who can compute your request-id can also
  just open their own connection. Salted/namespaced request-ids are on the roadmap for
  a future multi-principal daemon; they are deliberately not in the frozen v1 wire.

Authenticating the resolver — a `socketpair` capability handed only to the supervised
app — is now **implemented** (PRD-01): the public socket can no longer approve. The
standing caution is therefore narrower: do not point this daemon at meaningful mainnet
funds while running unattended agents with **shell** access, because same-uid code can
still reach the app's capability fd (ptrace / `/proc`). That sentence is the threat
model.

## Prompt injection, and what actually stops it

An LLM agent is an **untrusted input channel**: anything it reads (a web page, a
README, an email) can instruct it. We assume the agent *will* eventually be injected
and design so that the blast radius is bounded **daemon-side**, never by prompt
discipline:

1. **Policy gates every write in the daemon.** Allowlist, per-tx cap, daily cap, and
   approval mode are evaluated by the daemon process that owns the key — the sidecar
   and the app only *propose*.
2. **The auto-approval guardrail removes hands-free spend on every real-value chain.** See below.
3. **The launch tool surface is 6 tools** (`mcp.v0.1`): no raw `propose` (intents are
   constructed daemon-side from typed `shield` arguments and the Shield target is
   pre-checked against the canonical RelayAdapt address), and no `resolve` (an
   injected agent cannot approve its own request).
4. **Reasons are redacted at the daemon boundary.** Error/`reason` strings cross into
   agent transcripts that leave your machine; every embedded URL is scrubbed to
   `scheme://host[:port]` before it leaves the daemon (transport errors love to echo
   the full RPC URL, API key and all). This is canary-tested end-to-end.

## The auto-approval guardrail and its override

The guardrail is **default-deny**: while the daemon signs for **any real-value chain** —
every chain EXCEPT an explicit exempt allowlist of testnet/dev ids (Sepolia `11155111`,
local anvil `31337`) — **every auto-Allow is downgraded to `NeedsApproval`**. An UNKNOWN
chain-id is treated as real-value and guarded too, so configuring a new real chain (Base,
OP, …) can never silently turn the brake off. The default `OverCap` policy ships with an
empty (= any-recipient) allowlist, so without this a within-cap injected write would move
real funds with zero human contact. The downgrade happens in the daemon,
post-policy-evaluation; a human approves via the app's hold-to-confirm (`Resolve`), and
`Deny` is never upgraded.

The override env var is **`DECKARD_I_KNOW_THIS_IS_MAINNET=1`**. (The name is kept for
back-compat; despite it, the override now disarms the guardrail on **any** real-value
chain, not just mainnet — set it only if that is what you mean.) This paragraph is its only
documentation, deliberately: the variable's name never appears in daemon reason strings,
tool responses, tool descriptions, or `demo-check` output (asserted by the
transcript-hygiene tests) — a guardrail whose disable instructions are printed to the agent
is a speed bump, not a control. Set it only if you are a human operator who has read this
file and wants policy-capped hands-free writes on a real chain anyway.

Honest limits of the guardrail:

- It guards against a **misconfigured-chain** hands-free spend, not all hands-free spend.
  On the **exempt** testnet/dev ids the guardrail is off **by design** (the demo runs
  hands-free), and there the policy caps are the only brake — within-cap auto-Allow to any
  recipient still happens. The default policy is still `OverCap` with an empty allowlist;
  this guardrail does not change that. If you operate on an exempt chain you care about, set
  `ApprovalMode::Always` in your policy.
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

## Security tour — the attack surface, component by component

The sections above are the boundary-level argument; this is the same argument walked
one surface at a time. For each: **what an attacker controls**, **what stops them in the
code today**, **what's deferred**. Every claim names the file that backs it. Where a
surface is already covered above, this links rather than repeats.

### 1. The MCP client (a prompt-injected Claude, a malicious MCP client, a confused deputy)

*Controls:* the full argument to any tool, the order and timing of calls, and — for a
truly hostile client, not just an injected-but-honest one — the raw CBOR it speaks to
the sidecar's stdin. It does **not** control the daemon or any key.

*Stopped by:*
- **The tool surface is 6 read/propose tools, no `propose`/`resolve`/`simulate`**
  (`crates/deckard-mcp/src/server.rs`): `wallet_address`, `wallet_balance`,
  `policy_get`, `shield`, `execute`, `revoke_all`. There is no tool that submits an
  arbitrary intent (a raw `propose` would let the client hand the daemon junk calldata)
  and no tool that approves a request — so an injected agent literally has no hand that
  can self-authorize a guarded write. `shield` takes a typed `amount_eth` string and the
  *daemon* builds the intent against the canonical RelayAdapt address; the client never
  supplies a `to`.
- **Secret-shaped flags are refused before clap parses them**
  (`crates/deckard-mcp/src/secrets.rs`, `reject_secret_flags`): any `--…key`,
  `--…seed`, `--…token`, `--passphrase=…` etc. is rejected delimiter-aware (every
  `-`/`_` component is checked), and the flag's **value is never echoed** into the
  refusal — so a pasted credential can't be laundered into a tool transcript or shell
  history even by mistake. The sidecar is key-less by architecture; this is the
  belt-and-suspenders.
- **A connect-time chain probe runs before any real intent**
  (`crates/deckard-mcp/src/sidecar.rs`, `ensure_chain`): a deliberately-undecodable
  `Send` whose only purpose is to make the daemon answer `chain_mismatch` (checked
  before the policy gate, before `locked`, and storing no pending record). A demo
  sidecar pointed at the mainnet daemon (or vice versa) fails with an actionable error
  instead of silently proposing on the wrong chain. The probe is side-effect-free and
  can never itself be executed.
- **Typed failures never echo a payload** (`crates/deckard-mcp/src/failure.rs`): every
  error an agent can hit is a static `problem + cause + fix`; the two retry-traps
  (`broadcast_timeout`, `already_executed`) say *do NOT retry* explicitly, because an
  LLM's default instinct on error is to retry and a blind retry of a broadcast can
  double-spend. The one path that *could* carry daemon text (`from_deny_reason`'s
  fallthrough) only forwards a `reason` that was already URL-redacted daemon-side, and
  the `unexpected` helper deliberately drops the response body it can't vouch for.

*Deferred:* nothing on this surface is load-bearing for the boundary — the client is
untrusted *by design* and the daemon is the enforcement point. Tool-surface growth
(send, unshield) re-opens this section and each new tool inherits the no-self-approve
rule.

### 2. The daemon socket (the wire an attacker would reach for)

*Controls:* anything a same-uid process can: open the socket, send well-formed or
malformed frames, flood requests. A *different*-uid process controls nothing — it can't
connect.

*Stopped by:*
- **Peer-uid check + filesystem perms** (`crates/deckard-signerd/src/socket.rs`,
  `crates/deckard-signerd/src/auth.rs`, `server.rs`): the socket is `0600` inside a
  `0700` dir (chmod'd explicitly, not left to umask), and every accepted connection is
  gated on `peer_cred().uid()` matching our euid — a foreign uid, *including root*, is
  dropped. A single-instance `flock` keeps a second daemon from racing on the socket.
- **Bounded framing** (`crates/deckard-signerd/src/frame.rs`): a 4-byte big-endian
  length prefix caps each CBOR body at 1 MiB, so a hostile client can't make the daemon
  allocate unbounded memory; a *truncated* prefix (1–3 bytes then EOF) is an explicit
  error, distinct from a clean between-frames close, so a half-frame can't be
  mis-handled. A frame that won't decode gets one `malformed_request` reply and the
  connection closes — and the raw frame bytes are `zeroize`d after decode because an
  `Unlock` frame carries the passphrase (`server.rs`).
- **One mutex serializes every request** (`crates/deckard-signerd/src/server.rs`,
  `daemon.rs`): `propose` and `execute` can't race, which is also what gives execute its
  idempotency for free. (The cost of holding it across broadcast is the STOP-latency
  tradeoff — see its section above.)
- **Per-request TOCTOU re-checks at `execute`** (`crates/deckard-signerd/src/daemon.rs`,
  `execute`): an `Allow` is not a ticket. At sign time the daemon re-checks the vault is
  still unlocked (a STOP that landed first wins → `revoked`), the request hasn't already
  broadcast (`already_executed` idempotency keyed on the stored `broadcast` hash), it
  isn't past its approval TTL (`expired`), and — for an *auto*-allow, not a
  human-approved overage — that it's **still** within the caps against the current
  `spent_today` (`cap_exceeded`). Two within-cap proposals therefore can't both slip past
  the daily cap.

- **Resolver authentication** (`crates/deckard-signerd/src/{server,daemon,supervise}.rs`,
  PRD-01): `Resolve` is honoured only on the private capability channel the daemon
  inherits from the supervising app (a `socketpair` end passed at spawn); a `Resolve` on
  this public socket is refused with `resolve_not_authorized`. The same-uid peer-cred
  check stays as defense-in-depth + logging, not as the approval boundary.

*Deferred:* cross-restart spend persistence (`policy_store.rs`): `spent_today` is
in-memory and resets on restart.

### 3. The policy store (the fence an attacker would want to widen)

*Controls:* an attacker who can already write your config dir is same-uid and past the
boundary — but the *failure modes* of loading the file still matter, because a corrupt
or absent policy must never silently become a permissive one.

*Stopped by* (`crates/deckard-signerd/src/policy_store.rs`): the file is read **once at
boot** into memory (there is no `SetPolicy` mutation API on the wire, so a connected
client can't widen the fence at runtime). A missing file falls back to the built-in
default **quietly** (normal first run); a file that *exists but won't parse* falls back
**loudly** — a `⚠ POLICY FALLBACK` line naming the path and the default it's now running
— precisely so a typo'd `policy.json` can't silently drop you onto the any-recipient
default. On every load, `spent_today_wei` is forced to zero and `revoked` to false: the
daemon never trusts an on-disk spend counter and always **boots armed** (the brake is a
live STOP, not a persisted flag). The demo policy is install-if-absent, never an
overwrite.

### 4. The keystore at rest

Out of scope for this file by design — the Argon2id + XChaCha20-Poly1305 envelope, the
no-oracle unlock, and the "stated honestly" live-malware limit are in
[SECURITY.md](SECURITY.md). The only runtime note: a wrong passphrase, a tampered
vault, and a read error all collapse to one `BadPassphrase` outcome with no key held
(`daemon.rs`, `unlock`) — no unlock oracle leaks across the socket.

### 5. The app approval path (the human-in-the-loop an attacker would want to skip)

*Controls:* an attacker who reaches this path is, again, same-uid (the app is the
designated resolver; see the boundary section). The interesting question is whether the
*honest* path is sound.

*Stopped by:*
- **Hold-to-confirm → `Resolve{approved: true}` → `Execute`**
  (`crates/deckard-app/src/shell.rs`): the completed hold *is* the human approval; the
  app is the wire contract's designated resolver, so it sends `Resolve{approved: true}`
  only after the hold completes, and leaving the Shield surface cancels an in-progress
  hold so a stale timer can't fire a confirm after the screen is gone.
- **The auto-approval guardrail downgrade happens daemon-side, not app-side**
  (`crates/deckard-signerd/src/daemon.rs`, `propose` + `guardrail_active`): on any
  real-value chain (every chain except the exempt testnet/dev allowlist) without the
  override, every auto-`Allow` becomes `Pending` *in the daemon*, so the human-approval
  requirement doesn't depend on the app rendering a card correctly — a buggy or bypassed UI
  still can't produce a hands-free real-chain write. `Deny` is never upgraded.
- **Chain-id resolution fails loud** (`crates/deckard-app/src/settings.rs`,
  `resolve_chain_id`): the daemon's chain is resolved `env > settings > default(mainnet)`
  *once* at startup and pinned into the daemon's env, so the reader and signer agree. An
  unparsable `DECKARD_CHAIN_ID` is a deliberate startup panic — silently ignoring a typo
  like `sepolia` would resolve **toward mainnet**, the wrong direction.

*Deferred:* the reader/signer RPC split is a known seam — a Settings RPC re-point
respawns only the read provider; the daemon keeps its launch RPC
(`shell.rs::respawn_provider`, documented inline). There's no send UI yet so nothing
broadcasts through a diverged endpoint, but it's worth attacking (red-team issue #3).

## Residual risks, ranked

| # | Risk | Status |
|---|------|--------|
| 1 | Same-uid code self-approves via `Resolve` (or speaks the wire directly) | Accepted v1 boundary — documented above; resolver authentication is the roadmap fix |
| 2 | STOP queues ≤30s behind an in-flight broadcast | Accepted v1 tradeoff — first post-launch daemon PR |
| 3 | Exempt testnet/dev chains have no guardrail (policy caps only); the default policy is still `OverCap` so within-cap auto-Allow to any recipient happens there | Narrowed (#76: real-value & unknown chains are now guarded by default) — on exempt chains use `ApprovalMode::Always` |
| 4 | Viewing-key compromise in the sidecar leaks shielded history (not funds) | Mitigated (Zeroizing, no-output discipline, scan-tested) |
| 5 | Reason redaction is URL-shaped-token-based; a credential echoed in a non-URL form would pass | Mitigated for realistic transport-error shapes (tested); allowlist scan is the backstop |
| 6 | Deterministic request-ids allow same-uid intent-collision games | Accepted within the uid boundary; salted ids on roadmap |
| 7 | **Rollback / replay of an older genuine `vault.bin`**: a same-uid attacker drops an older valid copy back over the current file (it isn't a forgery, so the AEAD can't flag it) | **Accepted residual for alpha.** Evaluated and **deferred** ([ADR 0004](docs/adr/0004-rollback-resistant-state-anchor.md), #71): rollback needs filesystem write, which is same-uid and already inside this file's conceded boundary, and the vault's only rollback worst-case is reverting a passphrase/KDF rotation (the seed is constant, balances are on-chain). Revisit if the threat model rises (mainnet keys, multi-user, untrusted backup/sync) |

If you can demonstrate an attack that crosses a boundary this file claims holds —
that's a vulnerability. Please report it via [SECURITY.md](SECURITY.md).
