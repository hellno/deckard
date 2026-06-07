# helios-walkaway — Deckard R2 spike

Proves the **walkaway beat** on Ethereum **mainnet**, headless: embed Helios as a
library, sync a verified light client, serve a verified `eth_getBalance`, then
**cut the centralized EL RPC on camera and keep serving verified reads** by failing
over to an independent second EL. The head never freezes (it's consensus-driven);
only state reads drop and recover.

It also **measures** the two numbers the demo timing hinges on: cold vs warm sync,
and cut→failover latency.

## Why this is honest

Verified against `a16z/helios @ 0.11.1` source (not docs):

- Helios has **no native multi-EL/CL failover** — one `EthereumClient` = one EL +
  one CL. The failover here is **Deckard's own supervisor** (`src/upstreams.rs`).
- The consensus client pushes each sync-committee-verified execution header into
  the execution provider's cache, so **`get_block_number`/head is EL-independent**
  (served from cache). Only `get_balance`→`eth_getProof` hits the EL. That's *why*
  cutting EL1 leaves the head live and a second already-synced client recovers
  state reads instantly — both clients re-verify against the same CL + checkpoint,
  so failover is honest re-derivation, never a cached stale value.

## Run

Defaults are the two keyless, no-log public ELs (`publicnode` proxied/cuttable +
`drpc` failover) and the Nimbus light-client beacon API — which is exactly the
privacy-correct demo posture (no IP↔address-correlating vendor in the read path):

```bash
# Availability (cut-the-RPC): sync, cut the primary EL, keep serving verified reads
cargo run --release                 # warm if a cached checkpoint exists, else cold
WIPE=1 cargo run --release          # force a COLD start (wipe cached checkpoint)

# Integrity (the moat): point Helios at a MALICIOUS RPC, watch it refuse the lie
SCENARIO=lie WIPE=1 cargo run --release
#   malicious RPC claims 1,000,000,000 ETH → Deckard: REJECTED (invalid account proof)
#   (a centralized wallet would display the billion; Deckard verifies the proof)
```

Measured on this M-series desktop (mainnet, 2026-06-05): **cold ≈ 11s, warm ≈ 2s**,
cut→failover ≈ one block (≈2–15s, gated by the per-block proof cache; the supervisor
mechanism itself is ~250–500ms). Verified deposit-contract balance ≈ 86,313,877 ETH.

> ⚠ **CL choice matters — 200 ≠ syncs.** `CL` must serve the `light_client/*` routes
> **and** full `/eth/v2/beacon/blocks/{slot}` blocks whose `tree_hash_root` matches the
> verified header. Verified against Helios sync (2026-06-05):
> - ✅ **Nimbus** `http://testing.mainnet.beacon-api.nimbus.team` (~11 s) — HTTP, no-SLA team box
> - ✅ **dRPC** `https://eth-beacon-chain.drpc.org` (~10.4 s) — HTTPS, keyless (the proven second CL)
> - ❌ **Lodestar** `lodestar-mainnet.chainsafe.io` — 200 but head stuck at timestamp 0
> - ❌ **PublicNode** `ethereum-beacon-api.publicnode.com` — 200 but `invalid sync committee period`

> ⚠ **Key-restricted ELs don't work through the proxy.** An Alchemy/Infura key with
> an origin/IP allowlist returns `-32600 "origin not on whitelist"` for proxied (and
> off-allowlist) requests. Use keyless no-log providers, or a key allowed for your IP.

Env:

| var          | meaning                                    | default |
|--------------|--------------------------------------------|---------|
| `EL1`        | primary EL (proxied + cuttable)            | `https://ethereum-rpc.publicnode.com` |
| `EL2`        | independent failover EL                     | `https://eth.drpc.org` |
| `CL`         | beacon **light-client API** endpoint        | `http://testing.mainnet.beacon-api.nimbus.team` |
| `CHECKPOINT` | pinned weak-subjectivity root (`0x..` B256) | community fallback (ethPandaOps) if unset |
| `DATA_DIR`   | FileDB dir (warm-start cache lives here)    | `$TMPDIR/deckard-helios-spike` |
| `WIPE`       | set to force a COLD start                   | unset |
| `ADDR`       | address to read                             | deposit contract |

Exit code 0 = PASS (`ReadStatus` went `Verified → Degraded{failover}` and the
post-cut balance is still verified).

## Files

- `read_status.rs` — `ReadStatus { Verified | Degraded | Unsynced }` (Deckard-owned).
- `upstreams.rs`   — the failover supervisor (Shape A).
- `proxy.rs`       — a killable HTTP/1.1 reverse proxy = the on-camera "cut".
- `main.rs`        — the scenario + measurements.
