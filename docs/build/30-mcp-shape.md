# MCP Tool Surface (CLI + MCP sidecar)

> The agent-facing surface for Deckard, plus the freeze-first `Intent`/`Decision`/daemon-socket contract every other build doc codes against · serves demo beat 2 (agent shields on receive via MCP) + acceptance step "MCP sidecar registered in Claude Desktop, secrets never in transcript" (deliverable #6) · status (spec). Part of the Deckard build docs.

> **⚠ AMENDED AT LAUNCH — the `mcp.v0.1` profile (2026-06-10, per the approved
> open-source-demo plan).** The shipped surface (`crates/deckard-mcp`) is **7 tools**, every
> name `deckard_`-prefixed for Claude Desktop's shared tool namespace:
> `deckard_wallet_address`, `deckard_wallet_balance`, `deckard_policy_get`,
> `deckard_shield`, `deckard_status`, `deckard_execute`, `deckard_revoke_all`.
> `deckard_status` reads the approval state of a `request_id` (`pending` / `allowed` /
> `denied{reason}` / `expired`) plus a `remaining_ms` approval-TTL countdown and a `tx_hash`
> once executed — it is the agent's poll-for-approval read (maps to the additive
> `SignerRequest::StatusView`, NOT the existing `SignerRequest::Status`, which is unchanged).
> Two tools specced below were
> **cut from launch**: raw **`propose`** (security: it would let an untrusted client submit
> an arbitrary `Intent`, and the demo needs only `shield`+`execute`; a daemon-side
> `Shield.to == RelayAdapt` pre-check landed as defense-in-depth regardless) and
> **`simulate`** (deferred; app-native review is the v0.1 simulation surface — its
> post-launch home is the daemon, so the approval card and the agent see identical
> numbers). `wallet_balance` is public-only in v0.1: the shielded field is the honest
> string "unavailable — read it in the Deckard app (v1 limitation)", never a fake 0.
> Acceptance T1 asserts exactly the 7-tool profile; T3/T4 run via an over-cap `shield`;
> T5 is dropped with `simulate`; T9 is a structural allowlist transcript walk with a
> seeded canary. The rest of this doc is kept as written (the frozen wire contract is
> unchanged) — read it through that lens.

## Why this exists (2-4 sentences, concrete)

The hero beat is "the agent (Claude Desktop) auto-shields an inbound payment." Claude reaches Deckard through one Rust binary — `deckard-mcp` — that is **both a CLI and an MCP server** (the `@splits/splits-cli` pattern: one binary, `--mcp` auto-exposes every command as a tool). That binary is a **key-less client**: it never holds the secp256k1 key, never signs; it only proposes intents to the process-isolated signer daemon (`deckard-signerd`, owned by `00-test-harness.md`) and renders native approval cards. This doc **owns the freeze-first contract** (`Intent`, `Decision`, the daemon socket API) so T-Privacy, T-Custody, and T-Agent can build in parallel against frozen types.

## Where it sits — Depends on / Unblocks (cross-doc + demo)

**Depends on**
- `deckard-signerd` — the process-isolated signer daemon. The daemon's *implementation* and the STOP/revoke red-team test live in `00-test-harness.md`; this doc defines the **socket API it must expose** so both sides freeze the same wire contract.
- The shield path — `shield(amount)` ultimately lands in Kohaku's Railgun integration (`10-kohaku-shield.md`). The MCP `shield` tool produces a `kind: Shield` `Intent`; this doc owns the `Intent` shape, `10-kohaku-shield.md` owns what the daemon does with it.
- Helios over private RPC (`20-helios-sidecar.md`) — the read tools (`wallet_balance`, `simulate`) source verified state from Helios, not a raw vendor RPC.

**Unblocks / what this doc freezes for others**
- `00-test-harness.md` implements `deckard-signerd` against the socket API frozen here, and its STOP test asserts `revoke_all()` then `propose`/`execute` deny.
- `10-kohaku-shield.md` references the `Intent{kind:Shield}` shape and the `shield` tool.
- T-UX (deliverable #9) renders the native approval card this doc specifies (`needs_approval` → card → poll).

**Demo beat:** beat 2 (live receive → agent calls `shield` via MCP → approval → tx). Reliability backup (v1-demo-plan §Reliability): the same tool surface is callable by an in-app agent loop if Claude Desktop flakes on stage — so the MCP layer must be a thin shell over the daemon socket, with **no logic that only Claude can trigger**.

## Architecture / approach

```
┌─────────────────┐  MCP stdio (JSON-RPC 2.0)  ┌──────────────────┐  UDS (CBOR) ┌──────────────────┐
│ Claude Desktop  │ ─────────────────────────► │ deckard-mcp      │ ──────────► │ deckard-signerd  │
│ / Cursor / Codex│  list_tools / call_tool    │ (CLI + MCP, KEY- │  propose/   │ (holds key,      │
└─────────────────┘                            │  LESS)           │  execute/   │  process-isolated│
        ▲                                       └──────────────────┘  revoke_all │  policy gate)    │
        │ approval card (native, GPUI)                   │                       └──────────────────┘
        │                                                │ raise card / read status        │
        └──────────────── Deckard GPUI app ◄─────────────────────────────────────────────┘
                          (renders the card, owns the lock screen / STOP button)
```

Three processes, three trust levels:

1. **`deckard-mcp`** — the agent surface. Key-less. Translates tool calls → `Intent` → daemon `propose`/`execute`. Speaks MCP over stdio to the LLM host and a Unix-domain-socket (UDS) RPC to the daemon. This is the **anti-pattern inversion** of `mcpdotdirect/evm-mcp-server` / `dcSpark/mcp-cryptowallet-evm`, which load the raw key (`EVM_PRIVATE_KEY` / `EVM_MNEMONIC`) into the MCP process where the tool layer can reach it (05-agentic-wallets.md [10]). Deckard's MCP process has **no key material at all**.
2. **`deckard-signerd`** — holds the decrypted key in its own address space, runs the policy gate (`Decision`), signs. Defined here as a socket; implemented in `00-test-harness.md`.
3. **Deckard GPUI app** — owns the user. It renders the **native approval card** (not a browser `approvalUrl` like Base MCP) and the STOP button, and it is where the keystore is unlocked.

The CLI and the MCP server are the **same binary, same command tree** — `cli.serve()` for the CLI, `--mcp` for the server, exactly the Splits/`incur` shape: "no manual config, no copy-pasting tool definitions" (04-splits.md [1]; verified: `splits-cli` v0.2.9 depends on `incur ^0.3.13` + `viem ^2.48.2`, bin `splits` → `dist/cli.js`). Every CLI subcommand auto-registers as an MCP tool with the snake_case name `namespace_command` (Splits: `transactions list` → `transactions_list`, `accounts get` → `accounts_get` — verified against the published source).

## Concrete interface (commands, types, crate names, RPC methods, file layout)

### Crates

- MCP server: **`rmcp`** (the official Rust MCP SDK, `modelcontextprotocol/rust-sdk`) — provides `#[tool]` macros, stdio + streamable-HTTP transports, JSON-RPC 2.0 framing. ⚠ confirm the current `rmcp` version supports the stdio + streamable-HTTP transports we need at build time.
- CLI parsing: **`clap`** v4 (derive). The same command structs feed both `clap` and the `rmcp` tool registry via a thin macro/codegen layer (our equivalent of `incur`).
- Daemon RPC: UDS via **`tokio`** `UnixListener`/`UnixStream`; framing with **`serde`** + **`ciborium`** (CBOR — compact, no string-quoting of binary calldata).
- Types crate: **`deckard-contract`** — a `no_std`-friendly crate holding `Intent`, `Decision`, `Policy`, and the RPC enums, depended on by `deckard-mcp`, `deckard-signerd`, and the GPUI app so the contract is one source of truth.
- EVM types: `alloy-primitives` (`Address`, `U256`, `Bytes`) — already in `Cargo.toml`.

### THE FREEZE-FIRST CONTRACT (owned here)

```rust
// crate: deckard-contract  — frozen 2026-06-05, reference, do not redefine elsewhere.
use alloy_primitives::{Address, U256, Bytes, B256};

/// What the agent wants to do. The ONLY thing that crosses mcp → daemon for a write.
/// The agent never sends raw signed bytes — only intent; the daemon decides + signs.
pub struct Intent {
    pub to:       Address,        // target (token contract, Railgun adapter, recipient)
    pub token:    Option<Address>,// None = native ETH; Some = ERC-20 contract
    pub value:    U256,           // wei (native) or token base units
    pub calldata: Bytes,          // empty for a plain send; encoded call otherwise
    pub kind:     IntentKind,     // discriminator the policy gate switches on
}

pub enum IntentKind {
    Send,                 // plain transfer
    Shield  { /* Railgun deposit; see 10-kohaku-shield.md for adapter/calldata */ },
    Unshield,
    ContractCall,         // generic write (forward-compat for plugins)
}

/// The daemon's verdict. Returned by `propose`. The agent cannot forge `Allow`.
pub enum Decision {
    Allow,                                  // within policy → safe to `execute`
    Deny          { reason: String },       // policy violation; terminal
    NeedsApproval { request_id: RequestId },// human must approve via native card
}

pub type RequestId = B256;  // opaque; the agent polls status on it

/// Policy the agent is allowed to READ (so it can stay inside its fence) but never write.
pub struct Policy {
    pub per_tx_cap_wei:      U256,
    pub daily_cap_wei:       U256,
    pub spent_today_wei:     U256,
    pub allow_to:            Vec<Address>,  // empty = any
    pub auto_shield_min_wei: U256,          // the demo rule: auto-shield inbound ETH ≥ X
    pub require_approval:    ApprovalMode,  // Never | OverCap | Always
    pub revoked:            bool,           // set true by revoke_all / STOP
}

pub enum ApprovalMode { Never, OverCap, Always }
```

### Daemon socket API (the wire the harness implements)

> **Status: implemented in `crates/deckard-signerd` (issue #4).** This doc owns the wire; the daemon implements it. v1 scope = `Send` only (Shield → T-Privacy, Helios reads → `20`, `deckard-mcp` → `30`).

UDS at `$XDG_RUNTIME_DIR/deckard/signerd.sock` (Linux) — macOS fallback `$TMPDIR/deckard-$UID/signerd.sock` — socket mode `0600` inside a `0700` dir, **length-delimited CBOR** (4-byte big-endian length prefix + body, max 1 MiB), one request per frame. Caller auth is `SO_PEERCRED`/`LOCAL_PEERCRED` **same-uid only**; single-instance via `flock` on a sibling `signerd.lock`.

```rust
// deckard-mcp / deckard-app (key-less) → deckard-signerd
enum SignerRequest {
    Unlock    { passphrase: String },             // -> Unlock(UnlockOutcome)  (decrypt + hold the key)
    Lock,                                          // -> Ack  (zeroize the key → Locked; deny in-flight)
    Resolve   { request_id: RequestId, approved: bool }, // -> Ack  (close an approval loop)
    Propose   { intent: Intent },                 // -> Decision   (policy check, NO signing yet)
    Execute   { request_id: RequestId },          // -> ExecuteResult (sign + broadcast; only if Allow/approved)
    Status    { request_id: RequestId },          // -> ApprovalStatus (poll for native-card result)
    RevokeAll,                                     // -> Ack  (STOP: zeroize the key → Locked, deny in-flight)
    PolicyGet,                                     // -> Policy (read-only snapshot for the agent)
    // read-only, key-less helpers:
    Address,                                       // -> Address (or Deny{"locked"} when Locked)
    Balance   { shielded: bool },                 // -> BalanceReport (public only in v1; shielded_wei = 0)
}

enum ExecuteResult { Broadcast { tx_hash: B256 }, Denied { reason: String } }
enum ApprovalStatus { Pending, Allowed, Denied { reason: String }, Expired }
enum UnlockOutcome { Unlocked { address: Address }, BadPassphrase, NoVault }
```

**Unlock / Lock / Resolve (the operator state machine).** The daemon is `Locked` (no key) ⇄ `Unlocked { vault }`:
- `Unlock{passphrase}` reads the keystore (`deckard-core`'s `vault.bin` in the config dir), decrypts, and holds the key → `Unlocked{address}`. The wire passphrase is a plain `String` (`Zeroizing` isn't `Serialize`); the daemon moves it into `Zeroizing` on receipt, scrubs the raw frame, and never echoes or logs it. Wrong passphrase / tampered vault → `BadPassphrase`; missing file → `NoVault`.
- `Lock` and `RevokeAll` both zeroize + drop the key → `Locked` and deny every in-flight approval (`Pending` **and** `Allowed`). Re-arm only via a fresh `Unlock` (which also starts a clean request session).
- `Resolve{request_id, approved}` closes the loop a `NeedsApproval` opened: it flips that `Pending` record to `Allowed`/`Denied`. Without it nothing turns `Pending` into executable; the native GPUI card (T-UX) is the human-facing caller.

Invariants frozen here, asserted by `crates/deckard-signerd/tests/*` (cross-process red-team → `00-test-harness.md`):
- **One decision function.** `policy::evaluate(&Intent, &Policy) -> Decision` is the single source of the verdict; both `MockSigner` and the daemon call it (parity is unit-asserted), so the mock and the real daemon can never drift. The daemon adds only process-level pre-checks `evaluate` can't express (`Locked` → `Deny{"locked"}`, `chain_id` mismatch → `Deny{"chain_mismatch"}`, `kind != Send` → `Deny{"unsupported_v1"}`).
- `Propose` **never signs** and never broadcasts. It returns a `Decision`. A `Decision::Allow`/approved `RequestId` is the *only* token that lets `Execute` sign. (v1 `RequestId` = `keccak256` of a stable encoding of the intent, so a client that got `Allow` derives the id locally to execute it; a `NeedsApproval` id rides the wire. Production should switch to a salted, returned id.)
- `Execute` re-checks **policy** at sign time (TOCTOU guard): an approval granted before `Lock`/`RevokeAll` is still denied (`Denied{"revoked"}`); an *auto*-allow is re-run against the spend caps against the **current** `spent_today` (`Denied{"cap_exceeded"}`) so two within-cap proposals can't both broadcast past the daily cap — a human-approved overage carries its own consent and isn't re-capped. A broadcast id never signs twice (`Denied{"already_executed"}`); a stale (TTL-expired) request — `Pending` **or** `Allowed` — is `Denied{"expired"}`. A re-`Propose` of an identical intent is idempotent (it never resets a live card's TTL, downgrades an approval, or re-raises a `Denied`). It builds an **EIP-1559** tx via alloy fillers (pending nonce, fee/gas estimation, `chain_id` from the intent) and broadcasts via the config RPC (`DECKARD_RPC_URL`/`DECKARD_CHAIN_ID`) under a bounded timeout.
- **Policy (v1):** loaded from `policy.json` in the config dir, with a safe default if absent (per-tx 0.05 ETH, daily 0.2 ETH, empty allowlist = any, auto-shield-min 0.01 ETH, approval-over-cap). `spent_today_wei` is in-memory, UTC-midnight rollover, resets on restart (cross-restart persistence is a fast-follow). No `SetPolicy` yet.
- `RevokeAll` is idempotent and irreversible for the session (re-arm only via `Unlock`, `08-security-keystores.md`).
- The MCP/app process holds **no key, no decrypted seed, no signing capability** — verified by the red-team script in `00-test-harness.md` (memory + fd scan finds no key; there is no UDS method that returns raw key bytes).

### MCP tool surface (concrete list)

Read tools (no approval, key-less, safe to call freely — the "observe" half, 05-agentic-wallets.md [21]):

| Tool | Maps to | Returns | Approval |
|---|---|---|---|
| `wallet_address` | `SignerRequest::Address` | `{ address }` | none |
| `wallet_balance` | `SignerRequest::Balance{shielded}` | `{ public_wei, shielded_wei, token_balances[] }` (Helios-verified, 20-helios-sidecar.md) | none |
| `simulate` | local eth_call/fork against Helios state | `{ asset_changes[], gas, warnings[] }` (Tenderly-style preview, 05 [13]) | none |
| `policy_get` | `SignerRequest::PolicyGet` | `Policy` snapshot | none |
| `status` | `SignerRequest::StatusView{request_id}` (additive; the existing `Status` path is unchanged) | the approval state of a `request_id`: `pending` / `allowed` / `denied{reason}` / `expired`, plus `remaining_ms` (approval-TTL countdown), `tx_hash` once executed, and a `lifecycle` label | none |

> ⚠ **Cross-doc need (from `20-helios-sidecar.md` "Integration into the app"):** the `wallet_balance` and `simulate` responses must carry a `read_status` field (`ReadStatus { Verified | Degraded | Unsynced }`), and `ReadStatus` should be defined in `deckard-contract` (here) since it rides the wire. Without it the "never silently serve an untrusted read" rule isn't enforceable at the contract level. `20` owns the semantics/transitions; `30` owns the final type + field placement.

Write tools (route through `propose` → `Decision`; "execute validated intents, not raw LLM suggestions", 05 [10]):

| Tool | Builds | Approval |
|---|---|---|
| `propose` | `Intent` → `Decision` | returns `needs_approval` when over cap / `ApprovalMode::Always` |
| `execute` | `Execute{request_id}` | only succeeds on `Allow` or an `Allowed` approval |
| `shield` | `Intent{kind:Shield}` (the demo HERO; calldata from `10-kohaku-shield.md`) | per `Policy.require_approval`; demo runs `auto_shield_min_wei` with `Never`/`OverCap` so the beat is hands-free |
| `revoke_all` | `RevokeAll` | none to call; it *is* the brake (STOP). Always available. |

The agent's demo loop: receive watcher fires → `wallet_balance` → `simulate` the shield → `shield(amount)`. If `Decision::Allow`, `execute`; the auto-shield rule keeps beat 2 free of a human prompt.

### Approval flow for writes (native card, not a browser URL)

```
agent: propose(Intent)  ──► daemon: Decision::NeedsApproval{ request_id }
                                          │
Deckard GPUI raises a NATIVE card ◄───────┘  (shows simulate() asset-changes + to/value)
   user taps Approve / Deny on the desktop, in-process
                                          │
agent: poll status(request_id) every ~750ms ──► Pending → Allowed | Denied{reason} | Expired
   on Allowed: agent calls execute(request_id) ──► tx_hash
```

Contrast with **Base MCP** (verified against `docs.base.org/ai-agents`): a write returns `{ approvalUrl, requestId }`, the user opens a **browser/Base Account** link, and the assistant polls `get_request_status(requestId)` until `confirmed` (05 [4]). Deckard keeps the identical poll *shape* (`status(request_id)`) but the review surface is a **native GPUI card** — local-first, no browser round-trip, no hosted account, and the card reuses Deckard's own `simulate` output. Approvals expire (`ApprovalStatus::Expired`) so a stale `request_id` can't be executed later.

### Security discipline (from Splits)

- **MCP mode refuses flag-based secrets.** Verified Splits behavior: with `SPLITS_MCP_MODE=1` the CLI "refuses flag-based secrets (`--api-key`, `--private-key`) so secrets don't appear in tool-call transcripts" and "the private key never appears in any command's response — only the derived address." Deckard mirrors this: when launched with `--mcp` (or `DECKARD_MCP_MODE=1`), `deckard-mcp` **hard-rejects** any flag that could carry a secret (`--passphrase`, `--rpc-token`, `--key`); secrets live only in a `0600` config or the OS keychain, and stdin is the only accepted entry path. This is largely moot because `deckard-mcp` is key-less, but the rule still guards RPC tokens and the keystore passphrase from entering tool-call transcripts.
- **No secret in any response.** Read tools return addresses/balances/policy only — never key bytes, never the passphrase, never an RPC bearer token.
- **The sidecar is key-less.** The key boundary is the daemon's process, not a flag.

### Transport

- **stdio (primary)** — JSON-RPC 2.0 over stdin/stdout, the Claude Desktop / Cursor / Codex registration path. Registration mirrors Splits' `claude mcp add splits -e SPLITS_API_KEY=... -- npx @splits/splits-cli --mcp`; Deckard:
  `claude mcp add deckard -- /usr/local/bin/deckard-mcp --mcp`  (no secret env var needed — it is key-less; it auto-discovers `signerd.sock`).
- **Optional authenticated localhost HTTP** — `--mcp-http --bind 127.0.0.1:7423 --auth-token-file <0600>`, for the in-app backup driver and for clients that don't speak stdio. Bound to loopback only; bearer token from a `0600` file, never a flag.
- **Caller auth + single-instance:** UDS to the daemon uses **peer-cred** (`SO_PEERCRED` / `LOCAL_PEERCRED`) so only the same-uid Deckard/MCP process connects; the daemon is single-instance (flock on the socket dir). HTTP mode adds the bearer token on top. `revoke_all`/STOP is reachable on every transport.

## v0 baseline / spike plan + acceptance test (agent-runnable asserts)

**v0 baseline (today):** none of this exists. `src/wallet.rs` is a plaintext-hex EOA with the signer **in-process** (the anti-pattern). The freeze-first job is: publish `deckard-contract` (the types above) and a **mock `deckard-signerd`** that answers the socket API from an in-memory policy, so T-Agent/T-UX build before the real daemon (`00-test-harness.md`) lands.

**Spike order (½ day, the "freeze first" of v1-demo-plan §Parallel tracks):**
1. Publish `deckard-contract` with the types above; `cargo build`.
2. Stand up `deckard-mcp` over `rmcp` stdio exposing the 8 tools, talking to a **mock daemon** (in-memory `Policy`, deterministic `tx_hash`).
3. Write an MCP test client (Rust, `rmcp` client, or `@modelcontextprotocol/inspector`) that lists + calls each tool.
4. Claude Desktop dry-run: register, confirm tools appear, run the demo loop against the mock.

**Acceptance test (agent-runnable; the shot-list style of v1-demo-plan):**

```
Scenario "MCP surface: read-free, write-gated, secret-tight" (mock daemon, then real):
  setup: deckard-contract built; deckard-mcp --mcp talking to a mock signerd
         with Policy{ per_tx_cap_wei: 0.05e18, auto_shield_min_wei: 0.01e18,
                      require_approval: OverCap, revoked: false }.

  T1 list_tools                          assert: exactly {wallet_address, wallet_balance, simulate,
                                                 policy_get, propose, execute, shield, revoke_all}
  T2 call wallet_address / wallet_balance / policy_get
                                         assert: succeed with NO approval; response JSON contains
                                                 no 64-hex-char key, no "passphrase", no bearer token
  T3 propose(Intent{kind:Send, value: 0.2e18})  // over per_tx_cap
                                         assert: Decision == NeedsApproval{request_id}  (NOT Allow)
  T4 execute(request_id) before approval assert: ExecuteResult::Denied (no tx, never signs on Pending)
  T5 simulate the over-cap shield        assert: returns asset_changes + warnings, signs nothing
  T6 shield(0.02e18)  with require_approval=OverCap and 0.02 ≤ per_tx_cap
                                         assert: Decision == Allow; execute → tx_hash present
  T7 secret-refusal: invoke any tool with --passphrase=x / --key=x in MCP mode
                                         assert: hard error "secrets not accepted in MCP mode";
                                                 the rejected value never echoed in the response
  T8 revoke_all(), then execute(prior Allowed request_id)
                                         assert: Denied{reason:"revoked"}  (TOCTOU guard holds)
  T9 transcript scan: grep the ENTIRE tool-call transcript (T1..T8)
                                         assert: zero 64-hex-char strings, zero "passphrase",
                                                 zero RPC bearer tokens  ← the key-leak gate (deliverable #6)
  --- Claude Desktop dry-run (manual, recorded) ---
  D1 `claude mcp add deckard -- deckard-mcp --mcp`; tools list in the UI
  D2 ask Claude to "shield 0.02 ETH"; assert it calls simulate → shield → execute,
     an over-cap amount raises a NATIVE card (no browser opens), and STOP denies the next execute.
```

T9 is the demo's load-bearing assertion (v1-demo-plan deliverable #6: "secrets never in transcript"). It runs headless in CI; D1–D2 are the on-camera rehearsal.

## Risks & fallbacks

- **`rmcp` maturity / churn.** The official Rust MCP SDK is young. *Fallback:* hand-roll the JSON-RPC 2.0 stdio framing (it's small) behind the same tool registry; the contract crate is transport-agnostic so the swap is local. ⚠ unverified: exact `rmcp` version + transport set at build time.
- **`incur`-style auto-exposure has no Rust equivalent.** We replicate it with a `clap`-derive → `rmcp`-tool macro. *Fallback:* register the ~8 tools by hand — the surface is small enough that hand-registration is cheap and the "auto" property matters more for a 40-command CLI than for ours.
- **Approval-poll latency vs. agent speed** (the 05-agentic-wallets §safe-signing tension): per-write human prompts collapse agent speed. *Resolution (already in the design):* the auto-shield rule runs `ApprovalMode::Never`/`OverCap` inside the policy fence so the HERO beat needs no prompt; cards fire only over cap.
- **Claude Desktop flakes on stage** (v1-demo-plan §Reliability backup). *Fallback:* the in-app agent loop calls the same `deckard-mcp` over localhost HTTP — guaranteed because the MCP layer is a thin shell over the daemon socket with no Claude-only logic.
- **UDS peer-cred portability.** `SO_PEERCRED` (Linux) vs `LOCAL_PEERCRED` (macOS) differ. *Fallback:* on macOS gate on socket file mode `0600` + `$XDG_RUNTIME_DIR` owner-only dir; add a per-launch nonce in the socket dir.

## Open questions

- Does `Intent` need a `chain_id`/nonce field at freeze time, or does the daemon own nonce/chain entirely? (Leaning: daemon owns it — the agent should not pick nonces. Confirm with `00-test-harness.md`.)
- Should `simulate` live in `deckard-mcp` (key-less, calls Helios directly) or in the daemon? Putting it in the daemon keeps one Helios client; putting it in MCP keeps the daemon minimal. (Leaning: daemon, so the approval card and the agent see identical numbers.)
- Approval-card timeout default (30s? 60s?) and whether `Expired` auto-denies or requires re-propose.
- For the localhost-HTTP backup driver, is a static `0600` bearer token enough, or do we want per-launch token rotation?

## Sources (repos + docs, linked)

- splits-cli — one binary CLI+MCP, `--mcp` auto-exposure, `SPLITS_MCP_MODE=1` secret refusal, key never in responses — https://github.com/0xSplits/splits-cli (verified: v0.2.9, deps `incur ^0.3.13` + `viem ^2.48.2`, bin `splits`→`dist/cli.js`, tool naming `namespace_command`)
- incur (the CLI→MCP framework Splits builds on) — https://github.com/wevm/incur
- Base MCP approval flow — `send()`/`swap()` return `{approvalUrl, requestId}`, assistant polls `get_request_status(requestId)` until confirmed, smart wallet signs server-side, keys never exposed to AI — https://docs.base.org/ai-agents (verified)
- Coinbase Payments MCP — local desktop, no API key, x402 pay + spend limits + approval thresholds — https://www.coinbase.com/developer-platform/discover/launches/payments-mcp (05-agentic-wallets.md [3])
- Anti-pattern: raw-key EVM MCP servers — `EVM_PRIVATE_KEY`/`EVM_MNEMONIC` in the MCP process — https://github.com/mcpdotdirect/evm-mcp-server · https://github.com/dcSpark/mcp-cryptowallet-evm
- rmcp — official Rust MCP SDK — https://github.com/modelcontextprotocol/rust-sdk
- MCP spec (JSON-RPC 2.0, tools, stdio) — https://modelcontextprotocol.io/specification
- Kohaku (Railgun shield path; `ethereum/kohaku`, TypeScript-primary with ~476KB Rust — standalone-Rust-crate consumability is R1, owned by `10-kohaku-shield.md`) — https://github.com/ethereum/kohaku · https://ethereum.github.io/kohaku/railgun/intro/ (⚠ Rust-crate-stability unverified here by design)
- Safe-signing canon (simulate-before-sign, scoped policy, human-in-loop, key isolation) — 05-agentic-wallets.md [1][2][13]
