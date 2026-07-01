# ADR 0005 — Policy and authorization model

- **Status:** Accepted (2026-07-01)
- **Deciders:** @hellno (maintainer)
- **Context inputs:** [`crates/deckard-contract/src/policy.rs`](../../crates/deckard-contract/src/policy.rs)
  (the flat `Policy` + the pure `evaluate` / `evaluate_order` functions),
  [`crates/deckard-signerd/src/policy_store.rs`](../../crates/deckard-signerd/src/policy_store.rs)
  (loads `policy.json`, the loud fail-safe default),
  [`crates/deckard-signerd/src/daemon.rs`](../../crates/deckard-signerd/src/daemon.rs)
  (single-mutex serialization, execute-time cap re-check, #108 reserve-before-sign),
  [`docs/agent-authorization-map.md`](../agent-authorization-map.md),
  [`docs/adr/0002-agent-wallet-and-session-keys.md`](0002-agent-wallet-and-session-keys.md),
  [`docs/adr/0003-crate-trust-boundary.md`](0003-crate-trust-boundary.md),
  [`docs/build/30-mcp-shape.md`](../build/30-mcp-shape.md). A 2026-06-23 deep-research pass
  (primary-source, adversarially verified) on authorization standards/libraries, and **two** codex
  cross-model adversarial reviews — one of the thesis, one of an earlier draft of this ADR that caught
  the spec bugs corrected below.
- **Relates:** [`#29`](https://github.com/hellno/deckard/issues/29) (policy variants),
  [`#31`](https://github.com/hellno/deckard/issues/31) (wire evolution),
  [`#33`](https://github.com/hellno/deckard/issues/33) (EIP-7702 session keys),
  [`#48`](https://github.com/hellno/deckard/issues/48) (per-origin permissions). Convention: the
  executable work lives in GitHub issues; the decision + the "why" live here.

## Context

The agent's spending fence is a single **flat struct** (`policy.rs:16-37`):

```rust
struct Policy {
    per_tx_cap_wei, daily_cap_wei, spent_today_wei: U256,
    allow_to: Vec<Address>,            // EMPTY = any address  ← sentinel foot-gun
    auto_shield_min_wei: U256,         // advisory ONLY — evaluate() does not switch on it
    require_approval: ApprovalMode,    // Never | OverCap | Always  ← ONE global mode
    revoked: bool,
    allow_swap_tokens: Vec<Address>,   // EMPTY = any token    ← second sentinel
}
```

This does not scale. Every new capability bolts another bespoke top-level key onto the struct, and the
approval mode is **global** — it cannot say "auto-shield freely but card every send." The roadmap is
many more wallet actions (send, bridge, message-signing, contract calls). On the flat model each one
is a hand-maintained field threaded through `evaluate`, the loader, the demo policy, and the tests —
the "a key per rule, maintained forever" sprawl we want to kill. It also default-**allows** on the
most dangerous axis: an empty `allow_to` means "any recipient."

Three structural facts constrain the redesign (all verified in code; ignoring them was how the first
draft of this ADR went wrong):

1. **There are TWO pure decision functions, not one.** `evaluate(intent, policy)` decides
   `Intent`s (`IntentKind` = Send/Shield/Unshield/ContractCall — **there is no `Swap` kind**);
   `evaluate_order(order, policy, wallet, now)` decides swaps from a separate `SwapOrder` wire type
   and **never inspects caps** (`policy.rs:195-196`). Both are pure and shared by `MockSigner` and
   `deckard-signerd` (the parity contract) — the crown jewel to preserve.
2. **`Policy` lives in the frozen `deckard-contract` crate** with a byte-stable JSON+CBOR round-trip
   charter (`lib.rs` round-trip tests; #28/#31 rules), and is returned whole over CBOR by the
   `PolicyGet` RPC (`daemon.rs:343-346`). Changing its shape is an **intentional, versioned breaking
   change** to a freeze-first crate — see §6.
3. **Value is `uint256` wei**, compared in `U256`. This rules out every general-purpose authorization
   engine as a cap enforcer (§3).

## Decision

**Replace the flat `Policy` with a small, default-deny, per-action *typed rule list*, evaluated by the
same pure function(s), with all value comparisons in native Rust `U256`.**

### 1. The shape

`policy.json` becomes a versioned object with one **global** daily wall and an array of typed rules
(the RFC 9396 "array of typed objects" shape):

```jsonc
{
  "version": 1,
  "default": "deny",                          // no matching rule ⇒ Deny{no_rule}; "allow" is FORBIDDEN
  "daily_cap_wei": "200000000000000000",      // ONE global daily wall (single SpendStore + #108)
  "auto_shield_min_wei": "10000000000000000", // ADVISORY agent hint (via policy_get); NOT enforced
  "rules": [
    { "action": "send",  "approval": "over_cap", "per_tx_cap_wei": "50000000000000000",
      "recipients": ["0xA1b2…"] },            // omitted ⇒ DenyAll; "any" ⇒ Any
    { "action": "shield", "approval": "never" },
    { "action": "swap",   "tokens": ["0xToken…"] }   // evaluate_order reads tokens only; always NeedsApproval
  ]
}
```

In Rust, each action's constraints live **inside its own variant**, so adding a capability is one
localized variant + one match arm — never a new top-level field:

```rust
struct Policy {
    version: u32,
    default_effect: Effect,        // = Deny; Allow is intentionally NOT representable in v1
    revoked: bool,
    daily_cap_wei: U256,           // ONE global daily wall — mirrors the single SpendStore (#108)
    auto_shield_min_wei: U256,     // ADVISORY: agent reads via policy_get to decide WHETHER to propose;
                                   // evaluate() does NOT switch on it (preserves today's semantics)
    rules: Vec<Rule>,              // loader REJECTS duplicate actions (loud), so "find rule" is unambiguous
}

#[serde(tag = "action")]           // internally tagged ⇒ the JSON above
enum Rule {
    Send         { approval: ApprovalMode, per_tx_cap_wei: Option<U256>, #[serde(default)] recipients: Allowlist },
    Shield       { approval: ApprovalMode },                                   // value==0; no recipient (own 0zk), no per-tx cap
    Unshield     { approval: ApprovalMode, per_tx_cap_wei: Option<U256> },     // forward-compat; NOT yet reachable (see §2)
    Swap         { #[serde(default)] tokens: Allowlist },                      // read by evaluate_order only
    ContractCall { approval: ApprovalMode, #[serde(default)] targets: Allowlist }, // forward-compat; NOT yet reachable
}

enum Allowlist { DenyAll, Any, Only(Vec<Address>) }   // a real lattice — no "empty Vec = any" sentinel
impl Default for Allowlist { fn default() -> Self { Allowlist::DenyAll } }   // #[serde(default)] needs this
enum ApprovalMode { Never, OverCap, Always }
enum Effect { Deny }                                  // present-but-empty `rules` ⇒ everything denied
```

Per-tx caps and recipient/token allowlists are **per action**; the daily cap is **one global wall**
(a per-action daily cap with a single `SpendStore` counter would be incoherent — `Send` would consume
`Unshield`'s budget). Per-action daily budgets are a deferred, explicit `SpendStore`-schema change, not
v1.

**User-facing name:** in the app and docs this is called **"Rules"** (e.g. "the agent's Rules"). The
internal `deckard-contract` type stays `Policy` and the file stays `policy.json` to avoid extra wire
churn — a later cosmetic rename is optional, not part of v1.

### 2. Evaluation semantics (preserve the pure-function invariant)

`evaluate` / `evaluate_order` keep their signatures and their `Decision { Allow | Deny{reason} |
NeedsApproval{request_id} }` return, with `deny_reasons` tags and the `RequestId::ZERO` placeholder.
`evaluate` becomes:

1. `revoked` ⇒ `Deny{REVOKED}` — STOP stays a top-level brake, not a rule.
2. Find the rule whose `action` matches `intent.kind`. **No rule ⇒ `Deny{NO_RULE}`** (default-deny).
   Unknown action in the file, or a duplicate action, ⇒ the loader **fails loudly** (typed deser +
   duplicate check), never a verdict.
3. Calldata-shape check (the existing `calldata_ok` invariant: `Send` empty, `Shield`/`Unshield`/
   `ContractCall` non-empty) ⇒ `Deny{UNDECODABLE}` on mismatch.
4. Constraints: per-tx cap (rule) and the global daily cap, both in `U256` (`spent_today + value`);
   recipients/targets via the `Allowlist` lattice (`DenyAll` or off-`Only` ⇒ `Deny{OFF_ALLOWLIST}`).
5. `approval`: `Never` = allow-within-cap else `Deny{OVER_CAP}`; `OverCap` = allow-within-cap else
   `NeedsApproval`; `Always` = `NeedsApproval`.

**Swaps** stay on `evaluate_order`, unchanged except that it reads the `Swap` rule's `tokens`
allowlist (replacing the old `allow_swap_tokens`); it inspects no caps and **always** returns
`NeedsApproval` for a well-formed order. The `Swap` rule therefore carries `tokens` only — no
`approval`/`per_tx_cap` (those would be dead fields that lie to the policy author).

**Reachability honesty:** the v1 daemon denies anything but `Send`/`Shield` with `unsupported_v1`
*before* `evaluate` (`daemon.rs:546`), and the one signed `ContractCall` (the shaped relayer-approve)
is admitted by `shaped_approve_admission` and **skips `evaluate`** (`daemon.rs:529-540`, `:709-749`).
So `Unshield` and `ContractCall` rules are defined **forward-compat but not yet reachable** — an
author writing a permissive `Unshield`/`ContractCall` rule is not granting live authority. The ADR
states this so no one is misled.

**Consent and the daily cap (documenting EXISTING behavior, not a new rule):** a human-approved write
proceeds even if over the daily cap — `resolve` sets `approved` (`daemon.rs:483-486`) and `execute`
skips the cap re-check for approved requests (`daemon.rs:1131`). The daily cap is a hard wall only for
*auto-allowed* (no-human) writes, which `execute` **does** re-evaluate against current `spent_today`.
This is consistent with default-deny: an `ApprovalMode::Never` over-cap write has no card → `Deny`;
`OverCap`/`Always` raise a card whose approval carries the overage.

Send autonomy is therefore **entirely policy-expressed** — "always card" vs "within cap" vs "only to
these recipients" is what the `send` rule says, not a hardcoded branch.

### 3. No general policy engine enforces wei caps — native `U256` only

Verified 2026-06-23 against primary specs (3-0 adversarial vote): every mature, audited
general-purpose authorization engine maxes out at a **64-bit signed integer** (max ≈ 9.2 × 10¹⁸),
~58 orders of magnitude below the `uint256`/wei ceiling (2²⁵⁶−1 ≈ 1.15 × 10⁷⁷):

- **AWS Cedar** — the only integer type is `Long` (i64); the decimal extension is 4 digits; arithmetic
  past `Long` throws. ([cedar docs: datatypes](https://docs.cedarpolicy.com/policies/syntax-datatypes.html))
- **Biscuit** — spec: "An integer is a signed 64 bits integer"; operations fail on overflow.
  ([SPECIFICATIONS.md](https://github.com/eclipse-biscuit/biscuit/blob/main/SPECIFICATIONS.md))

So **cap enforcement lives in native Rust `U256`** in our own `evaluate`. No engine dependency can
do it correctly, and a silent narrowing-to-i64 would be a value bug in the trust core.

### 4. One vocabulary, two enforcers — do NOT unify the mechanisms

The constraint *vocabulary* (cap, allowlist, expiry, approval-mode) is shared. The *enforcer* is not:

- **`#48` per-origin grants** are **software-enforced** — `deckard-signerd` refuses to sign out of scope.
- **`#33` EIP-7702 session keys** are **chain-enforced** — an audited on-chain delegate refuses.

These have different trust roots, failure modes, and revocation. Modeling them as "the same caveat
mechanism" invites the single most dangerous error in the repo: claiming chain-strength for a software
check (`MANIFESTO.md` forbids it; `agent-authorization-map.md` keeps them separate). This ADR unifies
**naming only**. The grant/attenuation design stays with `#33`/`#48`, **deferred** (see ADR 0002).

When grants do land, composition is **monotonic narrowing** (a grant can only restrict the global
fence, never widen it) — and the `Allowlist` lattice (`Any` = ⊤, `DenyAll` = ⊥) is why that
intersection is well-defined instead of the old "empty Vec accidentally means any."

### 5. Defaults, the fail-safe, and migration

- **Built-in default** (no file): shield `approval: never` (auto-allow to self), send `approval: always`
  with `recipients: "any"` (every send is human-approved — maintainer default 2026-06-23), swap always cards.
- **Three named presets ship** (the `#29` variants), all default-deny: **shield-only** (one shield rule,
  everything else denied — the safest useful robot); **ask-me-everything** (send/shield/swap all
  `approval: always`, a human taps every action); **locked** (panic — `rules: []`, everything denied;
  a switchable frozen rulebook, distinct from the runtime `revoked` STOP brake).
- **Malformed or legacy-v0 `policy.json` ⇒ most-restrictive deny-all + a loud log.** Deliberate change
  from today's "permissive-but-capped" fallback (`policy_store.rs:24-36`). Deny is the safe direction;
  the loudness keeps it from looking like silent corruption.
- **`Allowlist` serde:** "omitted ⇒ `DenyAll`" is **not** free — it requires `#[serde(default)]` on the
  field plus `impl Default for Allowlist = DenyAll`. Without that, a missing `recipients` is a *parse
  error* → fallback (deny-all) — a different code path. The implementation MUST add the explicit
  default.
- **No silent semantic migration.** The old `allow_to: [] = any` does **not** auto-translate; a v0 file
  is detected (no `version` key) and **rejected loudly** with "rewrite to v1," never reinterpreted —
  otherwise every existing policy's recipient axis would flip from "any" to "deny-all" invisibly.
- **`policy.demo.json` is rewritten to v1** with explicit `recipients` (`"any"` only where intended);
  `just demo` must **upgrade-not-skip** a v0 file (today it installs-if-absent, `justfile:125-129`, so
  it would leave a stale flat file → deny-all → the demo's hero auto-shield silently stops). Document a
  `rm ~/.deckard/demo/policy.json` fallback.

## Standards posture (surveyed 2026-06-23, primary-source)

| Standard / library | Disposition | Why |
|---|---|---|
| **RFC 9396** Rich Authorization Requests (Proposed Standard) | **Adopt the shape** | `authorization_details` = array of typed objects with a required `type`; our `rules[]` keyed by `action` mirrors it. No runtime dep. |
| **ERC-7715 / ERC-7710** (both **Draft** as of 2026-06-23; reference impl audited by Consensys Diligence, Aug 2024, 0 critical) | **Adopt the shape (later)** | The delegation + caveat-enforcer model for `#33`/`#48` grants. Draft ⇒ not frozen; adopt the model, not Solidity-as-dependency. |
| **NIST SP 800-162 (ABAC)** / XACML | **Reference for vocabulary** | PEP (`signerd` gate) / PDP (`evaluate`) / PAP (authoring, *next*) / PIP (balances, `spent_today`, sim). Decision model maps to our `Allow`/`Deny`/`NeedsApproval` (≈ obligation). |
| **AWS Cedar** (`cedar-policy`, Lean-4-proven 7 properties, DRT, Apache-2.0) | **Design bar, NOT a dependency** | Best-in-class proof story, but the proofs cover Cedar's *semantics* not our binary (DRT is empirical), the structural checks are `Vec::contains`/`<`, and a second engine + serde boundary is *more* audit surface for the one place we keep parity tight. We hold `evaluate` to Cedar's properties (default-deny, forbid-trumps-permit, order-independence) via property tests instead. 64-bit `Long` also can't do wei. |
| **Biscuit** (`biscuit-auth`) | **Reject as a dependency; reference the model** | No completed third-party audit (FAQ: "informally audited"); 64-bit ints. Its offline append-only monotonic-narrowing attenuation is the right *concept* for `#33`/`#48` grants. |

**No single standard or library fits the whole problem.** The fit is a composition: RAR/ERC-7715
*shape*, ABAC/macaroon *vocabulary*, native-`U256` *enforcement*, single-pure-fn *implementation*.

## 6. Frozen-contract implications

Changing `Policy` is an **intentional, versioned breaking change** to the freeze-first
`deckard-contract` crate. `Intent`, `Decision`, the `deny_reasons` vocabulary, and the RPC enums stay
frozen — only `Policy` changes shape. Concretely:

- A **v0 file is detected by the absent `version` key** and rejected with a loud "unsupported legacy
  policy" message, not a generic parse-fail (which would read as corruption and silently deny-all).
- The JSON **and CBOR round-trip tests are rewritten** for the new shape (not deleted), preserving the
  byte-stable charter for v1.
- `PolicyGet` returns the new shape; every by-name reader of the flat fields is updated (see Cost).

## Invariants this must respect

- **`evaluate` / `evaluate_order` stay single pure functions** shared by `MockSigner` and the daemon
  (the parity contract). Extensibility comes from adding `Rule` variants, never from a data-driven
  interpreter or a second engine.
- **Typed, strict deserialization**; **loader rejects duplicate actions**; unknown/malformed ⇒ loud
  load failure ⇒ fail-safe deny-all, never a silent verdict.
- **Default-deny.** No matching rule ⇒ deny. `DenyAll`, not "empty = any." `default: "allow"` is
  forbidden (a wallet must not be allow-by-default).
- **Wei stays `U256`** end to end; the daily cap stays one global counter (single `SpendStore`).
- **STOP / `revoked` zeroizes and overrides everything**, re-checked at execute (TOCTOU guard), unchanged.

## Consequences

- **Positive:** adding a wallet action is a localized `Rule` variant + match arm + a rule in the demo
  policy — no god-struct churn; per-action approval modes; default-deny by construction; the
  `Allowlist` lattice removes the empty-Vec foot-gun and makes future grant-intersection sound.
- **Cost (full surface — a breaking change to a freeze-first crate):** `policy.rs` (`evaluate`,
  `evaluate_order`, `calldata_ok`); `policy_store.rs` (loader, default, v0 detection, fallback);
  the JSON+CBOR round-trip tests (`lib.rs`); `mock.rs` / `MockSigner::new`; the MCP acceptance test
  (`tests/harness_slice.rs`); `deckard-mcp/src/sidecar.rs` `policy_json()` (+ its test fixtures);
  `deckard-app/src/welcome.rs` `agent_policy_rows` (the GUI policy card); `policy.demo.json`; the
  `just demo-check` jq paths + intended-values (`justfile:467-485`); and `just demo` upgrade-not-skip.
  A CHANGELOG entry documents the demo-machine upgrade step.
- **Cap-reservation: verify, don't rebuild.** The daemon already serializes behind one mutex and
  re-evaluates auto-allows at execute (`daemon.rs:1131`); #108 added durable reserve-before-sign
  (`daemon.rs:1164`). The work is to **verify** that path covers the first *value-bearing* auto-allowed
  `Send` (today's only auto-allow is `value == 0` shield), not to build new reservation machinery.
- **Deferred:** in-app policy authoring / a `SetPolicy` RPC (file-based + named variants first, per
  `#29`); per-action daily budgets (a `SpendStore`-schema change); the `#33`/`#48` grant/attenuation
  model; `#31` wire-kind discovery (reframed as protocol message-kind discovery only, decoupled from
  authorization).

## Status / next step

**Accepted.** The executable work shipped as **one** issue (#135): the policy-model v2 foundation
(PR1, #160) + agent-send over MCP (PR2, #161) + the named starter presets (PR4, #162) + the
cap-reservation verification, with `evaluate`/`evaluate_order` mock⇄daemon parity green. `#29`
retargets to this schema — in-app authoring / a `SetPolicy` RPC is the next step (presets are
launch-time only today). `#31` is reframed and deferred; `#33`/`#48` are annotated to adopt this
vocabulary while keeping distinct enforcers.
