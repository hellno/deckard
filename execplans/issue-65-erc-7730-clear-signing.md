# Issue #65 — ERC-7730 Clear-Signing Descriptor Spike

## 1. Title

Issue #65 — consume ERC-7730 descriptors for clear signing.

## 2. Context

Deckard's clear-signing UX currently has purpose-built surfaces for shield/swap and CoW EIP-712 digest machinery, but it does not have a generic path for ERC-7730 clear-signing metadata. Issue #65 asks for a first-class spike so Deckard can align message-signing UX with ERC-7730 instead of inventing a local schema.

This is discovery plus a small parser/normalizer spike. It affects the key-less typed-data / transaction review layer, not signing authority.

## 3. Source Of Truth

- User instructions: start work on GitHub issue #65 after closing #62.
- GitHub issue: https://github.com/hellno/deckard/issues/65
- Repo guidance: `AGENTS.md`, `PLANS.md`
- Design guidance: `DESIGN.md` clear-signing review language, but this spike does not change UI rendering.
- ADRs / docs: `docs/WALLETBEAT-COMPATIBILITY.md`, `docs/adr/0001-dapp-connectivity-architecture.md`
- Relevant code files: `crates/deckard-contract/src/*`, `crates/deckard-core/src/cow_types.rs`
- External standards: ERC-7730 (`https://eips.ethereum.org/EIPS/eip-7730`), Clear Signing build docs (`https://clearsigning.org/build/`)

## 4. Current State Analysis

- `deckard-contract::Intent` supports generic `ContractCall`, but there is no structured review model for arbitrary calldata or EIP-712 messages.
- `deckard-core::cow_types` computes a CoW order EIP-712 digest, but this is protocol-specific and not a general descriptor consumer.
- ERC-7730 describes descriptor context, metadata, and display formats. Wallets must bind descriptors to the reviewed chain/contract/message before applying labels.
- Clear Signing docs emphasize wallet-owned trust policy: registry metadata can be low-quality or malicious; wallets decide what reaches the signing screen.

## 5. Target State

- Add a small typed ERC-7730 descriptor model and normalizer spike.
- Normalize only the minimum Deckard needs now: context binding, metadata owner/contract name, display intent, and ordered field labels/paths/formats.
- Provide explicit fallback states for descriptor-missing, descriptor-invalid, context-mismatch, format-missing, and unsupported-format cases.
- Add fixtures for descriptor-present and descriptor-invalid fallback.
- Add a short ADR/design note that records where descriptor lookup/parsing should live and what Deckard must not trust.

## 6. Security And Trust Invariants

- ERC-7730 metadata is never a security oracle.
- Descriptor formatting is applied only after chain/address or message context binding succeeds.
- Missing, invalid, mismatched, or unsupported descriptors fall back to blind/undecodable signing warnings.
- Message signatures remain human-approved; no auto-approval is introduced.
- No seed, key, passphrase, or decrypted vault material is touched.

## 7. UX And Design Constraints

- This spike does not render GPUI screens, so screenshots are not required.
- Future UI must use `DESIGN.md` clear-signing primitives: transaction-as-hero, plain labels, caution line for warnings, and no calm rendering for blind-signing.
- Fallback copy should be plain: "Descriptor missing", "Descriptor invalid", "Descriptor does not match this contract", "Unsupported descriptor format".

## 8. Execution Plan

1. Add `clear_signing` module to `deckard-contract` with ERC-7730 descriptor structs and normalized review structs.
2. Implement context-bound normalization for EVM contract descriptors.
3. Add fallback helper and error taxonomy.
4. Add JSON fixtures for valid and invalid descriptors.
5. Add tests for descriptor-present rendering and invalid/mismatched fallback.
6. Add ADR describing the consumption path and trust rules.
7. Run `cargo fmt --all --check`, focused tests, `just check`, and `cargo test --workspace`.

## 9. Validation Criteria

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Task-specific checks:

- `cargo test -p deckard-contract clear_signing`
- Tests prove descriptor-present normalization and descriptor-invalid fallback.

## 10. Failure Signals

- Descriptor labels render without context binding.
- Unknown formats are silently accepted as safe.
- Registry/source metadata is treated as trusted proof of contract safety.
- Parser changes pull heavy runtime dependencies into key-less binaries.

## 11. Risks And Tradeoffs

- This is intentionally a small normalizer, not a full ERC-7730 interpreter.
- It does not yet decode calldata or EIP-712 typed values; it prepares the review model and fallback policy.
- Full registry trust, caching, revocation, and schema validation remain future work.

## 12. Out Of Scope

- Hosting or building an ERC-7730 registry.
- Auto-approval of message signatures.
- GPUI rendering.
- Full calldata/EIP-712 value interpolation.
- Onchain registry or attestation verification.

## 13. Status Notes

- 2026-06-23: Created plan after #62 was verified and closed.
