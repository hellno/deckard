# Issue #94 — EIP-5792 wallet call API

## 1. Title

Implement a narrow, fail-closed EIP-5792 browser-bridge path for local-chain WalletBeat QA.

## 2. Context

Issue #94 asks Deckard to support the EIP-5792 wallet call API methods needed by WalletBeat after the provider/account/signature/transaction lanes are in place:

- `wallet_getCapabilities`
- `wallet_sendCalls`
- `wallet_getCallsStatus`
- optional `wallet_showCallsStatus`

Deckard now supports the clear-signable transaction primitives needed for a first safe batch lane:

- native ETH send
- ERC-20 `transfer(address,uint256)`
- ERC-20 `approve(address,uint256)`

PR #147 added a signerd-backed local-chain WalletBeat QA lane, so this PR can exercise EIP-5792 through the real daemon approval path without production profiles or real funds.

## 3. Security/product decision

First implementation is **local-chain-compatible but not smart-account atomic**:

- `wallet_sendCalls` accepts `atomicRequired: false` only.
- `atomicRequired: true` is refused until Deckard has an atomic execution account/path and clear UI.
- Calls execute sequentially through existing `eth_sendTransaction` classification/signerd approval machinery.
- Unsupported capabilities are refused unless marked `optional: true`.
- Unknown calldata and unsupported transaction shapes remain fail-closed.
- EIP-7702 authorization payloads are refused/gated, not silently ignored.

This is an EIP-5792 compatibility bridge for clear-signable calls, not a claim that Deckard can perform atomic multi-call execution.

## 4. Goals

- Add `wallet_getCapabilities` with per-chain support for `wallet_sendCalls` v2.0.0 and non-atomic execution only.
- Add `wallet_sendCalls` parser for v2.0.0 `{ version, from, chainId, atomicRequired, calls }`.
- Convert each call into the same internal `Intent` path used by `eth_sendTransaction`.
- Execute accepted calls sequentially through the existing backend/signerd path.
- Store a bridge-local batch record keyed by returned id.
- Add `wallet_getCallsStatus` returning v2.0.0 status with `atomic: false` and transaction-hash receipts.
- Keep `wallet_showCallsStatus` optional/refused for now.
- Update the extension allowlist.
- Extend local-chain WalletBeat QA to cover the EIP-5792 methods.

## 5. Non-goals

- Atomic batching.
- Smart account execution.
- Paymaster capabilities.
- EIP-7702 authorization support.
- Arbitrary calldata / Aave / Safe / multisend support.
- Mainnet-funded or production-profile testing.

## 6. TDD plan

1. RED: add browser-bridge tests for capabilities, sendCalls success, atomicRequired refusal, and getCallsStatus.
2. GREEN: implement bridge-local EIP-5792 model + sequential execution.
3. RED/GREEN: update extension and QA script to exercise the new methods.
4. REFACTOR: keep the transaction classifier shared with `eth_sendTransaction`.

## 7. Verification

- `cargo test -p deckard-browser-bridge eip5792 -- --nocapture`
- `pnpm run qa:extension`
- `pnpm run qa:walletbeat:local-chain`
- Full DoD before PR:
  - `cargo fmt --all --check`
  - `just check`
  - `cargo test --workspace`
  - `pnpm run qa:extension`
  - `pnpm run qa:extension:real`
  - `pnpm run qa:walletbeat`
  - `pnpm run qa:walletbeat:signatures`
  - `pnpm run qa:walletbeat:transactions`
  - `pnpm run qa:walletbeat:local-chain`
  - `git diff --check`

## 8. Status

- [x] Branch created from merged `origin/main`.
- [x] Issue #94 and current bridge/signerd transaction code read.
- [x] Plan created.
- [x] RED tests observed.
- [x] Bridge implementation complete.
- [x] QA lane extended.
- [x] Full local DoD.
- [ ] PR opened and CI checked.
