# deckard-contract

Frozen contract owned by `docs/build/30-mcp-shape.md` — do not redefine these types elsewhere.

This crate is the single source of truth for the wire every Deckard process speaks:

- **`Intent`** — the only thing that crosses `deckard-mcp → deckard-signerd` for a write. Carries `chain_id` (multi-chain ready); the daemon owns the nonce.
- **`Decision`** — the daemon's verdict from `propose`: `Allow` / `Deny{reason}` / `NeedsApproval{request_id}`.
- **`Policy`** — the agent-readable spending fence (caps, allowlist, approval mode, `revoked`).
- **RPC enums** (`SignerRequest` / `SignerResponse` / `ExecuteResult` / `ApprovalStatus` / `BalanceReport`) — the daemon socket API. serde-derived → CBOR (ciborium) on the UDS, JSON for MCP.
- **`Signer`** — a *sync* trait; the real UDS client does a fast blocking round-trip off the UI thread (an async wrapper is the daemon ticket's call).
- **`MockSigner`** — an in-memory, deterministic implementation so T-Agent, T-UX, and the test harness can build and run the acceptance scenario **before** the real signer daemon exists.

## Zero key material

This crate carries **no key material at all** — types + a trait + a mock. It never signs, never holds a key. The key boundary is the daemon's process (`deckard-signerd`, owned by `docs/build/00-test-harness.md`), not this crate.

## Deterministic mock

`MockSigner` is pinned for byte-stable tests: `address = 0x1111…11`, broadcast `tx_hash = 0xABAB…AB`, and `request_id`s assigned `0x0101…01`, `0x0202…02`, … in order. See `mock.rs` for the policy decision matrix (caps, allowlist, approval, and the TOCTOU revoke guard).

## Encodings

Every type round-trips through both `serde_json` (the MCP encoding) and `ciborium` / CBOR (the daemon-socket encoding); see the tests. Normal dependencies are exactly `alloy-primitives` + `serde`; `serde_json` and `ciborium` are dev-dependencies only.
