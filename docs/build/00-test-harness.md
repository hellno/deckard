# 00 · v0 Test Harness — local devnet + agentic self-test

> Purpose: a fully-controlled local chain + a headless agentic runner so the agent, CI, and an AI coding agent can self-test every later feature. · Serves: the whole demo acceptance test (shot-list steps 1–5 of [v1-demo-plan.md](../research/v1-demo-plan.md)) — this doc is the substrate the other three build docs test against. · Status: spec. Part of the Deckard build docs.

## Why this exists

The v1 demo is one continuous mainnet recording of `receive → instant auto-shield → walkaway` (v1-demo-plan §"The video"). You cannot rehearse that on mainnet — it costs gas, you can't *trigger* an inbound payment on cue, and you can't safely cut RPCs. So before any feature lands we build a local environment we fully control plus a headless runner that drives the exact shot-list and asserts pass/fail. The shot-list **is** both the CI gate and the storyboard (v1-demo-plan §"Acceptance test = the shot list"), so the harness is the single thing that proves "shootable, today."

## Where it sits — Depends on / Unblocks

- **Depends on:** Foundry (anvil/cast/forge), Docker + Kurtosis (for the Helios lane), Sepolia RPC keys. The Intent/Decision/daemon-socket contract is **owned by [30-mcp-shape.md](30-mcp-shape.md)** — this harness drives it but does not define it.
- **Unblocks:** [10-kohaku-shield.md](10-kohaku-shield.md) (shield/unshield spiked on the anvil-fork lane), [20-helios-sidecar.md](20-helios-sidecar.md) (verified reads + walkaway on the Kurtosis/Sepolia lanes), [30-mcp-shape.md](30-mcp-shape.md) (the runner is a deterministic, LLM-free client of the daemon socket — proves the contract before Claude Desktop is in the loop).
- **Demo:** every beat. Step 1 receive-watcher, step 2 shield, step 3 walkaway, fast-follow steps 4 STOP / 5 allocate.

## Architecture / approach

Three lanes, because **one chain can't do everything** and the real constraint is Helios.

> **The Helios constraint, stated honestly.** Helios is a light client: its consensus layer verifies the execution layer against the beacon chain's **sync committee**, rooted at a trusted weak-subjectivity **checkpoint**; the execution layer then uses an *untrusted* EL RPC for verified data ([a16z/helios README](https://github.com/a16z/helios/blob/master/README.md), [config.md](https://github.com/a16z/helios/blob/master/config.md)). Concretely Helios needs **two** upstreams: a `--consensus-rpc` that "must be a consensus node that supports the light client beaconchain api," and an `--execution-rpc` that "must be an execution node that supports the light client execution api" (README/config.md). **A bare `anvil --fork-url` has no beacon chain and no CL at all**, so Helios cannot point at plain anvil. This is the single fact that shapes the lane split — do not paper over it.

| Lane | Chain | Helios? | What it's for | Cost/speed |
|---|---|---|---|---|
| **A · anvil-fork** | `anvil --fork-url <mainnet>` | **No** (no CL) | Fast EL iteration: shield/unshield spikes, receive-watcher, send tx, MCP/daemon contract, agent loop. The default dev + CI lane. | seconds, free |
| **B · Kurtosis devnet** | `ethpandaops/ethereum-package` (EL + CL over Docker) | **Yes**, end-to-end local | Full Helios integration + walkaway with zero external dependencies; CL light-client API local. | minutes to spin up |
| **C · Sepolia** | public Sepolia | **Yes** (public beacon + EL light-client RPC) | Helios/walkaway integration against a real network; Kohaku/Kohaku-extension is Sepolia-only ([03-kohaku.md](../research/03-kohaku.md)); shield fallback target if the alpha Railgun crate misbehaves on mainnet. | live testnet |

**Recommended default split:** Lane A for everyday dev + the per-push CI gate (fast, deterministic, can trigger receives on demand via cheatcodes). Lane B/C for the Helios+walkaway integration, run nightly/gated. The mainnet hero is shot only after A is green and B **or** C proves Helios continuation (per v1-demo-plan §"Reliability plan").

The **agentic runner** is a headless Rust driver (a `#[tokio::test]` integration test plus a `deckard-harness` bin) that: brings up a lane, runs the scenario DSL, and asserts. It speaks to the signer daemon over the socket defined in 30-mcp-shape.md. It has a **deterministic mode** (a `FakeModel` adapter that replays scripted intents) so the gate never depends on a live LLM, and a **live mode** that drives Claude Desktop via the MCP sidecar for the real take.

## Concrete interface

### File layout

```
crates/harness/                 # the runner (new crate)
  src/lib.rs                     # Lane, Scenario, Runner, asserts
  src/lanes/anvil.rs             # spawn anvil --fork-url, cast helpers
  src/lanes/kurtosis.rs          # kurtosis run + endpoint discovery
  src/lanes/sepolia.rs           # env-driven endpoints
  src/model.rs                   # ModelAdapter trait: FakeModel | ClaudeMcp
  tests/shot_list.rs             # #[tokio::test] the acceptance scenario
fixtures/
  addresses.mainnet.json         # USDC, Railgun contracts (see below)
  accounts.json                  # HD mnemonic + derived payer/wallet/extra
  scenarios/shield_on_receive.json
scripts/
  anvil-fork.sh  kurtosis-up.sh  fund.sh  trigger-receive.sh
.github/workflows/harness.yml
```

### Lane A — anvil fork (the controllable chain)

```bash
# Fork mainnet at a pinned block so contracts (USDC, Railgun) exist and fixtures are deterministic.
anvil --fork-url "$MAINNET_RPC_URL" --fork-block-number 22000000 \
      --mnemonic "test test test test test test test test test test test junk" \
      --accounts 10 --balance 10000 --chain-id 31337 --port 8545
```

Default mnemonic gives 10 accounts × 10000 ETH; it is public — dev only ([Foundry: Anvil overview](https://getfoundry.sh/anvil/overview/)).

**Cheatcodes that make the chain fully controllable** (exact names verified against [Foundry · Anvil custom methods](https://getfoundry.sh/anvil/custom-methods)). These drive the demo's "live receive" beat in tests — we *trigger* inbound payments on command:

| Need | Method | Use in the harness |
|---|---|---|
| Fund any address | `anvil_setBalance` | top up payer / wallet |
| Send *as* a whale (e.g. a USDC holder) | `anvil_impersonateAccount` / `anvil_stopImpersonatingAccount` | move real USDC into the wallet to fire the receive watcher |
| Force-mine | `anvil_mine` / `evm_mine` | confirm the inbound tx, advance state |
| Advance time | `evm_increaseTime` / `evm_setNextBlockTimestamp` | age checkpoints, test timeouts |
| Poke storage directly | `anvil_setStorageAt` | set an ERC-20 balance slot without a transfer (fastest "receive") |
| Inject code / nonce | `anvil_setCode` / `anvil_setNonce` | mock a contract if needed |
| Mining policy | `evm_setAutomine` / `evm_setIntervalMining` | step-mode vs interval for deterministic tests |
| Save/restore | `evm_snapshot` / `evm_revert` | reset between scenario steps cheaply |

Two ways to "trigger a live receive," fastest first:
1. **Storage poke** — compute the ERC-20 balance slot and `anvil_setStorageAt` (no real holder needed). Best for ETH/native and for an instant deterministic bump.
2. **Impersonate a real holder** — `anvil_impersonateAccount(<USDC_whale>)` then `cast send <USDC> "transfer(address,uint256)" <wallet> <amt> --from <whale> --unlocked`, then `anvil_mine`. Best for an end-to-end `Transfer` log the receive-watcher consumes (drives step 1 from real logs).

cast/forge scripting examples:
```bash
cast rpc anvil_setBalance "$PAYER" 0xDE0B6B3A7640000        # 1 ETH
cast rpc anvil_impersonateAccount "$USDC_WHALE"
cast send "$USDC" "transfer(address,uint256)" "$WALLET" 1000000 \
     --from "$USDC_WHALE" --unlocked --rpc-url http://127.0.0.1:8545   # 1 USDC (6 dp)
cast rpc anvil_mine 1
```

### Lane B — Kurtosis local EL+CL devnet (Helios can point at it)

```bash
# Spins up EL (geth/reth) + CL (lighthouse/teku/…) over Docker, exposes beacon + EL RPC.
kurtosis run --enclave deckard-devnet github.com/ethpandaops/ethereum-package
kurtosis enclave inspect deckard-devnet   # discover EL RPC + beacon (CL) ports
```

`ethpandaops/ethereum-package` deploys both layers and exposes a Beacon API (CL) and JSON-RPC (EL); it supports a fresh `kurtosis` genesis or a public-network shadowfork, and light clients like Helios can point at the local endpoints ([ethpandaops/ethereum-package](https://github.com/ethpandaops/ethereum-package)). Then point Helios at the local endpoints:
```bash
helios ethereum --network kurtosis \
  --consensus-rpc http://127.0.0.1:<CL_PORT> \
  --execution-rpc http://127.0.0.1:<EL_PORT> \
  --checkpoint <first beacon block hash of an epoch from the local CL>
# Helios serves a verified local JSON-RPC on http://127.0.0.1:8545
```
> **Update (cross-doc, from `20-helios-sidecar.md`):** two things below are now stale. (1) **LC support is resolved** — Lighthouse/Nimbus/Lodestar serve the light-client API **on by default** and ethereum-package runs all forks from genesis (no `--light-client-server` needed on current Lighthouse; it's disable-only now). (2) **Kurtosis is DEFERRED off the v1 critical path** — the mainnet spike proved the walkaway without it, so Lane B is a post-demo hermetic-CI nice-to-have, not a gate; v1 runs on mainnet + Sepolia (Lane C). The original text below is kept for when the Kurtosis lane is picked up.

⚠ ~~**unverified:** that the chosen Kurtosis CL client serves the **light-client beaconchain API** out of the box~~ (resolved — see note above) — Lighthouse gates this behind `--light-client-server` (and the EL needs the light-client execution API). The harness's `kurtosis.rs` must set the CL/EL flags to enable both, and assert Helios reaches `synced` before proceeding. Spike this on day one of the Helios lane; if a client won't serve it, fall back to Lane C (Sepolia) for the walkaway integration.

### Lane C — Sepolia

`MAINNET_RPC_URL` unused; set `SEPOLIA_EXECUTION_RPC` + `SEPOLIA_CONSENSUS_RPC` (a Nimbus/Lodestar beacon supporting the light-client API) and run `helios ethereum --network sepolia --checkpoint <fresh>`. This is the Kohaku-compatible lane (Sepolia-only) and the shield fallback.

### Helios as a library (Rust, in-process)

Latest Helios is **0.11.1** (Feb 2026); the crate was restructured from the umbrella `helios` into `helios-ethereum` exposing `EthereumClientBuilder` ([docs](https://docs.rs/zemse-helios-ethereum/latest/zemse_helios_ethereum/) shows the `EthereumClientBuilder` re-export). The README's `ClientBuilder::new().network(...).consensus_rpc(...).execution_rpc(...).build()` + `client.start()` + `client.get_balance(addr, BlockTag::Latest)` pattern is the shape; the harness depends on it as a lib so reads are verified in-process. ⚠ **unverified:** exact 0.11.x builder type path and method signatures — pin the version and confirm against `helios-ethereum` docs when wiring 20-helios-sidecar.md. Useful flag: `--strict-checkpoint-age` (`-s`) errors on >2-week-old checkpoints (README).

### Fixtures (mainnet, available via fork)

`fixtures/addresses.mainnet.json` — verified mainnet addresses:
```json
{
  "USDC":               "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
  "RailgunSmartWallet": "0xc0BEF2D373A1EfaDE8B952f33c1370E486f209Cc",
  "RailgunRelayProxy":  "0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9",
  "USDC_WHALE":         "<a large USDC holder at the pinned block — discover via Etherscan/cast>"
}
```
Railgun addresses verified via Etherscan: SmartWallet `0xc0BEF2D373A1EfaDE8B952f33c1370E486f209Cc`, Relay proxy `0xfa7093cd…` ([Etherscan: Railgun Relay](https://etherscan.io/address/0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9)). USDC is the canonical mainnet token. The exact Railgun contracts 10-kohaku-shield.md targets are **owned by that doc** — keep this fixture file the single source and let 10-kohaku-shield.md add what it needs. ⚠ **unverified:** pick + verify a `USDC_WHALE` holding ≥ demo amount at the pinned fork block before relying on the impersonate path.

`fixtures/accounts.json` — deterministic roles off the anvil mnemonic:
```json
{ "mnemonic": "test test test test test test test test test test test junk",
  "wallet": "m/44'/60'/0'/0/0", "payer": "m/44'/60'/0'/0/1", "extra": ["…/0/2","…/0/3"] }
```
`wallet` is the address under test (the one Deckard's keystore holds); `payer` sends the inbound tx.

### Scenario DSL (`fixtures/scenarios/*.json`)

A flat list of steps the runner executes; each step has a `lane` op and an `assert`. Mirrors v1-demo-plan's shot-list verbatim:
```json
{
  "name": "Shield-on-Receive, Trustless",
  "lane": "anvil-fork",
  "setup": { "unlock_keystore": true, "helios": "kurtosis|sepolia|none",
             "agent_policy": "auto-shield inbound ETH above 0.01" },
  "steps": [
    { "op": "receive", "from": "payer", "asset": "ETH", "amount": "0.05",
      "assert": "watcher_fires_within_seconds <= 5 && source == verified_logs" },
    { "op": "agent_intent", "intent": "shield", "amount": "0.05",
      "assert": "private_balance_up && public_balance_down && link_broken && tx_confirmed" },
    { "op": "cut_rpc", "target": "primary",
      "assert": "balances_still_verified_via_helios && no_crash" },
    { "op": "agent_intent", "intent": "execute", "after": "stop",
      "assert": "denied" },
    { "op": "allocate", "fraction": 0.1, "assert": "rule_honored" }
  ]
}
```
`op: "agent_intent"` is dispatched through the daemon socket **as defined in 30-mcp-shape.md** — this harness does not define the Intent/Decision shape, it constructs and submits whatever that doc specifies. In deterministic mode `FakeModel` emits the intent directly; in live mode the same intent originates from Claude Desktop via the MCP sidecar.

### Model adapter (LLM-free determinism)

```rust
pub trait ModelAdapter {
    /// Given the observed receive event, produce the next intent to submit to the daemon.
    async fn next_intent(&mut self, ctx: &ScenarioCtx) -> Intent; // Intent type owned by 30-mcp-shape.md
}
pub struct FakeModel { script: Vec<Intent> }   // replays fixtures, no network, CI default
pub struct ClaudeMcp { /* drives Claude Desktop over the MCP sidecar */ } // live take only
```

## v0 baseline / spike plan + acceptance test

**Build order (this is the v0 baseline — build it before features):**
1. `scripts/anvil-fork.sh` + `fixtures/` + cast helpers → can fork, fund, and *trigger a receive* on demand.
2. `crates/harness` Lane A + `FakeModel` + the daemon-socket client → run the scenario headless, deterministic.
3. `tests/shot_list.rs` asserting steps 1–2 against the anvil-fork lane (no Helios).
4. Lane B (Kurtosis) + Helios-as-lib → add step 3 (walkaway) end-to-end local; Lane C as fallback.
5. CI wiring.

**Agent-runnable acceptance test (run this to self-verify the harness itself):**
```bash
# A0 · tools present
anvil --version && cast --version && forge --version          # assert: exit 0
# A1 · fork comes up with real contracts
bash scripts/anvil-fork.sh & sleep 3
cast code 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 --rpc-url http://127.0.0.1:8545 \
  | grep -q '0x60'                                             # assert: USDC bytecode present (non-empty)
# A2 · cheatcode-driven receive works
WALLET=$(cast wallet address --mnemonic "test test test test test test test test test test test junk" --mnemonic-index 0)
cast rpc anvil_setBalance "$WALLET" 0x16345785D8A0000 --rpc-url http://127.0.0.1:8545   # 0.1 ETH
cast balance "$WALLET" --rpc-url http://127.0.0.1:8545 | grep -q 100000000000000000     # assert: balance set
# A3 · the deterministic scenario passes with no LLM and no Helios
cargo test -p harness --test shot_list -- --nocapture        # assert: steps 1–2 PASS (FakeModel, anvil-fork)
# A4 · the Helios lane reaches synced and survives an RPC cut (Kurtosis or Sepolia)
HARNESS_LANE=sepolia cargo test -p harness --test shot_list helios_walkaway -- --ignored --nocapture
#   assert: helios.status == synced; after cut_rpc, get_balance still returns a verified value; no panic
```
A0–A3 are the per-push gate. A4 is the nightly/gated Helios lane.

### CI wiring (`.github/workflows/harness.yml`)

```yaml
on: [push, pull_request]
jobs:
  anvil-fork-gate:                 # every push — fast, deterministic, the real gate
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: foundry-rs/foundry-toolchain@v1
      - run: bash scripts/anvil-fork.sh & sleep 3
      - run: cargo test -p harness --test shot_list        # A1–A3, FakeModel, no Helios
    env: { MAINNET_RPC_URL: ${{ secrets.MAINNET_RPC_URL }} }
  helios-walkaway-nightly:         # nightly + manual — the integration lane
    if: github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: foundry-rs/foundry-toolchain@v1
      - run: cargo test -p harness --test shot_list helios_walkaway -- --ignored   # A4
    env:
      SEPOLIA_EXECUTION_RPC: ${{ secrets.SEPOLIA_EXECUTION_RPC }}
      SEPOLIA_CONSENSUS_RPC: ${{ secrets.SEPOLIA_CONSENSUS_RPC }}
# add `on: schedule: - cron: '0 6 * * *'` at top level for nightly.
```
Pin `--fork-block-number` so the fork lane is reproducible and doesn't hammer the upstream RPC.

## Risks & fallbacks

- **Helios won't run on plain anvil** (no CL). *Fallback:* that's why Lanes B/C exist; the anvil-fork gate runs `helios: "none"` and step 3 is exercised on Kurtosis/Sepolia only.
- **Kurtosis CL doesn't expose the light-client beacon API by default** (⚠ unverified). *Fallback:* enable the flags (`--light-client-server` on Lighthouse + EL light-client API), else use Lane C (Sepolia) for walkaway; the mainnet hero only needs *one* of B/C green.
- **Helios crate API moved** (umbrella → `helios-ethereum`/`EthereumClientBuilder`, latest 0.11.1). *Fallback:* pin the version; 20-helios-sidecar.md owns the exact builder wiring and confirms signatures.
- **Live LLM flakiness** in the take. *Fallback:* `FakeModel` is the CI default; an in-app agent loop can drive the same MCP tools if Claude Desktop flakes on stage (v1-demo-plan §"Reliability plan").
- **Fork RPC rate limits / drift.** *Fallback:* pinned block + a cached fork; run a local reth archive if needed.
- **USDC_WHALE balance changes by block.** *Fallback:* prefer the `anvil_setStorageAt` balance-slot poke for determinism; reserve impersonation for the real-`Transfer`-log path.

## Open questions

- Which Kurtosis CL client (Lighthouse/Teku/Nimbus) reliably serves the **light-client beaconchain API** for Helios with minimal flags, and how long is its spin-up vs Sepolia? (⚠ unverified — spike both.)
- Is fork-lane block-pinning compatible with 10-kohaku-shield.md's Railgun proof generation (do circuits/POI need live state newer than the pin)?
- Does the daemon socket (30-mcp-shape.md) expose a test/inject hook the runner can use to fast-path `agent_intent` without a full MCP round-trip in deterministic mode?
- For the walkaway beat, what's the cleanest "cut the RPC" primitive in tests — drop the upstream via a local proxy (toxiproxy) we can kill, or swap Helios's `execution_rpc` to a dead URL and assert it continues from cache/secondary?
- Do we need a second EL upstream for Helios to *continue* after a cut, or does Helios serve cached/verified reads from its last finalized state (determines whether step 3 is "continues live" vs "shows last-verified")?

## Sources

- v1 demo plan (shot-list, lanes, reliability) — [docs/research/v1-demo-plan.md](../research/v1-demo-plan.md)
- Kohaku research (Sepolia-only extension; Railgun alpha crate) — [docs/research/03-kohaku.md](../research/03-kohaku.md)
- Foundry · Anvil overview (default mnemonic, 10×10000 ETH, fork) — https://getfoundry.sh/anvil/overview/
- Foundry · Anvil custom methods (exact cheatcode names) — https://getfoundry.sh/anvil/custom-methods
- a16z/helios README (consensus+execution light-client API requirement, CLI flags, custom networks) — https://github.com/a16z/helios/blob/master/README.md
- a16z/helios config.md (`consensus_rpc`/`execution_rpc`/`checkpoint`, `max_checkpoint_age`, fallbacks) — https://github.com/a16z/helios/blob/master/config.md
- helios-ethereum `EthereumClientBuilder` (restructured crate; latest 0.11.1) — https://docs.rs/zemse-helios-ethereum/latest/zemse_helios_ethereum/
- ethpandaops/ethereum-package (Kurtosis EL+CL devnet, beacon + EL RPC, fresh genesis or shadowfork) — https://github.com/ethpandaops/ethereum-package
- Etherscan · Railgun Relay/SmartWallet mainnet contracts — https://etherscan.io/address/0xfa7093cdd9ee6932b4eb2c9e1cde7ce00b1fa4b9
