# Experimental browser bridge (EIP-1193 vertical slice)

> Experimental. Not audited. Not for real funds or real mainnet keys.

This milestone proves one narrow path:

```text
local test dapp
  -> injected `window.ethereum` EIP-1193 provider
  -> unpacked browser extension
  -> localhost Deckard bridge endpoint on 127.0.0.1
  -> shared key-less wallet client/session primitives
  -> deckard-signerd
  -> selected account/address returned to the dapp
```

It does **not** ship a production wallet extension. The extension contains no keys, seed phrases,
signing logic, or durable wallet state. It only forwards a tiny allowlist of methods to a local
Deckard process.

## Architecture

The browser bridge is a dapp/browser interface. It is intentionally separate from `deckard-mcp`,
which is an MCP/agent interface. Both are key-less local clients over shared wallet capabilities:

```text
deckard-signerd
      ↑
crates/deckard-wallet-client
      ↑                         ↑
crates/deckard-mcp      crates/deckard-browser-bridge
                                ↑
                            extension/
```

`deckard-wallet-client` owns reusable non-browser-specific pieces:

- signer-daemon client access (`SignerClient` wiring)
- chain id configuration (`DECKARD_CHAIN_ID`, default `1`)
- wallet/account address lookup
- common failure mapping for daemon denies and socket errors

`deckard-browser-bridge` owns browser/dapp-specific pieces:

- the loopback `/rpc` HTTP endpoint
- EIP-1193 request/response types
- EIP-1193 error mapping for unsupported methods
- per-origin in-memory dapp sessions
- dev/mock account mode for local extension and dapp testing

`deckard-mcp` stays focused on MCP/agent interaction and reuses the same wallet client primitives for
its CLI/tools. It does not own or serve the browser bridge.

## Repo map

- Desktop app / UI: `crates/deckard-app` (`deckard` GPUI binary).
- Local signer daemon / signer API: `crates/deckard-signerd`, over a same-uid Unix-domain socket.
- Shared key-less wallet client primitives: `crates/deckard-wallet-client`.
- Agent/MCP interface: `crates/deckard-mcp`.
- Browser/dapp interface: `crates/deckard-browser-bridge` (`deckard-browser-bridge` binary).
- Browser connector scaffold: `extension/`.
- Local test dapp: `examples/browser-bridge-dapp/index.html`.
- Browser bridge tests: `crates/deckard-browser-bridge/src/lib.rs`.

## Supported methods

- `eth_chainId` -> returns the configured `DECKARD_CHAIN_ID` as hex.
- `eth_accounts` -> returns the account only after this origin has an active in-memory dapp session.
- `eth_requestAccounts` -> asks Deckard for the current unlocked address (or returns a dev mock address)
  and grants this origin an in-memory session.

Unsupported methods return an EIP-1193-style error object with code `4200`.

The injected provider also exposes `isConnected()`, `on`, and `removeListener` with a minimal event
surface for `accountsChanged`, `chainChanged`, `connect`, and `disconnect`.

## Provider discovery

The injected provider supports EIP-6963 multi-wallet discovery in addition to the legacy
`window.ethereum` path:

- announces `eip6963:announceProvider` on page load
- re-announces when the dapp dispatches `eip6963:requestProvider`
- uses `name: "Deckard"`, `rdns: "com.deckard.wallet"`, and a self-contained SVG data URI icon

`personal_sign`, `eth_sendTransaction`, broad signing, hardware wallets, native messaging, and store
distribution are intentionally not implemented in this bridge slice. Kohaku remains useful for
wallet-internal provider/backend integration, not as the browser-facing dapp provider.

## Dapp sessions

The bridge stores sessions in memory only, keyed by origin:

- `origin`
- `chain_id`
- `account`
- `permissions`
- `created_at`
- `last_seen`
- `revoked`

This is deliberately minimal. Restarting the bridge clears sessions. Future work should move this into a
reviewed permissions registry with explicit approval UI, anti-phishing copy, revocation UX, and persistence.

## Run in dev/mock mode

This is the smallest way to test the browser bridge without an unlocked wallet:

```sh
cargo run -p deckard-browser-bridge -- \
  --bind 127.0.0.1:8765 \
  --dev-mock-account 0xdeC0ded0000000000000000000000000000001193
```

Alternatively, the same dev mock can be supplied through the environment:

```sh
export DECKARD_BRIDGE_DEV_ACCOUNT=0xdeC0ded0000000000000000000000000000001193
cargo run -p deckard-browser-bridge -- --bind 127.0.0.1:8765
```

## Run against local Deckard

For the automated real-daemon proof, run:

```sh
npm run qa:extension:real
```

That command creates a temporary throwaway QA vault, starts `deckard-signerd`, unlocks it with
the fixed QA passphrase, starts the browser bridge without `--dev-mock-account`, loads the
unpacked extension in Playwright Chromium, and verifies the local dapp account/chain flow. The
expected address is `0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266`; the expected chain id is
`0x7a69` from `DECKARD_CHAIN_ID=31337`.

The same suite records the expected local failure behavior:

- locked wallet -> `eth_requestAccounts` rejects with code `4900` and locked-wallet guidance
- missing daemon socket -> code `4900` and daemon-not-running guidance
- wrong chain configuration -> code `4900` and chain-mismatch guidance

In one terminal, run Deckard's normal demo stack and unlock a throwaway wallet:

```sh
export RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/<your-key>
just demo
```

In another terminal, run the bridge against the demo app's daemon:

```sh
just demo-bridge
```

`demo-bridge` reuses the demo world's `DECKARD_SOCKET_PATH` and `DECKARD_CHAIN_ID` — the same values
`just demo` launches the app with — so the bridge always dials the daemon the app actually spawned
(`~/.deckard/demo/signerd.sock`) and can't silently fall back to the per-uid default socket. For the
fast-unlock QA world use `just qa-bridge` instead (pair it with `just qa`, unlocked with the
passphrase `deckard-qa`).

The bridge calls the same shared key-less wallet client path that `deckard-mcp address` uses, and now
logs which socket it dials at startup. If you run it by hand instead of via the recipe, you **must**
set `DECKARD_SOCKET_PATH` (and `DECKARD_CHAIN_ID`) to match the launcher — otherwise it resolves the
per-uid default socket, which `just demo`/`just qa` never use, and reports a misleading "daemon is not
running" error even though the daemon is up on the world socket.

## Load the unpacked extension

Chromium/Chrome/Brave:

1. Open `chrome://extensions` (or `brave://extensions`).
2. Enable **Developer mode**.
3. Click **Load unpacked**.
4. Select the repository's `extension/` directory.

The extension injects `window.ethereum` and forwards only `eth_chainId`, `eth_accounts`, and
`eth_requestAccounts` to `http://127.0.0.1:8765/rpc`.

## Open the local test dapp

Serve the test page from localhost so it has a stable origin:

```sh
python3 -m http.server 8777 --directory examples/browser-bridge-dapp
```

Open <http://127.0.0.1:8777/> in the browser where the unpacked extension is loaded. Then click:

1. **eth_requestAccounts** -> should show the Deckard/mock address.
2. **eth_chainId** -> should show the chain id, for example `0xaa36a7` for Sepolia.

## Security notes

- The bridge binds to loopback only (`127.0.0.1` / `localhost`) and rejects non-loopback Host headers.
- The extension has no keys and performs no signing.
- The dapp origin is sent to the bridge and bound to an in-memory session before `eth_accounts` returns
  anything.
- CORS is permissive in this milestone because the extension is the intended caller and the API supports
  only address disclosure in dev/local mode. A production bridge needs explicit origin allowlisting,
  CSRF/rebinding hardening, a stronger browser-to-native transport decision, and user-visible approval.
- Use throwaway wallets only. This bridge is not for real funds.

## Follow-up work

- Decide native messaging vs hardened localhost using the PRD-04 spike evidence.
- Add a real approval UI for `eth_requestAccounts` and per-origin revocation.
- Persist permissions safely.
- Consider a small integration test for the loopback `/rpc` HTTP boundary.
- Add clear-signing/message-signing only after Deckard has the reviewed intent model for it.
