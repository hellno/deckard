# Wire-contract evolution — capability discovery + the five rules

> The rules for how the frozen `deckard-contract` wire (Intent / Decision / Policy / SignerRequest,
> CBOR over the UDS and JSON over MCP) is allowed to grow. Issue **#31**. The registry and the
> rules live in code (`crates/deckard-contract/src/capabilities.rs`) and here; this doc is their
> prose home. Additive changes ship under this doc; a breaking change would need a new spec version.

## Why capability discovery (and not a version handshake)

The wire is frozen by convention with no version field. Upcoming features add new request *kinds*
(EIP-7702 session keys, x402, dapp-origin proposals — **#198** / **#204** / **#32** / **#34**).
Without written evolution rules, each one is an ad-hoc breaking change to a contract we call frozen.

We add exactly one request:

```rust
SignerRequest::Hello   // -> SignerResponse::Hello(HelloInfo {
                       //        spec_version: "YYYY-MM-DD",
                       //        capabilities: Vec<String>,   // e.g. ["core", "mcp.v0.1"]
                       //        impl_name:    String,        // informational; never branched on
                       //    })
```

A client asks *does this daemon understand request kind X?* and reads the `capabilities` list. There
is **no negotiated handshake** (à la MCP ≤2025-11). We chose discovery over negotiation because MCP
itself is moving away from the handshake, and the backward-compat valve here needs **zero** migration:

> An **old** daemon that receives `Hello` fails to decode the unknown enum variant and returns the
> existing frame-decode error. That failure *is* the backward-compat valve, and it already ships. No
> existing frame changes; the round-trip fixtures prove the freeze holds.

`Hello` is answered in **every** daemon state, including `Locked` — it reveals capability names only,
never vault state, policy contents, or key material.

Grounded in verified prior art: RFC 9987 (ssh-agent — 20 years unversioned, later formalized with
named extensions + a generic-failure valve), MCP date-versioning, protobuf never-reuse discipline,
EIP-5792 capability maps, varlink `GetInfo`.

## The five rules (normative)

1. **Capability discovery, not version negotiation.** `Hello` returns `HelloInfo` as above. Old peers
   answer a variant they predate with the existing decode error — that rejection is the valve.
2. **Date-version (`YYYY-MM-DD`), bump only on a breaking change.** Additions ship under a new
   capability *name*, **never** a version bump. Capability names are never reused. `spec_version`
   moves only when an existing frame changes shape or meaning in a non-additive way.
3. **Protobuf discipline on the CBOR.** Map-key names are forever: never reuse, rename, or retype a
   key; a removed key's name goes to a `reserved` note and is never recycled. Decoders **MUST ignore
   unknown struct keys** and **MUST reject unknown enum variants**. (The wire structs carry no
   `deny_unknown_fields`, on purpose — that is what makes struct growth additive.)
4. **Two distinguishable failures.** "unsupported message / capability" is a *frame-decode error*
   (`malformed_request` on the wire). "supported but failed / refused" is a `Decision::Deny { reason }`
   drawn from the frozen deny-reason vocabulary (`deny_reasons`, issue #28). A caller can always tell
   *"this daemon can't do that"* from *"this daemon won't do that."*
5. **In-repo home.** The rules + the capability registry live in this repo:
   `crates/deckard-contract/src/capabilities.rs` (the code) and this doc (the prose). A standalone
   spec document, CDDL schema, golden-vector files, and crates.io publication are deferred until a
   second, independent implementer appears.

## Capability registry

The baseline capabilities every current build advertises. This table is the human-readable mirror of
`capabilities()` in `capabilities.rs`; keep them in lockstep (add a row here when you add a const
there). A name here is permanent (rule #3).

| Capability | Status | Since | Defining doc |
|---|---|---|---|
| `core` | stable | 2026-06-05 | [`30-mcp-shape.md`](30-mcp-shape.md) — the frozen `deckard-contract` socket API (unlock / propose / execute / status / revoke_all / policy_get / address / balance / pending / activity). |
| `mcp.v0.1` | stable | 2026-06-10 | [`31-agent-quickstart.md`](31-agent-quickstart.md) — the key-less MCP sidecar's agent-tool profile over `core` (its tool list is drift-guarded by a test in `deckard-mcp`). |
| `origin.dapp` | stable | 2026-07-07 | This doc (issue #198) — `ProposalOrigin::Dapp { origin }` on `Propose` / `ProposeOrder` / `ProposeMessage`, echoed back on pending/activity records. The origin string is display-only attribution (rendered verbatim, never a trust root, never a policy input); `App`/`Agent` frames are byte-unchanged and an old decoder rejects the `Dapp` tag with `malformed_request` (the rule-#1 valve). |

## Adding a capability (the #198 / #204 extension point)

Registering a new request *kind* or origin variant is deliberately small — one edit in each of three
places, mirroring the deny-vocabulary discipline in `deny_reasons`:

1. **`capabilities.rs`** — add `pub const CAP_… : &str = "…";` (with a doc comment: meaning + since),
   and push it into `capabilities()` in registry order (append; never reorder or reuse a name).
2. **This table** — add its row (capability · status · since · defining doc).
3. **The wire, additively** — a new `SignerRequest` / enum variant is appended; old peers reject it
   via the valve (rule #1). `spec_version` does **not** change (rule #2).

Concretely, **#198** (`ProposalOrigin::Dapp` on the wire) shipped exactly this way — the
`origin.dapp` row above — and **#204** registers its origin variant the same way. An old daemon that
receives a `Dapp`-tagged frame rejects it with `malformed_request` — the correct, safe degradation.
(The reverse direction fails equally loudly: an old *client* reading a new daemon's pending/activity
list that contains one `Dapp`-origin record rejects the whole frame, since `origin` is a required
field of every record. That is the valve working as designed, not a partial decode.)

## How the rules are proven (tests)

- **E1 — `Hello` shape** (`deckard-contract`, `wire_evolution::e1_…`): `spec_version` matches
  `^\d{4}-\d{2}-\d{2}$`; `capabilities ⊇ {core, mcp.v0.1}`.
- **E2 — freeze holds** (`e2_…` + the untouched `signer_request_roundtrip` / `signer_response_roundtrip`
  fixtures): existing frames encode byte-identically after `Hello` was added; golden bytes pin the unit
  requests.
- **E3 — unknown variant rejected loudly** (`e3_…`, and `deckard-signerd/tests/hello.rs` at the socket):
  a future variant decodes to an `Err`, never a silent misparse or panic; the daemon replies
  `malformed_request`, keeps serving, and signs nothing.
- **E4 — unknown struct key ignored** (`e4_…`): a newer producer's extra `HelloInfo` field is skipped
  and the known fields still decode.
- **Parity** (`capabilities::tests` + `deckard-signerd/tests/hello.rs`): the daemon and the mocks build
  `Hello` from the one `hello_info()` source, so their `spec_version` + `capabilities` are identical.

## Not in scope (deferred)

Extractable standalone spec document, CDDL schema (RFC 8610), golden-vector files, crates.io
publication, a standalone spec repo, an ERC draft. Any change to existing frames or to the deny-reason
vocabulary (that is #28). The daemon's active chain id in `HelloInfo` is **not** included: the in-scope
rule is "capability names only — no state," and a wrong-chain client is already refused at first
`Propose` with `Deny{chain_mismatch}`. It can be added additively later — precisely by these rules —
if first-contact chain detection proves worth the state it exposes.
