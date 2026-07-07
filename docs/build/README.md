# Deckard Build Specs

> Concrete, parallelizable implementation specs for the v1 demo. These implement
> [`../research/v1-demo-plan.md`](../research/v1-demo-plan.md) (the locked demo) and draw on the research
> KB in [`../research/`](../research/). Written + repo-verified 2026-06-05.

**The demo they build toward:** *receive money → it's instantly private → and you can't switch it off* —
live on mainnet, agent-driven (Claude Desktop via MCP), shielded via Railgun, verified by Helios, with the
**walkaway** beat (cut the RPC on camera, Deckard keeps working). CROPS-aligned, open-source, self-custodial.

## The docs

> **Live build status for every track lives in [`STATUS.md`](../../STATUS.md) — the single source of truth.**
> This page is the spec *index* (what each doc owns), not a status surface; the deep docs are the spec.

| Doc | Owns |
|---|---|
| [`00-test-harness.md`](00-test-harness.md) | The **test substrate**: local lanes (anvil fork / Sepolia) + a headless agentic runner that drives the shot-list and self-asserts; mainnet fixtures; CI. |
| [`10-kohaku-shield.md`](10-kohaku-shield.md) | The **hero action**: auto-shield via Kohaku's pure-Rust `railgun` crate. **R1 retired** — full shield→unshield runs from our edge, and shield is *instant* (no client proof; ZK proving is on the spend). |
| [`20-helios-sidecar.md`](20-helios-sidecar.md) | **Trustless reads + walkaway** via embedded `helios-ethereum` 0.11.1 (git-only). **R2 proven** + verified reads **integrated** into `EthProvider`/signerd behind `verified-reads`. |
| [`30-mcp-shape.md`](30-mcp-shape.md) | The **agent surface** (one binary = CLI + MCP server, key-less) **and the freeze-first contract**. Contract + daemon socket built; the `deckard-mcp` binary is not yet. |
| [`31-agent-quickstart.md`](31-agent-quickstart.md) | The **canonical agent-facing quickstart**: install one-liner, the 6 `mcp.v0.1` tools, policy fields, every deny reason with its fix, demo constants. The tool list is drift-guarded by a test in `deckard-mcp`. |
| [`32-agent-loop-prompt.md`](32-agent-loop-prompt.md) | The **first agent demo runner** (#61): a Claude Desktop MCP prompt that loops — poll balance → detect a deposit → shield within the policy → execute — idempotent via the agent's own context, backpressure on over-cap (the human approves in the Activity feed, #60), STOP-aware. |
| [`40-wire-evolution.md`](40-wire-evolution.md) | The **wire-contract evolution rules** (#31): `Hello` capability discovery, the five additive-evolution rules, and the capability registry (`core`, `mcp.v0.1`). The keystone for new request kinds (7702, x402 #32/#34, dapp-origin #198/#204). Registry mirrored in `deckard-contract/src/capabilities.rs`. |

## Build order (what gates what)

```
        ┌─ freeze the contract (deckard-contract crate: Intent / Decision / Policy / daemon UDS API) ─┐  [owned by 30]
        │                                                                                              │
   00 harness Lane A (anvil fork) + fixtures/addresses.mainnet.json + FakeModel runner  ◄─────────────┘
        │  (the substrate everything tests against — build FIRST)
        ▼
   ┌────────────┬─────────────────┬──────────────────┬───────────────────┐
   │ T-Custody  │ T-Privacy (10)  │ T-Trustless (20) │ T-Agent (30)      │   ← run in parallel
   │ keystore→  │ railgun shield  │ Helios lib +     │ MCP binary +      │
   │ signer     │ spike (R1) on   │ walkaway         │ contract impl;    │
   │ daemon     │ fork/Sepolia    │ supervisor (R2)  │ FakeModel first,  │
   │            │                 │                  │ Claude Desktop    │
   └────────────┴─────────────────┴──────────────────┴───────────────────┘
        ▼
   integrate on Lane B (Kurtosis EL+CL) / Lane C (Sepolia) → mainnet hero when green
```

**Start immediately, in parallel:** the **contract crate** (tiny, unblocks all), **harness Lane A**, and the
two risky hero spikes (**10** shield, **20** walkaway). T-Custody and T-Agent build against the frozen
contract + FakeModel before the daemon/Claude are wired.

## The freeze-first contract (owned by `30-mcp-shape.md`)

A shared `deckard-contract` crate: `Intent{to,token,value,calldata,kind}` · `Decision{Allow | Deny{reason}
| NeedsApproval{request_id}}` · `Policy` (agent-readable) · the daemon UDS socket API
(`propose / execute / status / revoke_all / policy_get / address / balance`). Every other track codes
against this; the harness's `FakeModel` exercises it before any LLM is in the loop.

## Shared seams (single sources of truth — don't fork them)

- **`deckard-contract` crate** — the types above. (30 owns; 00/10/20 reference.)
- **`fixtures/addresses.mainnet.json`** — Railgun + USDC + whale addresses. (00 hosts; 10 fills Railgun set.)
- **EIP-1193 provider** — Helios plugs into `RailgunBuilder::new(chain, impl IntoEip1193Provider)`. Note (per `20`
  "Integration into the app"): `EthereumClient` is *not* EIP-1193 natively, so v1 = Helios's localhost JSON-RPC
  server on the primary client (no supervisor failover for Railgun's reads), prod = a `HeliosEip1193` adapter over
  the supervisor. The daemon's own `wallet_balance`/`simulate` reads use the typed supervisor (with failover).
  (20 provides; 10 + T-Core consume.)
- **`ReadStatus { Verified | Degraded | Unsynced }`** — attached to every read; the UI/agent must see it;
  **never silently fall back to untrusted RPC.** (20 owns the semantics; the type belongs in `deckard-contract` and
  the `read_status` field on `wallet_balance`/`simulate` is **proposed, not yet frozen** in `30`.)

## The two hero-beat spikes

- **R1 — shield from Rust (10):** ✅ **retired + integrated.** Kohaku's `railgun` crate runs full
  shield→transfer→unshield from our own dep edge (`spikes/shield-railgun/`), and the shield is now wired into
  Deckard (key-less builder in `deckard-core` + signerd calldata-broadcast + a black-box `shield_e2e`). Proving
  time *measured*: **shield is instant** (no client ZK proof, ~ms); ZK proving is on the *spend* (~10s cold,
  ~halved with `parallel`) — so "instant auto-shield, prove-on-spend" is honest. Still open: railgun's per-crate
  license (no upstream `license` field) + the `vendor/eip-1193-provider` fork — resolve before ship.
- **R2 — walkaway (20):** ✅ proven on mainnet **+ verified reads integrated.** Helios has **no native multi-EL/CL
  failover** (one client = one EL + one CL); the head is **consensus-driven and EL-independent** (served from
  cache), so cutting the EL keeps the head live while a second synced client recovers state reads via Deckard's
  own supervisor (Shape A). Both read paths (`EthProvider` + signerd) now route through Helios behind a
  `verified-reads` feature. **Key finding: cut the *EL* on camera, never the *CL*** — a dead CL freezes the head
  and Helios won't self-heal. The CL is the fragile, no-SLA dependency; Nimbus + dRPC are the two proven CLs.

## Acceptance = the shot list (lives in `00-test-harness.md`)

One headless scenario (`receive < N s → shield asserts balance↑/public↓/link-broken → cut RPC asserts
still-verified`) that an AI coding agent runs to self-verify. Green on Lane A/C ⇒ the mainnet video is shootable.

## Tracked cross-doc open questions

- ~~Does the Kurtosis CL serve the light-client beacon API out of the box, or need flags?~~ **Resolved + DEFERRED in `20`:** yes, OOTB (Lighthouse/Nimbus/Lodestar serve LC by default; use `cl_type: lighthouse`). But the mainnet spike proved R2 **without** Kurtosis, so the Kurtosis lane is **deferred off the v1 critical path** (post-demo hermetic-CI nice-to-have). v1 tests on mainnet + Sepolia public endpoints. — `20`/`00`
- Does Helios's EIP-1193 provider serve the log ranges Railgun UTXO sync needs, or does Subsquid carry history? — `10`/`20`
- `simulate` in the MCP binary (key-less, calls Helios) vs in the daemon (agent + approval card see identical numbers)? — `30`/`20`
- `railgun` crate license inheritance vs Deckard's 0BSD posture. — `10`
- `rmcp` (official Rust MCP SDK) version/transports vs hand-rolled JSON-RPC stdio. — `30`

## Not here (fast-follow — see `../research/roadmap.md`)

STOP-on-camera beat · allocate/donate slice · **EIP-7702 session keys** · **x402 / MPP as wallet plugins**
(the pluggable MCP tool registry is designed for this) · stealth addresses · hardware-wallet signing · audit.
