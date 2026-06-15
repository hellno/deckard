# Railgun shield/swap — local testing (agentic + manual)

Native-ETH **Send** works against any local anvil chain. **Railgun shield/swap do not** — they call
the *real deployed Railgun contracts* and read the on-chain Merkle-tree state, which only exist on a
real network. So to exercise shield/swap locally you fork **Sepolia at the pinned block `10822990`**
(where those contracts live) and point the app at the fork.

## The only "blocker" is a Sepolia archive RPC — and it can be keyless

`anvil`'s `--fork-url` needs a Sepolia **archive** endpoint that can serve block `10822990`. It is
**not** read by any Deckard binary (custody never touches it). The zero-setup option needs no key:

```sh
export RPC_URL_SEPOLIA=https://sepolia.drpc.org     # keyless public endpoint (may rate-limit)
cast chain-id  --rpc-url "$RPC_URL_SEPOLIA"         # -> 11155111
cast block 10822990 --rpc-url "$RPC_URL_SEPOLIA" --field number   # -> 10822990 (proves archive depth)
```

Verified 2026-06-14: the keyless public endpoint serves the fork block and a full shield syncs/mines.
For heavy/automated use, a free **keyed** endpoint (Alchemy/Infura/dRPC) is steadier.

## Path A — the human demo: `just demo`

```sh
export RPC_URL_SEPOLIA=https://sepolia.drpc.org
just demo          # anvil fork of Sepolia + app + daemon (uses ~/.deckard/demo; Ctrl-C tears anvil down)
just demo-fund     # terminal 2: anvil_setBalance 10 ETH onto the onboarded wallet
just demo-check    # doctor: foundry, RPC, fork, signerd build, app unlocked on the right chain
```

## Path B — agentic / clicky QA: a fresh throwaway config against the fork

`just demo` reuses `~/.deckard/demo` (which may hold a vault whose passphrase you don't know). For an
isolated, repeatable run, fork by hand into a fresh `DECKARD_CONFIG_DIR`:

```sh
# 1. fork Sepolia at the Railgun block (chain id 11155111 is preserved — do NOT pass --chain-id)
anvil --fork-url https://sepolia.drpc.org --fork-block-number 10822990 --port 8545 --silent &

# 2. launch the app against the fork in a throwaway config dir
CFG=$(mktemp -d /tmp/deckard-rail-XXXX)
cargo build -p deckard-signerd
DECKARD_CONFIG_DIR="$CFG" DECKARD_SOCKET_PATH="$CFG/signerd.sock" \
DECKARD_CHAIN_ID=11155111 DECKARD_RPC_URL=http://127.0.0.1:8545 \
DECKARD_VERIFIED_READS=0 DECKARD_DEMO_FORK_BLOCK=10822990 \
DECKARD_SIGNERD_BIN="$PWD/target/debug/deckard-signerd" \
  cargo run

# 3. in the app: onboard a throwaway wallet, then fund the onboarded address:
ADDR=$(DECKARD_CONFIG_DIR="$CFG" DECKARD_SOCKET_PATH="$CFG/signerd.sock" \
       DECKARD_CHAIN_ID=11155111 DECKARD_RPC_URL=http://127.0.0.1:8545 \
       cargo run -q -p deckard-mcp -- address | jq -r .address)
cast rpc anvil_setBalance "$ADDR" "$(cast to-hex "$(cast to-wei 10 ether)")" --rpc-url http://127.0.0.1:8545
```

## Shield walkthrough (what "passing" looks like)

1. Portfolio after funding: **Public 100% · Private 0%**, 10 ETH.
2. **Shield** → compose. The recipient is **pre-filled with the wallet's own `0zk` address** (this
   alone exercises the Railgun SLIP-0010/babyjubjub key derivation).
3. Review card (clear-signing): amount, **Railgun fee 0.25%**, "you'll receive (private)" = amount × 0.9975.
4. **Hold-to-shield** (amber fill) → deposit broadcast → mined (fork block advances by 1); the status
   strip flips to a green "Private. Spendable now."
5. **Refresh the Portfolio** — the split bar then flips **public → private** (e.g. 0.1 shielded →
   Private 0.0997 / Public 9.8999). The balance does **not** update until Refresh: that resync-on-refresh
   is the behavior PR #30 added (`refresh_portfolio` also calls the shielded handle's `resync`).

## Gotchas

- `DECKARD_VERIFIED_READS=0` for the demo/fork — Helios verified reads are mainnet-only.
- Restart anvil per run; the in-memory Railgun DB re-syncs from the fork each launch (~10 s). See #12
  for persisting the Railgun DB (cold sync ~11 min/launch otherwise).
- POI (proof-of-innocence) is `None` on a fork — expected; shielding still works.
- Driving the GPUI app for clicky QA: see [`headless-gui-screenshots.md`](headless-gui-screenshots.md)
  (Linux) and the macOS recipe in the team memory; never let a subagent run `cargo` on the app (cold
  builds stall watchdogs).
</content>
