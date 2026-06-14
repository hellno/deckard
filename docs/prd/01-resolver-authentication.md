# PRD-01 — Resolver authentication (capability-gated `Resolve`)

> Phase 1a of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). Closes `THREAT-MODEL.md`
> residual-risk #1. **Foundational and independent** — has standalone security value even if no dapp
> connectivity ever ships, and is a hard prerequisite for PRD-04.

## Why this exists

Today `crates/deckard-signerd/src/auth.rs` authorizes a connection on `same_uid()` alone. Because
`Resolve{request_id, approved:true}` is just another frame on the same socket, **any same-uid process
can approve a pending request** — including, in future, a browser-reachable proposer. The research
(`docs/research/10-dapp-connectivity.md §22–28`) confirms this is the classic confused-deputy problem:
uid is *ambient authority*; peer-cred proves *who* connected but not *which role*. Every hardware/
companion signer (Trezor, Ledger, GridPlus, Frame) separates *request* (promiscuous endpoint) from
*approval* (a surface the requester cannot drive). Deckard must manufacture the same split in software.

Right now the risk is *accepted* because the proposer set is small and human-controlled. PRD-04 breaks
that assumption. This PRD must land first.

## Goals

- The GPUI app is the **sole** principal that can send `Resolve` / `RevokeAll`-as-approval-grant.
- Approval authority is an **unforgeable capability** the daemon hands only to the app process it
  controls — not derivable by any other same-uid process.
- The public proposer socket continues to accept *proposals and reads* from same-uid callers
  (unchanged), but **rejects `Resolve`** with a typed error.
- STOP/`RevokeAll` (the brake) remains reachable from *any* transport (it can only ever reduce
  authority, never grant it) — see Non-goals.

## Non-goals

- Multi-user / multi-principal daemon (still single-uid). Salted request-ids (residual-risk #6) are a
  separate future item.
- Changing the keystore, mainnet guardrail, or STOP-latency behavior.
- Authenticating the *dapp origin* (that's PRD-05). This PRD authenticates the **resolver**, not the
  requester.

## Design

### The capability channel

`crates/deckard-signerd/src/supervise.rs` already has the app **spawn + supervise the daemon child**.
That direction matters: the *app* is the parent. Two viable capability mechanisms (pick per
portability, decide in implementation and record in the PRD's decision note):

1. **`socketpair()` inherited by the daemon child (recommended).** Before `Command::spawn`, the app
   creates an `AF_UNIX` `SOCK_SEQPACKET`/`SOCK_STREAM` pair; it passes one end to the child as an
   inherited fd (e.g. via `CommandExt::pre_exec` / explicit fd inheritance) and keeps the other. This
   **control channel** is the only place the daemon accepts `Resolve`. No other process can obtain the
   app's end (`research §26`, kernel inheritance). Portable across Linux + macOS.
2. **`SCM_RIGHTS` fd-pass after connect.** App connects to the public socket, the daemon passes back a
   dedicated control fd via ancillary data; thereafter `Resolve` is accepted only on that fd. Slightly
   more wire complexity; also portable (`research §25, 27`).

**Fallback (weaker, only if neither fd path is feasible):** a per-launch random cookie written `0600`
by the daemon, read by the app, presented on `Resolve`. Weaker because any process that can read the
user's files can steal it — prefer an fd capability. Document explicitly if used.

### Daemon changes (`crates/deckard-signerd`)

- `auth.rs`: keep `same_uid()` for the public socket (defense-in-depth + logging). Add the concept of a
  **control channel** that is authenticated by construction (the inherited/passed fd), not by uid.
- `server.rs` / `daemon.rs`: route `SignerRequest::Resolve` **only** from the control channel. A
  `Resolve` arriving on the public socket returns a typed denial (new `deny_reasons` entry, e.g.
  `RESOLVE_NOT_AUTHORIZED`) — URL-redacted, no payload echo, per existing transcript-hygiene rules.
- Keep `RevokeAll`/STOP reachable on every channel (it only zeroizes; it cannot grant). Add a test
  asserting STOP from the public socket still works while `Resolve` from it is refused.
- `peer_uid` staleness caveat (`research §24`/`peercred` timing): capture creds at accept; do not
  re-trust across the connection lifetime.

### App changes (`crates/deckard-app`)

- `supervise.rs` + `shell.rs`: establish the control channel at spawn; thread its handle to the place
  that today sends `Resolve` after a completed hold-to-confirm (`shell.rs:142` neighborhood). The
  hold-to-confirm contract is unchanged — only the *channel* the `Resolve` rides changes.

### Contract changes (`crates/deckard-contract`)

- No change to `SignerRequest::Resolve`'s shape is required if the channel (not the frame) carries the
  authority. If a token/cookie fallback is used, add it as a typed field and ensure its `Debug` is
  redacted. Prefer the no-wire-change fd approach.

## Cross-platform notes

- Linux: `SO_PEERCRED`/`socketpair`/`SCM_RIGHTS` all available.
- macOS: no `SO_PEERCRED` (use `LOCAL_PEERCRED`; pid via `LOCAL_PEERPID`); `socketpair`/`SCM_RIGHTS`
  available — which is why the fd-capability approach is the portable primitive (`research §27`).
- `unsafe` for raw fd handling: confine to the **app crate** (`unsafe_code = "deny"` → add a scoped,
  documented `#[allow(unsafe_code)]` with a `// reason:` comment, matching the existing `eth.rs`
  pattern). `deckard-core` stays `#![forbid(unsafe_code)]` — keep fd plumbing out of it. Prefer a
  vetted existing dependency already in the tree (`nix`, `tokio`) over hand-rolled `libc`; **no new
  dependency without approval** (DoD #4) — `nix` and `tokio` are already present.

## Acceptance tests (add to `crates/deckard-signerd/tests/`)

- `resolve_rejected_on_public_socket`: a same-uid client on the public socket gets the typed denial for
  `Resolve`; the pending record stays `Pending`.
- `resolve_accepted_on_control_channel`: the same `Resolve` over the control capability flips
  `Pending → Allowed`.
- `stop_still_works_on_public_socket`: `RevokeAll` from the public socket zeroizes + denies in-flight
  (the brake is never gated).
- `second_proposer_cannot_self_approve`: end-to-end — proposer A proposes (over cap → `NeedsApproval`);
  proposer A (public socket) cannot `Resolve`; only the control channel can. This is the red-team
  assertion that maps to residual-risk #1.
- Pure-fn parity unaffected (mock vs daemon `evaluate` unchanged).

## Definition of Done

PRD-series DoD (see `README.md`) **plus**:
- `THREAT-MODEL.md` residual-risk table updated: #1 moves from "Accepted v1 boundary" to "Mitigated"
  with the mechanism named; the "Deferred" notes in §2 (daemon socket) updated accordingly.
- A short decision note in this PRD recording which capability mechanism was chosen and why.
- The four acceptance tests above run by **default** `cargo test` (not `#[ignore]`).

## Risks & fallbacks

- **fd inheritance across `Command::spawn` is fiddly / platform-specific.** Fallback: `SCM_RIGHTS`
  post-connect; last resort: `0600` cookie (documented as weaker).
- **App-restart / daemon-survives races** (the supervise loop respawns the daemon): define the
  re-handshake so a restarted app re-establishes the control channel without a window where `Resolve`
  is ungated. Test the respawn path.

## Sources

`docs/research/10-dapp-connectivity.md §22–29`; `THREAT-MODEL.md` (boundary section, residual-risk #1);
man7 unix(7); blog.cloudflare.com/know-your-scm_rights; capability-myths-demolished.
