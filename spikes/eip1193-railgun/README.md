# eip1193-railgun — Deckard T-Trustless #3 spike

**Question:** can an embedded **Helios** light client be the EIP-1193 provider
Kohaku's `railgun` crate reads through — so Deckard's shield path gets **verified**
chain reads instead of trusting a raw vendor RPC?

**Answer: YES — the v1 localhost path works, with one one-line provider fix.**

This spike boots a verified Helios mainnet light client with its **localhost
JSON-RPC server**, points an alloy provider at it, wraps that provider through
**Kohaku's real `IntoEip1193Provider` adapter**, and drives every method Railgun's
read/sync path calls THROUGH Helios — then (Tier-1) links the full `railgun` crate
and drives its real `RpcSyncer`/`RailgunBuilder` against the live mainnet RAILGUN
smart wallet through Helios.

## The crux, settled against source (`ethereum/kohaku @ 618c53f`)

- `RailgunBuilder::new(chain: ChainConfig, provider: impl IntoEip1193Provider)`.
  **`IntoEip1193Provider` is Kohaku's OWN trait** (in its `eip-1193-provider`
  crate), **not** alloy's. `Eip1193Provider` is a narrow **7-method typed trait**
  (`get_chain_id`, `get_block_number`, `logs`, `eth_call`, `estimate_gas`,
  `gas_price`, `transaction_count`) — **not** a generic `request(method, params)`.
- Kohaku **ships an alloy adapter**: `impl IntoEip1193Provider for DynProvider`
  wraps any alloy provider as `Arc<dyn Eip1193Provider>`. So
  `ProviderBuilder::new().connect(url).erased()` (a `DynProvider`) satisfies the
  trait **with no custom adapter** — confirmed by upstream's own `sync_utxo.rs` test.
- The **read/sync/balance** path touches the provider in exactly 3 places:
  `RpcSyncer.latest_block` → **eth_blockNumber**, `RpcSyncer.events` →
  **eth_getLogs** (tail range only; Subsquid carries history), and
  `SmartWalletUtxoVerifier.verify_root` → **eth_call**. `balance()`/`register()`
  are local (no RPC). All 7 trait methods — and all 3 read-path methods — are in
  Helios 0.11.1's served set (`core/src/jsonrpc/mod.rs`).

## The one required fix (the spike's real finding)

alloy's `Provider::call` **defaults to the `pending` block tag**
(`alloy-provider 1.8.3` `trait.rs:198`). Kohaku's `Alloy::eth_call` adapter calls
`inner.call(req)` with no block override, so it sends `eth_call(…, "pending")`.
Helios is a light client with **no pending block** → `"block not found: pending"`.
This rides the read/sync path (`verify_root` → eth_call), so v1 must pin eth_call
to `latest`:

```rust
// Deckard-side, one line — makes Kohaku's UNMODIFIED adapter work against Helios:
ProviderBuilder::new()
    .with_default_block(BlockId::latest())   // installs alloy's BlockIdLayer
    .connect(helios_localhost_url).await?
    .erased()                                // → DynProvider : IntoEip1193Provider
```

No Kohaku patch, no Helios patch. (The production `HeliosEip1193` adapter the
sidecar doc plans would set `latest` itself; this is the v1 shortcut.)

## Run

```bash
# Tier-2 (default, light): Helios localhost server + Kohaku's real eip-1193-provider
# adapter; drives eth_chainId/blockNumber/getLogs/eth_call through Helios + logs
# every JSON-RPC method via a pass-through proxy + measures the loopback hop.
cargo run                       # warm if a cached checkpoint exists, else cold
WIPE=1 cargo run                # force a COLD start

# Tier-1 (heavy): ALSO link Kohaku's full `railgun` ZK crate and drive the real
# RailgunBuilder::build() + RpcSyncer (eth_blockNumber + eth_getLogs) through Helios.
cargo run --features railgun
```

Exit 0 = PASS (every method the read path called is in Helios's served set).

| env | meaning | default |
|---|---|---|
| `EL` | untrusted execution RPC (must serve `eth_getProof`) | `https://ethereum-rpc.publicnode.com` |
| `CL` | beacon **light-client** API (200 ≠ syncs — see `20-helios-sidecar.md`) | `http://testing.mainnet.beacon-api.nimbus.team` |
| `CHECKPOINT` | pinned weak-subjectivity root (`0x..` B256) | community fallback if unset |
| `DATA_DIR` | FileDB dir (warm-start checkpoint cache) | `$TMPDIR/deckard-eip1193-railgun-spike` |
| `WIPE` | force a COLD start | unset |
| `WINDOW` | `eth_getLogs` window (#blocks back from head) | `2000` |

Measured (M-series, mainnet, dev build): cold sync ≈12s, warm ≈3.5s; 323 RAILGUN
events in a 2000-block window verified through Helios; loopback hop adds ≈1ms/call
over the in-process typed `HeliosApi` call.

## Why mainnet (not Sepolia)

Helios is proven to sync only on **mainnet** public CLs (Nimbus/dRPC); `Network::Sepolia`
has `consensus_rpc = None` (you must supply a Sepolia beacon LC endpoint). The RAILGUN
smart wallet is live on mainnet (`ChainConfig::mainnet()` → `0xFA7093…`), so the seam
is proven there. Upstream's shield **integration** tests fork **Sepolia** — that R1
shield→unshield test (the `10-kohaku-shield.md` job) is separate and needs a Sepolia LC
endpoint; this spike de-risks only the read/provider seam.

## Files

- `src/helios.rs` — build Helios with `.rpc_address()` localhost server; sync→servable.
- `src/proxy.rs` — method-logging HTTP pass-through (the Task-4 enumeration tap).
- `src/main.rs` — Tier-2: wire the adapter, drive the 3 reads, measure the hop.
- `src/railgun_tier1.rs` — Tier-1 (`--features railgun`): real `RailgunBuilder` + `RpcSyncer`.
