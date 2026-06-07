# deckard-contract

Frozen contract owned by `docs/build/30-mcp-shape.md` — do not redefine these types elsewhere.

This crate is the single source of truth for the wire every Deckard process speaks:

- **`Intent`** — the only thing that crosses `deckard-mcp → deckard-signerd` for a write. Carries `chain_id` (multi-chain ready); the daemon owns the nonce.
- **`Decision`** — the daemon's verdict from `propose`: `Allow` / `Deny{reason}` / `NeedsApproval{request_id}`.
- **`Policy`** — the agent-readable spending fence (caps, allowlist, approval mode, `revoked`).
- **`evaluate(&Intent, &Policy) -> Decision`** — the **one** pure decision function. Both `MockSigner` and the real `deckard-signerd` call it, so the verdict can never drift between the mock and the daemon (parity is unit-asserted). It returns `RequestId::ZERO` as a placeholder for `NeedsApproval`; the stateful caller mints the real id.
- **RPC enums** (`SignerRequest` / `SignerResponse` / `ExecuteResult` / `ApprovalStatus` / `BalanceReport` / `UnlockOutcome`) — the daemon socket API. `SignerRequest` includes `Unlock{passphrase}` / `Lock` / `Resolve{request_id, approved}` for the daemon's lock state machine + approval loop (`Unlock` → `SignerResponse::Unlock(UnlockOutcome)`; `Lock`/`Resolve` → `Ack`). serde-derived → CBOR (ciborium) on the UDS, JSON for MCP.
- **`Signer`** — a *sync* trait (`unlock`/`lock`/`resolve`/`address`/`balance`/`policy`/`propose`/`execute`/`status`/`revoke_all`); the real UDS client does a fast blocking round-trip off the UI thread (an async wrapper is the daemon ticket's call).
- **`MockSigner`** — an in-memory, deterministic implementation (calls `evaluate`, no duplicated decision logic) so T-Agent, T-UX, and the test harness can build and run the acceptance scenario **before** the real signer daemon exists.

## Zero key material

This crate carries **no key material at all** — types + a trait + a mock. It never signs, never holds a key. The key boundary is the daemon's process (`crates/deckard-signerd`; cross-process red-team owned by `docs/build/00-test-harness.md`), not this crate.

## Deterministic mock

`MockSigner` is pinned for byte-stable tests: `address = 0x1111…11`, broadcast `tx_hash = 0xABAB…AB`, and `request_id`s assigned `0x0101…01`, `0x0202…02`, … in order. See `mock.rs` for the policy decision matrix (caps, allowlist, approval, and the TOCTOU revoke guard).

## Encodings

Every type round-trips through both `serde_json` (the MCP encoding) and `ciborium` / CBOR (the daemon-socket encoding); see the tests. Normal dependencies are exactly `alloy-primitives` + `serde`; `serde_json` and `ciborium` are dev-dependencies only.
