# PRD-02 — Clear-signing v2 + message-signing intents

> Phase 1b of [ADR 0001](../adr/0001-dapp-connectivity-architecture.md). Extends the shared
> clear-signing engine and the `Intent` surface to **off-chain signatures**, the real drainer vector.
> Independent of transport; needed by PRD-03 (curated swap needs Permit2/EIP-712) and PRD-04.

## Why this exists

A dapp's most dangerous request is not a transaction — it is an **off-chain signature that authorizes
value movement with no on-chain tx the wallet ever simulates**. In 2024, EIP-2612 `permit` signatures
(56.7%) and `setOwner` (31.9%) were ~88.6% of wallet-drainer losses (`research §30`). Deckard today can
sign one structured off-chain payload (`SwapOrder` via `SignOrder`, EIP-712) but has **no general
message-signing path and no decode/clear-signing for arbitrary typed data**. Before any dapp can ask
Deckard to sign, the clear-signing card (`DESIGN.md` "the shared trust engine") must render *what is
actually being authorized*, in plain language, danger-early — never a raw hash.

## Goals

- Add typed message-signing to the contract: `personal_sign` (EIP-191) and `eth_signTypedData_v4`
  (EIP-712), routed through `propose → Decision → human approval`, **never auto-allowed**.
- **Decode and clear-sign** before the human: EIP-712 domain binding (chainId + verifyingContract),
  recognition of high-risk shapes (`permit`, Permit2, Seaport orders, `setOwner`/ownership transfer),
  and a distinct, alarming screen for EIP-7702 delegation.
- **Consume ERC-7730 descriptors** where available to render human-readable intent; **fall back to an
  explicit "blind signing — we can't decode this" warning** for the uncovered long tail.
- Hard-handle the chainId-mismatch and unknown-`verifyingContract` cases (warn loudly; the research
  shows most wallets silently don't — `research §33`).

## Non-goals

- The transport that *delivers* these requests (PRD-04). This PRD makes the daemon able to *safely
  sign a message it is handed*; where the message comes from is out of scope.
- Building the ERC-7730 registry. We *consume* descriptors; curation/registry shipping is PRD-05.
- Supporting `eth_sign` (raw 32-byte blind hash). **Refuse it outright** — MetaMask is removing it
  (`research §31`); Deckard never offers it.

## Design

### Contract (`crates/deckard-contract`)

Add message-signing as first-class, mirroring the existing `SwapOrder`/`SignOrder`/`PendingPayloadView`
pattern (do not overload `Intent`, which is a *transaction* shape):

- New request variants on `SignerRequest` (rpc.rs), e.g. `ProposeMessage { message: SignMessage }` →
  `Decision`, and `SignMessage { request_id }` → a `SignMessageResult { signature: Bytes }` (reuse the
  `SignOrderResult` shape). Message signing **never broadcasts** — distinct from `Execute`.
- A `SignMessage` enum:
  - `PersonalSign { bytes: Bytes }` (EIP-191 personal_sign; display decoded UTF-8 when valid, else hex
    + a "binary message" caution).
  - `TypedDataV4 { typed_data: ... }` (EIP-712). Parse the domain (`name, version, chainId,
    verifyingContract, salt`) and the primary type.
- A new `PendingPayloadView::Message(SignMessageView)` so the GUI inbox (existing
  `PendingList`/`PendingRecord`) renders message requests alongside tx/order/approve.
- New `deny_reasons` entries: `ETH_SIGN_REFUSED`, `CHAINID_MISMATCH`, `DELEGATION_REFUSED`,
  `UNDECODABLE_TYPED_DATA`.

Parse all untrusted typed-data bytes through the bounded `Reader` in `keystore.rs` style (DoD #5):
strict length/depth caps so a hostile EIP-712 blob can't OOM/recurse the daemon before validation.

### Decision logic (`crates/deckard-contract/src/policy.rs`)

Add a pure `evaluate_message(&SignMessage, &Policy, wallet, now) -> Decision` next to `evaluate` /
`evaluate_order` (keep the "one decision function, mock⇄daemon parity" charter):

- Messages are **always `NeedsApproval`** in v1 (like swap orders) — no auto-allow path.
- EIP-712: deny on `CHAINID_MISMATCH` (domain.chainId ≠ daemon chain). Surface unknown
  `verifyingContract` as a card-level danger flag (not necessarily a deny — the human decides), per
  EIP-712 SHOULD/MAY (`research §33`).
- Recognize and **flag danger-early** (red, top of card, per `DESIGN.md`): unlimited/large `permit` &
  Permit2 allowances, Permit2 batch, Seaport orders transferring assets for ~zero, `setOwner`/owner
  transfer. These mirror `PendingPayloadView::Approve`'s existing shaped-approve treatment.

### EIP-7702 delegation (`research §34`)

EIP-7702's own Security Considerations say wallets MUST NOT offer a generic "sign arbitrary
delegation" interface ("there is no safe way"). Decision (record in PRD): **refuse delegation
authorizations by default** (`DELEGATION_REFUSED`); if ever supported, gate behind a curated
trusted-delegator allowlist (a PRD-05 concern) and a *distinct* "you are handing control of your
account to `<address>`" screen — never the normal sign card. v1: refuse.

### Clear-signing card (`crates/deckard-app`, the shared review component)

Extend the existing clear-signing card (`DESIGN.md` "Clear-signing review card") to render
`SignMessageView`:
- Plain-language **headline** of what's being authorized; one canonical key/value list grouped by
  whitespace; danger in red at the top; **deliberate hold** to confirm (never a tap).
- ERC-7730: if a descriptor exists for the `verifyingContract`/call, render its human-readable fields.
  If not, show an explicit **"Blind signing — Deckard can't decode this request. Only continue if you
  fully trust the source."** caution (amber icon + risk word, per `DESIGN.md`), demoted-but-present.
- Origin shown as **unverified** (PRD-05 may upgrade it). Per `research §29`, never let a claimed name
  substitute for the decoded effects.

### `wallet_addEthereumChain` guard (`research §36`)

Even pre-transport, codify the rule: a chain add/switch must sign only with the **user-submitted**
chainId, never one returned by a (possibly malicious) RPC; require an explicit confirm naming requester
+ target chain. Land the daemon-side invariant here; the UI lands with PRD-04.

## Acceptance tests

- `eth_sign_refused`: a raw-hash sign request is denied with `ETH_SIGN_REFUSED`, nothing signed.
- `typed_data_chainid_mismatch_denies`: EIP-712 with domain.chainId ≠ daemon chain → `CHAINID_MISMATCH`.
- `permit_flagged_danger`: a `permit`/Permit2 typed-data produces a `Decision::NeedsApproval` whose
  pending view carries the danger flag (assert the flag, not just the approval).
- `delegation_refused`: an EIP-7702 authorization → `DELEGATION_REFUSED`.
- `message_never_broadcasts`: signing a message returns a signature, never a `tx_hash`; `Execute` on a
  message request is rejected.
- `undecodable_typed_data_bounded`: an oversized/deeply-nested EIP-712 blob is rejected by the bounded
  reader without OOM/panic (fuzz-style vector).
- Parity: `evaluate_message` gives identical verdicts in `MockSigner` and the daemon.
- Transcript hygiene: no message bytes/signature leak into any reason string (extend the existing
  allowlist scan).

## Definition of Done

PRD-series DoD **plus**: new ⌘K commands for any user-initiated signing actions; `DESIGN.md` card spec
honored (verify visually per `just check` build + a screenshot in the PR); the EIP-7702 refusal and the
blind-sign fallback are documented in `THREAT-MODEL.md` (new "message signing" surface section).

## Risks & fallbacks

- **EIP-712 parsing is a parser-security surface.** Mitigate with the bounded reader + fuzz vectors;
  keep the parser in `deckard-core` under its strict lints.
- **ERC-7730 coverage gap** (`research §35`): most contracts have no descriptor. The blind-sign
  fallback is therefore the *common* path, not the exception — design it to be honestly alarming, not
  normalized-away.
- **Scope creep into a full ABI decoder.** v1: decode the *high-risk shapes* (permit/Permit2/Seaport/
  owner-transfer) + ERC-7730 descriptors; everything else is explicit blind-sign. Don't build a
  general decoder.

## Sources

`docs/research/10-dapp-connectivity.md §30–36`; eips.ethereum.org eip-712, eip-7702, erc-7730,
eip-3085/3326; MetaMask MIP-3; scamsniffer 2024 drainer report; existing `swap_order.rs`/`SignOrder`.
