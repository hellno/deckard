# Helios Light-Client Sidecar

> Embed a16z Helios so every read is verified locally, and to power the demo's WALKAWAY beat (cut the centralized RPC on camera, keep working). Serves demo beat 3 + acceptance step 3. This is risk **R2**. Status: **core mechanism proven on mainnet** — the spike shows verified reads survive a cut EL (failover) and a lying RPC is rejected (cold ≈11s, warm ≈2s, cut→failover ≤1 block; `spikes/helios-walkaway/`). **App integration is still unbuilt** (EIP-1193 provider for Railgun, ReadStatus on the wire, CL-rebuild, receive-watcher, `simulate`) — see "Integration into the app." Part of the Deckard build docs.
>
> **Verification note (2026-06-05):** every API/architecture claim below was re-derived from the actual a16z/helios source at tag `0.11.1` (ref `204c998a`) and adversarially re-checked by a second pass — *not* from memory. The numbers come from a runnable spike that actually syncs mainnet and survives a cut EL on this desktop. Anything still unverifiable is flagged ⚠.

## Why this exists (concrete)

Deckard today reads chain state from whatever RPC it's pointed at and **believes the answer** — a trusted-server assumption Deckard's whole pitch rejects. [Helios](https://github.com/a16z/helios) (a16z, Rust, MIT) turns an *untrusted* execution-layer RPC into a *verified* local endpoint: it re-derives every balance from a Merkle proof and checks it against the consensus-layer sync committee, so **the RPC cannot lie to you.** We embed it as a Rust library and route **all** of Deckard's reads through it.

**The core property is integrity, not availability.** Deckard can even *ship its own default RPC* and the user need not trust it — every instance runs Helios locally and verifies, so a Deckard-hosted (or any) RPC is a convenience, not a trust dependency (and the user can point at their own RPC with one setting). That is the moat: self-custody of *keys* is half the story; Helios makes your *view of the chain* self-custodial too.

Two demonstrations of the one property, both proven by the spike in `spikes/helios-walkaway/`:
- **Integrity (the moat) — `SCENARIO=lie`:** point Helios at a *malicious* RPC that rewrites the balance in every `eth_getProof`. Deckard **refuses the read** (`invalid account proof`) instead of showing the fake 1,000,000,000 ETH a centralized wallet would display. *No centralized wallet can do this.*
- **Availability (the beat) — default scenario:** sync a real mainnet client, serve the verified deposit-contract balance (≈86.3M ETH), then **cut the primary EL RPC** on camera and keep returning that verified balance via a second EL. Headless, exit-coded PASS.

## A naming caveat: our "walkaway beat" ≠ Vitalik's "walkaway test"

Vitalik's **"walkaway test"** (X, Jan 2026) is a property of the **protocol**: Ethereum should be able to *ossify* — keep running safely and stay useful **even if core developers stop shipping upgrades** — which is why he frames **quantum resistance** as urgent (be safe for decades before a crisis forces rushed changes). That is *not* what Deckard's demo "walkaway beat" means.

Deckard's beat is the **user-side analogue**: you don't depend on any *particular* infrastructure operator (RPC vendor / the EF's endpoints) to use or verify Ethereum. The two rhyme — both are "the system survives if a privileged party walks away" — and Helios is in fact a *component* of Vitalik's vision: an ossified chain only stays usable for normal people if anyone can **verify it cheaply without trusting operators**, which is exactly the light-client thesis ("don't trust, verify"). So position Deckard as **walkaway-test-*aligned*** (verify-it-yourself reads, no operator dependence, quantum-readiness on the roadmap via Kohaku's PQ account — see `06-privacy.md`) — **not** as "the walkaway test." To avoid the clash, prefer naming the beat **"verified reads / no-trusted-RPC"** (integrity) with **"cut-the-RPC"** as its availability demo; keep "walkaway" as an internal nickname only.

## Where it sits — Depends on / Unblocks (cross-doc + demo)

**Depends on:**
- A private/proxied upstream EL RPC URL and a CL light-client RPC URL — see "Privacy interplay" below and the network plumbing in `00-test-harness.md`.
- Nothing in the signer/keystore path: Helios is read-only. It never touches the key.

**Unblocks:**
- **Beat 2 / receive watcher** (`10-kohaku-shield.md`, deliverable #3): the watcher polls `eth_getLogs` / `eth_getBlockByNumber` through the Helios client so the "payment landed" event is itself verified. (Useful nuance, verified below: `get_logs` does **not** go through Helios's 60s head-age gate, so the watcher keeps working a bit differently from `Latest`-tag state reads.)
- **Beat 3 / walkaway** (deliverable #2): this doc *is* beat 3.
- **MCP `balance` / `simulate` reads** (`30-mcp-shape.md`): the daemon answers read intents from the Helios client. The `Intent`/`Decision`/daemon-socket contract is **owned by `30-mcp-shape.md`** — this doc only specifies that read intents resolve against the local Helios endpoint and that read status carries a `Verified|Degraded|Unsynced` flag.
- **`00-test-harness.md`**: owns spinning up the CL for a local devnet so this client has a consensus source to verify against (the Kurtosis section below now has the exact answer).

## Crate + API — verified against source at tag `0.11.1`

This section supersedes the earlier (memory-written) spec. The earlier version had three concrete errors, now fixed: the git tag (`0.11.1`, **no** `v`), `.checkpoint()` did not take a `?`, and the mainnet CL default is not `lightclientdata.org`.

**Crate — depend on `helios-ethereum`, NOT the umbrella `helios`.**
```toml
# Cargo.toml
helios-ethereum = { git = "https://github.com/a16z/helios", tag = "0.11.1" }

# Helios's workspace patches ethereum_hashing; [patch] does NOT inherit through a
# git dependency, so mirror it or the consensus crates fail to build:
[patch.crates-io]
ethereum_hashing = { git = "https://github.com/ncitron/ethereum_hashing", rev = "7ee70944ed4fabe301551da8c447e4f4ae5e6c35" }
```
- The umbrella `helios` crate re-exports everything (`helios::ethereum::*`) **but also pulls `helios-opstack` → libp2p → a yanked `core2 0.4.0`, which fails to resolve today.** Depending on `helios-ethereum` directly avoids opstack/linea/libp2p entirely (smaller tree, no p2p stack) and builds clean. Verified: the spike builds against `helios-ethereum` in ~2.5 min release.
- **Not on crates.io.** `helios-ethereum` on crates.io is stale at `0.1.0` (published 2024-10-27); `0.11.1` is **git-only**. Pin the tag (`0.11.1`, not `v0.11.1` — the `v`-prefixed tag is a 404). Re-verify the builder API at any bump (pre-1.0).
- **alloy alignment is a non-issue (resolved).** Helios pins the `alloy` meta-crate `1.0.37` (caret), which resolves `alloy-primitives` up to Deckard's pinned `1.6.0` — they **unify to a single `alloy-primitives 1.6.0`** in the lock. `Address`/`B256`/`U256`/`BlockId` are the same type at the Helios↔Deckard boundary; no duplicate-types conflict. (Verified in the spike's `Cargo.lock`: one `alloy-primitives`, version `1.6.0`.) revm pins `29.0.1`.

**Library construction — verbatim shape, corrected (`ethereum/src/builder.rs`):**
```rust
use helios_ethereum::config::networks::Network;
use helios_ethereum::database::FileDB;
use helios_ethereum::{EthereumClient, EthereumClientBuilder};
use alloy::primitives::B256;

// Turbofish <FileDB> pins the builder's DB type param up front (the builder is
// generic `EthereumClientBuilder<DB: Database>`; `.with_file_db()` exists ONLY on
// `<FileDB>`, `.with_config_db()` only on `<ConfigDB>`).
let client: EthereumClient = EthereumClientBuilder::<FileDB>::new()
    .network(Network::Mainnet)              // Mainnet | Sepolia | Holesky | Hoodi
    .consensus_rpc(consensus_rpc)?          // -> Result<Self>  (needs ?)  our private CL LC-API
    .execution_rpc(untrusted_el_rpc)?       // -> Result<Self>  (needs ?)  our private/proxied EL
    .checkpoint(trusted_checkpoint_b256)    // -> Self          (NO ?)     takes a B256
    .strict_checkpoint_age()                // -> Self          refuse a >14d checkpoint, don't warn
    // .load_external_fallback()            // -> Self          community checkpoints — gate behind a flag
    .data_dir(deckard_data_dir().join("helios"))
    .with_file_db()                         // pins DB=FileDB; persists last finalized checkpoint
    .build()?;                              // -> Result<EthereumClient>

client.wait_synced().await?;                // returns once CONSENSUS bootstrapped (see caveat ↓)
```
Fallible setters returning `Result<Self>` (need `?`): `consensus_rpc`, `execution_rpc`, `fallback`, `verifiable_api` (all generic over `T: IntoUrl`, so `&str`/`String`/`Url` all work). Infallible setters returning `Self`: `network`, `checkpoint(B256)`, `data_dir(PathBuf)`, `rpc_address(SocketAddr)`, `config(Config)`, `load_external_fallback()`, `strict_checkpoint_age()`, `with_file_db()`, `with_config_db()`.

**⚠ `wait_synced()` is NOT "ready to serve reads."** It returns once the *consensus* checkpoint is bootstrapped; the latest *execution* head isn't pushed into cache until the next optimistic update (≤1 slot, ~12 s). Until then `get_block_number()` / any `Latest`-tag read fails the 60 s head-age gate with `OutOfSync`. **Poll `get_block_number()` until `Ok` after `wait_synced()`** (the basic.rs example sleeps 15 s for exactly this; the spike polls). This caught us — measure "time to first servable head," not "time to `wait_synced`."

**The read surface lives on the `HeliosApi<N>` trait** (`EthereumClient = HeliosClient<Ethereum>` derefs to `Arc<dyn HeliosApi<Ethereum>>`). The methods Deckard uses, with signatures:
- `get_balance(Address, BlockId) -> Result<U256>` · `get_nonce(..) -> Result<u64>` · `get_code(..) -> Result<Bytes>` · `get_storage_at(..) -> Result<B256>` · `get_proof(..) -> Result<EIP1186AccountProofResponse>`
- `get_block_number() -> Result<U256>` · `get_block(BlockId, full) -> Result<Option<..>>` · `call(&TxReq, BlockId, Option<StateOverride>) -> Result<Bytes>` · `get_logs(&Filter) -> Result<Vec<Log>>`
- **Status observables (these power `ReadStatus`):** `syncing() -> Result<SyncStatus>` (`None`=synced, `Info`=catching up), `current_checkpoint() -> Result<Option<B256>>`, `new_checkpoints_recv() -> watch::Receiver<Option<B256>>` (fires on each sync-committee update — a liveness signal), `wait_synced()`, `shutdown()`.
- Pass `Latest` as `alloy::eips::BlockNumberOrTag::Latest.into()` (a `BlockId`). `U256` head does **not** cleanly `.into()` a `BlockId` — use the tag.

## Architecture — and the one fact the whole walkaway rests on

Helios is an EL light client: it takes beacon block headers verified by the **CL sync committee** and combines them with an *untrusted* EL RPC to return verified EL data. The untrusted EL must serve correct Merkle proofs (`eth_getProof`); it cannot lie about state without detection. Source: a16z, ["Building Helios"](https://a16zcrypto.com/posts/article/building-helios-ethereum-light-client/).

**The load-bearing detail (verified in `core/src/client/node.rs` + `execution/providers/`):** when the consensus client verifies a new header, a background task **pushes the verified execution block into the execution provider's in-memory cache** (`execution.push_block(block, Latest)`). So:

- **`get_block_number()` / head reads from that cache — it does NOT call the EL RPC.** The head is *consensus-driven* and EL-independent.
- **Only proof-bearing state reads hit the EL.** `get_balance` → `get_account` → `eth_getProof` against the untrusted EL, then verifies the returned account against the cached header's state root.
- Each `Latest`-tag read first runs `check_head_age()`, which **hard-fails with `OutOfSync` once the cached head is >60 s old.** (`get_logs`, `get_transaction`, receipts, `send_raw_transaction` skip this gate — relevant for the receive-watcher.)
- Helios's `CachingProvider` **caches the account proof per block**, so repeated `get_balance` of the same address at the same head is served from cache with no EL call until the head advances.

These four facts dictate the entire failover design and the demo's behavior. They are why the walkaway is honest and why it's demoable.

## The walkaway beat (R2) — the chosen design, proven

**Verified constraint:** one `EthereumClient` has exactly one EL and one CL (`execution_rpc`/`consensus_rpc` are single URLs). **Helios has no native multi-EL/CL failover.** Continuation after a cut is *Deckard's* logic.

**Chosen shape: (A) two synced clients + a supervisor.** Build `primary` (EL #1 = the "centralized" one we cut) and `secondary` (EL #2 = independent EL), both verifying against the same CL + checkpoint, both already synced. The supervisor routes reads to `primary`; on error/timeout it fails over to `secondary` and the first success becomes active. Both clients are equally trustless — failover re-derives the proof from an independent untrusted EL and re-verifies; it is **not** a cached stale value. This is `spikes/helios-walkaway/src/upstreams.rs`. We rejected shape (B) (tear down + rebuild on EL #2) because (A) needs no rebuild and the second client is already at the head.

**Cut the EL, not the CL — that's where the property lives.** Because the head is CL-driven and cached:
- **Cut EL #1 (CL stays up):** the head keeps advancing and `get_block_number()` *still returns* — proven by reading the **cut primary's own client** after the cut (`head_of_primary` returns from the CL-pushed cache with its EL dead). State reads fail on EL1 and recover on EL2. The transition is `Verified → Degraded{failover→EL2}` and it **stays Degraded on the backup** — the supervisor does not auto-probe back to the primary yet (recovery-to-`Verified` is a TODO, see Integration). For the demo that's fine: the balance is still verified the whole time; only the trust label reads "degraded/failover."
- **Cut the CL instead:** the head freezes; after 60 s every `Latest`-tag read hard-fails `OutOfSync` and `syncing()` flips to `Info`. **And Helios does not self-heal a dead CL** — when the consensus channel closes, the node logs *"consensus client stopped, shut Helios down manually"* and stops (`core/src/client/node.rs`); transient CL blips are retried inside the consensus loop, but a sustained CL death requires Deckard to **rebuild** the client against CL #2 (warm-start from the cached checkpoint, ~2 s). So cutting the CL is the *graceful-degradation* path ("verified locally, head frozen → NOT VERIFIED"), not a "keeps working" beat. **Don't cut the CL on camera.**

**The cache cushion (measured, important for the shoot).** After the EL cut, reads stay `Verified` from the per-block proof cache until the head advances to a *new* block, which forces a cache-miss `eth_getProof` → that's when failover actually fires. So the **cut→failover wall-clock is gated by the block cadence (0–12 s), not the supervisor** (which adds ~250–500 ms once a real EL read is attempted). Two spike runs bracketed this exactly: **1998 ms** (cut landed late in a slot) and **14744 ms** (cut landed just after a block). On camera this reads *well*: the verified balance never blinks — it holds through the cut and re-verifies via the backup within a block. If you want an instant visible flip, the supervisor can proactively issue a `get_proof` on cut-detection instead of waiting for the cached read to expire.

**`ReadStatus` transitions, mapped to real Helios observables** (this is the **target** contract; what the spike implements today is noted per row):

| State | Condition (observable) | Demo meaning |
|---|---|---|
| State | Condition (observable) | Demo meaning | Spike today |
|---|---|---|---|
| `Verified` | served by the primary EL (head fresh) | trustless, happy path | ✅ implemented |
| `Degraded { reason: "failover→EL2" }` | primary EL read errored, secondary EL read succeeded; head still fresh | **the walkaway** — re-verified via backup, balance unchanged (stays Degraded; no auto-probe back) | ✅ implemented |
| `Degraded { reason: "checkpoint:community" }` | running on `load_external_fallback` (ethPandaOps) checkpoint | verified, but checkpoint source untrusted — show a trust note | ⛔ not yet — daemon build task |
| `Unsynced { reason: "head frozen…" }` | every EL failed **and** `syncing()==Info` (head age >60 s, CL dark) | NOT VERIFIED — never serve raw RPC | ✅ classified via `syncing()` |
| `Unsynced { reason: "all EL upstreams down" }` | every EL failed but head still fresh | NOT VERIFIED — can't produce a proof | ✅ implemented |
| `Unsynced { reason: "checkpoint too old" }` | `strict_checkpoint_age` rejects a >14 d checkpoint at build/sync | NOT VERIFIED — re-bootstrap from a fresh checkpoint | ⛔ not yet (the builder *can* fail; not surfaced as a status) |

(The spike uses `Verified` as "served by primary," not literally `syncing()==None` on every read; for the demo the two coincide. The checkpoint-status rows are the daemon's job, not the spike's.)

Hard rule (unchanged): **never silently fall back to a raw untrusted RPC.** Verified-or-visibly-degraded, never quietly-trusted. The wire shape of how `ReadStatus` rides on a read response is **proposed, not yet frozen** in `30-mcp-shape.md` (see "Integration into the app").

## Integration into the app (how this wires in)

> Status: **designed here, not yet built** — the spike is standalone and `src/wallet.rs` is still a plaintext EOA (per `30`). This section closes the cross-doc seam the README lists as "20 provides the EIP-1193 provider + ReadStatus," and resolves the two open placement questions.

**One read module, in the daemon, key-less.** A single `Upstreams` supervisor (the Shape-A failover wrapper, which itself holds 1–2 `EthereumClient`s) lives inside `deckard-signerd` as a **read-only module with no handle to the key.** This resolves the "daemon read path vs MCP `Decision` resolver" question in favor of the daemon — matching `30`'s lean (*"simulate in the daemon so the approval card and the agent see identical numbers"*). Helios is read-only and never touches the keystore (already a stated dependency), so co-locating the read module with the signer adds no key-access path; its only outbound traffic goes to the already-untrusted EL/CL over the private/proxied upstreams.

```
   ┌─────────────── deckard-signerd (one process) ───────────────┐
   │   key module (isolated)            read module (NO key)      │
   │     sign / policy gate               Upstreams (Helios)      │
   └──────────────▲──────────────────────────────▲──────────────┘
   UDS: propose/  │                  UDS: wallet_balance/simulate │  (key-less,
   execute        │                  + ReadStatus on every read   │   ReadStatus-tagged)
   ┌──────────────┴───────────┐  ┌────────────────┴───┐  ┌────────┴──────────────────┐
   │ deckard-mcp (thin shell) │  │ GPUI app (UI badge)│  │ Railgun shield (EIP-1193) │
   └──────────────────────────┘  └────────────────────┘  └───────────────────────────┘
```

**Three consumers, two read paths:**
1. **Daemon socket reads** (`wallet_balance`, `simulate`) — typed `HeliosApi` calls through the supervisor, so they get EL-cut failover **and** a `ReadStatus`. The GPUI UI badge and the MCP agent both consume these → one source of truth, identical numbers.
2. **Railgun's chain reads** (UTXO/TXID sync, balance, state) — Railgun wants `RailgunBuilder::new(chain, impl IntoEip1193Provider)`, but `EthereumClient` exposes the typed `HeliosApi`, **not** an EIP-1193 `request(method, params)` JSON interface. Decision:
   - **v1 (demo) — Helios's built-in localhost JSON-RPC server.** Build the primary client with `.rpc_address(127.0.0.1:<ephemeral>)` (verified: `EthereumClientBuilder::rpc_address(SocketAddr)`; `HeliosClient::new` then spawns `jsonrpc::start`, which serves the `eth_*` subset Helios implements — the methods Railgun needs for live/state reads, all proof-checked; it is **not** a full JSON-RPC surface, so ⚠ confirm Railgun only calls served methods) and hand Railgun an **alloy HTTP provider** pointed at it. Least code, reuses Helios's own correct mapping. Accepted tradeoffs: (i) a loopback hop + a port (bind `127.0.0.1`, same-uid only); (ii) the server is per-`EthereumClient`, so Railgun's reads hit the primary only and do **not** get the supervisor's EL-cut failover — fine, because the shield completes *before* the on-camera cut and Railgun's reads are never the thing being cut. ✅ **PROVEN end-to-end** (`spikes/eip1193-railgun/`, 2026-06-06, against `kohaku@618c53f`): `IntoEip1193Provider` is Kohaku's *own* narrow 7-method trait (its `eip-1193-provider` crate) — **not** alloy's and **not** a generic `request(method,params)`; an alloy `DynProvider` (`ProviderBuilder::new().connect(url).erased()`) satisfies it via Kohaku's **shipped** `Alloy` adapter (`impl IntoEip1193Provider for DynProvider`) — **no custom adapter for v1** (confirmed by upstream's own `sync_utxo.rs`). The read/sync/balance path calls exactly **3** of those methods, all in Helios's served set: `eth_blockNumber` (`RpcSyncer.latest_block`) + `eth_getLogs` (`RpcSyncer.events`, tail range only) + `eth_call` (`SmartWalletUtxoVerifier.verify_root`); `balance()`/`register()` are local. **One required fix:** alloy's `Provider::call` defaults to the `pending` block tag (`alloy-provider 1.8.3` `trait.rs:198`), which Helios (a light client) can't serve (`block not found: pending`) — build the provider with `ProviderBuilder::new().with_default_block(BlockId::latest())` (installs alloy's `BlockIdLayer`) so the **unmodified** adapter's `eth_call` targets `latest`. One line, Deckard-side, no Kohaku/Helios patch. Historical UTXO ranges go to Subsquid, not Helios (10).
   - **production — a thin Rust adapter.** `struct HeliosEip1193(Arc<Upstreams>)` implementing Kohaku's `Eip1193Provider` trait (7 methods) by mapping each → a typed `HeliosApi` call (`get_block_number`, `get_logs`, **`call` at `Latest`** — same pin-to-latest discipline, since it bypasses the alloy `pending` default entirely). Removes the loopback hop and puts Railgun's reads behind the same failover + `ReadStatus`. Build post-demo; keep it the single place a Helios↔Railgun API change touches.

**`ReadStatus` on the wire — cross-doc proposal to `30` (it owns the contract).** For "every read carries Verified|Degraded|Unsynced" to be enforceable, `ReadStatus` must live in `deckard-contract` (the shared type home `30` owns) and ride on the read responses. Proposed delta:
- define `enum ReadStatus { Verified, Degraded{reason}, Unsynced{reason} }` in `deckard-contract` — **20 owns the semantics/transitions** (table above); the **type lives with the contract** so it can serialize on the wire.
- `wallet_balance` → `{ public_wei, shielded_wei, token_balances[], read_status }`
- `simulate` → `{ asset_changes[], gas, warnings[], read_status }`

Today `30`'s read responses omit `read_status`; without it the "never silently trust" rule can't be enforced at the wire. (30 owns the final shape — this is the ask.)

**CL-death handling (build task).** The supervisor gains a frozen-head detector: when `syncing()` flips to `Info` (head age >60 s) and no EL failover recovers it, **rebuild** the client against CL #2 (warm from the cached checkpoint, ~2 s), surfacing `Unsynced{reason:"reconnecting CL"}` in the gap. Never serve a raw read while reconnecting.

**Deckard-side file layout:**
```
src/chain/helios.rs        # EthereumClient wrapper: build, wait_synced → servable, shutdown
src/chain/upstreams.rs     # Upstreams supervisor (Shape A) + CL-rebuild-on-frozen
src/chain/read_status.rs   # ReadStatus (re-exported from deckard-contract)
src/chain/eip1193.rs       # v1: localhost-server wiring · prod: HeliosEip1193 adapter
~/.../Deckard/helios/      # data_dir: cached finalized checkpoint (with_file_db)
```
The spike (`spikes/helios-walkaway/`) already implements `read_status.rs` + `upstreams.rs` in portable form — lift them in, add `helios.rs` (build/servable wrapper) and `eip1193.rs`.

## Inputs, trust, and the checkpoint

**Three inputs (verified):**
1. **Untrusted EL RPC** (`execution_rpc`) — must support `eth_getProof`. (Not all public RPCs do — it's the gating filter; see providers.)
2. **CL light-client RPC** (`consensus_rpc`) — must speak the beacon light-client REST API. The mainnet default in source is `https://ethereum.operationsolarstorm.org` (a CNAME to the Nimbus-team `testing.mainnet.beacon-api.nimbus.team` box) — **not** `lightclientdata.org` (that's a16z's old default, currently 503). Sepolia/Holesky/Hoodi have **no** default CL (`consensus_rpc=None`) — you must supply one.
3. **Weak-subjectivity TRUSTED CHECKPOINT** (`checkpoint`, a `B256` beacon block root) — the one thing trusted on cold start. `max_checkpoint_age = 1_209_600` s = **exactly 14 days** for every network. `strict_checkpoint_age()` refuses an older one instead of warning — run strict in the demo build.

**Checkpoint sources, in descending trust** (unchanged, all verified to exist):
- **User-pinned** (best): a recent finalized root from a source you trust; Deckard ships a recent default + lets the user override.
- **Cached** (good): `FileDB` persists the last finalized root to `data_dir/checkpoint` (32 raw bytes); next start re-uses it if fresh. This is what makes warm start ~2 s vs ~11 s cold.
- **Community fallback** (weakest): `load_external_fallback()` / `CheckpointFallback` queries ethPandaOps's list, which the example code itself calls *"NOT guaranteed to be secure."* Treat as last resort, surface as `Degraded` when used.

## Beacon light-client providers — and a caveat the route-200 check misses

A provider qualifies only if it serves the `/eth/v1/beacon/light_client/*` REST namespace **and** full `/eth/v2/beacon/blocks/{slot}` blocks whose `tree_hash_root` matches the verified header. **Serving the LC routes with HTTP 200 is necessary but NOT sufficient** — Helios fetches the full block to extract the execution payload header, and rejects it on a hash mismatch. We learned this the hard way:

| Endpoint | LC routes 200? | Helios actually syncs? | Notes |
|---|---|---|---|
| `http://testing.mainnet.beacon-api.nimbus.team` (Nimbus) | yes | **yes (verified — cold 11 s, warm 2 s)** | Helios's shipped mainnet default backend. Plain HTTP, no SLA, team "testing" box. **Use this for the spike.** |
| `https://lodestar-mainnet.chainsafe.io` (ChainSafe) | yes | **NO in our test** — head stuck at timestamp 0 (`out of sync`) | Routes return 200 but Helios couldn't derive a fresh execution head against it on 2026-06-05. ⚠ re-test before relying. |
| `https://ethereum-beacon-api.publicnode.com` (PublicNode) | yes (`/updates` `count` param buggy) | **NO** — `sync failed: invalid sync committee period` | keyless, HTTPS, no-log policy, but the `/updates` bug breaks Helios bootstrap. Don't use as a Helios CL. |
| `https://eth-beacon-chain.drpc.org` (dRPC) | yes | **yes (verified — cold 10.4 s, head 25253907)** | keyless, HTTPS. **The proven public second CL.** |
| `https://www.lightclientdata.org` (a16z old default) | **503** | — | down. |
| beaconcha.in / checkpoint-sync hosts (sigp, attestant, ethpandaops) | 404 on LC routes | — | checkpoint-sync only; **not** an LC API. |

**Most commercial EL-RPC providers do NOT expose the light-client subset** (Ankr's beacon endpoint 404s on `light_client/*`; QuickNode serves it only if you provision your own Lighthouse-backed beacon endpoint; Chainstack/Blockdaemon/Nodereal unconfirmed). Of the keyless mainnet LC servers, only two are **proven to actually drive a Helios sync**: **Nimbus-testing** and **dRPC**. Lodestar and PublicNode return 200 on the routes but fail Helios sync (timestamp-0 head; `invalid sync committee period`, respectively). 200 ≠ syncs.

**Chosen approach for the hero (CEO review): public CLs — and the prerequisite is now met.** Two independent, keyless, proven-to-sync public CLs:
- **Primary: Nimbus** `http://testing.mainnet.beacon-api.nimbus.team` (cold ~11 s). Plain HTTP, no-SLA team box.
- **Second: dRPC** `https://eth-beacon-chain.drpc.org` (cold ~10.4 s, verified 2026-06-05). HTTPS, keyless.

Self-hosting a Lighthouse CL stays as the fallback only if a pre-shoot rehearsal shows both publics are flaky. Honest caveat: both are best-effort, **no-SLA** hosts; integrity is guaranteed by the sync committee + checkpoint regardless of which CL you use — only **liveness** and **metadata** depend on the provider. Still do a health-check of both in the hour before the take, and only ever cut the EL on camera, never the CL.

**Self-host fallback (smallest path).** The `light_client/*` namespace is standard ([beacon-APIs spec](https://github.com/ethereum/beacon-APIs)). Which CLs serve it:

| CL | LC server default | Flag |
|---|---|---|
| **Lighthouse** | **ON by default** | disable-only: `--disable-light-client-server`. Just run `--http`. **Easiest self-host.** |
| **Nimbus** | **ON by default** | `--light-client-data-serve=true` (default). |
| **Lodestar** | **ON by default** | `lightclient` is in the default REST namespaces; disable-only `--disableLightClientServer`. |
| **Teku** | ⚠ **conflicting reports** | one source: `--light-client-support-enabled` default `true`; another: `--Xrest-api-light-client-enabled` default `false`. **Resolve or avoid Teku.** |
| **Grandine** | ⚠ unverified | no documented LC flag; couldn't confirm the routes. **Avoid for now.** |

## Privacy interplay

Helios closes the **integrity** gap (no server can lie about state) but **not** the **metadata** gap — and the gap is asymmetric:

| Upstream | Sees IP? | Sees user address? | How |
|---|---|---|---|
| **EL (execution RPC)** | yes | **YES** | `eth_getProof` (address is a param, backs balance/nonce/code/storage), `eth_call` (`to`/`from`/`data`), `eth_getLogs` (address/topics). **The real leak surface.** |
| **CL (beacon LC RPC)** | yes | **no** | every LC endpoint carries only slots / sync-committee periods / block roots — **no user address ever crosses the CL.** |

So spend the privacy budget on the **EL**; the CL needs IP hygiene only, not address hygiene. Mitigations, ranked: (1) **self-host the EL** (closes it fully; heavy); (2) **self-hosted Helios `verifiable-api` server** in front of your EL — note that a *third-party-hosted* verifiable-api leaks the same IP+address (the address is in the URL path), so it's only a privacy win if **you** run it; (3) **a proxy Deckard controls**; (4) **a no-log keyless public EL that supports `eth_getProof`** — PublicNode (documented no-log + "IP not correlated to wallet addresses"), dRPC, BlastAPI public. **Tor is out of scope for v1.**

**Both EL upstreams (primary + failover) must be no-log/keyless — not Infura/Alchemy** (the IP↔address leak Deckard's pitch rejects). ⚠ "No-log" is a provider *policy*, not a cryptographic guarantee; attribute it to the provider, never assert it. The honest claim: *"verified locally, and no default IP↔address-correlating vendor in the read path"* — not *"private reads."* (The spike's defaults — publicnode + dRPC + Nimbus — are exactly this posture.)

## Measured (M-series desktop, mainnet, 2026-06-05, from the spike)

> These are **observed values from real runs on this build host**, reproducible via the spike's commands — they are **not** asserted in CI or stored as committed artifacts. Re-measure on the actual demo machine. The spike prints current-run values; only the PASS/FAIL verdict is asserted.

| Metric | Number | Notes |
|---|---|---|
| **Cold sync** (build → first servable verified head) | **≈ 10.9 s** | with `strict_checkpoint_age` + no user pin, the stale built-in default is rejected so `load_external_fallback` fetches a fresh community checkpoint; includes the ~12 s-bounded wait for the first execution head push. (`FileDB` otherwise falls back to the *built-in default* checkpoint, not the community one — the external fallback is conditional.) |
| **Warm sync** (cached `data_dir/checkpoint`) | **≈ 2.2 s** | ~5× faster; this is the demo-day number — **pre-sync, ship warm** |
| **Cut → failover (wall-clock)** | **≈ 2–15 s** | gated by block cadence (per-block proof cache), **not** the mechanism |
| **Failover mechanism alone** | ~250–500 ms | one failed EL attempt + one success on EL2, once a real `eth_getProof` is forced |
| **Head liveness after EL cut** | advanced 25252833 → 25252835 ✓ | served from CL cache with EL1 dead — EL-independent, as designed |
| **Verified balance correctness** | 86,313,877.35 ETH (deposit contract) | identical pre- and post-cut (0 wei drift) |
| Release build (helios-ethereum tree) | ~2.5 min | revm + alloy + bls; binary 19 MB |

Implication for the demo: the beat is **"warm-start instant"** (pre-sync to ~2 s) and the cut keeps the balance verified through one block. Cold start (~11 s) is a "syncing…" state if ever shown un-pre-synced.

**Measured — `eip1193-railgun` spike (M-series, mainnet, `--release`, 2026-06-06).** Helios warm sync ≈ 2.1 s / cold ≈ 10.5 s (consistent with above). All read-path methods (`eth_chainId`/`eth_blockNumber`/`eth_getLogs`/`eth_call`) resolve through Helios's localhost server; a 2000-block `eth_getLogs` window on the live RAILGUN wallet returned ~100–326 verified events, and Kohaku's real `RpcSyncer` parsed 414 `SyncEvents` through it. **Loopback-hop overhead: direct typed `HeliosApi` head read ≈ 0.75 ms/call vs alloy→Helios-localhost ≈ 1.0 ms/call → Δ ≈ 0.27–0.31 ms/call** (HTTP-serialize + two loopback syscalls + jsonrpsee dispatch). That hop is **cheap enough to ship the v1 localhost path for the demo and defer the production `HeliosEip1193` adapter** (which removes this hop and adds failover + `ReadStatus`).

## Local end-to-end testing (Kurtosis) — DEFERRED (not v1-critical)

> **Decision (CEO review):** Kurtosis is **deferred off the v1 critical path.** The mainnet spike already proves the whole R2 beat (sync, verified balance, cut-the-EL failover, refuse-a-lie) with **zero** Kurtosis, so a local devnet is not required to ship the demo. Its only added value is a *fully offline, deterministic CI lane where you own the CL* (no public-beacon flakiness in tests) — a post-demo hardening nice-to-have, not a gate. v1 testing runs on mainnet + Sepolia public endpoints. **TODO (post-demo): build the hermetic Kurtosis CI lane** (needs the hand-written Helios devnet `Config` below). The findings below are kept so that build is cheap when we pick it up. Note: Kurtosis is *not* a wallet feature and is *not* mainnet — it's a private throwaway devnet (a few GB, laptop-fine) used only for testing; it can't be shipped to users and can't replace Helios (it's the thing Helios verifies *against* in a test).

A plain **anvil** node has no consensus layer, so Helios cannot verify against it. The (now-answered) gating question was whether the Kurtosis `ethpandaops/ethereum-package` CL serves the LC API out of the box. **Answer: yes, with zero/near-zero flags** — Lighthouse, Nimbus, Lodestar all serve the LC API **on by default**, and ethereum-package runs **all forks from genesis** (Altair + sync committee live at slot 0). Minimal config:

```yaml
# lc-devnet.yaml — CL answers the light_client/* routes out of the box
participants:
  - el_type: geth
    cl_type: lighthouse   # serves LC by default; pass extra flags via cl_extra_params if ever needed
    count: 1
```
`kurtosis run github.com/ethpandaops/ethereum-package --args-file lc-devnet.yaml`, then point Helios's `consensus_rpc`/`execution_rpc` at the enclave's mapped CL/EL ports.

- **Option A (the deferred hermetic-CI lane):** the full Kurtosis devnet — CL and EL are internally consistent, so you can cut the EL against a CL you fully control with no public dependency. **Requires a hand-built Helios `Config`** (the `Network` enum hardcodes mainnet's CL and the testnets are `None`) with the devnet `chain_id`, both RPCs, and a fresh checkpoint (genesis/first-finalized root). Not built (deferred). When picked up, coordinate with `00-test-harness.md`.
- **Option B (anvil-fork EL + real mainnet CL) does NOT work** — and it's a trap worth stating: Helios verifies EL responses against the `state_root` the mainnet CL header attests to. A forked anvil matches that root only at the exact fork block with zero mutations; the instant it advances/mines, the root diverges and Helios's verification **fails** (not "works with stale data"). Plus the mainnet CL head keeps advancing while the fork doesn't, tripping the 60 s gate. Don't build the walkaway on it.
- **Gotchas:** `finality_update` only returns meaningfully after ~2 epochs finalize (~12.8 min at 12 s slots) — don't assert on it immediately post-`kurtosis run`. Keep all fork epochs at 0 (default). For the EL-only failover logic, unit-test the supervisor with mocked clients (no real verify) — the spike already isolates it in `upstreams.rs`.

## The spike (`spikes/helios-walkaway/`)

A standalone crate (own `[workspace]`, not part of deck's build) that proves the beat headless and prints the measurements above. Files mirror Deckard's intended layout:
- `read_status.rs` — `ReadStatus { Verified | Degraded | Unsynced }` (Deckard-owned).
- `upstreams.rs` — the failover supervisor (Shape A): `get_balance` with failover, `head()` (EL-independent), outage classification via `syncing()`.
- `proxy.rs` — a killable **and optionally lying** HTTP/1.1 reverse proxy: the kill switch is the on-camera "cut"; `lie=true` rewrites the `balance` in every `eth_getProof` response (a malicious RPC).
- `main.rs` — two scenarios + measurements; exit 0 = PASS.

Run:
- **Availability (cut-the-RPC):** `cargo run --release` (warm) / `WIPE=1 cargo run --release` (cold). Defaults to the privacy-correct posture (publicnode proxied + dRPC failover + Nimbus CL).
- **Integrity (refuse a lie):** `SCENARIO=lie WIPE=1 cargo run --release`. Proven result: malicious RPC claims **1,000,000,000 ETH**, Deckard returns `REJECTED — invalid account proof` (a centralized wallet would show the billion). Note it still *syncs and serves the head through the lying RPC* — only the proof-bearing balance read catches the lie, because the head is CL-verified.

See the README for the CL-choice and key-restricted-EL caveats.

**Sibling spike — `spikes/eip1193-railgun/` (the Railgun EIP-1193 seam, T-Trustless #3).** Boots the *same* Helios localhost server (`.rpc_address`) and settles the "v1 localhost vs forced-adapter" question in "Integration" above. **Tier-2** (default, light): an alloy `DynProvider` through Kohaku's *own* `IntoEip1193Provider` adapter drives `eth_chainId`/`eth_blockNumber`/`eth_getLogs`/`eth_call` through Helios, logged by a method-recording pass-through proxy. **Tier-1** (`--features railgun`, heavy): links the *full* `railgun` ZK crate (ark-circom/wasmer/groth16) — `RailgunBuilder::new(ChainConfig::mainnet(), <Helios DynProvider>).build()` OK, then Kohaku's real `RpcSyncer` drives `eth_getLogs` through Helios → 414 parsed `SyncEvents` from the live mainnet RAILGUN wallet. Verdict: **v1 WORKS** with the one-line `with_default_block(latest)` fix; the `railgun` crate compiles standalone from the spike's dep edge (mirrors 3 `[patch]`es). Numbers in "Measured" below.

**Acceptance test (the R2 slice; the spike implements steps 1–3):**
```
Scenario "Helios verified reads" (mainnet hero):
  1. build EthereumClient(EL1,CL,checkpoint); wait_synced(); poll until head servable
       assert: first servable head within the pre-sync window (cold ~11s / warm ~2s)
  2. read a KNOWN value (deposit contract balance) at the head
       assert: get_balance matches an independent source; ReadStatus == Verified
  3. INTEGRITY (the moat): point Helios at a MALICIOUS RPC (tampered eth_getProof balance)
       assert: get_balance REJECTS the read (invalid account proof); never returns the fake value
               (head still syncs through the liar — only the proof-bearing read catches it)
  4. AVAILABILITY (cut-the-RPC): cut EL1 (kill the proxy)
       assert: supervisor fails over to EL2, returns a VERIFIED balance, head still advances
               (Verified -> Degraded{failover→EL2}; stays Degraded on the backup), within ≤1 block + mechanism
  5. STALE CHECKPOINT: start with a >14d checkpoint + strict_checkpoint_age
       assert: build/sync FAILS visibly (Unsynced); NEVER silently serves raw RPC
```
Steps 2–4 are the on-camera beats (verified read, refuse-a-lie, cut-the-RPC); the same headless run + screen capture is the cut. The spike implements steps 1–4 across its two scenarios (default = 1/2/4, `SCENARIO=lie` = 3); step 5 is an unwritten guard test.

## Risks & fallbacks

- **R2 — no native EL/CL failover (verified).** Live "cut and continue" needs our supervisor (Shape A). *Status: the core is proven on mainnet (balance-read failover + lying-RPC rejection); the app-level wiring (EIP-1193, ReadStatus-on-wire, CL-rebuild, receive-watcher, simulate) is unbuilt.* Fallback for the EL: "verified locally, head frozen" badge if even (A) misbehaves.
- **The CL is the fragile, no-SLA dependency** (but redundancy is now real). A single CL stalling >60 s on camera hard-fails *every* `Latest`-tag read — looks like a crash. And Helios doesn't auto-recover a dead CL (requires a rebuild against CL #2). *Chosen mitigation (CEO review): public CLs, redundancy proven* — **Nimbus primary + dRPC second, both verified to drive a Helios sync** (Nimbus ~11 s, dRPC ~10.4 s). Rehearse/health-check both in the hour before, and **only ever cut the EL on camera, never the CL.** Self-host a Lighthouse CL only as the fallback if both publics look flaky at rehearsal. (Lodestar + PublicNode beacon return 200 but fail Helios sync — don't use them as CLs.)
- **Cache cushion shifts failover timing.** Cut→visible-failover is ≤1 block because of the per-block proof cache; the balance holds verified through the cut (good), but the visible `Degraded` flip waits for the next block. Force it with a proactive `get_proof` on cut-detection if an instant flip is wanted.
- **Checkpoint trust / community fallback "not secure."** Ship a recent pinned default + user override; run `strict_checkpoint_age`; mark community-sourced checkpoints `Degraded`.
- **API churn / git-pin.** Pre-1.0, git-pinned. Keep the thin wrapper (`upstreams.rs`/`read_status.rs`) so a Helios API change touches one place. Re-verify builder method names at each bump.
- **Devnet has no CL.** anvil can't be Helios-verified — use Kurtosis (Option A) or Sepolia. Owned by `00-test-harness.md`.

## Open questions — status after this deep-dive

- ~~Cold vs warm sync time; real failover latency~~ → **measured** (≈11 s / ≈2 s; failover ≤1 block). Re-measure on the actual demo machine.
- ~~Published crates.io release?~~ → **no**; `helios-ethereum` git-only at `0.11.1` (crates.io stale at 0.1.0).
- ~~alloy alignment~~ → **resolved**; unifies to one `alloy-primitives 1.6.0` (umbrella `alloy 1.8.3`). The `eip1193-railgun` spike confirms **Kohaku's `railgun` independently resolves to the identical `alloy 1.8.3`/`alloy-primitives 1.6.0`**, so Helios + Railgun link in one process with no version conflict (mirror 3 `[patch]`es: `ethereum_hashing` + the `ruint` & `ark-circom` forks).
- ~~CL approach + redundant second~~ → **decided + proven:** public CLs, **Nimbus primary + dRPC second** (both verified to drive a Helios sync, ~11 s / ~10.4 s). Lodestar + PublicNode beacon fail (200 but no sync). Self-host = flaky-rehearsal fallback only. Remaining minor: resolve the **Teku** default-flag contradiction or just avoid Teku.
- ~~Does Deckard auto-rebuild on a dead CL?~~ → **specced** as a supervisor build task (frozen-head detector → rebuild against CL #2, ~2 s warm) in "Integration into the app." Not yet built.
- ~~Failover (Shape A) in the daemon read path vs MCP `Decision` resolver?~~ → **decided:** one key-less `Upstreams` in `deckard-signerd` (see "Integration into the app"). Matches `30`'s "daemon so the numbers match" lean.
- ~~EIP-1193 adapter for Railgun~~ → **RESOLVED + PROVEN** (`spikes/eip1193-railgun/`): v1 = Helios localhost server + alloy `DynProvider` through Kohaku's *own* `IntoEip1193Provider` (no custom adapter), with the one-line `with_default_block(latest)` fix (alloy's `call` defaults to `pending`; Helios has none). Both the adapter-only path and the **full `railgun` crate** (linked under `--features railgun`; `RailgunBuilder::new(ChainConfig::mainnet(), <Helios DynProvider>).build()` + Kohaku's real `RpcSyncer` drove `eth_getLogs` through Helios → 414 SyncEvents) verified on mainnet. The `railgun` crate **compiles standalone** from our dep edge (retires 10's R1c). Production `HeliosEip1193` adapter still wanted (drops the loopback hop + adds failover/ReadStatus) and must likewise pin `eth_call`→`latest`. Loopback-hop overhead: see "Measured."
- **`read_status` on `30`'s read responses:** proposed (define `ReadStatus` in `deckard-contract`, add the field to `wallet_balance`/`simulate`). Needs `30`'s sign-off — it owns the contract.

## Sources (repos + docs)

- [a16z/helios](https://github.com/a16z/helios) @ tag `0.11.1` (ref `204c998a`) — `Cargo.toml` (workspace members, `alloy 1.0.37`/`revm 29.0.1`, ethereum_hashing patch); `ethereum/src/builder.rs` (builder signatures); `ethereum/src/lib.rs` (`EthereumClient` alias); `ethereum/src/config/networks.rs` (mainnet CL default, 14-day age, Hoodi); `ethereum/src/database.rs` (FileDB warm-start); `ethereum/src/consensus.rs` + `ethereum/src/rpc/http_rpc.rs` (LC endpoints, CL-death behavior); `core/src/client/{mod,api,node}.rs` (HeliosApi trait, CL-driven head, 60 s gate); `examples/{basic,client,checkpoints,call}.rs`.
- [Beacon LC API spec](https://github.com/ethereum/beacon-APIs) · [Lighthouse Book](https://lighthouse-book.sigmaprime.io/help_bn.html) · [Nimbus light-client-data](https://nimbus.guide/light-client-data.html) · [Lodestar beacon-cli](https://chainsafe.github.io/lodestar/run/beacon-management/beacon-cli/) — CL light-client-server defaults/flags.
- [ethpandaops/ethereum-package](https://github.com/ethpandaops/ethereum-package) — Kurtosis CL config (`cl_extra_params`), all-forks-from-genesis.
- [a16z: Building Helios](https://a16zcrypto.com/posts/article/building-helios-ethereum-light-client/) — design.
- `docs/research/06-privacy.md` — Infura IP+address leak; Helios as the embeddable Rust light client.
- `docs/research/v1-demo-plan.md` — beat 3, R2 spike, mainnet-regardless walkaway framing.
- **Live spike: `spikes/helios-walkaway/`** — the runnable proof + measurements.
