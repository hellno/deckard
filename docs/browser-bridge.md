# Experimental browser bridge (EIP-1193 vertical slice)

> Experimental. Not audited. Not for real funds or real mainnet keys.

This milestone proves one narrow path:

```text
local test dapp
  -> injected `window.ethereum` EIP-1193 provider
  -> unpacked browser extension
  -> localhost Deckard bridge endpoint on 127.0.0.1
  -> Deckard sidecar/session handling
  -> selected account/address returned to the dapp
```

It does **not** ship a production wallet extension. The extension contains no keys, seed phrases,
signing logic, or durable wallet state. It only forwards a tiny allowlist of methods to a local
Deckard process.

## Repo map

- Desktop app / UI: `crates/deckard-app` (`deckard` GPUI binary).
- Existing local daemon / signer API: `crates/deckard-signerd`, over a same-uid Unix-domain socket.
- Key-less sidecar / API surface: `crates/deckard-mcp`; this milestone adds the experimental
  `deckard-mcp browser-bridge` localhost endpoint here instead of creating a competing daemon.
- Existing account/address state: the unlocked signer daemon answers `SignerRequest::Address`, surfaced
  by `Sidecar::wallet_address()`.
- Browser connector scaffold: `extension/`.
- Local test dapp: `examples/browser-bridge-dapp/index.html`.
- Tests: `crates/deckard-mcp/src/browser_bridge.rs`.

## Supported methods

- `eth_chainId` -> returns the configured `DECKARD_CHAIN_ID` as hex.
- `eth_accounts` -> returns the account only after this origin has an active in-memory dapp session.
- `eth_requestAccounts` -> asks Deckard for the current unlocked address (or returns a dev mock address)
  and grants this origin an in-memory session.

Unsupported methods return an EIP-1193-style error object with code `4200`.

`personal_sign`, `eth_sendTransaction`, broad signing, hardware wallets, Kohaku, native messaging, and
store distribution are intentionally not implemented in this PR.

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
cargo run -p deckard-mcp -- browser-bridge \
  --bind 127.0.0.1:8765 \
  --dev-mock-account 0xdeC0ded0000000000000000000000000000001193
```

Alternatively, the same dev mock can be supplied through the environment:

```sh
export DECKARD_BRIDGE_DEV_ACCOUNT=0xdeC0ded0000000000000000000000000000001193
cargo run -p deckard-mcp -- browser-bridge --bind 127.0.0.1:8765
```

## Run against local Deckard

In one terminal, run Deckard's normal demo stack and unlock a throwaway wallet:

```sh
export RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/<your-key>
just demo
```

In another terminal, run the bridge on loopback:

```sh
export DECKARD_CHAIN_ID=11155111
cargo run -p deckard-mcp -- browser-bridge --bind 127.0.0.1:8765
```

The bridge uses the existing Deckard socket path (`DECKARD_SOCKET_PATH` or the default) and calls the
same key-less sidecar path as `deckard-mcp address`.

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
- Add EIP-6963 provider announcement.
- Persist permissions safely.
- Add clear-signing/message-signing only after Deckard has the reviewed intent model for it.
