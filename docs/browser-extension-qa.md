# Browser extension QA

Deckard's browser connector is a local-only, experimental extension. This QA harness is a
small Playwright smoke suite, not a full end-to-end wallet test suite. It checks that the
unpacked extension loads in Chromium, injects the EIP-1193 provider, and can connect the local
test dapp through the Deckard browser bridge.

The suite follows Playwright's Chrome extension guidance: extensions run in Chromium through a
persistent browser context, loaded with `--disable-extensions-except` and `--load-extension`.
Manifest V3 uses a service worker, so the extension id is derived from the service worker URL.

## Install

Install the Node dependencies:

```sh
npm install
```

Install Playwright's bundled Chromium:

```sh
npm run qa:browser:install
```

Use the bundled Playwright Chromium for this harness. Do not switch it to system Chrome or Edge;
those browsers no longer support the extension sideload flags that this local QA path needs.

## Run

Run the extension smoke tests headless:

```sh
npm run qa:extension
```

Run headed while debugging:

```sh
npm run qa:extension:headed
```

The default test starts `deckard-browser-bridge` in dev/mock mode on `127.0.0.1:8765`, serves
`examples/browser-bridge-dapp` on `127.0.0.1:8777`, loads `extension/` unpacked, and verifies:

- the Manifest V3 service worker loads
- `window.ethereum` is injected into the local test dapp
- Deckard announces its provider through EIP-6963
- `eth_accounts` returns `[]` before permission
- `eth_requestAccounts` returns the deterministic dev/mock address
- `eth_accounts` returns that address after permission
- `eth_chainId` returns Sepolia (`0xaa36a7`)

Run the real-daemon proof:

```sh
npm run qa:extension:real
```

Run it headed while debugging:

```sh
npm run qa:extension:real:headed
```

The real-daemon suite does not use `--dev-mock-account`. It creates a temporary throwaway
vault with `deckard-core --example qa-vault`, starts `deckard-signerd`, unlocks it through
`deckard-signerd --example qa-unlock`, starts `deckard-browser-bridge`, then drives the same
local dapp through the unpacked extension. It verifies:

- `eth_accounts` returns `[]` before permission
- `eth_requestAccounts` returns the unlocked QA daemon address
  `0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266`
- `eth_accounts` returns that address after permission
- `eth_chainId` reflects `DECKARD_CHAIN_ID=31337` (`0x7a69`)

It also probes and documents three expected failures through the bridge endpoint:

- locked wallet -> EIP-1193 error code `4900` with locked-wallet guidance
- missing daemon socket -> EIP-1193 error code `4900` with daemon-not-running guidance
- wrong chain configuration -> EIP-1193 error code `4900` with chain-mismatch guidance

If you already have a compatible bridge running on `127.0.0.1:8765`, stop it before running the
suite. To intentionally reuse it instead, set:

```sh
DECKARD_QA_REUSE_BRIDGE=1 npm run qa:extension
```

## Artifacts

Generated artifacts are ignored by git:

- `playwright-report/` — HTML reports
- `test-results/` — screenshots, traces, and videos
- `.playwright/` — persistent Chromium profiles
- `blob-report/` — Playwright blob reports

The passing dapp-connection test writes `connected-dapp.png` under `test-results/extension/...`.
The real-daemon dapp-connection test writes `real-daemon-connected-dapp.png` under the same
tree.
On failure, Playwright retains traces, screenshots, and videos according to
`playwright.extension.config.ts`.

The real-daemon suite stores temporary vaults under `/tmp/deckard-extension-*` and removes them
on normal teardown. If a test is interrupted, remove stale `/tmp/deckard-extension-*` directories
manually.

## Security

- Never use or commit a real seed phrase, private key, production wallet, browser profile, or
  mainnet-funded account in this harness.
- The checked-in test uses only a deterministic mock address:
  `0xdec0ded000000000000000000000000000001193`.
- The real-daemon proof uses only the deterministic throwaway `qa-vault` address:
  `0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266`, unlocked with the fixed QA passphrase
  `deckard-qa`.
- The extension has no keys and performs no signing.
- Keep transaction and signing flows out of this harness until Deckard has reviewed approval UI
  and local-chain-only signing tests.
- Do not log extension storage, browser profile contents, cookies, local storage, seeds, keys, or
  wallet state.

## WalletBeat

Use the WalletBeat lane when you need to verify the public wallet-test UI against Deckard's
unpacked extension:

```sh
npm run qa:walletbeat
```

See `docs/walletbeat-qa.md` for the exact scope and artifact paths.

## Remote and browser friction

Playwright uses an isolated bundled Chromium profile, so normal wallet extensions such as
MetaMask or Rabby are absent by design. That makes the automated proof deterministic, but it does
not cover provider-selection behavior in a normal browser profile with multiple installed wallets.

When the bridge and dapp run over SSH on another machine, the browser must still be able to reach
both loopback ports. Forward both ports to the browser host, for example:

```sh
ssh -L 8765:127.0.0.1:8765 -L 8777:127.0.0.1:8777 <host>
```

## Scope

This suite proves the browser connector account/chain slice. It does not replace Rust
unit/integration tests or a future transaction/signature test suite on a local chain.
