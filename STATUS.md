# Deckard — STATUS

> **Single source of truth for where the build stands.** Specs/design live in `specs/` + `docs/build/`;
> the git log is the audit trail. The READMEs and `specs/*` are reference, not status — update *this* file
> when a track changes state.
>
> Last updated: **2026-06-11** · at commit `2edc6d5`.

Legend: ✅ done + tested · 🟡 partial / integrated-not-finished · 🧪 spiked (proven, not wired in) · ⬜ todo · ⏸ deferred

> **Reality check (2026-06-11):** the gaps the 2026-06-06 Codex audit flagged are now **closed**. The shield hero
> is **app-reachable** (compose → review → hold-to-confirm, a real shielded-balance view: merged Total + composition,
> real shield lifecycle states, full privacy mask) **and agent-reachable** via the `deckard-mcp` sidecar (Claude
> Desktop drives `deckard_shield` → `deckard_execute`). The demo FLOW is wired: `just demo` / `demo-fund` /
> `demo-check` stand up a forked Sepolia + funded EOA + the app against `policy.demo.json`, with a "DEMO FORK — not
> mainnet" banner on screen.
>
> What is **still** honestly true: the network-dependent `signerd/shield_e2e` remains `#[ignore]` (needs a local
> `anvil`/fork + archive RPC, not run by default `cargo test`); the MCP acceptance suite runs by default but drives a
> **mock signerd** (deterministic tx hash, no live broadcast); the **demo `.gif` re-record is pending**; the README's
> **TTHW measurement is pending** — the harness for it is now in place (the README quick prompt + the agent
> quickstart `docs/build/31-agent-quickstart.md`, issue #27): run the quick prompt in a fresh agent session against a
> running `just demo` and record the minutes here; **Send UI stays gated**
> ("next release") and **Swap is a disabled TODO** (Send + Swap is the first post-launch milestone). "Tested" below
> still includes some mocked-transport / fake-daemon unit tests — noted per row.

## The v1 demo — *receive → instantly private → can't switch it off* (agent-driven)

| Beat | What | Status | Where | Next |
|---|---|---|---|---|
| **1 · receive** | detect inbound + surface it | 🟡 receive (addr + QR) built; manual refresh; **auto-detect watcher ⬜** | `deckard-app/receive.rs`; `policy.auto_shield_min_wei` exists | a Helios `get_logs` watcher that emits the shield intent |
| **2 · shield (HERO)** | shield received funds via Railgun | ✅ **app-reachable** (compose → review → hold-to-confirm) + **shielded-balance view** (Total + composition, real lifecycle states, privacy mask); core builder + daemon broadcast; black-box `shield_e2e` proves the privacy property (`#[ignore]`, network) | `deckard-app/shield_view.rs` + `render_shield` in `shell.rs`; `deckard-core/shield.rs` + `railgun_keys.rs` (KAT-gated 0zk derivation) + signerd broadcast + `shield_e2e` | re-record demo `.gif`; auto-watcher trigger |
| **3 · walkaway** | cut the RPC on camera, stay verified | ✅ R2 spiked + verified-reads integrated; 🟡 in-app live cut | spike `5e3a16d`; reads `9e19e9a`; `ReadStatus` badge in app | failover supervisor (`Degraded`) + the in-app cut |
| **agent surface (MCP)** | Claude Desktop drives `shield` via MCP | ✅ `deckard-mcp` shipped — key-less CLI + MCP stdio sidecar, `mcp.v0.1` 6-tool profile over the daemon socket | `deckard-mcp` (`server.rs`/`sidecar.rs`); `docs/build/30-mcp-shape.md` | (post-launch) `simulate` (deferred to the daemon) |

## Crates / tracks

| Crate | State | Tests (passing) | Key commits |
|---|---|---|---|
| `deckard-contract` | ✅ `Intent`/`Decision`/`Policy` + `ReadStatus` + `calldata_ok` non-empty invariant | 32 | `da29a37` `9e19e9a` `a0a37fd` |
| `deckard-core` | ✅ `EthProvider` (C1) + balances/Multicall3 (C2) + encrypted keystore (C3) + Helios verified reads + key-less shield builder + KAT-gated Railgun seed→0zk viewing-key derivation (`railgun_keys.rs`) | 13+ | `e1aa079` `42e04ad` `57f21bc` `9e19e9a` `3aae92d` |
| `deckard-signerd` | ✅ process-isolated signer daemon + policy gate + STOP/zeroize + Helios read + calldata broadcast + **mainnet guardrail** (chain-1 auto-Allow → `NeedsApproval`, resolved by the app's hold-to-confirm) + RelayAdapt pre-check + **reason/RPC redaction** | `daemon_e2e` · `parity` · `anvil_e2e` · `shield_e2e` (#[ignore]) | `a24f62c` `9e19e9a` `2f28b8b` `72ad5cb` |
| `deckard-mcp` | ✅ key-less CLI + MCP stdio sidecar (`mcp.v0.1` 6-tool profile: `deckard_wallet_address` / `wallet_balance` / `policy_get` / `shield` / `execute` / `revoke_all`); holds no key, proposes Intents to the daemon socket; secret-flag hard-reject + transcript canary scan | `acceptance` 9, all run by default against a *mock signerd* (T1 six-tool profile · T6 within-cap shield → mock `tx_hash` · T7/T9 secret-free transcript) | `a82cc38` |
| `deckard-app` (GPUI) | ✅ onboarding / portfolio / receive / palette / settings + **shield view** (compose → review → hold-to-confirm + shielded-balance composition + privacy mask) + `ReadStatus` badge + socket signer client + **env plumbing** (`DECKARD_CONFIG_DIR`/`SOCKET_PATH`/`CHAIN_ID`/`RPC_URL`, `DECKARD_VERIFIED_READS`, `DECKARD_DEMO_FORK_BLOCK`) + **"DEMO FORK — not mainnet" banner**; 🟡 Send UI gated ("next release"), Swap ⬜ | `send_path` | C1–C3 + the shield/MCP/demo work |

> Test caveats: `signerd/shield_e2e` is `#[ignore]` (network — needs a local `anvil`/fork + archive RPC, NOT run by default `cargo test`); `anvil_e2e` runs by default but **silently skips if `anvil` is missing**; `deckard-core`'s reads are tested against a *mocked* transport (live-Helios path is untested by default); `deckard-app`'s `send_path` test uses a *fake recording daemon*, not real signerd/chain. The **MCP acceptance suite runs by default but drives a *mock signerd*** (deterministic `mock_tx_hash`, no live chain) — it asserts the tool surface, the decision/approval flow, and the secret-free transcript, not a real broadcast. The daemon STOP/zeroize + propose→Decision→execute tests, the mainnet-guardrail/redaction tests, and the railgun-key KAT are real and run by default.

> **Honest note — the `railgun` `testing` feature.** `railgun` is a **dev-dependency of `deckard-signerd`** only (`[dev-dependencies]`, `features = ["testing"]`): the black-box `shield_e2e` test uses it to register an ephemeral 0zk recipient and read the private balance to *prove* the privacy property. The **daemon binary/lib gains no railgun dependency** — it never syncs, proves, or holds a key. In `deckard-core` the `shield` feature (default-on) pulls railgun for the key-less `ShieldBuilder` calldata path only, and `deckard-mcp` depends on `deckard-signerd` with `default-features = false` so the sidecar never drags railgun in either. The `testing` feature + the rev pin match the proven shield spike so the workspace-root `[patch]` set resolves identically.

## Spikes (de-risk, proven, standalone under `spikes/`)

| Spike | Proves | Commit |
|---|---|---|
| `helios-walkaway` | R2 walkaway on mainnet (cut EL → keep verified; refuse a lying RPC) | `5e3a16d` |
| `eip1193-railgun` | Helios's localhost server **is** Railgun's EIP-1193 provider (one-line `with_default_block(latest)` fix) | `a64d4c5` |
| `shield-railgun` | full shield→sync→balance→transfer→unshield from our edge; shield is instant, proving is on the spend | `c30cdd4` |

## v0 wallet base (`specs/SPEC-v0.md`)

balances ✅ · receive ✅ · keystore + onboarding ✅ (BIP-39 vault) · **send** (daemon + app-socket path ✅/tested, UI gated 🟡) · **swap** ⬜ (CoW, deferred; button disabled)

## Remaining for a minimum recordable demo

The reachability + visible-state gaps the 2026-06-06 audit flagged are now built. What's left is **recording + polish**, not new mechanism:

- **A · Beat 2 (hero) — shield on screen** ✅ built: shield view (compose → review → hold-to-confirm) + shielded-balance composition + privacy mask; viewing key derived from the seed (`railgun_keys.rs`, KAT-gated). Re-record the demo `.gif`.
- **B · Beat 3 — in-app walkaway** 🟡: the `ReadStatus` badge already renders; the single-client "cut → NOT VERIFIED → reconnect → Verified" framing is the minimum viable cut. (Shape-A failover supervisor still deferred.)
- **C · Beat 1 — receive landing** 🟡: balance refreshes on manual refresh (QR exists); auto-watcher → auto-shield is the fuller version, still ⬜.
- **D · the "agent-driven" spine** ✅: `deckard-mcp` ships — drive `deckard_shield`/`deckard_execute` from Claude Desktop, or fall back to an in-app agent loop over the same daemon socket.
- **E · one continuous take + polish** to `DESIGN.md` (onboarding → funded → 3 beats) — pending, plus the `.gif` re-record.

**Smallest take:** A + B + C with D as the live agent (or narrated stand-in). **Largest remaining item is recording the take, not a build.**

## Open risks / track-before-ship

- **CL is the fragile, no-SLA dependency** (walkaway): Nimbus + dRPC are the two proven CLs; **cut the EL on camera, never the CL.** [`20`]
- **`vendor/eip-1193-provider`** native-only fork (dodges a `wasm-bindgen` exact-pin conflict) + **railgun license** (no upstream license field, same R1e) — resolve both before ship. [`10`]
- Shield is instant (no client proof); spend proving ~10s cold / ~halved with `parallel` → a "spending…" UX for unshield. [`10`]
- Daemon holds its mutex across a broadcast (documented v1 tradeoff). MCP and railgun key-derivation for balance-display have **landed**; the receive-watcher (auto-detect → auto-shield) is still deferred.
- On mainnet (chain 1) the signer's **guardrail** downgrades every auto-Allow to `NeedsApproval`, resolved only by the app's hold-to-confirm — so a prompt-injected MCP client can't move real funds hands-free. The override env var is documented **only** in `THREAT-MODEL.md` and never appears in a reason string.

## Deferred → `docs/research/roadmap.md`

STOP-on-camera beat · allocate/donate · EIP-7702 session keys · x402/MPP plugins · stealth addresses · hardware-wallet signing · audit · Kurtosis hermetic-CI lane · production `HeliosEip1193` adapter · on-camera unshield/transfer.
