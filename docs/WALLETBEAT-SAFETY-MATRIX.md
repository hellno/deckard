# WalletBeat safety matrix

This document is the tracking gate for WalletBeat scam-alert and transaction-simulation coverage.
It exists to prevent Deckard from treating safe refusal as full scam/simulation support.

Status values:

- **supported** — Deckard has positive behavior for the fixture.
- **safe refusal** — Deckard intentionally refuses the request and does not sign/broadcast it.
- **tracked gap** — WalletBeat expects positive safety behavior that Deckard does not yet implement; a linked issue owns the gap.
- **blocked** — depends on a larger architectural prerequisite.

Closure rule for #95: every fixture in the pinned WalletBeat checkout must appear below and every
`tracked gap` / `blocked` row must link to a GitHub issue.

## Current feature boundary

Deckard currently provides:

- reviewed `personal_sign` and `eth_signTypedData_v4`
- explicit raw `eth_sign` refusal
- reviewed native ETH send
- reviewed ERC-20 `transfer(address,uint256)` and `approve(address,uint256)` classification
- non-atomic EIP-5792 batches made only from current clear-signable transaction shapes
- fail-closed refusal for unknown/arbitrary calldata

Deckard does **not** yet provide:

- address reputation or known-scam lists
- recent-contract or previous-interaction context
- first-class infinite-approval / Permit risk warnings
- revm-style transaction simulation
- token/NFT asset-delta rendering
- misleading-selector / fake-airdrop / volatile-outcome / guaranteed-revert detection

## Scam-alert fixtures

Source: `.walletbeat/walletbeat/src/constants/test-scam-alerts.ts`.

| WalletBeat fixture | Risk type | Deckard status | Current behavior | Required follow-up |
| --- | --- | --- | --- | --- |
| `recent-contract-1` | recent deploy | tracked gap | Deckard can refuse/route unknown calldata, but does not know whether a contract was recently deployed. | #150, #73 |
| `previous-interaction-1` | previous interaction | tracked gap | Deckard does not track or display per-wallet contract interaction history. | #150 |
| `wallet-own-1` | known scam / custom recipient | tracked gap | Deckard can review a native send, but does not score arbitrary recipients as suspicious. | #149, #135 |
| `known-scam-eth-send` | known scam address | tracked gap | Deckard does not maintain a scam-address fixture/list or reputation source. | #149, #135 |
| `allow-infinite-usdc` | infinite ERC-20 approval | tracked gap | Deckard classifies ERC-20 `approve`, but does not yet elevate unlimited allowance/spender risk as a dedicated warning. | #151, #135 |
| `allow-infinite-permit` | infinite permit signature | tracked gap | Deckard supports EIP-712 review and chain-id mismatch denial, but does not yet render Permit-specific owner/spender/value/deadline risk rows. | #151, #135 |

## Transaction-simulation fixtures

Source: `.walletbeat/walletbeat/src/components/Tabs/TransactionSimulationsTab.svelte`.

| WalletBeat fixture | Function | Deckard status | Current behavior | Required follow-up |
| --- | --- | --- | --- | --- |
| `erc20-mint` | `mintHundred()` | tracked gap | Arbitrary mint calldata is refused today; Deckard does not simulate token mint deltas. | #152, #73, #74 |
| `erc721-mint` | `mintOne()` | tracked gap | ERC-721 mint calldata is refused; Deckard does not simulate NFT mint deltas. | #152, #73, #74 |
| `erc1155-mint` | `mintOne()` | tracked gap | ERC-1155 mint calldata is refused; Deckard does not simulate semi-fungible token deltas. | #152, #73, #74 |
| `erc20-transfer` | `transfer(address,uint256)` | tracked gap | Deckard can classify ERC-20 transfer calldata, but simulation requires pre/post asset delta rendering instead of selector-only display. | #152, #73, #74 |
| `erc721-transfer` | `safeTransferFrom(address,address,uint256)` | tracked gap | ERC-721 transfer calldata is refused; Deckard does not render NFT ownership deltas. | #152, #73, #74 |
| `erc1155-transfer` | `safeTransferFrom(address,address,uint256,uint256,bytes)` | tracked gap | ERC-1155 transfer calldata is refused; Deckard does not render balance deltas. | #152, #73, #74 |
| `all-token-transfer` | `simulateFunctionV1()` | tracked gap | Mixed token effects require real simulation and multi-asset rendering. | #152, #153, #73, #74 |
| `misleading-selector` | selector looks like `transfer(address,uint256)` | tracked gap | This is a known dangerous ambiguity: selector-only classification cannot prove actual contract semantics. Deckard needs simulation/metadata before claiming support. | #153, #73, #74 |
| `fake-airdrop` | `claim()` drains while emitting misleading mint-like event | tracked gap | Requires state-diff simulation and event/balance reconciliation. | #153, #73 |
| `volatile-outcome` | block-dependent mint/burn | tracked gap | Requires simulation volatility/race warning and possibly repeated preflight near signing time. | #153, #73 |
| `failing-transaction` | guaranteed revert | tracked gap | Requires preflight revert detection before signing. | #153, #73 |

## QA gate expectations

`pnpm run qa:walletbeat:safety` validates that:

1. every WalletBeat scam-alert fixture id appears in this matrix;
2. every WalletBeat transaction-simulation fixture id appears in this matrix;
3. every `tracked gap` or `blocked` row has at least one linked issue;
4. the matrix has no `untracked gap` marker;
5. the current local-chain QA lane continues to prove the already-supported/refused primitives.

This matrix should be updated before bumping `DECKARD_WALLETBEAT_REF` if WalletBeat adds or renames fixtures.
