# Issue #46 — Clear-signing v2 + message-signing intents

## 1. Title

Issue #46 — message-signing intents for `personal_sign` and EIP-712.

## 2. Context

Deckard currently signs and broadcasts native transactions and signs CoW swap orders after human approval, but it has no generic off-chain message-signing intent. Dapp-origin signatures are a high-risk drainer surface, so message signing must be represented as a first-class reviewed payload before any EIP-1193 bridge methods expose it.

This is security/product work across the frozen wire contract, signer daemon, shared clients, and approval rendering.

## 3. Source Of Truth

- User instruction: start issue #46 now.
- GitHub issue: https://github.com/hellno/deckard/issues/46
- Repo guidance: `AGENTS.md`, `PLANS.md`
- Design guidance: `DESIGN.md` clear-signing review pattern
- ADRs/docs: `docs/adr/0001-dapp-connectivity-architecture.md`, `docs/adr/0005-erc-7730-clear-signing.md`, `docs/WALLETBEAT-COMPATIBILITY.md`
- Relevant code: `crates/deckard-contract/src/rpc.rs`, `policy.rs`, `deny_reasons.rs`; `crates/deckard-signerd/src/daemon.rs`, `signing.rs`, `request_id.rs`; `crates/deckard-app/src/activity_view.rs`
- Standards: EIP-191, EIP-712, ERC-7730, EIP-7702

## 4. Current State Analysis

- `PendingPayloadView` supports only transaction, order, and shaped approve payloads.
- `deckard-signerd` can sign CoW EIP-712 digests only through `SignOrder` after approval.
- There is no wire type for personal messages or arbitrary typed-data review.
- The browser bridge intentionally supports only account/chain methods today.
- ERC-7730 normalization exists from #65, but it is not wired into message-signing flows.

## 5. Target State

- Add a first-class `SignMessage` payload with safe review metadata.
- Add pure `evaluate_message` policy logic: messages never auto-allow; revoked denies; chain mismatch denies; raw `eth_sign` denies; EIP-7702 delegation denies by default.
- Add wire RPCs to propose and sign a stored, approved message.
- Add deterministic message request ids that cannot collide with transaction or order ids.
- Add pending/activity views so the app can show message-signing requests without treating them as broadcasts.
- Keep browser bridge exposure out of this PR unless the reviewed internal path is already complete.

## 6. Security And Trust Invariants

- Private keys, seed phrases, passphrases, raw scalar bytes, and signatures are never logged.
- Message bytes may be user-visible but must not appear in denial reason strings or daemon logs.
- Messages are never auto-approved in v1.
- `eth_sign` raw-hash signing is refused.
- EIP-7702 delegation authorization is refused by default until a dedicated reviewed allow path exists.
- Typed-data chain id must match the active signer chain before approval/signing.
- Signing is offline and does not broadcast.

## 7. UX And Design Constraints

- Message-signing approval rows must use plain language and danger/caution lines.
- `personal_sign` must show decoded UTF-8 where possible, otherwise a byte-length/hash-style summary.
- EIP-712 typed data must show domain, verifying contract, primary type, digest, and warnings.
- Unknown/untrusted origin is shown as unverified.
- Full screenshot proof is deferred if no new app-visible route exercises the card in automation; tests must cover summary/render helpers.

## 8. Execution Plan

1. Write failing tests for contract message types, policy decisions, and id separation.
2. Implement `message_signing` wire types and policy evaluation.
3. Add signer-daemon propose/sign handlers with stored message payloads.
4. Add client helpers for message propose/sign.
5. Extend pending/activity payload rendering for message requests.
6. Update docs/threat model for refusal/fallback behavior.
7. Run Deckard DoD and open a PR.

## 9. Validation Criteria

Default Deckard Definition of Done:

```text
cargo fmt --all --check
just check
cargo test --workspace
```

Task-specific checks:

- `cargo test -p deckard-contract message_signing`
- `cargo test -p deckard-signerd message`
- `cargo test -p deckard-signerd request_id`

## 10. Failure Signals

- A message proposal returns `Decision::Allow`.
- Raw `eth_sign` or EIP-7702 authorization can reach signing.
- Typed data with mismatched chain id reaches approval/signing.
- Message bytes appear in deny reasons/log strings.
- A message payload is executable/broadcast through `Execute`.

## 11. Risks And Tradeoffs

- Generic EIP-712 JSON parsing is intentionally staged: this PR can define the reviewed typed-data/digest model, but bridge-side raw JSON parsing may need a later PR.
- UI integration is limited to existing pending/activity rendering unless the current app has a direct message-signing compose surface.
- Adding message wire variants is an additive protocol change; older clients will ignore unsupported variants.

## 12. Out Of Scope

- Browser bridge `personal_sign` / `eth_signTypedData_v4` exposure.
- `eth_sendTransaction`.
- EIP-5792 batch calls.
- ERC-7730 registry fetching/caching.
- Supporting raw `eth_sign`.

## 13. Status Notes

- 2026-06-23: Created plan after PR #132 merged and branch `feature/issue-46-message-signing-intents` was created from updated `main`.
