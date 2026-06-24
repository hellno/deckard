# Issue #137 — Browser bridge reviewed message signing

## 1. Title

Expose reviewed message signing over EIP-1193 without making the bridge a signer.

## 2. Context

Issue #46 added first-class message-signing intents to `deckard-contract` and `deckard-signerd`, including policy gates for `personal_sign`, reviewed EIP-712 typed data, raw `eth_sign` refusal, and EIP-7702 refusal. The browser bridge still only supports account/network methods, so WalletBeat's Signatures tab and dapp signature flows cannot exercise the reviewed path.

## 3. Goals

- Add `personal_sign` support to `crates/deckard-browser-bridge`.
- Add `eth_signTypedData_v4` support with strict EIP-712 JSON parsing and digest computation.
- Refuse `eth_sign` deterministically without creating an approval card.
- Keep signing authority in signerd; the bridge parses and routes only.
- Add a WalletBeat Signatures QA lane for local/dev mode.

## 4. Non-goals

- `eth_sendTransaction` (#138).
- EIP-5792 batch calls (#94).
- Real-mainnet WalletBeat runs.
- Raw `eth_sign` approval UI.

## 5. Acceptance criteria

- `personal_sign` can parse common param orderings and produces a 65-byte hex signature after approval/signing.
- `eth_signTypedData_v4` parses WalletBeat-style typed data, computes an EIP-712 digest, and produces a 65-byte hex signature after approval/signing.
- Wrong-account, malformed params, missing session, denied/expired/revoked, and raw `eth_sign` fail closed with EIP-1193-style JSON-RPC errors.
- Local WalletBeat signature QA covers simple message, SIWE, and typed-data signatures in dev mode.
- `cargo fmt --all --check`, `just check`, and `cargo test --workspace` pass before PR.

## 6. Test plan

1. Browser-bridge unit tests for param parsing and account/session checks.
2. Browser-bridge tests for dev-mode `personal_sign`, `eth_signTypedData_v4`, and `eth_sign` refusal.
3. Existing signerd message-signing E2E tests remain the real approval/signing coverage.
4. WalletBeat signature QA script runs against the dev bridge and stores JSON/screenshot artifacts.

## 7. Safety notes

- Dev bridge signatures are QA-only and must be deterministic dummy signatures; the bridge still holds no real key in dev mock mode.
- Production bridge requests must route to `WalletClient` / signerd and never sign locally.
- `eth_sign` stays refused because it has no clear-signable semantics.

## 8. Status

- [x] Repo issue created: #137.
- [x] GitHub Project item marked In Progress.
- [x] Parser tests.
- [x] Bridge implementation.
- [x] WalletBeat signature QA lane.
- [ ] Full DoD + PR.
