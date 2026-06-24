# WalletBeat QA

Deckard tracks WalletBeat as a compatibility target, but the local automated lane is deliberately
small. It runs WalletBeat's public test page against the unpacked Deckard extension and verifies
the safe provider/account/network steps before any signing or transaction flows exist.

## Run

Install Node dependencies and Playwright Chromium:

```sh
npm install
npm run qa:browser:install
```

Run the WalletBeat safe provider lane:

```sh
npm run qa:walletbeat
```

Run headed while debugging:

```sh
npm run qa:walletbeat:headed
```

Validate the scam-alert / transaction-simulation safety matrix:

```sh
npm run qa:walletbeat:safety
```

The script:

- clones a pinned WalletBeat beta checkout into `.walletbeat/`
- installs WalletBeat dependencies with `pnpm install --frozen-lockfile`
- starts `deckard-browser-bridge` in dev/mock mode on `127.0.0.1:8765`
- starts WalletBeat on `127.0.0.1:8788`
- loads `extension/` unpacked in Playwright's bundled Chromium
- opens `/test/`, selects `EIP Support`, and runs the first four steps

Those four steps cover EIP-6963 discovery, EIP-1193 request/account/network behavior, and the
EventEmitter surface used by WalletBeat's EIP-2700 checks.

## Artifacts

The run writes:

- `test-results/walletbeat/walletbeat-safe-provider-results.json`
- `test-results/walletbeat/walletbeat-safe-provider.png`

The JSON report records the pinned WalletBeat ref, the local account, the chain id, each checked
step, and any browser console/page errors.

## Configuration

Useful overrides:

- `DECKARD_WALLETBEAT_REF` — WalletBeat git ref to test, default pinned to the beta snapshot used
  when this harness was added
- `DECKARD_WALLETBEAT_PORT` — WalletBeat dev server port, default `8788`
- `DECKARD_WALLETBEAT_BRIDGE_PORT` — Deckard browser bridge port, default `8765`
- `DECKARD_WALLETBEAT_HEADED=1` — run Chromium headed
- `DECKARD_QA_REUSE_BRIDGE=1` — reuse an already-running compatible bridge

## Scope

The default safe provider lane is not a full WalletBeat pass. Transaction, signature, and batch-call
coverage live in dedicated lanes. Scam-alert and transaction-simulation coverage is tracked by
`docs/WALLETBEAT-SAFETY-MATRIX.md`; that matrix is deliberately not a claim of full support. It
forces every WalletBeat safety fixture to be classified as supported, safely refused, or linked to a
follow-up issue before #95 can close.

Tracked follow-up work:

- #93 — local-chain transaction and signature lane
- #94 — EIP-5792 batch-call support
- #95 — scam-alert and transaction-simulation coverage

Never use a real seed phrase, private key, production wallet, browser profile, or mainnet-funded
account for WalletBeat QA. Signing and transaction work should use deterministic unsafe test
mnemonics, a throwaway encrypted QA vault, and a local chain such as Anvil or Hardhat.
