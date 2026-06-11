# Wire-Contract Evolution + Protocol Publication

> Evolution rules for the frozen `deckard-contract` wire (Intent / Decision / Policy / the daemon
> socket), a capability-discovery message that makes the contract versionable without breaking a
> single existing frame, and the publication path that turns the wire into an open **local
> agent-custody protocol** others can implement · unblocks `50-session-keys-7702.md` and
> `60-x402.md` (both add capabilities; neither may break the freeze) · status (spec) ·
> researched 2026-06-11. Part of the Deckard build docs.

## Why this exists (2-4 sentences, concrete)

`30-mcp-shape.md` froze the contract ("frozen 2026-06-05, do not redefine elsewhere") but defined
no way to evolve it: there is no version field, no capability discovery, and "frozen by convention
+ round-trip tests" cannot answer "what may a v2 add, and how does an old peer react?". The next
two build docs both extend the wire (new request kinds, new policy fields), and the strategic goal
— second implementations of the key-less-client/policy-gated-daemon pattern — requires a spec a
stranger can implement and test against. Prior art says this is a solved problem: ssh-agent ran
20 years as an unversioned convention and was formalized late as **RFC 9987 (May 2026)** using
exactly the mechanisms below.

## Where it sits — Depends on / Unblocks

**Depends on**
- `30-mcp-shape.md` — owns the frozen contract this doc gives evolution rules to.
- `crates/deckard-contract` — the byte-stable round-trip tests are the compatibility baseline.

**Unblocks**
- `50-session-keys-7702.md` — adds `SessionGrant`/`SessionRevoke` requests as **capabilities**.
- `60-x402.md` — adds `SignIn` and `PaymentAuthorization` requests as **capabilities**.
- External implementers: a second client (another agent framework) or a second daemon (another
  wallet) can target the published spec + conformance vectors.

## Architecture / approach — the five rules (adopted from prior art)

1. **Capability discovery instead of a version handshake.** One new request,
   `SignerRequest::Hello`, returns `{ spec_version, capabilities: Vec<String>, impl: String }`.
   An **old daemon** receiving `Hello` fails to decode the unknown enum variant and returns its
   existing frame-decode error — loud, distinguishable, and already shipped. That failure *is*
   the backward-compat valve (the ssh-agent `SSH_AGENT_FAILURE` trick, RFC 9987; the varlink
   `GetInfo` shape). No existing frame changes.
2. **Date-version the spec; bump only on breaking change.** `spec_version` is `YYYY-MM-DD` (MCP's
   scheme). Additive work — new request variants, new optional fields — ships under a new
   **capability name** (`session-keys-7702`, `x402-exact`, `sign-in-with-x`), never a version
   bump. Capability names live in a registry file and are never reused (EIP-5792's
   capability-map pattern; new capabilities may get their own build doc the way 5792
   capabilities get their own ERCs).
3. **Protobuf discipline on the CBOR.** Field (map-key) names are forever: never reuse, never
   rename, never change a field's type; removed fields go to a `reserved` list in the spec.
   Decoders MUST ignore unknown map keys in structs (serde's default — pinned by an explicit
   test, never `deny_unknown_fields` on wire types) and MUST reject unknown enum variants (the
   loud valve). The schema is published as **CDDL (RFC 8610)** next to the Rust types.
4. **Two distinguishable failures.** "Unsupported message/capability" must be tellable apart from
   "supported but failed" (RFC 9987's one great trick; ssh-agent's
   `SSH_AGENT_EXTENSION_FAILURE`). Concretely: unknown variant → frame-decode error;
   known-but-denied → `Decision::Deny { reason }` from the frozen vocabulary.
5. **Independent-spec posture, in-repo home (decision 2026-06-11).** The protocol spec text, CDDL,
   golden test vectors, and conformance asserts live in this repo and `deckard-contract` ships to
   crates.io. A standalone spec repo is **gated on a second implementer showing up**; a thin ERC
   (capability names + the JSON display encoding only, 5792-style) is gated further still. This is
   the WalletConnect / eth-infinitism bundler-spec path: spec repo + reference impl + compliance
   suite first, formal standards later.

## Concrete interface

```rust
// deckard-contract — ADDITIVE. Existing variants/fields untouched.
enum SignerRequest {
    // ... existing variants unchanged ...
    Hello,                                   // -> HelloInfo (key-less, lock-state-independent)
}

struct HelloInfo {
    spec_version: String,                    // "2026-06-11" — date of last BREAKING change
    capabilities: Vec<String>,               // e.g. ["core", "mcp.v0.1"]
    impl_name: String,                       // "deckard-signerd 0.x" — display only, never branched on
}
```

Baseline capability names (registered at adoption): `core` (the 30-mcp-shape socket API as
shipped), `mcp.v0.1` (the 6-tool profile). The registry is a table in the spec doc:
`name · status (active/reserved) · since spec_version · defining doc`.

**Publication artifacts** (the work items this doc creates):
- `docs/spec/agent-custody-protocol.md` — the extractable spec: RFC 2119 conformance language,
  the trust model (key-less client / policy-gated key-holding daemon / human approval surface),
  message semantics, the Deny-reason vocabulary, the capability registry, and the five rules
  above as normative text.
- `docs/spec/wire.cddl` — CDDL for every wire type, CI-checked against the Rust types via golden
  vectors.
- `docs/spec/vectors/` — CBOR hex golden vectors (one per message kind), the cross-implementation
  conformance currency. Noise's known gap is *no official test vectors* — do not repeat it.
- `deckard-contract` published to crates.io (publishing adds no dependencies; the crate is already
  the freeze boundary).

## Invariants (frozen here)

- Every existing frame from before this doc decodes identically after it (asserted by the
  existing round-trip tests, which never change for an additive release).
- `Hello` is answerable in every daemon state including `Locked` — it reveals capabilities, never
  state, never key material, never policy contents.
- No code path ever branches on `impl_name`. Feature detection is capabilities only.
- A capability, once registered, never changes meaning; superseding one means registering a new
  name and (eventually) moving the old one to `reserved`.

## Acceptance test (agent-runnable asserts)

```
Scenario "evolution rules hold" (mock daemon + real daemon, parity-asserted):
  E1 Hello                         assert: spec_version matches /^\d{4}-\d{2}-\d{2}$/;
                                           capabilities ⊇ {"core","mcp.v0.1"}
  E2 old-peer valve: replay a pre-Hello client session (golden vectors)
                                   assert: byte-identical responses (freeze holds)
  E3 unknown-variant frame sent to the daemon
                                   assert: frame-decode error, connection survives,
                                           no panic, nothing signed
  E4 struct frame with an extra unknown map key
                                   assert: decodes; unknown key ignored (rule 3 pinned)
  E5 cddl validate vectors/*.hex against wire.cddl
                                   assert: all pass; CI job exists
  E6 cargo publish --dry-run -p deckard-contract
                                   assert: clean
```

## Risks & fallbacks

- **Premature standardization** (tempo took 81 releases to settle its surface). *Mitigation:* the
  spec is dated and explicitly pre-1.0; rules 2–3 make additive iteration cheap; the standalone
  repo and ERC are gated on external demand, not calendar.
- **CDDL/serde drift.** *Mitigation:* golden vectors are the single source of truth — both the
  CDDL check and the Rust round-trip tests consume the same files (E5).
- **`Hello` as fingerprinting surface.** It is reachable by any same-uid local process.
  *Accepted:* it reveals only capability names, which the binary's existence already reveals.
- **crates.io name squatting / supply-chain.** *Mitigation:* publish early (E6 is cheap), pin by
  version everywhere, no install scripts (Rust has none).

## Open questions

- Should `Hello` also carry the active chain id (so a wrong-chain client fails at handshake
  instead of first `Propose`)? Leaning yes — it duplicates a `Deny{chain_mismatch}` that today
  fires late.
- Registry governance once a second implementer exists: PR-into-this-repo (MCP's SEP shape) or
  move registry + spec to the standalone repo at that moment?
- Does the JSON (display) encoding get the same never-rename guarantee as CBOR, or is JSON
  explicitly non-normative? Leaning: normative for `deckard-mcp` tool responses (agents parse
  them), non-normative elsewhere.

## Sources

- RFC 9987 — SSH Agent Protocol (Standards Track, May 2026; numbered messages, named extensions,
  generic-failure valve, IANA registries) — https://datatracker.ietf.org/doc/rfc9987/ — (spec, high)
- OpenSSH PROTOCOL.agent + agent restrictions (extension-typed constraints; policy bolted on late
  via extensions, incl. CVE-2023-51384 as the cautionary tale) —
  https://raw.githubusercontent.com/openssh/openssh-portable/master/PROTOCOL.agent ·
  https://www.openssh.org/agent-restrict.html — (spec, high)
- MCP versioning (date versions, bump-on-breaking-only; the 2026-07-28 draft moves version into
  per-request `_meta` — direction confirms the valve-over-handshake trend) —
  https://raw.githubusercontent.com/modelcontextprotocol/modelcontextprotocol/main/docs/specification/draft/basic/versioning.mdx — (spec, high)
- Protobuf evolution rules (never reuse/renumber, unknown-field tolerance) —
  https://protobuf.dev/programming-guides/proto3/ — (spec, high)
- CDDL — RFC 8610 (+9682) — https://datatracker.ietf.org/doc/html/rfc8610 — (spec, high)
- EIP-5792 (capability maps, `optional` flag, new capabilities as separate ERCs) —
  https://eips.ethereum.org/EIPS/eip-5792 — (spec, high)
- varlink service introspection (`GetInfo`, namespaced errors — the local-socket exemplar) —
  https://raw.githubusercontent.com/varlink/varlink.github.io/master/Service.md — (spec, high)
- Independent-spec precedents: WalletConnect spec repo —
  https://github.com/WalletConnect/walletconnect-specs ; 4337 bundler spec + compliance suite —
  https://github.com/eth-infinitism/bundler-spec ·
  https://github.com/eth-infinitism/bundler-spec-tests — (github, high)
- Noise Protocol Framework (structure exemplar; evolves by naming; lacks official test vectors —
  the gap we close with `vectors/`) —
  https://raw.githubusercontent.com/noiseprotocol/noise_spec/master/noise.md — (spec, high)
