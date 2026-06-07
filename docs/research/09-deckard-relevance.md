# Cross-cutting Synthesis — the Opportunity Surface for Deckard

> Threads that recur across files 01–08, mapped to Deckard (a native Rust/GPUI, cross-platform,
> self-custodial desktop wallet — a bare EOA today, with an LLM operator-wallet vision).
> Part of the Deckard wallet research KB. Researched 2026-06-05.

**This is a synthesis of observations and opportunities — explicitly NOT a roadmap, timeline, or
prioritization.** It exists to make the recurring structure legible before the separate product-planning
step. Bracketed references point to the source files (e.g. `[02]` = `02-account-abstraction.md`), where
the primary citations live.

---

## Thread 1 — The EOA→smart-account hinge is the master variable

This is the single most load-bearing fact across the whole KB: **almost every advanced capability a
2026 wallet differentiates on is unreachable from a bare EOA.** Session keys, gas sponsorship,
pay-gas-in-token, atomic batching, on-chain spend limits, passkey signers, social recovery, and the
entire agentic permission stack all presuppose a smart account or a 7702-delegated EOA `[01][02][05]`.

- **EIP-7702 is the address-preserving bridge.** It's Final, live since Pectra (May 7 2025), and lets
  an existing EOA delegate to contract code *without changing its address or key* — so Deckard's
  already-in-the-field accounts could gain smart-account behavior in place `[02]`. The shipped
  production pattern is "**7702 + 4337 together**," with `Simple7702Account` (audited, in the canonical
  eth-infinitism repo) as an off-the-shelf delegation target `[01][02]`.
- **The Rust tooling already exists** (this matters despite the "ignore language" note — it lowers the
  cost of every smart-account option): alloy's `TransactionBuilder7702`, the `alloy-eip7702` crate,
  `alloy_rpc_types_eth::erc4337` types, and two Rust ERC-4337 bundlers (Alchemy's **Rundler**, modular
  **Silius**) that could even run in-process for a local-first app `[02]`.
- **7702 is also a live attack surface.** In its first month, the dominant on-chain 7702 activity was
  malicious delegation to sweeper bytecode (Wintermute's "CrimeEnjoyor" finding — count-dominance, not
  value-stolen, and point-in-time) `[02][08]`. The lesson is UX: *what a user/agent authorizes when
  signing a 7702 tuple* is the security-critical moment.
- **Native protocol AA (EIP-8141)** is proposed but only "Considered for Inclusion" for the late-2026
  Hegota fork — **not shipped, not a confirmed headliner.** 4337+7702 is the only shipped path through
  2026, so it's the substrate to reason about, not native AA `[01]`.

*Opportunity surface:* a wallet that treats 7702 delegation as a first-class, legible, reversible
operation (clear "what am I delegating to" UX, easy reset-to-EOA) addresses both the capability gap and
the dominant observed attack class at once.

## Thread 2 — The operator-wallet blueprint has already converged

Independent vendors arrived at the same architecture for "an LLM that drives a wallet," and Deckard's
stated vision is the same shape `[05]`:

- **The axiom: the agent never sees the seed.** The LLM is a *scoped signer*; the key stays isolated
  behind a policy gate the model cannot bypass `[05]`.
- **Dual-key model:** an operational, scoped *agent key* + a non-custodial *owner key* that retains
  override (halt, withdraw, revoke). Maps onto a Rust design where a bounded, policy-gated signing path
  is separate from the master seed `[05]`.
- **MCP is the integration surface.** A local stdio/HTTP daemon exposing wallet ops (`simulate`,
  `sign`, `transfer`, `set spend limit`) as LLM tools is now the standard "sidecar." Coinbase Payments
  MCP and the relaunched Base MCP (every write needs explicit user approval) are the safe-pattern
  references; raw "private key in env var" EVM MCP servers are the anti-pattern to improve on `[05]`.
- **Splits is a directly copyable design point.** `@splits/splits-cli` is *one binary that is both a
  CLI and an MCP server*; in MCP mode it **refuses flag-based secrets** so keys never enter tool-call
  transcripts, and the key lives only in a `0600` config file. An agent becomes a signer by registering
  its EOA and attaching it to a subaccount — never receiving a seed `[04][05]`.
- **Simulate-before-sign** is a self-contained safety primitive that works regardless of account type
  (compute expected asset changes, block on deviation) — EOA-compatible today `[05][01]`.
- **The EOA tension:** on-chain-enforced scoping (ERC-4337 spend caps, ERC-7715/7710 session keys —
  the latter shipped in MetaMask Apr 2026) requires a smart account. On a bare EOA the *same
  scope/expiry/limit semantics* can be replicated in a **local software policy gate** between the LLM
  tools and the secp256k1 key — at the cost of being software-enforced rather than chain-enforced `[01][05]`.
- **The economic/identity layer is real but frontier:** x402 (HTTP-402 stablecoin payments; now a Linux
  Foundation foundation), Google AP2 "Mandates" (signed, scoped pre-authorizations), and ERC-8004
  (on-chain agent identity/reputation). ERC-20 paymasters mean an agent could transact entirely in
  stablecoins it holds, never needing the user to top up ETH `[01][05]`.

*Opportunity surface:* the **local-MCP-sidecar + simulate + scoped-policy + key-isolation** stack is
proven and shipping — and the institutional pattern (TEE + policy engine, à la Turnkey/Privy) has a
self-custodial local analog: an enforced limit layer the LLM cannot bypass, with the seed encrypted at
rest. Splits' revocable, low-blast-radius credential model (`centaur`/`iron-proxy` worldview) is the
security posture to study for an autonomous local operator `[04][05][08]`.

## Thread 3 — Privacy is a stack of independent properties, and the neglected layer is Rust-native

Vitalik's "maximally simple L1 privacy roadmap" has four pillars: payment privacy, address-per-app,
private reads, and network-level obfuscation `[06]`. Key structural observations:

- **The operational/metadata layer is the most wallet-controllable and the most neglected.** Mainstream
  wallets still default to IP-leaking RPC (Infura sees your IP + address). This is a gap a *native
  client controls directly*, with no protocol dependency `[06]`.
- **Helios is a Rust light client built to embed in wallets** — so the "untrusted-RPC → verifiable
  local-RPC" path involves no language bridge for Deckard `[06]`.
- **Privacy primitives are separable, not one toggle:** stealth addresses (ERC-5564/6538) break the
  address graph; FHE confidential tokens (Zama ERC-7984, mainnet Dec 2025) hide amounts; shielded pools
  (Railgun, Privacy Pools) do balance privacy. They're complementary, exposed as distinct properties
  `[06]`.
- **Shielded-pool compliance models are mutually exclusive design choices** — Privacy Pools proves
  *inclusion* in an allowlist; Railgun PPOI proves *non-membership* in a blocklist; Labyrinth does
  threshold reveal. A wallet that let the user/agent pick per-transaction would span all three rather
  than hard-coding a posture `[06]`.
- **EIP-8182** (proposed for Hegota, H2-2026) would give an *EOA-today* wallet private transfers with no
  new address format and a shared anonymity set — i.e. payment privacy *without* first migrating to
  smart accounts; its design contemplates ECDSA/hardware-wallet signing `[06]`.
- **Kohaku's crypto core is Rust** (the `railgun`/`railgun-ts` split) and EF contributors explicitly name
  CLI/native wallets as targets — so the EF reference privacy work is consumable by a native Rust app
  without the WASM/TS wrapper `[03][06]`.
- **Regulatory backdrop favors self-custodial integrators** over service operators (Tornado sanctions
  vacated; the conviction risk lands on operators) — matching Deckard's non-custodial posture `[06]`.

*Opportunity surface:* operational privacy (embedded light client, address-per-dapp, private RPC) is an
under-served, wallet-controllable layer that compounds when a non-human (LLM) is transacting across many
dapps and would otherwise leave a linkable trail.

## Thread 4 — A public, codified scorecard already defines "good wallet"

**Walletbeat** is the "L2BEAT of wallets": a live site (`beta.walletbeat.eth.limo`) backed by an MIT
repo whose rubric is machine-readable and self-assessable without permission `[07]`. It rates five
attribute groups — Security, Privacy, Self-sovereignty, Transparency, Ecosystem — plus a Stages maturity
ladder. WalletScrutiny complements it with reproducible-build verdicts `[07]`.

Where a native self-custodial EOA desktop wallet structurally lands (descriptive, per the rubric `[07]`):

- **Strong by construction:** self-sovereignty/ownership (keys generated and held on-device),
  `accountUnruggability` (no provider can take over), and `transparency.openSource` — Deckard's **0BSD
  license clears the FOSS bar**. Planned BIP-39/32/44 exportable seed backup maps onto `accountPortability`.
  OS-CSPRNG key generation is likely already a PASS on RNG.
- **`PARTIAL`, not `PASS`, on storage:** an Argon2id + XChaCha20-Poly1305 keystore reads as
  "standardized-KDF-encrypted / OS-sandboxed" = `PARTIAL`; a `PASS` needs a hardware/secure-enclave path.
  The v0 plaintext key sits at the `PARTIAL`/`FAIL` boundary.
- **FAIL/unrated until external milestones land** (independent of code quality): security audits, a
  funded bug bounty, default-private RPC (`l1ProviderIndependence` wants user-set self-hosted RPC before
  first request), privacy non-correlation, hardware support, and Ecosystem items (account abstraction,
  batching, ENS, WalletConnect/EIP-6963). Note `accountRecovery` credits only **3+-guardian social
  recovery** — seed backup alone does not score there.

*Opportunity surface:* the rubric is a ready-made, permissionless checklist. The operator-wallet vision
intersects directly with `accountUnruggability`/`securityBestPractices` — a *local* LLM agent keeps keys
on-device and aligns with the rubric, whereas any cloud component able to move funds without on-device
key control would jeopardize those ratings.

## Thread 5 — The security floor is non-negotiable, the ceiling is smart-account-gated

- **v0's plaintext-hex key on disk is below the universal field floor** — no mainstream wallet stores
  cleartext keys; even Foundry/`cast` encrypts. The locked Argon2id + XChaCha20-Poly1305 envelope is the
  single highest-value security change and is *cryptographically ahead* of the Web3 Secret Storage v3
  standard (AES-128-CTR + PBKDF2/scrypt + keccak MAC) `[08]`.
- **But "ahead" means non-interoperable.** BIP-39 mnemonic backup is the genuine cross-wallet recovery
  layer; an optional `eth-keystore` (scrypt + AES-128-CTR) export is the portability escape hatch `[08]`.
- **The Rust primitives are mature and audited:** `k256`, `chacha20poly1305`, `argon2`, `zeroize`,
  `eth-keystore` — the envelope is buildable from audited pure-Rust crates `[08]`.
- **Secure Enclave is secp256r1-only** — it can gate the keystore *unlock secret* via Touch ID today,
  but **cannot hold the Ethereum secp256k1 key itself.** Full enclave/passkey *signing* needs a smart
  account (P-256 on-chain via RIP-7212 on L2s, EIP-7951 on mainnet since Fusaka, Dec 3 2025) `[08][01]`.
- **Hardware wallets (Ledger/Trezor)** are the strongest off-the-shelf single-key-risk reduction
  available to a desktop EOA, independent of any smart-account work `[08]`.
- **Clear-signing (EIP-712 + ERC-7730)** gives machine-readable transaction intent — the natural source
  for an LLM (or user) to understand *what a signature does* before approving. The registry is now
  EF-governed but coverage is partial `[01][08]`.

## What's reusable in Rust today (consolidated)

A cross-cutting note because so much of the relevant stack is already Rust — it lowers the cost of
several options above. (Maturity varies; see source files.)

| Capability | Rust artifact | File |
|---|---|---|
| EOA signing / keys | alloy (`alloy-signer-local`, `k256`) — already in Deckard | `[02][08]` |
| Encrypted keystore | `argon2`, `chacha20poly1305`, `zeroize` (custom envelope); `eth-keystore` (interop export) | `[08]` |
| EIP-7702 | alloy `TransactionBuilder7702`, `alloy-eip7702` crate, alloy 7702 signing (PR #2499) | `[02]` |
| ERC-4337 | `alloy_rpc_types_eth::erc4337` types; bundlers Rundler (Alchemy) & Silius | `[02]` |
| Light client / private reads | Helios (a16z) — embeddable Rust light client | `[06]` |
| Shielded pools | Kohaku's `railgun` crate (pure-Rust core, alpha) — consumable without the WASM/TS layer | `[03][06]` |

## The white space (observational)

Across every file, one gap recurs: **no shipping consumer wallet offers safe, scoped, revocable
end-to-end LLM-operator control as a product.** It exists today only as infra-provider plumbing
(Turnkey, Coinbase Agentic Wallets, Splits' CLI/MCP) plus MetaMask's just-launched Advanced Permissions
`[01][04][05]`. A **native, local-first, self-custodial desktop operator wallet in Rust** sits in an
under-occupied niche — and the adjacent neglected layer (operational/RPC privacy) is also
wallet-controllable and Rust-native. Whether and how to occupy that niche is a product-planning question,
not a research conclusion.

## Consolidated open questions

The sharpest unresolved items pulled across files (full lists in each file's "Open questions"):

- **EOA vs smart account:** Is a smart-account/7702 layer worth adopting *purely* to gain chain-enforced
  spend caps and ERC-7715 session keys, or do local software-enforced limits suffice for an EOA operator,
  and under what threat model? `[01][02][05]`
- **Policy boundary:** What does a defensible policy engine for an LLM signer look like in a local-first
  app with no TEE — separate signing process, OS sandbox, future enclave dependency — and how much can be
  enforced on-chain (permissions) vs locally? `[05][08]`
- **Autonomy fencing:** How to configure the "fenced area" (per-tx approval vs daily budgets vs
  allowlists) without collapsing agent speed to human speed? `[05]`
- **MCP transport:** stdio vs local HTTP for a native GPUI app, and how to process-isolate the policy gate
  from the model context? `[05]`
- **Hardware-backed signer path:** OS keystore + Touch ID for *unlock* vs an on-chain P256/passkey signer
  that *requires a smart account* — these are distinct concerns. `[01][08]`
- **Privacy posture:** which compliance model (allowlist/blocklist/threshold), shielded-by-default vs
  opt-in, and the desktop UX/perf cost of an embedded Helios light client vs privacy-respecting hosted RPC?
  Will EIP-8182 make the Hegota cut with an EOA-compatible ECDSA path? `[06]`
- **Real adoption signal:** primary-sourced 7702-delegation and ERC-5792/7715 adoption curves (vs
  vendor/WalletConnect-routed samples); x402's daily-volume trajectory after its early-2026 decline. `[01][02][05]`
- **Kohaku as a dependency:** are its Rust crates consumable standalone with a stable API and clear
  license, given it's an EF GitHub-org project with no formal product launch? `[03][06]`
