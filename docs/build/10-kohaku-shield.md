# Kohaku / Railgun Shield Integration

> Auto-shield received funds into an owner-only private balance using Kohaku's pure-Rust `railgun` crate · serves demo beat 2 (HERO "receive → instantly private") and acceptance step 2 (`shield(amount)` → private ↑, public ↓, link broken) · status: spec. Part of the Deckard build docs.

## Why this exists (2-4 sentences, concrete)

Beat 2 of the video is the hero action: a payment lands, the agent calls `shield(amount)`, and the funds move into a Railgun shielded pool where the balance is visible only to the owner's `0zk` viewing key. We consume the pure-Rust `railgun` crate inside `ethereum/kohaku` (an alloy-based `rlib`), **not** the `@kohaku-eth/railgun` WASM/TS wrapper, so Deckard's native GPUI/Rust process links it directly with no JS bridge. This is risk **R1** (alpha tooling, `@kohaku-eth/railgun@0.0.1-alpha.22`): the open question was whether the crate is standalone-consumable from Rust — the repo's integration tests answer **yes** (verified below), so the spec is "wire it up + spike on a fork," not "reverse-engineer a TS lib."

## Where it sits — Depends on / Unblocks (cross-doc + demo)

- **Depends on `20-helios-sidecar.md`** — all chain reads (UTXO/TXID sync, balance, on-chain state) go through an EIP-1193 provider; in the demo that provider is Helios over a private RPC. `RailgunBuilder::new(chain, provider)` takes `impl IntoEip1193Provider`, which is the seam (verified, see Interface).
- **Depends on `30-mcp-shape.md`** — owns the `Intent` / `Decision` / daemon-socket CONTRACT. The `shield(amount)` intent shape and the rule "auto-shield inbound ETH above X" are defined there; this doc only describes what the daemon does when it receives a `Shield` decision. Do **not** redefine the Intent enum here.
- **Depends on `00-test-harness.md`** — the anvil-mainnet-fork / Sepolia-fork harness, env vars (`RPC_URL_SEPOLIA`, `RPC_URL_MAINNET`), and the assertion runner used by the R1 spike below.
- **Unblocks** beat 2 / acceptance step 2 entirely. Without a working shield, the HERO action does not exist and the video has no payload.
- **Sibling, not dependency:** the **receive watcher** (deliverable #3) detects the inbound tx and emits the intent; it lives in T-Core, not here.

## Architecture / approach

The signer daemon (the only process holding key material — see `30-mcp-shape.md`) owns a long-lived `RailgunProvider`. The flow per the verified integration test `crates/railgun/tests/integration/transact_utxo.rs`:

```
inbound ETH lands at the EOA
  → receive watcher fires (Helios-verified logs)            [T-Core, 20-helios]
  → MCP: agent issues shield intent                          [30-mcp-shape]
  → daemon: railgun.shield().shield_native(zk_addr, value)
            .build(rng) → Vec<TxData>                        [this doc]
  → daemon signs + submits the shield deposit txs (EOA pays) [alloy / 08]
  → railgun.sync()                                           [reads via Helios]
  → railgun.balance(zk_addr) → private balance ↑, public ↓
  → UI renders "before/after, trail broken"                  [T-UX, #9]
```

Two distinct key materials, do not conflate:
1. the **EOA secp256k1 key** (`src/wallet.rs`, alloy `PrivateKeySigner`) — pays gas and signs the public shield-deposit tx;
2. the **Railgun spending+viewing keypair** (`railgun::account::signer::PrivateKeySigner`, a `RailgunSigner` over Poseidon/babyjubjub) — owns the `0zk` address and the private balance. Both live only in the signer daemon. The Railgun keys can be derived deterministically from the EOA seed (BIP-39 next increment) via `spending_key_path(index)` / `viewing_key_path(index)`, so backup is one seed.

**Shield is the only on-camera operation.** Shield is a *public* deposit tx from the EOA into the `railgun_smart_wallet` contract — the EOA paying it does not leak the private balance (that's the point of the pool). Private **transfer** and **unshield** must go through a **broadcaster** (4337 bundler) to avoid linking the EOA at withdrawal; those are fast-follow (see Shield lifecycle).

### Shield lifecycle — v1 vs fast-follow

| Step | What it does | EOA exposure | v1 demo? |
|---|---|---|---|
| **shield** | deposit ERC-20/ETH into the pool, credit a `0zk` note owned by the viewing key | EOA visibly deposits (expected) | **YES — the HERO** |
| **balance/sync** | sync UTXO/TXID state, decrypt owner notes, report private balance | read-only | **YES** (proves "trail broken") |
| private **transfer** | move value `0zk → 0zk`, encrypted | must use broadcaster or EOA leaks | fast-follow |
| **unshield** | withdraw `0zk → EOA/any address` | must use broadcaster or EOA leaks at exit | fast-follow (needed for R1 acceptance assert + fund recovery) |

The v1 demo needs **shield-on-receive + private-balance proof** only. Unshield is in the spike (to assert the link can be broken *and* funds recovered) but is not on camera.

### Proving

Proof generation (Groth16/BN254, ark-* + the patched ZK deps in the workspace `Cargo.toml`) happens **locally, in-process** inside `.build()` / `railgun.build(tx)`. There is no remote prover, so no metadata leak from proving — consistent with Deckard's local-first posture. **Cost is the open question** (see Open questions): the circuit + witness generation for a 1-in/2-out shield is the latency budget for "instant" auto-shield. Mitigation if proving is slow: enable the crate's `parallel` feature (the workspace patches `ark-*` for parallel proving), pre-warm the prover, and have the UI show "shielding…" between the deposit-tx confirmation and the proof landing rather than promising sub-second.

### Broadcasters / relayers (fast-follow, but spec'd now)

Transfer/unshield must not originate from the owner EOA or the privacy is lost at the edges. The crate submits these via a **4337-style broadcast**: `railgun.prepare_userop(tx, bundler, delegator_address, signer, fee_token, tail_calls, rng)` → a signable UserOperation; a *separate* `delegator` signer (the broadcaster's relay account, not the owner EOA) signs and the **bundler** submits it. A `TailCall` can atomically unwrap WETH→ETH on unshield. This is exactly what an unattended agent needs: it can move private funds without ever exposing the owner address (verified in `broadcast_utxo.rs`).

Compliance model: Railgun uses **Private Proof of Innocence (PPOI)** — a non-membership proof against blocklists, submitted to a POI node via JSON-RPC `ppoi_submit_transact_proof` (verified in `poi/client.rs`). Opt in with `RailgunBuilder::with_poi()`. v1 shield-on-receive does not strictly require POI submission to credit the private balance, but transfers/unshields out of the pool are gated on POI in production; build the daemon with `.with_poi()` so the path is exercised in the spike.

### RPC

Reads (UTXO sync, TXID sync, balance, `eth_call`) go through the `Eip1193Provider` Deckard hands `RailgunBuilder` — in the demo that is **Helios over a private RPC** (see `20-helios-sidecar.md`). UTXO syncing additionally uses a **Subsquid endpoint** (`ChainConfig.subsquid_endpoint`, with an `RpcSyncer` fallback chained after it) for fast historical scan — that path is a read of public pool events, not address-bearing, but note it as a second network dependency. Shield deposit tx submission is a normal alloy `send_transaction`. Transfer/unshield submission goes through the broadcaster/bundler.

## Concrete interface (commands, types, crate names, RPC methods, file layout)

**Crate:** `railgun`, version `0.1.0`, edition `2024`, `[lib] crate-type = ["rlib"]`, `default = []`, feature `js = ["dep:tsify","dep:wasm-bindgen"]` (we do **not** enable `js`), plus `parallel`, `testing`, `bench`. Depends on `alloy = "1.8"`, `ark-bn254 = "0.6"`, `tokio = "1.49"`. There is **no `license` field in the crate's own `Cargo.toml`**; the monorepo root `package.json` declares `"license": "MIT"` and the npm artifact is MIT — confirm the Rust crate inherits MIT before depending (see Risks). [crate Cargo.toml, root package.json]

**Cargo dependency** (no published crate; vendor or git-pin a commit):
```toml
[dependencies]
railgun = { git = "https://github.com/ethereum/kohaku", package = "railgun", rev = "<PIN_A_COMMIT>" }
# transitive: the workspace [patch] block patches ark-* for parallel/ZK — pin the rev so patches resolve
```

**Public API actually verified in `crates/railgun/src/`:**

```rust
// builder.rs
RailgunBuilder::new(chain: ChainConfig, provider: impl IntoEip1193Provider) -> RailgunBuilder
  .with_utxo_syncer(syncer: Arc<dyn UtxoSyncer>)   // ChainedSyncer of Subsquid + RpcSyncer
  .with_database(db: Arc<dyn Database>)             // persists synced UTXOs / POI proofs
  .with_poi()                                       // enable PPOI submission
  .build().await -> Result<RailgunProvider, RailgunProviderError>

// provider.rs — the handle the daemon holds
impl RailgunProvider {
  async fn register(&mut self, signer) -> ...       // register a 0zk account to track
  async fn sync(&mut self) -> Result<(), _>         // sync UTXO/TXID state via the provider
  async fn balance(&mut self, addr: RailgunAddress) -> HashMap<AssetId, u128>
  fn shield(&self) -> ShieldBuilder
  fn transact(&self) -> TransactionBuilder
  async fn build<R: Rng>(tx, rng) -> ProvedTransaction { tx_data, .. }   // direct (EOA-submitted)
  async fn prepare_userop<R: Rng>(tx, bundler, delegator, signer, fee_token, tail_calls: Vec<TailCall>, rng)
    -> SignableUserOp                               // broadcaster path (4337)
}

// transact/shield_builder.rs
ShieldBuilder::new(chain) 
  .shield(recipient: RailgunAddress, asset: AssetId, value: u128) -> Self
  .shield_native(recipient: RailgunAddress, value: u128) -> Self
  .build<R: Rng>(rng) -> Result<Vec<TxData>, ShieldError>   // submit each TxData via alloy

// transact/transaction_builder.rs
TransactionBuilder::new()
  .transfer(from_signer, to: RailgunAddress, asset, value, memo) -> Self
  .unshield(from_signer, to: Address, asset, value) -> Result<Self, _>

// account/signer.rs
trait RailgunSigner { fn sign(&self, inputs: U256) -> Result<SpendingSignature, _>; fn address(&self) -> RailgunAddress; }
PrivateKeySigner::new_evm(spending_key, viewing_key, chain_id: u64) -> Arc<Self>   // in-memory railgun keypair
spending_key_path(index: u32) -> String; viewing_key_path(index: u32) -> String   // derivation paths

// chain_config.rs
ChainConfig::mainnet() -> Self          // railgun_smart_wallet, wrapped_base_token, subsquid_endpoint
ChainConfig::sepolia() -> Self
ChainConfig::from_chain_id(id) -> Option<Self>
```

**RPC / methods touched:** standard `eth_*` reads through the EIP-1193 provider (→ Helios); Subsquid GraphQL for historical UTXO scan; `eth_sendTransaction`/`eth_sendRawTransaction` for the shield deposit; 4337 bundler `eth_sendUserOperation` for broadcast transfer/unshield; POI JSON-RPC `ppoi_submit_transact_proof`.

**Deckard file layout (proposed):**
```
src/shield/
  mod.rs          // pub fn handle_shield_decision(value, asset) -> ShieldResult  (called by daemon)
  client.rs       // owns RailgunProvider lifecycle: build once, sync, expose shield/balance
  keys.rs         // derive railgun spending+viewing keys from the EOA seed
spikes/r1_shield/ // standalone bin for the acceptance test below (or a #[test] in tests/)
```

## v0 baseline / spike plan + acceptance test (agent-runnable asserts)

**v0 baseline:** none. `src/wallet.rs` is a bare alloy EOA persisting plaintext hex; there is no Railgun code yet.

**R1 spike** — port `crates/railgun/tests/integration/transact_utxo.rs` into Deckard against `00-test-harness.md`'s anvil fork. That test already does the full shield→transfer→unshield with concrete balance asserts; reproducing it from *our* dependency edge proves standalone-consumability. Spike on **Sepolia fork first** (the upstream test forks Sepolia at block `10822990` and uses `RPC_URL_SEPOLIA`), then repeat on an **anvil mainnet fork** with `ChainConfig::mainnet()` before the mainnet hero.

```
Scenario R1 "shield + unshield, standalone Rust" (anvil fork; Sepolia first, then mainnet fork):
  setup: anvil --fork-url $RPC; RailgunProvider built with our Eip1193 provider (Helios in demo,
         plain alloy provider in spike); two railgun accounts registered; WETH deposited+approved
         to chain.railgun_smart_wallet.

  1. railgun.shield().shield(acct1, weth, 1_000_000).build(rng); submit each TxData; railgun.sync()
       assert: railgun.balance(acct1)[weth] == 997_500            # pool fee taken, private balance up
       assert: railgun.balance(acct2)[weth] == None
  2. shield_native(acct1, 100_000) → submit → sync
       assert: railgun.balance(acct1)[weth] increases             # native wrapped + shielded
  3. TransactionBuilder::transfer(acct1, acct2, weth, 5_000, "..") → railgun.build → submit → sync
       assert: balance(acct1) down 5_000; balance(acct2)[weth] == 5_000   # private transfer, no public trace
  4. TransactionBuilder::unshield(acct1, EOA, weth, 1_000) → railgun.build → submit → sync
       assert: WETH.balanceOf(EOA) increased (~998 after fee); balance(acct1) down  # link broken, funds recovered

  GREEN = R1 passes → attempt mainnet hero. RED = take a Fallback (below).
```

The exact numeric asserts (`997_500`, `5_000`, `998`) are copied from the verified upstream test, so a regression in our integration edge is immediately visible. Mark the spike `#[ignore]` (network) and run it explicitly in CI like upstream does.

**Demo acceptance (mirrors `v1-demo-plan.md` step 2):** after R1 is green, the on-camera assert is `private balance ↑, public ↓, link broken; tx confirms`, driven by the receive watcher → MCP `shield` decision → `handle_shield_decision`.

## Risks & fallbacks

- **R1a — alpha API churn.** `0.0.1-alpha.x` (latest `alpha.22`, 2026-05-26); the Rust crate is `0.1.0` and unpublished. *Mitigation:* git-pin a specific commit `rev`; vendor the crate if needed. Do not track `master`.
- **R1b — mainnet reliability of the alpha crate.** *Fallback (a):* shield on **Sepolia** for the video (`ChainConfig::sepolia()`), keep the Helios walkaway beat on mainnet — explicitly sanctioned by `v1-demo-plan.md`. The upstream test is itself Sepolia, so Sepolia is the better-trodden path.
- **R1c — crate not standalone-consumable / build breaks.** Largely *retired* by the verified `rlib` + alloy + integration tests, but if the workspace `[patch]` deps or edition-2024 toolchain fight Deckard's build: *Fallback (b):* a thin Node bridge to `@kohaku-eth/railgun@0.0.1-alpha.22` (MIT, published, WASM) spoken to over the daemon socket — slower and adds a JS runtime, last resort.
- **R1d — proving cost makes "instant" a lie.** *Mitigation:* `parallel` feature + pre-warm; UI shows a "shielding…" state. *Fallback:* shrink the demo amount / pre-shield a warm pool note so the on-camera proof is a 1-out path.
- **R1e — licensing.** Crate `Cargo.toml` has **no `license` field**; root `package.json` and npm say **MIT**. Deckard is 0BSD. MIT is compatible to vendor/depend on, but **confirm the Rust crate inherits MIT** (open a clarifying issue / check the eventual crate publish) before shipping. ⚠ partial: per-crate license not explicitly declared in-tree.
- **R1f — Subsquid/broadcaster centralization.** UTXO sync leans on a Subsquid endpoint and broadcast leans on a 4337 bundler — both are network deps that aren't Helios. For the demo, sync is a read of public events (acceptable); shield (the hero) needs no broadcaster. Flag for the "walkaway" narrative: shield-on-receive itself only needs the EOA + the pool contract.
- **Alternate shielded path (c):** **Privacy Pools** (`@kohaku-eth/privacy-pools`, live on mainnet since Mar 2025) if Railgun is unworkable — but it's marked WIP in the SDK and uses the opposite (allowlist-inclusion) compliance model, so treat as a true last resort, not a drop-in.

## Open questions

- **Proving wall-clock on a desktop:** how long does `ShieldBuilder::build()` / `RailgunProvider::build(tx)` take for a 1-in/2-out shield on an M-series Mac, with and without `parallel`? This sets the "instant" UX claim. (Bench in the R1 spike with `criterion` — the crate already ships `benches/`.) ⚠ unmeasured.
- **Does the crate's EIP-1193 provider accept Helios cleanly,** or does it need methods Helios doesn't serve (e.g. heavy log ranges for UTXO sync that Helios proxies but Subsquid actually answers)? Verify in the `20-helios-sidecar.md` integration. ⚠ unverified.
- **Per-crate license:** does `railgun` (no `license` field) inherit the monorepo MIT for a downstream Rust dependency? ⚠ partial.
- **Mainnet broadcaster availability:** is there a public Railgun 4337 bundler/broadcaster Deckard can use for unshield, or must we run one? (Not needed for v1 shield-on-receive; needed for the unshield fast-follow.) ⚠ unverified.
- **POI standby:** Railgun's ~1-hour unshield-only standby period (per `06-privacy.md`) — does it affect the on-camera unshield in the spike? (Shield + private-balance proof are unaffected.) ⚠ unverified against the crate.

## Sources (repos + docs, linked)

- `ethereum/kohaku` — privacy SDK monorepo, workspace `Cargo.toml` (8 crates, `alloy 1.8`, `[profile.release-wasm]`) — https://github.com/ethereum/kohaku/blob/master/Cargo.toml — (source, verified)
- `crates/railgun/Cargo.toml` — `name = "railgun"`, `0.1.0`, edition 2024, `crate-type = ["rlib"]`, `js`/`parallel`/`testing` features, `[[bin]] main` — https://github.com/ethereum/kohaku/blob/master/crates/railgun/Cargo.toml — (source, verified)
- `crates/railgun/src/{builder,provider}.rs` — `RailgunBuilder::new(chain, impl IntoEip1193Provider)`, `RailgunProvider::{register,sync,balance,shield,transact,build,prepare_userop}` — https://github.com/ethereum/kohaku/tree/master/crates/railgun/src — (source, verified)
- `crates/railgun/src/transact/{shield_builder,transaction_builder}.rs` — `ShieldBuilder::{shield,shield_native,build}`, `TransactionBuilder::{transfer,unshield}` — https://github.com/ethereum/kohaku/tree/master/crates/railgun/src/transact — (source, verified)
- `crates/railgun/tests/integration/transact_utxo.rs` — full shield→transfer→unshield with balance asserts on a Sepolia anvil fork (the R1 reference) — https://github.com/ethereum/kohaku/blob/master/crates/railgun/tests/integration/transact_utxo.rs — (source, verified)
- `crates/railgun/tests/integration/broadcast_utxo.rs` — 4337 broadcaster transfer/unshield via `prepare_userop` + bundler + `delegator` (EOA-unlinking path) — https://github.com/ethereum/kohaku/blob/master/crates/railgun/tests/integration/broadcast_utxo.rs — (source, verified)
- `crates/railgun/src/poi/client.rs` — PPOI submission via JSON-RPC `ppoi_submit_transact_proof` — https://github.com/ethereum/kohaku/blob/master/crates/railgun/src/poi/client.rs — (source, verified)
- `@kohaku-eth/railgun` npm — latest `0.0.1-alpha.22` (2026-05-26), 20 versions, license MIT (maturity signal) — https://www.npmjs.com/package/@kohaku-eth/railgun — (registry, verified via registry.npmjs.org)
- Railgun PPOI (non-membership compliance model, broadcasters, 1h standby) — https://docs.railgun.org/wiki/assurance/private-proofs-of-innocence — (docs, high)
- Internal: `docs/research/03-kohaku.md`, `docs/research/06-privacy.md`, `docs/research/v1-demo-plan.md`
