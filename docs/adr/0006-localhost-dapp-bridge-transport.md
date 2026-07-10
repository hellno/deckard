# ADR 0006 — Localhost dapp-bridge transport (accepted as the alpha wire)

- **Status:** Accepted (2026-07-10)
- **Deciders:** @hellno (maintainer)
- **Context inputs:** [ADR 0001](0001-dapp-connectivity-architecture.md) (the umbrella decision this
  amends), `docs/browser-bridge.md`, `SECURITY.md`, `THREAT-MODEL.md`, and the shipped bridge
  (`crates/deckard-browser-bridge/`, `extension/`).
- **Supersedes / relates:** amends ADR 0001's transport preference for the `0.0.1-alpha` window;
  resolves issue [#202](https://github.com/hellno/deckard/issues/202); re-scopes the PRD-04 spike
  ([#49](https://github.com/hellno/deckard/issues/49)) and PRD-04 ([#50](https://github.com/hellno/deckard/issues/50)).

## Context

ADR 0001 **preferred native messaging** (an OS stdio channel a web page cannot reach) and recorded a
localhost RPC port as the **rejected alternative** — it is web-reachable (DNS-rebinding, cross-site
WebSocket/`fetch` hijacking) and only defensible with origin-allowlisting + a token + Host checks.

But what actually shipped (#62/#63 discovery + connect, #139 message signing, #141 `eth_sendTransaction`,
#146 ERC-20 classify, #148 EIP-5792, #198 origin attribution) is a hand-rolled loopback HTTP server
hardcoded to `127.0.0.1:8765`, defended today only by a Host-header loopback check
(`lib.rs:1308-1312`) and permissive CORS. The plan and the shipped code disagree, and no ADR recorded
the wire we are actually running. `eth_requestAccounts` currently grants a session with **no human
approval** (`lib.rs:228-238`).

## Decision

**Accept the shipped `127.0.0.1:8765` loopback bridge as the transport for the `0.0.1-alpha` browser
demo — explicitly as a temporary, experimental wire — and do not invest heavily in hardening it now.**

1. **The two ends are the priority, not the pipe.** What we validate for the alpha is the two
   *endpoints* working and tested end-to-end: `deckard-core` signing (through the daemon's
   clear-signing approval) and the browser extension (EIP-1193/6963 injection + forwarding). The
   transport between them is a swappable constraint we expect to replace.
2. **Native messaging stays the post-audit target, not the alpha's work.** The safer wire remains the
   end state (PRD-04 / #49 / #50), sequenced behind the external audit per `SECURITY.md`. We do **not**
   build it now.
3. **We also do not pour effort into hardening localhost** (auth token, DNS-rebinding defenses beyond
   the existing loopback Host check). It is a wire we intend to retire; heavy investment there is
   wasted. The minimal existing loopback check plus the alpha's throwaway-testnet-keys posture is the
   accepted risk boundary.
4. **Per-site revoke is a session-scoped disconnect, not a ban** (#199): revoke drops the current
   session (the site must reconnect fresh); there is no persistent blocklist. Matches the "keep it
   light" posture here.

## Why this is acceptable (and only here)

`SECURITY.md` pins Deckard at `0.0.1-alpha`: unaudited, single-maintainer, **testnet keys only**. The
localhost wire's residual risk (a malicious page reaching the loopback port) is bounded by throwaway
keys and testnet-only operation. This acceptance is **scoped to the alpha** and does not survive
contact with real value — mainnet/real-key operation is gated behind the native-messaging replacement
and the audit.

## Consequences

- The connect-with-no-approval gap (`eth_requestAccounts` grants silently) is **assigned to #48**
  (per-origin permissions & connect-time approval), not to the alpha. Until then, a revoked site can
  re-grant itself by reconnecting — consistent with the session-scoped "disconnect, not ban" model.
- `docs/browser-bridge.md` is the operator reference for the wire; kept current (the "only 3 methods /
  not implemented" claims were corrected 2026-07-10).
- **#49/#50 re-scope:** native messaging is retained as the post-audit target; the localhost bridge is
  explicitly *not* the end state. Their bodies should reference this ADR as the interim decision.

## Rejected alternatives

- **Harden localhost heavily now** (token auth, full DNS-rebinding defense). Deferred: not worth the
  investment on a wire we plan to replace; the alpha's throwaway-key posture covers the gap.
- **Build native messaging first.** Slower to a rehearsable demo and gated behind the audit; it remains
  the target, just not the alpha's work.
