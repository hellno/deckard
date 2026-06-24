# Issue #138 / #93 — Browser bridge `eth_sendTransaction` and WalletBeat transactions QA

## 1. Title

Expose a fail-closed `eth_sendTransaction` browser-bridge path and add local WalletBeat transaction QA.

## 2. Context

PR #139 exposed reviewed message signing over the browser bridge. The next EIP-1193 gap for dapp and WalletBeat transaction coverage is `eth_sendTransaction`.

This is security-sensitive wallet RPC work. The bridge must not become a signer or a policy engine. It should parse JSON-RPC frames, enforce origin/account/session checks, lower supported requests into Deckard `Intent`s, then let signerd own approval, signing, and broadcast.

## 3. Source Of Truth

- User instruction on 2026-06-24: mark #137 / PR #139 Done, then start #138 / #93.
- GitHub issue #138: `Browser bridge: implement eth_sendTransaction through clear-signing approval`.
- GitHub issue #93: `WalletBeat QA: add local-chain transaction and signature lane`.
- `AGENTS.md` and `PLANS.md`.
- `crates/deckard-browser-bridge/src/lib.rs`.
- `crates/deckard-contract/src/intent.rs`.
- `crates/deckard-signerd/src/daemon.rs`.
- `crates/deckard-signerd/src/client.rs`.
- `tests/extension/browser-bridge-extension.spec.ts`.
- WalletBeat local checkout under `.walletbeat/walletbeat` for transaction fixture shapes.

## 4. Current State Analysis

- The bridge supports discovery/account methods plus `personal_sign`, `eth_signTypedData_v4`, and explicit `eth_sign` refusal.
- `eth_sendTransaction` is still an unsupported method.
- The daemon already supports `SignerRequest::Propose { intent }` and `SignerRequest::Execute { request_id }` for `IntentKind::Send` and `IntentKind::Shield`.
- The daemon currently refuses ERC-20 `Send` (`token = Some`) and generic non-shaped contract calls; arbitrary calldata must remain fail-closed.
- Existing WalletBeat transaction fixtures focus on ERC-20 approve/transfer, Aave supply, and EIP-5792 multicall. Full pass requires future ERC-20/classification work. This PR should still add a local transaction QA lane that proves the newly supported safe subset and deterministic refusal of unsupported contract calls.

## 5. Target State

- `eth_sendTransaction` is recognized by the bridge.
- A connected origin can send a strictly parsed native send with empty calldata.
- The bridge rejects requests before signerd when:
  - the origin has not connected via `eth_requestAccounts`,
  - `from` does not match the connected account,
  - params are malformed,
  - `to` is absent or invalid,
  - calldata is non-empty / contract-call shaped,
  - token/contract calls are not yet supported.
- Production path routes through signerd approval/execution.
- Dev mock path returns a deterministic dummy transaction hash for QA only.
- WalletBeat transaction QA stores JSON and screenshot artifacts under ignored `test-results/` paths.

## 6. Security And Trust Invariants

- The browser bridge remains key-less; it never holds private keys or signs locally outside explicit dev mock mode.
- The bridge never broadcasts directly. In production, signerd remains the only component that signs and broadcasts.
- Origin/session/account checks are enforced before proposing a transaction.
- Unknown calldata and ERC-20 contract calls fail closed until clear-signing render/classifier support exists.
- No real seed, private key, production profile, or mainnet-funded wallet is used in QA.
- Dev mock transaction hashes are clearly test-only and never imply real broadcast.

## 7. UX And Design Constraints

No GPUI visual change is planned in this PR. The existing signerd pending card path will render native sends if production signerd approval is exercised. WalletBeat/browser QA uses the dev mock and does not capture app secrets.

## 8. Execution Plan

1. Add failing bridge unit tests for `eth_sendTransaction`:
   - before connect returns `4100`,
   - native send returns a dev tx hash after connection,
   - `from` mismatch returns `4100`,
   - non-empty calldata is refused deterministically,
   - malformed params are refused.
2. Implement minimal parser and lowering to `IntentKind::Send` for native sends only.
3. Add production helper that proposes the intent, waits for approval when needed, and executes via signerd.
4. Add dev mock transaction hash helper.
5. Extend extension allowlist and Playwright extension coverage.
6. Add `scripts/walletbeat-transactions-qa.mjs` and package scripts.
7. Run focused tests, WalletBeat lanes, and full DoD.
8. Commit, push, open PR closing #138 and partially advancing #93.

## 9. Validation Criteria

Required:

```text
cargo fmt --all --check
just check
cargo test --workspace
pnpm run qa:extension
pnpm run qa:walletbeat
pnpm run qa:walletbeat:signatures
pnpm run qa:walletbeat:transactions
```

Focused during development:

```text
cargo test -p deckard-browser-bridge send_transaction -- --nocapture
```

## 10. Failure Signals

- `eth_sendTransaction` accepts calldata before a renderer/classifier exists.
- A request with a mismatched `from` reaches signerd.
- Production bridge signs/broadcasts without signerd approval/execution.
- WalletBeat QA requires a real wallet profile or prints secrets.
- EIP-5792/batch behavior sneaks into this PR.

## 11. Risks And Tradeoffs

- This PR will not pass WalletBeat's ERC-20 transaction fixtures end-to-end yet; those require ERC-20 transaction classification/rendering and possibly token metadata. It should document that as the next #93/#138 follow-up rather than expanding scope into arbitrary contract calls.
- The dev mock hash proves provider integration but not real chain inclusion. Production correctness is covered by signerd integration already present for `IntentKind::Send` and by future local-chain daemon QA if needed.

## 12. Status

- [x] GitHub Project updated: #137 / PR #139 Done; #138 / #93 In Progress.
- [x] Branch created from merged `origin/main`.
- [x] Plan created.
- [x] RED bridge tests added and observed failing.
- [x] Bridge implementation.
- [x] WalletBeat transactions QA lane.
- [x] Full local DoD.
- [ ] PR opened and CI checked.
