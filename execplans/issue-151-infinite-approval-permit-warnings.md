# Issue #151 — infinite approval and permit risk warnings

## 1. Title

Add first-class review warnings for unlimited ERC-20 approvals and Permit-style typed-data signatures.

## 2. Context

WalletBeat's scam-alert fixtures include:

- `allow-infinite-usdc`: ERC-20 `approve(spender, type(uint256).max)`
- `allow-infinite-permit`: EIP-712 / EIP-2612 style Permit with an unlimited allowance and long deadline

Deckard already routes ERC-20 `approve(address,uint256)` and `eth_signTypedData_v4` through the browser bridge and signerd approval flow. The missing safety feature is that these requests do not yet surface allowance-specific risk as first-class wire/UI metadata.

## 3. Security posture

- Do not auto-block all permits or approvals; the feature is review/warning metadata, not a blanket deny.
- Keep raw `eth_sign` refused.
- Keep unknown calldata fail-closed.
- Do not fetch production reputation or token metadata in this PR.
- Use local/test fixtures only.

## 4. Implementation plan

1. Extend the shared wire model:
   - add approval risk metadata to `PendingPayloadView::Approve`;
   - add optional `PermitReview` metadata to `TypedDataReview`;
   - add message risks for unlimited allowance and long deadline.
2. Derive ERC-20 approval risk in signerd's `payload_view` from decoded `approve` amount.
3. Derive Permit review metadata in the browser bridge from bounded JSON inspection before hashing/signing.
4. Render risk rows in the app Activity/Approval card.
5. Add tests before implementation:
   - approval view marks `U256::MAX` as unlimited;
   - typed-data parser extracts Permit owner/spender/value/deadline and risks;
   - app summary/details mention unlimited approvals and Permit risks.
6. Update WalletBeat safety matrix for #151 rows from raw tracked gaps to partial/positive support.
7. Extend local-chain QA to prove infinite approval and permit warning metadata with throwaway data only.

## 5. Acceptance criteria

- [x] ERC-20 `approve(address,uint256)` with `uint256::MAX` carries an unlimited-allowance warning.
- [x] Permit-like `eth_signTypedData_v4` carries owner/spender/value/deadline review rows.
- [x] Long permit deadlines and unlimited values are flagged.
- [x] Raw `eth_sign` remains refused.
- [x] WalletBeat safety matrix rows `allow-infinite-usdc` and `allow-infinite-permit` are updated with #151 positive coverage.
- [x] Local-chain QA uses only throwaway accounts / local chain.

## 6. Verification plan

Focused:

- `cargo test -p deckard-browser-bridge permit -- --nocapture`
- `cargo test -p deckard-signerd approval_risk -- --nocapture`
- `cargo test -p deckard-app approval_risk -- --nocapture`
- `pnpm run qa:walletbeat:safety`
- `pnpm run qa:walletbeat:local-chain`

Full before PR:

- `cargo fmt --all --check`
- `just check`
- `cargo test --workspace`
- `pnpm run qa:extension`
- `pnpm run qa:extension:real`
- `pnpm run qa:walletbeat`
- `pnpm run qa:walletbeat:signatures`
- `pnpm run qa:walletbeat:transactions`
- `pnpm run qa:walletbeat:local-chain`
- `pnpm run qa:walletbeat:safety`
- `git diff --check`

## 7. Status

- [x] Branch created from merged `origin/main`.
- [x] Issue #151 read.
- [x] Relevant bridge/signerd/app/wire code inspected.
- [x] RED tests observed.
- [x] Implementation complete.
- [x] QA/matrix updated.
- [x] Full local DoD.
- [ ] PR opened and handed off for signing.
