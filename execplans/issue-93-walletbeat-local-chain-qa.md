# Issue #93 — local-chain WalletBeat transaction/signature QA

## 1. Title

Add a real signerd-backed, local-chain-only WalletBeat QA lane.

## 2. Context

Issue #93 asks for WalletBeat transaction/signature QA that never touches production profiles, real seeds, or mainnet funds. The existing WalletBeat lanes prove the browser/provider surface in dev-mock mode:

- `qa:walletbeat`
- `qa:walletbeat:signatures`
- `qa:walletbeat:transactions`

Those are valuable smoke tests, but they intentionally bypass the signer daemon and approval channel. The next gap is a local-chain lane that uses a throwaway QA vault, an unlocked real `deckard-signerd`, the key-less browser bridge in production mode, and a test-only approval supervisor.

## 3. Goals

- Add `pnpm run qa:walletbeat:local-chain`.
- Use a throwaway temp `DECKARD_CONFIG_DIR` only.
- Seal the existing deterministic QA vault; do not print seed/private-key material.
- Start local Anvil if available; otherwise fail with an actionable message.
- Start real `deckard-signerd` with a private resolver capability channel.
- Start `deckard-browser-bridge` without `--dev-mock-account`.
- Exercise through `window.ethereum` and the extension:
  - `eth_requestAccounts`
  - native `eth_sendTransaction`
  - ERC-20 `transfer(address,uint256)` shaped calldata
  - ERC-20 `approve(address,uint256)` shaped calldata
  - `personal_sign`
  - SIWE-style `personal_sign`
  - `eth_signTypedData_v4` with local chain id
  - raw `eth_sign` refusal
- Store report/screenshot under ignored `test-results/walletbeat` paths.

## 4. Non-goals

- Full WalletBeat UI automation for every tab button.
- EIP-5792 batch calls (#94).
- Scam alerts / transaction simulation (#95).
- Production seed/profile testing.

## 5. Safety constraints

- No seed phrases, private keys, passphrases, vault bytes, or browser profile contents in logs/reports.
- `DECKARD_CONFIG_DIR` must be a fresh temp dir for each run.
- Chain id must be local Anvil (`31337` / `0x7a69`).
- Bridge must run in real-daemon mode, not dev mock.

## 6. TDD plan

1. RED: add package script and script assertions that require a real-daemon result marker, then run before helper exists.
2. GREEN: add a test-only signerd QA supervisor with capability-channel auto-approval.
3. GREEN: add local-chain WalletBeat script using the supervisor and bridge.
4. REFACTOR: keep shared script shape boring; no dependency changes unless unavoidable.

## 7. Verification

- `pnpm run qa:walletbeat:local-chain`
- `pnpm run qa:extension:real`
- full Deckard DoD before PR:
  - `cargo fmt --all --check`
  - `just check`
  - `cargo test --workspace`
  - existing WalletBeat lanes
  - `git diff --check`

## 8. Status

- [x] Branch created from merged `origin/main`.
- [x] Plan created.
- [x] RED test observed.
- [x] QA supervisor implemented.
- [x] Local-chain WalletBeat QA script implemented.
- [x] Full local DoD.
- [x] PR opened.
- [ ] CI checked.
