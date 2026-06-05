# Spike prompt: Helios as Railgun's EIP-1193 provider (Deckard T-Trustless #3)

> Feed this whole file to a fresh coding agent. It is self-contained. Goal: prove
> (or disprove) that an embedded Helios light client can serve as the EIP-1193
> provider Kohaku's `railgun` crate reads through, so Deckard's shield path gets
> **verified** chain reads instead of trusting a raw vendor RPC. Standalone spike —
> do **not** touch the app crates (`crates/`).

## Context you can trust (already verified against source — don't re-litigate)

- Helios is consumed as **`helios-ethereum`** (NOT the umbrella `helios` crate — the
  umbrella pulls `helios-opstack` → libp2p → a yanked `core2 0.4.0` that won't resolve).
  Git-only, tag `0.11.1` (no `v`). crates.io is stale at 0.1.0.
  ```toml
  helios-ethereum = { git = "https://github.com/a16z/helios", tag = "0.11.1" }
  alloy = { version = "1.0.37", features = ["eips", "rpc-types", "provider-http", "network"] }
  [patch.crates-io]
  ethereum_hashing = { git = "https://github.com/ncitron/ethereum_hashing", rev = "7ee70944ed4fabe301551da8c447e4f4ae5e6c35" }
  ```
  Mirror the `[patch]` or the consensus crates fail to build. `alloy` unifies to one
  `alloy-primitives 1.6.0`, so types match across helios + your spike. Add an empty
  `[workspace]` table to the spike's `Cargo.toml` so it stays standalone (the repo root
  is a workspace).
- **Helios ships a localhost JSON-RPC server.** `EthereumClientBuilder::rpc_address(SocketAddr)`
  + `with_file_db()` → on `build()`, `HeliosClient::new` spawns `jsonrpc::start(inner, addr)`
  serving a **verified** local endpoint at `http://<addr>`. It serves the `eth_*` subset in
  Helios's `core/src/jsonrpc/mod.rs` / `rpc.md` (getBalance, call, getCode, getLogs,
  getBlockByNumber, getTransactionReceipt, getProof, chainId, blockNumber, estimateGas,
  sendRawTransaction, subscribe(newHeads only), …) — **not** a full JSON-RPC surface.
- **`wait_synced()` ≠ ready.** After it returns, poll `get_block_number()` until `Ok`
  (the first execution head lands ~1 slot later, ≤12s; before that every `Latest` read
  fails the 60s `check_head_age` gate).
- **CL choice matters — 200 ≠ syncs.** On mainnet only **Nimbus**
  (`http://testing.mainnet.beacon-api.nimbus.team`) and **dRPC**
  (`https://eth-beacon-chain.drpc.org`) actually drive a Helios sync; Lodestar +
  PublicNode return 200 but fail. For **Sepolia** you must supply a Sepolia beacon
  light-client endpoint (`Network::Sepolia` has `consensus_rpc = None`).
- Working reference: `spikes/helios-walkaway/` already embeds Helios, syncs mainnet, and
  has a killable proxy + supervisor. Reuse its `Cargo.toml` shape and the
  build-then-poll-until-servable pattern.

## The two approaches (the spike must settle which one v1 uses)

- **v1 (least code): Helios localhost server + an alloy HTTP/EIP-1193 provider.** Build
  Helios with `.rpc_address(127.0.0.1:<ephemeral>)`, then hand `RailgunBuilder::new(chain, …)`
  an alloy provider pointed at `http://127.0.0.1:<port>`. Tradeoff: a loopback hop, and the
  server is per-`EthereumClient` (no failover supervisor in front).
- **production: a Rust adapter** `struct HeliosEip1193(EthereumClient/Upstreams)` implementing
  whatever trait `RailgunBuilder::new` wants, mapping `request(method, params)` → the typed
  `HeliosApi`. No hop, behind the supervisor.

The spike's job is to **prove v1 works end to end** and to discover exactly what (if anything)
forces the adapter.

## Tasks (do in order)

1. **Pin Kohaku's `railgun` API against source.** Read
   `github.com/ethereum/kohaku/tree/master/crates/railgun/src` (esp. `builder.rs`/`provider.rs`).
   Confirm the EXACT signature of `RailgunBuilder::new` and **what `impl IntoEip1193Provider`
   actually is** — is it alloy's provider trait, an `ethers`-style provider, or Kohaku's own
   trait? Write down the concrete trait + which provider types satisfy it. (This is the crux;
   everything else depends on it.) Note the crate is alpha (`0.1.0`/`rlib`) and Sepolia-oriented.
2. **Stand up a verified Helios localhost endpoint.** Bin that builds `helios-ethereum`
   `EthereumClientBuilder::<FileDB>::new().network(...).consensus_rpc(cl)?.execution_rpc(el)?`
   `.checkpoint(b256)`/`.load_external_fallback().rpc_address("127.0.0.1:0".parse()?)`
   `.with_file_db().build()?`, `wait_synced()`, poll until servable. Confirm
   `curl http://127.0.0.1:<port>` answers `eth_getBalance`/`eth_chainId` correctly (verified).
   Prefer **Sepolia** if that's where the Railgun contracts/tests live (check task 1); else mainnet.
3. **Wire Railgun to it.** Construct the alloy provider over the localhost URL and pass it to
   `RailgunBuilder::new(chain, provider)`. Build the Railgun client and perform the smallest
   read it supports (e.g. balance / a UTXO or pool-state sync init). Confirm the read resolves
   **through Helios** (watch Helios logs / the localhost server) and returns sane data.
4. **Enumerate the methods Railgun calls.** Instrument the localhost server (or a thin logging
   pass-through in front of it) to record every JSON-RPC `method` Railgun invokes during
   register/sync/balance. Cross-check each against Helios's served set. **Flag any method
   Helios does NOT serve** (likely heavy `eth_getLogs` ranges for historical UTXO scan — note
   `10-kohaku-shield.md` says Subsquid carries history, so confirm whether Railgun hits Helios
   for logs at all, or only Subsquid).
5. **Decide + measure.** Does the v1 localhost path work unmodified? If a method is missing or
   the trait doesn't accept an alloy HTTP provider, write the minimal adapter and note it's
   required for v1 (not just prod). Measure the per-call overhead of the loopback hop vs a
   direct typed `HeliosApi` call (rough is fine).

## Constraints

- Standalone crate at `spikes/eip1193-railgun/` with its own `[workspace]`; `.gitignore`
  `target/` + `Cargo.lock`. Do **not** edit anything under `crates/` or the app.
- Read-API only — no signing, no broadcasting, no real funds. Reads/sync only.
- If the Railgun crate can't be driven standalone from Rust (alpha API), say so plainly and
  fall back to: stand up the Helios localhost server, point a generic alloy provider at it,
  and prove the *provider* works for the eth_* methods Railgun's docs say it uses — i.e.
  de-risk the seam even if the full RailgunBuilder path is blocked.

## Success criteria (what "done" means)

- A clear YES/NO on: **"Does Helios's localhost server satisfy Railgun's `IntoEip1193Provider`
  and serve every method Railgun's read/sync path calls?"** with the exact trait + a method list.
- A runnable spike that boots Helios + a verified localhost endpoint and drives at least one
  Railgun (or generic alloy) read through it.
- A short report: v1 works as-is? / adapter required (and why)? / methods missing (→ Subsquid)?
  / loopback overhead. End with a recommendation for `20-helios-sidecar.md`'s "Integration"
  section (v1 localhost vs forced-adapter).

## Report format (return this)

```
<spike_report>
  <verdict>v1 localhost path: WORKS / NEEDS ADAPTER / BLOCKED</verdict>
  <railgun_provider_trait><!-- exact trait + satisfying types --></railgun_provider_trait>
  <methods_railgun_calls><!-- list; mark served-by-Helios vs missing/Subsquid --></methods_railgun_calls>
  <what_worked/>
  <what_failed/>
  <loopback_overhead/>
  <recommendation><!-- one paragraph for 20-helios-sidecar.md --></recommendation>
  <artifacts><!-- spike crate path + how to run --></artifacts>
</spike_report>
```
