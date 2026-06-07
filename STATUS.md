# Deckard — STATUS

> **Single source of truth for where the build stands.** Specs/design live in `specs/` + `docs/build/`;
> the git log is the audit trail. The READMEs and `specs/*` are reference, not status — update *this* file
> when a track changes state.
>
> Last updated: **2026-06-06** · at commit `a0a37fd`.

Legend: ✅ done + tested · 🟡 partial / integrated-not-finished · 🧪 spiked (proven, not wired in) · ⬜ todo · ⏸ deferred

> **Reality check (independent Codex audit, 2026-06-06):** the *mechanisms* are built + de-risked and the security
> state machine is real — but the **recordable demo FLOW does not exist end-to-end yet**. No receive-watcher, no
> agent/MCP trigger, no in-app live cut, and **shield is reachable only from the test/manual client, not the app
> or an agent.** "Tested" below includes `#[ignore]` network tests (need anvil + an archive RPC, not run by default
> `cargo test`) and some mocked/fake-daemon unit tests — noted per row. So: strong foundation, demo flow unwired.

## The v1 demo — *receive → instantly private → can't switch it off* (agent-driven)

| Beat | What | Status | Where | Next |
|---|---|---|---|---|
| **1 · receive** | detect inbound + surface it | 🟡 receive (addr + QR) built; **auto-detect watcher ⬜** | `deckard-app/receive.rs`; `policy.auto_shield_min_wei` exists | a Helios `get_logs` watcher that emits the shield intent |
| **2 · shield (HERO)** | auto-shield received funds via Railgun | 🟡 mechanism built (core builder + daemon broadcast) + black-box test — **but test is `#[ignore]` and shield is NOT app/agent-reachable** | spike `c30cdd4`; `deckard-core/shield.rs` + signerd broadcast + `shield_e2e` (#[ignore]) `a0a37fd` | a **trigger** (button/watcher/agent) + a **shielded-balance view** |
| **3 · walkaway** | cut the RPC on camera, stay verified | ✅ R2 spiked + verified-reads integrated; 🟡 in-app live cut | spike `5e3a16d`; reads `9e19e9a`; `ReadStatus` badge in app | failover supervisor (`Degraded`) + the in-app cut |
| **agent surface (MCP)** | Claude Desktop drives `shield` via MCP | ⬜ `deckard-mcp` not built; contract + daemon socket ✅ | `deckard-contract`, `deckard-signerd` | build `deckard-mcp` (key-less) over the daemon socket + `simulate` |

## Crates / tracks

| Crate | State | Tests (passing) | Key commits |
|---|---|---|---|
| `deckard-contract` | ✅ `Intent`/`Decision`/`Policy` + `ReadStatus` + `calldata_ok` non-empty invariant | 32 | `da29a37` `9e19e9a` `a0a37fd` |
| `deckard-core` | ✅ `EthProvider` (C1) + balances/Multicall3 (C2) + encrypted keystore (C3) + Helios verified reads + key-less shield builder | 13 | `e1aa079` `42e04ad` `57f21bc` `9e19e9a` `a0a37fd` |
| `deckard-signerd` | ✅ process-isolated signer daemon + policy gate + STOP/zeroize + Helios read + calldata broadcast | `daemon_e2e` 9 · `parity` 1 · `anvil_e2e` 3 · `shield_e2e` 1 | `a24f62c` `9e19e9a` `a0a37fd` |
| `deckard-app` (GPUI) | ✅ onboarding / portfolio / receive / palette / settings + `ReadStatus` badge + socket signer client; 🟡 Send UI gated ("next release"), Swap ⬜ | `send_path` | C1–C3 + this branch |

> Test caveats (per the audit): `signerd/shield_e2e` is `#[ignore]` (network + anvil + archive RPC); `anvil_e2e` runs by default but **silently skips if `anvil` is missing**; `deckard-core`'s reads are tested against a *mocked* transport (live-Helios path is untested by default); `deckard-app`'s `send_path` test uses a *fake recording daemon*, not real signerd/chain. The daemon STOP/zeroize + propose→Decision→execute tests are real (run by default).

## Spikes (de-risk, proven, standalone under `spikes/`)

| Spike | Proves | Commit |
|---|---|---|
| `helios-walkaway` | R2 walkaway on mainnet (cut EL → keep verified; refuse a lying RPC) | `5e3a16d` |
| `eip1193-railgun` | Helios's localhost server **is** Railgun's EIP-1193 provider (one-line `with_default_block(latest)` fix) | `a64d4c5` |
| `shield-railgun` | full shield→sync→balance→transfer→unshield from our edge; shield is instant, proving is on the spend | `c30cdd4` |

## v0 wallet base (`specs/SPEC-v0.md`)

balances ✅ · receive ✅ · keystore + onboarding ✅ (BIP-39 vault) · **send** (daemon + app-socket path ✅/tested, UI gated 🟡) · **swap** ⬜ (CoW, deferred; button disabled)

## Open risks / track-before-ship

- **CL is the fragile, no-SLA dependency** (walkaway): Nimbus + dRPC are the two proven CLs; **cut the EL on camera, never the CL.** [`20`]
- **`vendor/eip-1193-provider`** native-only fork (dodges a `wasm-bindgen` exact-pin conflict) + **railgun license** (no upstream license field, same R1e) — resolve both before ship. [`10`]
- Shield is instant (no client proof); spend proving ~10s cold / ~halved with `parallel` → a "spending…" UX for unshield. [`10`]
- Daemon holds its mutex across a broadcast (documented v1 tradeoff). Receive-watcher, MCP, and railgun key-derivation for balance-display are deferred.

## Deferred → `docs/research/roadmap.md`

STOP-on-camera beat · allocate/donate · EIP-7702 session keys · x402/MPP plugins · stealth addresses · hardware-wallet signing · audit · Kurtosis hermetic-CI lane · production `HeliosEip1193` adapter · on-camera unshield/transfer.
