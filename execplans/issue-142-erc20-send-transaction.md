# Issue #142 — ERC-20 `eth_sendTransaction` classifier

## 1. Title

Support narrow ERC-20 `transfer(address,uint256)` and `approve(address,uint256)` browser-bridge transaction shapes.

## 2. Context

PR #141 added native `eth_sendTransaction` through the browser bridge and intentionally refused non-empty calldata. The next WalletBeat/dapp compatibility step is to admit only the first two ERC-20 calldata shapes that can be reviewed plainly:

- `transfer(address,uint256)` selector `0xa9059cbb`
- `approve(address,uint256)` selector `0x095ea7b3`

This remains security-sensitive wallet RPC work. The bridge must parse and classify but must not sign, broadcast, or silently pass arbitrary calldata. Production signing/broadcast remains in `deckard-signerd`.

## 3. Source Of Truth

- Issue #142: browser bridge ERC-20 transfer/approve classifier.
- Issue #93: WalletBeat local-chain transaction/signature QA lane.
- `crates/deckard-browser-bridge/src/lib.rs`.
- `crates/deckard-contract/src/intent.rs` and `src/policy.rs`.
- `crates/deckard-signerd/src/daemon.rs`.
- `scripts/walletbeat-transactions-qa.mjs`.
- `tests/extension/browser-bridge-extension.spec.ts`.

## 4. Current State

- Native sends lower to `IntentKind::Send { token: None, calldata: empty }`.
- Non-empty browser calldata is refused before signerd.
- `Intent` already has `token: Option<Address>` for ERC-20 sends.
- `deckard-signerd` currently denies `token.is_some()` sends with `erc20_unsupported_v1`.
- The daemon already has a structured `PendingPayloadView::Approve` for exact approve calldata.
- Existing app rendering can show token sends generically as `tokens` and approvals as structured approve cards.

## 5. Target State

- Browser bridge decodes ERC-20 `transfer` calldata and lowers it to:
  - `IntentKind::Send`
  - `to = transfer recipient`
  - `token = Some(token contract)`
  - `value = token amount`
  - `calldata = empty`
- Browser bridge decodes ERC-20 `approve` calldata and lowers it to:
  - `IntentKind::ContractCall`
  - `to = token contract`
  - `token = None`
  - `value = 0`
  - `calldata = original exact approve calldata`
- Browser bridge refuses:
  - malformed calldata length/ABI words,
  - unknown selectors,
  - non-zero native value with ERC-20 calldata,
  - missing/mismatched `from`,
  - Aave/Safe/multisend/EIP-5792/arbitrary calldata.
- signerd admits ERC-20 token sends and generic exact approvals only as human-review transactions, never auto-allow.
- Execution for ERC-20 token sends broadcasts `transfer(recipient, amount)` to the token contract with `msg.value = 0`.
- Execution for approvals broadcasts the exact approve calldata to the token contract with `msg.value = 0`.

## 6. Security Invariants

- The bridge remains key-less.
- Unknown calldata remains fail-closed.
- ERC-20 classifier admission is exact selector + exact ABI length only.
- ERC-20 transactions always require a human card; token amounts are not compared to ETH caps.
- No native ETH is sent alongside ERC-20 transfer/approve calldata.
- Production signing and broadcast stay exclusively in signerd.

## 7. TDD Plan

1. Add failing browser-bridge tests:
   - ERC-20 transfer calldata returns a dev tx hash.
   - ERC-20 approve calldata returns a dev tx hash.
   - unknown selector stays refused.
   - malformed ERC-20 calldata stays refused.
   - ERC-20 calldata with native value is refused.
2. Add/adjust signerd tests for token send / generic approve admission and broadcast shaping if necessary.
3. Implement bridge calldata classifier.
4. Implement signerd token-send broadcast shaping and generic approve admission.
5. Extend WalletBeat transaction QA to exercise native send + ERC-20 transfer + ERC-20 approve.
6. Run focused tests, QA lanes, and full DoD.

## 8. Validation

Focused:

```text
cargo test -p deckard-browser-bridge erc20 -- --nocapture
cargo test -p deckard-signerd erc20 -- --nocapture
pnpm run qa:walletbeat:transactions
```

Full DoD:

```text
cargo fmt --all --check
just check
cargo test --workspace
pnpm run qa:extension
pnpm run qa:walletbeat
pnpm run qa:walletbeat:signatures
pnpm run qa:walletbeat:transactions
git diff --check
```

## 9. Status

- [x] Issue #142 created and added to the GitHub Project.
- [x] Branch created from merged `origin/main`.
- [x] Plan created.
- [x] RED tests added and observed failing.
- [x] Bridge classifier implemented.
- [x] signerd execution/admission implemented.
- [x] WalletBeat transaction QA extended.
- [x] Full local DoD.
- [ ] PR opened and CI checked.
