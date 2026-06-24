# Issue #95 — WalletBeat scam/simulation safety matrix

## 1. Title

Add a WalletBeat scam-alert / transaction-simulation safety matrix and QA gate.

## 2. Context

WalletBeat's scam-alert and transaction-simulation tabs are not simple RPC compatibility checks. They probe higher-level wallet safety behavior:

- reputation / suspicious-recipient warnings
- contract age and previous interaction context
- infinite approval and permit-signature risk display
- token/NFT asset-delta simulation
- adversarial outcome detection such as misleading selectors, fake airdrops, volatile outcomes, and guaranteed reverts

Deckard now supports a reviewed browser bridge for accounts, message signing, native sends, ERC-20 `transfer`/`approve`, EIP-5792 non-atomic batches, and a signerd-backed local-chain WalletBeat QA lane. It does **not** yet provide a full scam reputation engine or revm-style asset-delta simulation.

## 3. Decision

Do not close #95 by pretending unsupported simulation/scam behavior is implemented. Close it only as a coverage/gap-control gate:

1. Every WalletBeat scam/simulation fixture is listed in a committed matrix.
2. Every unsupported positive behavior has a linked follow-up issue.
3. A QA script fails if WalletBeat adds a new fixture without matrix coverage.
4. Current Deckard behavior is described honestly: supported, safe refusal, tracked gap, or blocked on architecture.

## 4. Acceptance criteria

- [ ] Read all WalletBeat scam-alert fixtures.
- [ ] Read all WalletBeat transaction-simulation fixtures.
- [ ] Build a fixture-by-fixture Deckard status matrix.
- [ ] Add a local QA script that validates matrix completeness against the pinned WalletBeat checkout.
- [ ] Assert unsafe/unsupported cases are refused, not signed, where current automation can do so safely.
- [ ] Create/link GitHub issues for every unsupported positive feature.
- [ ] Update GitHub Project so those follow-ups are visible.
- [ ] Do not close #95 until the matrix has no `untracked gap` rows.

## 5. Follow-up issue map

- #149 — address reputation and suspicious-recipient warnings.
- #150 — contract age and previous-interaction context.
- #151 — infinite approval and permit risk warnings.
- #152 — token and NFT asset-delta rendering.
- #153 — adversarial simulation outcome warnings.
- Existing #73 — revm preflight/simulation engine foundation.
- Existing #74 — verified token metadata foundation.
- Existing #135 — rules engine v2 / policy integration.

## 6. Verification plan

Focused:

- `pnpm run qa:walletbeat:safety`
- `pnpm run qa:walletbeat:local-chain`

Full DoD before PR:

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
- [x] Issue #95 and WalletBeat fixture files read.
- [x] Follow-up issues created.
- [x] Safety matrix committed.
- [x] QA gate committed.
- [x] GitHub Project updated.
- [x] Full local DoD.
- [ ] PR opened and CI checked.
