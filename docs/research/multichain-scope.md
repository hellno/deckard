# Multi-chain scope: adding non-Ethereum chains (Tempo, Arbitrum)

Status: research / scoping. Date: 2026-06-20.
Method: a parallel mapping+research sweep over all 7 crates plus external docs (11 agents), then an
adversarial challenge + completeness critic. Every code claim cites `file:line`; external claims cite
a source URL and a confidence level. This doc backs the multi-chain issue set
(#97 registry, #98 Arbitrum reads+send, #99 swap, #100 gas, #101 privacy, #102 Tempo spike; plus the
existing #76 guardrail and #77 opstack spike) and the DESIGN.md "Per-chain trust tiers" decision.

Product decision (2026-06-20): **one wallet, loud honest downgrade.** Mainnet stays the only
`Verified` tier; every other chain reads NOT VERIFIED via the existing `Unsynced` tag (no new badge).
See DESIGN.md "Per-chain trust tiers".

---

## TL;DR

- Deckard is **shallowly multi-chain**: `chain_id` is already threaded through the wire (`Intent`,
  `SwapOrder`, `EthProvider`, `signing`, the supervisor env) and the daemon gates
  `intent.chain_id == cfg.chain_id`. But the runtime **pins one chain at process launch**, so today
  multi-chain = relaunch-per-chain via `DECKARD_CHAIN_ID` + `DECKARD_RPC_URL`. The contract types need
  ~zero change for relaunch-per-chain.
- The headline feature does not travel: **verified reads are structurally mainnet-only.** Helios is
  hardwired to mainnet on two axes and `chain_id` is never passed to it. No Nitro light client for
  Arbitrum; no client for Tempo. Every non-mainnet chain is raw RPC, honestly `Unsynced`.
- **Arbitrum One is the cheap, clean proof of multi-chain** (standard ETH-gas L2; the only genuinely
  new work is the honest-downgrade UX, which is already-built via the `Unsynced` tag).
- **Tempo is a net-new integration, not a port**: it breaks the native-asset model, the send guard,
  the wei spend-cap unit, and the gas-in-ETH assumption at once. Deferred past v1 (scoped here).
- **Two safety items gate any real non-mainnet chain**: the hands-free guardrail keys on `chain==1`
  only (already filed as **#76**), and a silent mainnet-default footgun (`DEFAULT_CHAIN_ID=1`,
  `DEFAULT_RPC`=mainnet).
- Chain-id correction: **Tempo mainnet is `4217`; `42431` is the Moderato testnet.**

---

## The architecture: where chain lives today

`chain_id` flows end-to-end but is consumed thinly:

- Resolved once at app launch: `DECKARD_CHAIN_ID` env > persisted `Settings.chain_id` >
  `DEFAULT_CHAIN_ID=1` (`settings.rs`), into an immutable `Shell.chain_id` (`shell.rs:555`,
  field doc `shell.rs:342-346` "resolved ONCE at startup").
- Threaded into `EthProvider::spawn(rpc, chain_id)`, `AppSigner::launch`, the daemon (via
  `supervise.rs:271-273` process env), Railgun grant, swap.
- In `deckard-core` reads, `chain_id`'s **only** functional effect is `tokens_for(chain_id)`
  (token-list selection, `eth.rs:367` → `tokens.rs:113`). Everything else is implicitly mainnet or
  address-keyed.
- The daemon enforces `intent.chain_id == cfg.chain_id` (`daemon.rs:476` intents, `:729` orders) →
  `deny_reasons::CHAIN_MISMATCH`. So the wire supports select-per-intent; the daemon makes it
  relaunch-per-chain.

`Intent` (`intent.rs:13`) and `SwapOrder` (`swap_order.rs:16`) both carry `chain_id: u64` and are
documented "multi-chain ready." A `u64` holds 4217 / 42161 with no width problem.

---

## Subsystem coupling map

Severity legend: **blocker** = chain cannot work without changing this; **degrades** = a feature
silently turns off or shows wrong data; **trivial** = a small config addition; **none** = already fine.

### deckard-core read path

| Location | Coupling | Sev (Arb / Tempo) |
|---|---|---|
| `helios.rs:184` `.network(Network::Mainnet)` | Helios hardcoded to mainnet; `chain_id` never passed to `launch_verified` | **blocker** / **blocker** |
| `helios.rs:48` `DEFAULT_CONSENSUS_RPC` = mainnet Nimbus beacon | Only consensus RPC ever passed; L2s have no beacon CL | **blocker** / **blocker** |
| `eth.rs:243-303` `ReadPath::build` (verified on) | Pointed at a non-mainnet RPC it fails CLOSED after a ~90s+ stall, then serves raw RPC tagged `Unsynced` (never a false `Verified`) | degrades / degrades |
| `env.rs:26` `DECKARD_VERIFIED_READS` | The only escape from the stall; framed as "demo mode", defaults ON. Needs auto-disable for chain != 1 | degrades / degrades |
| `tokens.rs:113-119` `tokens_for` | `1`/`11155111` only, else empty. Arbitrum token addresses DIFFER from mainnet (USDC `0xaf88…5831`); Tempo needs a TIP-20 list | degrades / degrades |
| `balances.rs:28` `MULTICALL3 = 0xcA11…CA11` | Canonical on Arbitrum (works). On Tempo it IS predeployed too (research, see below) | none / none |
| `balances.rs:63-97` + `eth.rs:344` native balance | `getEthBalance`/`get_balance` assume native ETH wei | none / **blocker** (no native asset) |
| `eth.rs:388` `resolve_name` (ENS) | Mainnet-only ENS registry; silently fails on L2s | degrades / degrades |
| `eth.rs:49` `DEFAULT_RPC` = mainnet publicnode | Fallback only; overridden by `DECKARD_RPC_URL`. No per-chain default table | trivial / degrades |

Already chain-safe: `fetch_portfolio` + `EthProvider` thread `chain_id` end-to-end; per-token
`balanceOf` is `allowFailure`-tolerant; Multicall3 absence falls back gracefully; reads fail closed.

### Privacy (Railgun) + Swap (CoW) in deckard-core

| Location | Coupling | Sev (Arb / Tempo) |
|---|---|---|
| `shield.rs:45` + `shielded.rs:104` → `ChainConfig::from_chain_id` | Vendored kohaku crate (rev `618c53f`) matches ONLY `1` + `11155111`; else `None`. All Railgun per-chain constants (smart wallet, RelayAdapt, WETH, Subsquid, POI) live UPSTREAM, not in deckard-core | **blocker** / **blocker** |
| `cow_types.rs:52-58` `order_digest` | EIP-712 domain binds `chain_id` from `order.chain_id` — already chain-parametrized (test `cow_types.rs:303`) | none / none |
| `cow_types.rs:17-18` `GPV2_SETTLEMENT` / `GPV2_VAULT_RELAYER` | Canonical CREATE2 addresses identical on every CoW chain (correct for Arbitrum) | none / n-a |
| `cow_types.rs:120-126` `cow_api_base` | 2-arm match (`1`, `11155111`), else `None`. ONLY code blocker for CoW on a CoW chain | trivial (add `42161`) / n-a |
| `railgun_keys.rs:135-160` 0zk derivation | Chain-agnostic keys; address encodes `ChainId::evm(chain_id)` cleanly for any id | none / none |

Net: **Arbitrum** — CoW = trivial (`cow_api_base` arm + token list); Railgun = blocker (needs an
Arbitrum `ChainConfig` upstream; Railgun IS deployed on Arbitrum so feasible). **Tempo** — neither CoW
nor Railgun exists on-chain.

### Signer daemon (deckard-signerd)

| Location | Coupling | Sev (Arb / Tempo) |
|---|---|---|
| `signing.rs:45` `broadcast_intent` | Every write is a hardcoded EIP-1559 **type-2** tx via alloy fillers; `chain_id` via `set_chain_id` (`:75`). No legacy/2930/custom path | none / **blocker** (type-0x76, see contradiction below) |
| `signing.rs:60-83` `send_transaction` | alloy GasFiller assumes an ETH/wei EIP-1559 fee market | none / **blocker** (USD-stablecoin gas) |
| `daemon.rs:263-269` own Helios bootstrap | Daemon runs its OWN independent Helios, mainnet-only | degrades / degrades |
| `daemon.rs:64-74` `relay_adapt(chain_id)` | RelayAdapt target hardcoded `1`/`11155111`; Shield denied otherwise (`shield_to_mismatch`) | degrades / degrades |
| `daemon.rs:1093-1143` + `policy.rs:84` spend caps | `per_tx_cap_wei`/`daily_cap_wei` accounted off native `intent.value` | none / degrades (wei is a category error vs USD) |
| `daemon.rs:1363` `mainnet_guardrail_active` = `chain_id == 1` | Off chain 1, within-cap agent spends auto-allow hands-free. Real non-mainnet chains lose the human brake **silently**. → **issue #76** | **blocker (safety)** / **blocker (safety)** |
| `config.rs:43-49` `from_env` | `chain_id` default 1, `rpc_url` default mainnet; both env-overridable; only chain 0 refused | trivial / trivial |

Arbitrum signs with **zero daemon changes** (set the two env vars). Tempo cannot be signed today.

### Wire contract (deckard-contract)

- `chain_id` appears in exactly three fields: `Intent.chain_id`, `SwapOrder.chain_id`,
  `SignerRequest::RailgunViewGrant{chain_id,index}`. Rest is chain-agnostic.
- `ReadStatus::Unsynced{reason}` already models "no light client" honestly (test uses
  `reason: "verification disabled"`). **No new status needed**; at most a reason-string convention.
- `Policy` is GLOBAL, not per-chain (no `chain_id` field). Per-chain caps are a real future need;
  the #28/#31 additive-evolution + frozen-`evaluate()` rule means the safe path is an additive
  `#[serde(default)] chain_id` (mirroring how `allow_swap_tokens` was added) or a daemon-side
  chain→Policy map, not a reshape. (Only needed for Tempo's USD-unit caps; not for Arbitrum.)
- `deny_reasons` already has `CHAIN_MISMATCH` / `SHIELD_UNAVAILABLE` / `ERC20_UNSUPPORTED_V1` /
  `UNSUPPORTED_V1` to express degraded-on-other-chains refusals honestly.
- **Minimal contract change to serve a relaunch-per-chain Arbitrum: arguably zero.** Frozen:
  `Intent`/`SwapOrder`/`Decision`/`ReadStatus`/`evaluate`. The hard work is in the daemon and (for
  Tempo) the unit semantics.

### Config plumbing + UI/agent surfaces

- Chain is **fixed at process launch end-to-end**, not switchable at runtime (`shell.rs:1392-1396`
  re-points only the reader's RPC; daemon chain stays fixed).
- Persisted `Settings { chain_id: Option<u64>, rpc_url: String }` holds ONE chain. No network list.
- **No chain-selector UI** and **no ⌘K palette command** for switching networks (`settings_view.rs`
  Network card exposes only Custom RPC + Watch address). A switcher would live as a settings row + a
  palette `Command` + a `run_palette_command` arm (`shell.rs:2727`), but it cannot be a simple method
  call because chain is launch-fixed (needs daemon respawn).
- The agent/MCP does NOT pick a chain — it inherits `DECKARD_CHAIN_ID` (default 1) via `WalletClient`.
  `install --demo` hardcodes `11155111` + `http://127.0.0.1:8545` (no `--chain` flag).
- A new chain's identity is scattered across match-on-chain-id sites: `tokens_for`, `cow_api_base`,
  `DEFAULT_RPC` (mainnet-only, no per-chain table), the hardcoded ETH hero row (`welcome.rs:142-148`,
  `Ξ`/"Ethereum"/ETH/18dec), `is_fork_mode` (binary mainnet-vs-fork, mislabels a local non-mainnet
  chain as "DEMO FORK"). **There is no single chain-registry struct, and no explorer URL anywhere.**

---

## External research

### Tempo (Stripe/Paradigm payments L1)

- Mainnet chain id **4217**, symbol USD, RPC `https://rpc.tempo.xyz`, explorer `explore.tempo.xyz`.
  Testnet "Moderato" chain id **42431**, RPC `https://rpc.moderato.tempo.xyz`, faucet
  `tempo_fundAddress`. (high; docs.tempo.xyz/quickstart/connection-details) **The brief's 42431 is the
  testnet.**
- **No native gas token.** Fees paid in USD TIP-20 stablecoins. `eth_getBalance` returns a fixed
  ~4.24e75 placeholder; `BALANCE`/`SELFBALANCE` opcodes return 0; `CALLVALUE` returns 0. Read real
  balances via TIP-20 `balanceOf` (decimals **6**), and the fee-token preference from the FeeManager
  precompile `0xfeec…0000` (`getUserToken`/`setUserToken`). pathUSD precompile `0x20c0…0000`. (high)
- **Plain EIP-1559 type-2 txns DO work and auto-deduct stablecoin fees** — you do NOT need the 0x76
  type to transact. Default fee token is pathUSD; a zero-stablecoin account cannot send (fee
  pre-check fails). (high; docs.tempo.xyz/protocol/fees/spec-fee) — **but see the contradiction in
  "Open questions" below.**
- Tempo Transaction = EIP-2718 type **0x76**: fee_token selection, fee sponsorship, atomic call
  batching, 2D/concurrent nonces, scheduled execution, secp256k1/P256/WebAuthn/access-key sigs.
  Stock alloy cannot encode 0x76; needs the **tempo-alloy** extension crate (git dep,
  `--tag v1.4.2`, `TempoNetwork`/`TempoTxEnvelope`). (high; docs.tempo.xyz/sdk/rust)
- Standard infra predeployed at canonical addresses: **Multicall3**, CreateX, Permit2, Safe. (high)
- Consensus: Simplex BFT via Commonware, sub-second finality. No Ethereum beacon → Helios cannot be
  reused. Tempo has its OWN trust-minimized read path (`--follow.experimental.certify` + a
  `consensus_` RPC namespace with BLS finalization certificates), but client-side verification is a
  separate, larger spike, not Helios-shaped. (medium)

### Arbitrum (One + Sepolia)

- Arbitrum One **42161** (`https://arb1.arbitrum.io/rpc`); Sepolia **421614**
  (`https://sepolia-rollup.arbitrum.io/rpc`). dRPC mirrors exist. (high)
- **Native token is ETH (18 dec)** — Deckard's native-balance model works unchanged, unlike Tempo.
- **Multicall3 canonical `0xcA11…CA11`** on both. CAUTION: Arbitrum's chain-info page lists a
  different "L2 Multicall" (`0x842e…`/`0xA115…`) — do NOT use that; use canonical Multicall3. (high)
- Gas: a single fee bundling L2 execution + an L1 data-availability (calldata) component, folded into
  the returned gas limit. `eth_estimateGas` covers total cost but the number embeds the volatile L1
  fee and **drifts** (quotes go stale). For accurate/cappable fees use the NodeInterface precompile
  `0x…C8` `gasEstimateComponents` and/or ArbGasInfo `0x…6C`, re-estimate near send, buffer. Priority
  fee is effectively unused. (high)
- Finality: sequencer soft-confirmation ~250ms (trust-based), L1 security after the batch posts and
  finalizes (minutes), withdrawal finality ~7 days. A fresh balance is "sequencer-trusted", not
  proven. (high/medium)
- **Helios does NOT support Arbitrum** (Nitro, not OP-stack). `eth_getProof` is available but you'd
  still need a trusted L2 state root, which is the hard part no shipping lib solves for Nitro. Net:
  raw-RPC trust, honest `Unsynced`. (high)

### Light-client / verified-read landscape

- Helios supports Ethereum (mainnet/sepolia/holesky) consensus light clients, plus **helios-opstack**
  for OP Mainnet + Base (and Linea), and that OP path is **sequencer-trusted** (validates the
  sequencer's signed head, not L1-derived state) → `Degraded`, never `Verified`. (high/medium)
- No Helios for Arbitrum (Nitro) or Tempo. (high)
- `ReadStatus`'s three states already map the tiers: `Verified` = mainnet Helios; `Degraded` =
  opstack sequencer-trust (if ever added, #77); `Unsynced` = Arbitrum/Tempo raw RPC. No enum change
  needed to be honest.
- zk light clients (sp1-helios, op-succinct) are onchain verifier/bridge infra, not embeddable
  in-wallet readers. Watch-this-space, not a v1 dependency. (high/medium)

### CoW + Railgun chain support

- **CoW** supports (with exact orderbook base URLs from cow-sdk): mainnet `/mainnet`, Gnosis `/xdai`,
  **Arbitrum One `/arbitrum_one`** (underscore), Base `/base`, Sepolia `/sepolia`, Polygon, Avalanche,
  BNB, Linea, Plasma, Ink. GPv2 contracts are deterministic (same `0x9008…`/`0xC92E…` everywhere); the
  EIP-712 domain binds `chainId`. **CoW does NOT support Tempo.** (high)
- **Railgun** protocol is live on Ethereum, BNB, Polygon, **Arbitrum** (`RailgunSmartWallet`
  `0xFA70…A4b9`, `RelayAdapt` `0xB4F2…D89b`). But Deckard's vendored kohaku crate (`618c53f`) only
  ships mainnet+sepolia `ChainConfig`s, so shield fails closed on Arbitrum at the Deckard layer.
  Enabling it = a vetted dep bump + (maybe) authoring the Arbitrum config values. **Railgun not on
  Tempo.** (high)

---

## Feature matrix

| Feature | Ethereum | Arbitrum (42161) | Tempo (4217) |
|---|---|---|---|
| Verified reads (Helios) | works | n-a (raw RPC, Unsynced) | n-a (raw RPC, Unsynced) |
| Native-balance display | works | works (ETH/wei) | n-a (no native asset; placeholder balance) |
| Send / receive | works | works (zero signer change) | degrades (TIP-20 send; blocked by `erc20_unsupported_v1`; needs stablecoin) |
| ERC-20 / token list | works | degrades (add `ARBITRUM_TOKENS`) | degrades (TIP-20, 6 dec) |
| CoW swap | works | works (one `cow_api_base` arm) | n-a (CoW not deployed) |
| Railgun privacy | works | n-a in v1 (dep bump) / feasible | n-a (not deployed) |
| Agent approval queue | works | works* (*requires #76) | degrades (wei caps wrong unit) |
| Gas / fee handling | works | degrades (L1-DA fee drift) | n-a (USD-stablecoin Fee AMM) |

---

## Risks

1. **Trust dilution (presentational):** an always-`Unsynced` Arbitrum balance next to a `Verified`
   mainnet one risks the user reading "verified" onto an unverified row. The data is honest
   (fail-closed), the risk is the affordance. Mitigation (decided): reuse the existing loud
   "NOT VERIFIED" `Unsynced` treatment; never render a Tier-3 balance with the Verified row look.
2. **Silent safety regression (#76):** the `chain==1`-only guardrail. A hard prerequisite of shipping
   any real non-mainnet chain.
3. **Silent mainnet-default footgun:** `DEFAULT_CHAIN_ID=1` + `DEFAULT_RPC`=mainnet → set the chain id
   but forget the RPC and reads point at mainnet. Mitigation: per-chain default RPC + refuse on a
   chain-id/RPC-chain mismatch.
4. **Tempo phantom native balance:** wired naively, the ~4.24e75 placeholder renders as a giant fake
   ETH holding via the hardcoded hero row — the worst failure for a trust wallet. Strongest reason to
   keep Tempo out until the portfolio model can say "no native asset".
5. **Degrade-by-empty-list:** `tokens_for` returns empty for unknown chains → balances/swap go dark
   silently. Surface "token list not yet curated for <chain>" explicitly.
6. **Scope-creep into the trust core:** wiring helios-opstack or a Tempo cert verifier under
   multichain pressure risks weakening the one thing actually verified today (mainnet). Keep those as
   separate spikes (#77).

---

## Recommended phasing → issues

- **#76 (exists, prerequisite):** harden the guardrail to default-deny on every real-value chain.
- **#97 — chain capability registry** (foundation, ships no new chain): one struct keyed by chain_id
  ({default_rpc, native_asset: Option, verification tier, multicall3, cow_orderbook_base, railgun,
  is_real_value_chain, explorer_url, network_name}); replace the scattered match sites. Provides the
  `is_real_value_chain` data #76 wants.
- **#98 — Arbitrum One reads + send** (raw-RPC, honest `Unsynced`; auto-disable Helios for
  verification=None; network label). Depends on #76 + #97.
- **#99 — Arbitrum swap (CoW):** one `cow_api_base` arm + `ARBITRUM_TOKENS`; verify a live quote.
- **#100 — Arbitrum gas accuracy (optional polish):** NodeInterface L1-DA split + re-estimate + buffer.
- **#101 — Arbitrum privacy (Railgun):** commit to the kohaku dep bump IF a clean rev with an Arbitrum
  `ChainConfig` exists; else author the config values; explicit go/no-go. Needs dep approval.
- **#102 — Tempo feasibility spike** (Moderato testnet 42431): resolve the type-2-vs-0x76 contradiction
  via a live smoke test; confirm chain ids, Multicall3, FeeManager getter, balance behavior. Output: a
  Tempo go/no-go + whether tempo-alloy is required.
- Tempo full integration is deferred until #102 lands.

Cross-cutting honesty debt to fold into the relevant phases (from the completeness critic): reconcile
`docs/WALLETBEAT-COMPATIBILITY.md` (#16 chain-verification, #15 L2-out-of-scope, #24 ENS all change);
regenerate "Sepolia or mainnet" error strings (`shell.rs:1912/1997/2183`) from the registry; per-chain
explorer links (none exist); EIP-681 chain context in the Receive QR; **MCP tool descriptions +
`amount.rs parse_eth_to_wei` hardcode ETH/18-dec** (an agent on Tempo's 6-dec stablecoins is misled by
10^12) — a #F/#G concern, not Arbitrum.

---

## Open questions to verify before building

1. **Tempo type-2 contradiction (highest priority, #F):** the code map infers stock alloy cannot
   produce a valid Tempo tx (wei `maxFeePerGas`); Tempo's fee spec says plain type-2 works and fees
   convert via a Fee AMM. Settle with a live smoke test against `rpc.moderato.tempo.xyz` before calling
   Tempo a hard blocker vs degraded-but-works.
2. **kohaku Arbitrum config:** does a newer kohaku rev ship `ChainConfig::arbitrum()`, or must Deckard
   author POI start block / Subsquid endpoint / POI endpoint / deployment block itself? Decides whether
   #E is a dep-bump or a from-scratch authoring task.
3. **CoW Arbitrum live check:** confirm `https://api.cow.fi/arbitrum_one` (underscore) and that the
   same eip712 + erc20 `sellTokenBalance` + `appData "{}"` shape is accepted on a live Arbitrum quote.
4. **tempo-alloy compatibility:** does `tempo-alloy` (tag v1.4.2) compose with deckard-signerd's pinned
   alloy 1.8.3? Only matters for 0x76 (phase past #F).
5. **Multicall3 on Tempo:** read-path map said "not guaranteed"; Tempo research says predeployed at the
   canonical address. Confirm on-chain.
6. **Helios-opstack embeddable API:** does it expose the same localhost-server pattern `launch_verified`
   depends on? Only relevant if a Tier-2 OP-stack path (#77) is pursued.
