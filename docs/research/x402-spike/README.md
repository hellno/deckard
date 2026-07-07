# Spike: x402 build shape + fully-local facilitator loop (#206)

**Date:** 2026-07-07 · **Time-box:** ~1 day · **Status:** both questions answered with running evidence.
**Feeds:** the two build decisions #34 needs before it starts. Code here is throwaway; the deliverables
are the two decisions and this reproducible recipe.

This spike answers two questions #34 (x402 `exact`/EIP-3009 payments, the "$5/day" issue) assumed but
never proved:

1. **Build shape** — hand-roll the EIP-3009 `TransferWithAuthorization` typed data in `deckard-core`,
   or depend on `x402-types`/`x402-reqwest`?
2. **The loop** — does a fully-local x402 `exact` loop actually work on our existing anvil fork of
   Ethereum Sepolia, with a *self-hosted* facilitator (no hosted facilitator serves Ethereum Sepolia)?

---

## TL;DR — the two decisions

1. **Hand-roll the EIP-3009 struct in `deckard-core`.** Proven byte-for-byte: a 6-line `alloy sol!`
   struct (exactly the `cow_types.rs` precedent) produces the *identical* EIP-712 type hash, digest,
   and signature as x402-rs's own `TransferWithAuthorization`. Depending on `x402-chain-eip155` to get
   that type drags **222 crates** (three `rustls` versions, `hyper`, `reqwest`, the full
   `alloy-provider`/`rpc`/`transport` stack) into the `#![forbid(unsafe_code)]` trust core for zero
   cryptographic gain. Use x402-rs **only** as the demo-fixture facilitator binary, never a build dep —
   exactly as #34 already planned.

2. **The fully-local loop works. Stay on the Ethereum Sepolia fork.** `402 → sign EIP-3009 →
   facilitator /verify → /settle → on-chain` runs end-to-end against the real Circle USDC bytecode on
   `anvil --fork-url $RPC_URL_SEPOLIA --fork-block-number 10822990`. `./run.sh` reproduces it from a
   cold start and asserts the USDC moved, the nonce was consumed, gas was paid by the facilitator (not
   the buyer), and replay is rejected.

Both are green. **Go** for #34 on the Sepolia fork.

---

## Question 1 — hand-roll vs `x402-types`

### Verdict: hand-roll in `deckard-core`; x402-rs stays a demo binary.

### Evidence: the digest comparison ( `digest-cmp/` )

x402-rs does not do anything special to build the EIP-3009 authorization — it declares the same
Solidity struct via `alloy sol!` and calls `eip712_signing_hash`, which is what
`deckard-core/src/cow_types.rs` already does for CoW orders. The spike builds the *same* authorization
two ways and compares:

```
== EIP-3009 TransferWithAuthorization: hand-roll vs x402-rs ==
typeHash(handroll)  = 0x7c7c6cdb67a18743f49ec6fa9b35f50d52ed05cbed4cc592e13b44501c1a2267
typeHash(x402-rs)   = 0x7c7c6cdb67a18743f49ec6fa9b35f50d52ed05cbed4cc592e13b44501c1a2267
digest(handroll)    = 0x89d2449c668159cd27d97439dcfe47a3a6e3fc1edb7608de1e6bf0e8cb83200f
digest(x402-rs)     = 0x89d2449c668159cd27d97439dcfe47a3a6e3fc1edb7608de1e6bf0e8cb83200f
sig(handroll)       = 0x3e1236af…924de1b
sig(x402-rs)        = 0x3e1236af…924de1b
MATCH: typeHash=true digest=true signature=true
```

Four independent implementations agree on that digest and signature: deckard's hand-rolled `alloy
sol!`, x402-rs's `TransferWithAuthorization`, foundry's `cast wallet sign --data`, and — the ultimate
oracle — the **deployed USDC contract itself**, which accepted the hand-rolled signature on-chain
during `/settle`. The type hash `0x7c7c6cdb…` equals the contract's own
`TRANSFER_WITH_AUTHORIZATION_TYPEHASH()` constant. The printed vector is the KAT #34 gates against
([`eip3009-kat.json`](eip3009-kat.json)).

### Why not the dependency

| Axis | Hand-roll (`sol!` in `deckard-core`) | Depend on `x402-*` |
|------|--------------------------------------|--------------------|
| Trust-core surface | `alloy-primitives` + `alloy-sol-types` — **already compiled** by `deckard-core` for `cow_types` | `x402-chain-eip155` (client) = **222 transitive crates**: 3× `rustls`, `hyper`, `reqwest`, `tokio`, `tower`, full `alloy-provider`/`rpc-client`/`transport-http`/`contract` |
| Audit load | 6-line struct + a KAT test (the `cow_types` pattern) | a networked payments client inside the `#![forbid(unsafe_code)]` signer boundary |
| What you'd actually reuse | — | only the tiny serde wire structs (`PaymentPayload`/`PaymentRequirements`) — and those are **agent-side transport**, not the trust core. `x402-types` alone does *not* carry the `sol!` signing struct; that lives in the heavy `x402-chain-eip155`. |
| Fit with workspace lints | clean | x402-rs's `client` uses `rand::rng()` (deckard denies `thread_rng`) and has a `tracing`-under-wrong-feature bug we hit |
| alloy version | deckard `1.6.0` | x402 `1.6` (chain crate) — compatible major, but a second edge to manage |

### Header ownership (v1 `X-PAYMENT` vs v2 `Payment-*`)

The facilitator's `/verify` and `/settle` take **plain JSON bodies** in both protocol versions — the
version difference is purely *how the client and server exchange the challenge and payload over HTTP*,
which is **agent-side transport, not Deckard's concern** (this is exactly #34's "transport stays
agent-side"). Confirmed against x402-rs source:

- **v1:** request header `X-Payment` = base64(JSON `PaymentPayload`); response header `Payment-Response`.
- **v2:** 402 response header `Payment-Required` = base64(`PaymentRequired`); client retries with
  `Payment-Signature` = base64(signed `PaymentPayload`); response header `Payment-Response`.

Deckard receives the already-decoded `PaymentRequirements` from the agent and returns a signed
authorization. It never parses an HTTP header. The agent-side x402 wrapper (x402-reqwest or the
`deckard_fetch_paid` client in #208) owns all header encode/decode for both versions.

---

## Question 2 — the fully-local loop, PROVEN

`./run.sh` (self-contained; starts anvil + the facilitator, runs the whole loop, asserts, cleans up).
Cold-start transcript:

```
== 1/8 start anvil fork of Ethereum Sepolia @ block 10822990 ==
anvil up: chain=11155111 block=10822990
== 2/8 start x402-rs facilitator (config: eip155:11155111 -> fork) ==
/supported: {"kinds":[{"network":"eip155:11155111","scheme":"exact","x402Version":2}], "signers":{"eip155:11155111":["0xf39F…"]}}
== 3/8 fund throwaway buyer with 100 USDC via setStorageAt (slot 9) ==
buyer USDC = 100000000
== 4/8 buyer signs the EIP-3009 TransferWithAuthorization (EIP-712, gasless) ==
signature = 0x3e1236af…924de1b
== 5/8 POST /verify ==
{ "isValid": true, "payer": "0x06B92bd3…" }
== 6/8 POST /settle ==
{ "success": true, "transaction": "0xd927…", "network": "eip155:11155111", "payer": "0x06B92bd3…" }
== 7/8 assert on-chain effect ==
buyer     100000000 -> 99000000   (expect -1000000)
recipient 0 -> 1000000            (expect +1000000)
nonce consumed = true             (expect true)
gas paid by    = 0xf39f…92266     (facilitator, not the buyer)
== 8/8 replay protection — re-settle the same authorization MUST fail ==
{"success":false, "errorMessage":"…FiatTokenV2: authorization is used or canceled…"}
== RESULT: local x402/EIP-3009 facilitator loop PROVEN on the Sepolia fork ✅ ==
```

The settlement tx (`status 1`, block 10822992) emits the two expected events — `AuthorizationUsed(buyer,
nonce)` and `Transfer(buyer → recipient, 1000000)` — and its `from` is the facilitator's signer, so the
buyer paid **no gas**. This is the security property #34 rests on: the signature pins amount + recipient
+ validity window, so the facilitator can withhold settlement but cannot alter or steal.

### Facilitator choice

**x402-rs** ( <https://github.com/x402-rs/x402-rs>, Apache-2.0 ). Chosen over
`second-state/x402-facilitator` because it proved out immediately: CAIP-2 JSON config, an
`eip155:11155111` entry pointing at the fork, `cargo build -p x402-facilitator --no-default-features
--features chain-eip155` (~90 s, **no Docker needed**), and it serves `scheme:exact / x402Version:2`
on our chain out of the box. Ethereum Sepolia (`11155111`) is **not** in x402-rs's `KNOWN_NETWORKS`
table (base-sepolia, celo-sepolia, etc. are — no Ethereum), but its `ChainId` accepts arbitrary
`eip155:*` references, so a self-hosted config just works. That absence is also the confirmation that
no hosted facilitator serves this chain — self-hosting is mandatory, as #206 assumed.

---

## Spec corrections v2 forced

1. **Use protocol v2 (CAIP-2), not v1.** v1 identifies the chain by *network name*
   (`"base-sepolia"`); Ethereum Sepolia has no registered v1 name, so the fork loop must run under v2,
   where the network is the CAIP-2 id `eip155:11155111`.
2. **This USDC's EIP-712 domain name is `"USDC"`, version `"2"`** — *not* mainnet's `"USD Coin"`.
   Verified against the on-chain `DOMAIN_SEPARATOR()` (`0xb90e5057…`). The v2 402 challenge must carry
   `extra: { "assetTransferMethod": "eip3009", "name": "USDC", "version": "2" }` so the facilitator
   reconstructs the right domain without querying the contract. **#34's `PaymentRequirements` decoder
   must not assume the mainnet domain name.**
3. **v2 field renames:** requirements use `amount` (not v1's `maxAmountRequired`), the CAIP-2 `network`,
   and drop `resource`/`description`/`mimeType` from the requirements object (they moved to an optional
   `resource` block on the payload). The facilitator request is
   `{ x402Version, paymentPayload{ accepted, payload{ signature, authorization }, x402Version },
   paymentRequirements }`.
4. **v2 HTTP headers** are `Payment-Required` (402), `Payment-Signature` (retry), `Payment-Response`
   (result) — case-insensitive; x402-rs uses that capitalization. Agent-side only.

---

## The gotcha worth remembering: don't sign as an anvil default account

The loop first failed with `FiatTokenV2: invalid signature` even though the digest was provably correct
(matched on-chain domain separator + typehash; ecrecover returned the signer). Root cause: **anvil's
canonical test accounts (`0xf39F…`, `0x7099…`, `0x3C44…`) carry EIP-7702 delegation designators
(`code == 0xef0100…`) on *real* Sepolia.** USDC's `SignatureChecker` sees code at the `from` address
and routes to EIP-1271 `isValidSignature`, rejecting a plain EOA signature. Fix: sign as a **fresh,
randomly-generated codeless EOA** (the recipe uses deterministic throwaway keys `dec00d01`/`dec00d02`
and asserts `cast code $BUYER == 0x`). Applies to #34's demo fixtures and any 7702-era fork test.

---

## Go / no-go: stay on the Ethereum Sepolia fork

**Go — stay on the Sepolia fork.** Single strongest reason: **it is the demo world we already run**
(`just demo`, block 10822990, the same fork every other Deckard demo uses), it carries genuine
FiatTokenV2_2 EIP-3009 bytecode, and the full loop is now proven green on it with zero new
infrastructure. Falling back to an anvil fork of Base would buy only *hosted-facilitator parity* — and
this loop is **self-hosted by design** (`x402-rs` as a `demo-check` prerequisite, never a product
dependency), so hosted parity is irrelevant to the local demo. Base becomes relevant only in the later
"hosted-facilitator reach" phase #34 explicitly defers; keep it there.

---

## Reproduce

```bash
export RPC_URL_SEPOLIA=https://sepolia.drpc.org   # free archive tier; serves block 10822990

# 1. Build the facilitator once (no Docker):
git clone https://github.com/x402-rs/x402-rs && cd x402-rs
cargo build -p x402-facilitator --no-default-features --features chain-eip155
export FACILITATOR_BIN=$PWD/target/debug/x402-facilitator

# 2. From the deckard repo root, run the whole proof (starts anvil + facilitator, asserts, cleans up):
cd docs/research/x402-spike && ./run.sh

# 3. (Question 1) reproduce the digest comparison + regenerate the KAT:
cd digest-cmp && cargo run
```

Requirements: `foundry` (anvil, cast), `jq`, `curl`. Throwaway keys only; no secrets in any file here.

## Files

| File | What |
|------|------|
| [`run.sh`](run.sh) | self-contained proof of the loop (deliverable #1 / #2) |
| [`facilitator-config.json`](facilitator-config.json) | x402-rs config: the `eip155:11155111 → fork` entry |
| [`typed-data.json`](typed-data.json) | the EIP-3009 `TransferWithAuthorization` EIP-712 typed-data template |
| [`eip3009-kat.json`](eip3009-kat.json) | the known-answer vector (domain, message, typehash, digest, signature) for #34 |
| [`digest-cmp/`](digest-cmp/) | Question-1 harness: hand-roll vs x402-rs, byte-for-byte (standalone; git dep on x402-rs) |
