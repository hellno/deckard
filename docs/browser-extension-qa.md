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

The test starts `deckard-browser-bridge` in dev/mock mode on `127.0.0.1:8765`, serves
`examples/browser-bridge-dapp` on `127.0.0.1:8777`, loads `extension/` unpacked, and verifies:

- the Manifest V3 service worker loads
- `window.ethereum` is injected into the local test dapp
- `eth_accounts` returns `[]` before permission
- `eth_requestAccounts` returns the deterministic dev/mock address
- `eth_accounts` returns that address after permission
- `eth_chainId` returns Sepolia (`0xaa36a7`)

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
On failure, Playwright retains traces, screenshots, and videos according to
`playwright.extension.config.ts`.

## Security

- Never use or commit a real seed phrase, private key, production wallet, browser profile, or
  mainnet-funded account in this harness.
- The checked-in test uses only a deterministic mock address:
  `0xdeC0ded0000000000000000000000000000001193`.
- The extension has no keys and performs no signing.
- Keep transaction and signing flows out of this harness until Deckard has reviewed approval UI
  and local-chain-only signing tests.
- Do not log extension storage, browser profile contents, cookies, local storage, seeds, keys, or
  wallet state.

## Scope

This suite proves the browser connector slice. It does not replace Rust unit/integration tests,
the real daemon proof for `deckard-signerd`, or a future transaction/signature test suite on a
local chain.
