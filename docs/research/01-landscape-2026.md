# The 2026 Wallet Landscape — State of the Art

> A language-agnostic survey of Ethereum wallet architecture, standards, security, and product direction circa mid-2026. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- Account abstraction (AA) split into two tracks that now **compose rather than compete**: ERC-4337 supplies off-protocol infrastructure (EntryPoint, bundlers, paymasters), and EIP-7702 lets an existing EOA delegate to that infrastructure without changing address. The shipped production pattern is "7702 + 4337 together." [1][2][9][16]
- EIP-7702 went live on Ethereum mainnet on **May 7, 2025** in the Pectra hard fork (epoch 364032). It adds transaction type `0x04`, writing a persistent `0xef0100 || address` delegation pointer while the EOA keeps its address and key. [1][2][3]
- EIP-7702 has **no native gas sponsorship**; paymasters come from layering ERC-4337 on top. [2][16]
- ERC-4337 EntryPoint **v0.8.0** (March 26, 2025, `0x4337084d9e255ff0702461cf8895ce9e3b5ff108`) added native EIP-7702 handling and shipped the audited minimal **Simple7702Account** any EOA can delegate to. [4][10]
- The frontier of differentiation is the surrounding **standards mesh**: ERC-5792 (batched/atomic calls + capabilities), ERC-7715/7710 (scoped, time-boxed delegated permissions), ERC-7811 (unified balance), ERC-7683 (cross-chain intents), ERC-7730 (clear signing), and RIP-7212 (cheap on-chain passkeys). [5][6][7][8][11][13]
- MetaMask shipped ERC-7715/7710 **"Advanced Permissions"** to production on **April 6, 2026**, explicitly naming AI agents, subscriptions, DCA, vesting, and auto-compounding as use cases. Granting is an EIP-712 signature, not an on-chain tx. [6][17]
- Most wallets still lack the full stack in practice: EOAs remain dominant, ERC-5792/7715 are a sliver of real traffic, clear-signing coverage is partial, and cross-chain module portability is unsolved. [12][18][22]
- The agentic-payments layer (ERC-8004 trustless agent identity + x402 micropayments + session-key delegation) is the literal blueprint for an LLM operator wallet — but **none of it works on a bare EOA**; it requires keystore + smart-account/7702 + session-key layers. [19][20]
- Native protocol-level AA (EIP-8141) is proposed and "Considered for Inclusion" for the late-2026 Hegota fork, but is **not shipped and not a confirmed headliner**. 4337+7702 is the only shipped path through 2026. [21]

## Account abstraction in practice: ERC-4337 + EIP-7702 post-Pectra

Two composing tracks define what users actually get in 2026.

**ERC-4337** (Finalized; live on mainnet since ~March 1, 2023) is the off-protocol model: an `EntryPoint` singleton validates "UserOperations" submitted by bundlers, with optional paymasters for gas sponsorship. ethereum.org cites over 26M smart wallets and over 170M UserOperations as a point-in-time, lower-bound snapshot. [9]

**EIP-7702** (Standards-Track Core, created May 7 2024, activated in Pectra on May 7, 2025) introduces transaction type 4: an EOA signs an authorization tuple `(chain_id, address, nonce)` that writes a persistent `0xef0100 || address` delegation pointer into the account, so the EOA executes a chosen contract's code while keeping its address and private key. The original key retains full control; delegation resets by pointing at the null address. [1][2]

What a 7702-upgraded EOA gains: atomic transaction batching, gas sponsorship and pay-gas-in-token (via 4337 paymasters), session keys, and recovery logic — without migrating address. Crucially, EIP-7702 includes no gas-sponsorship mechanism of its own; it only enables sponsorship architecturally, and production wallets borrow 4337 paymasters. The prevailing architecture is therefore "7702 + 4337 together": keep EntryPoint/bundlers/paymasters, drop per-user contract deployment. [2][16]

⚠ unverified: a widely repeated secondary figure of "~14M EOAs signed at least one 7702 authorization" appears in aggregations but was not confirmed against a primary on-chain census; treat the magnitude as indicative, not exact. The Block independently reported 11,000+ authorizations in Pectra's first week. [22]

### EntryPoint versioning and the 7702 bridge

The canonical `eth-infinitism/account-abstraction` repo is the reference. EntryPoint v0.7 (`0x0000000071727De22E5E9d8BAf0edAc6f37da032`) moved simulation off-chain and was in production through 2024. **v0.8.0** (March 26, 2025; `0x4337084d9e255ff0702461cf8895ce9e3b5ff108`) is the pivotal release for operator-wallet relevance: it added native EIP-7702 authorization handling (the UserOp hash incorporates the 7702 delegation address) and introduced **Simple7702Account**, a fully audited minimal contract (ERC-165/721/1155/1271/4337) that any EOA can safely delegate to. A later v0.9.0 added parallelizable paymaster signing and a `paymasterSignature` field. Infra providers (Pimlico, Alchemy, ZeroDev, Biconomy, Gelato) serve both 4337 and 7702 from one stack. OtterSec (Dec 2025) documents "hidden risks" in paymasters (griefing/accounting bugs) — relevant security reading. [4][10][16][27]

## Native protocol AA on the roadmap (EIP-8141)

The frontier item is native, protocol-level AA. **EIP-8141** ("omnibus" AA) was created in the canonical `ethereum/EIPs` repo on **Jan 29, 2026** (Draft) with authors including Vitalik Buterin; Vitalik's public Ethereum Magicians unveiling followed on **Feb 28, 2026**. It introduces type-`0x06` "frame transactions" that separate a transaction into a verification phase and an execution phase (the spec defines frame MODE values DEFAULT/VERIFY/SENDER; "verification/execution" here is a paraphrase, not literal mode names), enabling native sponsored fees, multisig, alternative/quantum-resistant signatures, and gas in non-ETH tokens **without a separate bundler layer**. As of a late-March 2026 All Core Devs call, EIP-8141 holds **"Considered for Inclusion" (CFI)** status for the late-2026 Hegota fork — explicitly NOT a confirmed headliner (FOCIL is). The prior Glamsterdam fork headlines ePBS and Block-Level Access Lists, not AA. Bottom line: native AA is experimental, not shipped; 4337+7702 is the only shipped path through 2026. [21]

## Passkey / WebAuthn signers (RIP-7212 / P256)

Passkeys are now a mainstream signer option, enabled by **RIP-7212** — a precompile for secp256r1 (P256) verification taking `(hash, r, s, x, y)` at exactly **3,450 gas**. The "~100x cheaper" framing is specifically versus pure-Solidity P256 verification; the spec actually benchmarks the precompile as ~15% *slower* than `ecrecover`. RIP-7212 live status is firmly verified for **Arbitrum** (ArbOS 31 "Bianca," Finalized AIP); Optimism, Polygon, zkSync, and Kakarot are reported as committed/implemented via secondary aggregation. [7][13]

Because P256 is what Apple Secure Enclave, Android Keystore, and browser WebAuthn use, a smart account can use a hardware-backed biometric passkey as a signer with no seed phrase. Coinbase Smart Wallet / Base Account is the flagship (passkey primary signer, iCloud/Google sync, multi-owner). For a native desktop app, the analogous primitive is OS secure storage + Touch ID — but **on-chain passkey signers require a smart account**, which a v0 EOA cannot do without a 7702 delegation. RIP-7212 only matters once smart-account support exists. [13][15]

## Recovery, multisig, gas, and batching

| Capability | Standard / product | State in 2026 |
|---|---|---|
| Multisig | Safe (M-of-N, Modules, Guards) | Dominant; signers can be EOAs, passkeys, hardware [23] |
| Social/guardian recovery | Argent (2018-origin pattern); 7702 delegation | Production; Starknet on-chain recovery reported partly offchain-only [25] |
| Sponsored gas | ERC-4337 verifying paymaster; ERC-7677 API | Table-stakes; used as acquisition lever [14][28] |
| Pay gas in token | ERC-20 paymaster (Circle, Pimlico, ZeroDev) | USDC ≈62% of ERC-20 paymaster volume Q1 2026 (vendor-reported) [14] |
| Batched / atomic calls | ERC-5792 (`wallet_sendCalls`) | Spec non-Final; real usage a tiny fraction of traffic [5][18] |

**Recovery** is the headline reason smart accounts beat raw EOAs. A raw EOA (Deckard v0) has none of it: losing the key loses funds. **ERC-20 paymasters** are strategically important for an operator wallet — an autonomous agent can transact purely in stablecoins it holds, never needing the user to top up ETH. **ERC-5792** (`wallet_sendCalls` with an `atomicRequired` flag, plus `wallet_getCapabilities` for fingerprint-free feature discovery) is the surface enabling one-click approve+swap, but per WalletConnect/Reown tracking (WalletConnect-routed traffic only, not a neutral census), it remains a sliver versus legacy `eth_sendTransaction`/`personal_sign`. An EOA typically needs 7702 before it can batch atomically. [5][18][14]

## Session keys & granular permissions (ERC-7715 / ERC-7710)

This is the single most important standard cluster for an operator wallet. **ERC-7715** defines `wallet_grantPermissions`: an agent requests scoped authority and the wallet returns a permission, scoped by asset, amount, time window, and pattern, shown in plain language, granted via an **EIP-712 signature (not an on-chain tx)**. **ERC-7710** provides the underlying delegation framework (delegation chains, sub-delegation). MetaMask shipped this as **Advanced Permissions** on April 6, 2026 (requires a MetaMask Smart Account, not a bare EOA), with three types — **Periodic** (resets each period: subscriptions/DCA), **Streaming** (linear allowance: vesting), and **Revocation** — explicitly listing **AI agents** as a use case. ZeroDev (Kernel) and Biconomy (Nexus) also offer session keys. The pattern for an LLM agent: hold a session key bound by a 7715/7710 permission (e.g., "spend up to X USDC/day on DEX Y for 30 days") so the autonomous layer never touches the root key. [6][17][26]

## Intents, chain abstraction, unified balance (ERC-7683 / ERC-7811)

Chain abstraction — hide chains, show one balance, execute cross-chain from one signed intent — is described as 2026's dominant UX paradigm across three layers: Account, Execution (intents + solvers), and Liquidity. **ERC-7683** (co-authored by Uniswap Labs and Across, created April 11, 2024) lets solvers serve many protocols without bespoke integrations; it is **in DRAFT status, not Final**, and its current spec has materially evolved to a Steps/variables/payments model with an `IResolver` interface — the original "CrossChainOrder struct + ISettlementContract" framing is out of date. Production endpoints include Across, UniswapX, CoW, and Eco. [8]

"~88% of Across volume via ERC-7683" and the "Q3 2025 solver migration" are now confirmed against Across's own first-party docs — still a vendor self-report, not a neutral third-party dashboard. [35]

**ERC-7811** (`wallet_getAssets`, authored Nov 2024) is the primitive behind a single unified-balance number. Remaining gaps: thin-liquidity chains lack solvers, and smart-account modules/standards do not port cleanly across chains. [36]

## Embedded / MPC wallets vs. local self-custody

Wallet-as-a-service (Privy, Dynamic, Turnkey, Coinbase WaaS, Magic) optimizes onboarding: email/social sign-up, no seed phrase. The dominant model is **TEE + key-sharding**: Privy (acquired by Stripe, June 2025) generates the key inside a Trusted Execution Environment and splits it via Shamir's Secret Sharing into a 2-of-2 (enclave share + auth share); the key is reconstructed only briefly inside the enclave at signing and immediately wiped, so no single party — including Privy — ever holds the whole key. Turnkey markets an explicit "AI Agents" product with policy-gated signing. The architecturally interesting borrow for an operator wallet is the **policy engine**: a programmable allow/deny ruleset gating what the agent's signer can do — a layer a fully local single-keypair EOA lacks until it adds keystore + session-key tiers. [13]

## Security: simulation, clear-signing (ERC-7730), revoke tooling

The modern baseline is simulate-before-sign + clear-signing + risk scanning + approval management. Rabby is the reference UX (simulates every tx via the Tenderly Simulation API, scores approvals, surfaces revoke tooling); Blockaid is the dominant risk engine (integrated server-side into MetaMask and into WalletConnect). **ERC-7730 "clear signing"** had its governance transferred from Ledger to the Ethereum Foundation; the registry (`ethereum/clear-signing-erc7730-registry`) is live (~102 stars, ~357 commits, dozens of open PRs/issues) but coverage is **partial**, so most contracts still produce blind-signing. [11][24][29]

Threat backdrop: per Scam Sniffer, phishing/drainer losses fell ~83% YoY in 2025 to ~$83.85M (~106k victims), while signature-phishing spiked ~207% MoM in January 2026 (~$6.27M, ~4,741 victims). Note the figures trace to a single vendor (medium confidence on exact dollar amounts). A CMU CyLab study **published** Jan 2026 reported 270M+ address-poisoning attempts against 17M+ wallets — but that figure covers the **July 2022–June 2024 dataset period**, not January 2026 activity. [12]

## Agentic / AI-agent wallet primitives

A distinct 2025–2026 frontier targets autonomous on-chain agents. **ERC-8004 "Trustless Agents"** (canonical EIP, Draft, created Aug 13, 2025; mainnet ~Jan 29, 2026) defines on-chain Identity (ERC-721-based), Reputation, and Validation registries so agents are discoverable and trust-scored without a central intermediary. **x402** revives HTTP 402 for HTTP-native USDC micropayments. Together with 4337/7702 session-key delegation and ERC-7715/7710 scoped permissions they form an end-to-end loop: discover a service (8004), receive a 402 with terms, pay in USDC via a bounded session key, reputation-log the interaction. Each component is independently real; the unified "payment loop" is an architectural narrative from secondary sources, not one normative spec. None of it is possible on a bare v0 EOA. [19][20]

## Privacy: EF Kohaku SDK

The Ethereum Foundation's **Kohaku** initiative (unveiled ~Oct 8, 2025; part of a 47-member EF Privacy Cluster) is an open-source, modular privacy SDK that integrates shielded-pool protocols (Railgun, Privacy Pools) and per-dapp addresses directly into the wallet layer, with ERC-4337 relaying operational. It dovetails with an operator-wallet model: an agent transacting across many dapps benefits from per-dapp address isolation. (Reported via reputable crypto press; medium confidence pending a single canonical EF page per sub-claim.) [30]

## Modular smart accounts: ERC-7579 & ERC-6900

Beneath the user-facing features sits an account-modularity layer. **ERC-7579** (ratified 2024) is the de-facto modular standard, defining a shared ABI for validator/executor/hook/fallback modules; it underpins ZeroDev Kernel and Biconomy Nexus (both 7579 + EIP-7702 compatible). **ERC-6900** (Alchemy-led) is a competing standard. The practical 2026 problem: modular standards have **not** unified cross-chain, and EIP-1271 (smart-account signature validation) is still not universally honored by older dapps, creating a fragmented smart/legacy experience. [22]

## What this means for Deckard

- Deckard's v0 (a single alloy-generated secp256k1 EOA in the OS config dir) sits on the legacy side of the smart/legacy split: it has no batching, no recovery, no session keys, and no policy engine — the same gaps that smart accounts exist to close. [22]
- Every operator-wallet primitive surveyed (scoped session keys, ERC-20 gas payment, on-chain agent identity, simulate-before-sign co-signing) presupposes either a smart account or a 7702-delegated EOA; on a bare EOA none of them are reachable. [6][19]
- EIP-7702 is the lowest-friction bridge from an EOA to the smart-account feature set because it preserves the existing address and key — relevant given Deckard's locked-in keystore plans. [1][2]
- The desktop/native posture maps cleanly to OS-level secure storage + Touch ID for unlock, but on-chain passkey signing (RIP-7212) is a smart-account-only capability, so biometric unlock and on-chain passkey signers are distinct concerns. [7][13]
- ERC-7715/7710 in production (MetaMask, April 2026) demonstrates the exact pattern an LLM operator layer needs — a tightly scoped, time-boxed, revocable permission granted by signature — and is the closest shipped analog to Deckard's vision. [6][17]
- Clear-signing (ERC-7730) and transaction simulation are language-agnostic, EOA-compatible security features whose value increases when a non-human (LLM) is in the signing loop; the EF registry and `erc7730` validator are direct integration targets. [11][24]
- The white space: essentially no shipping consumer wallet offers safe, scoped, revocable LLM-operator control end-to-end as a product — it exists today only as infra-provider plumbing (Turnkey, Cobo) plus MetaMask's just-launched permissions feature. [22]
- Stablecoin-native onramps plus ERC-20 paymasters mean a wallet could in principle be funded and operated entirely in USDC without the user ever holding ETH — observationally aligned with an agent that transacts in stablecoins it already holds. [14]

## Open questions

- What is the actual, primary-sourced count of 7702-delegated EOAs and the real adoption curve of ERC-5792/7715 in on-chain traffic (vs. WalletConnect-routed samples)?
- Does EIP-8141 (native AA) advance from CFI to scheduled inclusion in Hegota, and if so, how does it change the 4337+7702 architecture a wallet should bet on?
- For a native (non-browser) desktop wallet, what is the cleanest path to a hardware-backed signer — OS keystore + Touch ID for local unlock vs. an on-chain P256/passkey signer requiring a smart account?
- Which modular-account substrate (Safe vs. Kernel vs. Nexus, ERC-7579 vs. ERC-6900) best supports session-key validators, recovery modules, and spend-limit hooks without forking the core account — given cross-chain module portability is unsolved?
- How mature and audited is the agentic stack (ERC-8004 + x402 + session keys) for real funds, and what is its incident/exploit history?
- What does a defensible policy engine for an LLM signer look like (allow/deny rules, rate/spend limits, simulation gating) and how much can be enforced on-chain via permissions vs. locally in the wallet?

## Sources

1. Pectra 7702 guidelines — https://ethereum.org/roadmap/pectra/7702/ — (docs, high)
2. EIP-7702: Set Code for EOAs — https://eips.ethereum.org/EIPS/eip-7702 — (spec, high)
3. Pectra Mainnet Announcement — https://blog.ethereum.org/2025/04/23/pectra-mainnet — (docs/primary, high)
4. Releases — eth-infinitism/account-abstraction — https://github.com/eth-infinitism/account-abstraction/releases — (github, high)
5. EIP-5792: Wallet Call API — https://eips.ethereum.org/EIPS/eip-5792 — (spec, high)
6. ERC-7715: Request Permissions from Wallets — https://eips.ethereum.org/EIPS/eip-7715 — (spec, high)
7. RIP-7212: Precompile for secp256r1 — https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md — (spec, high)
8. ERC-7683: Cross Chain Intents (canonical, Draft) — https://eips.ethereum.org/EIPS/eip-7683 — (spec, high)
9. Account abstraction — https://ethereum.org/en/roadmap/account-abstraction/ — (docs, high)
10. eth-infinitism/account-abstraction (repo) — https://github.com/eth-infinitism/account-abstraction — (github, high)
11. ethereum/clear-signing-erc7730-registry — https://github.com/ethereum/clear-signing-erc7730-registry — (github, high)
12. Scam Sniffer 2025 phishing-losses report — https://drops.scamsniffer.io/scam-sniffer-2025-crypto-phishing-losses-fall-83-to-84-million/ — (vendor report, medium)
13. How Privy embedded wallets work — https://privy.io/blog/how-privy-embedded-wallets-work — (blog, high)
14. Circle Paymaster — Pay Gas in USDC — https://www.circle.com/paymaster — (docs, high)
15. coinbase/smart-wallet — https://github.com/coinbase/smart-wallet — (github, high)
16. ERC-4337 vs EIP-7702 — https://docs.pimlico.io/guides/eip7702/erc4337-vs-eip7702 — (docs, high)
17. Introducing MetaMask Advanced Permissions — https://metamask.io/news/introducing-advanced-permissions — (blog/primary, high)
18. EIP-5792: The UX Breakthrough Everyone's Ignoring — https://walletconnect.com/blog/eip-5792-the-ux-breakthrough-everyone-s-ignoring — (blog, medium)
19. ERC-8004: Trustless Agents (canonical EIP) — https://eips.ethereum.org/EIPS/eip-8004 — (spec, high)
20. What ERC-8004 unlocks for agent infrastructure — https://www.allium.so/blog/onchain-ai-identity-what-erc-8004-unlocks-for-agent-infrastructure/ — (blog, medium)
21. EIP-8141: Native Account Abstraction (Frame Transactions) — https://github.com/ethereum/EIPs/blob/master/EIPS/eip-8141.md — (spec, high)
22. EOA vs Smart Wallets in 2026 — https://www.openfort.io/blog/eoa-vs-smart-wallet — (blog, medium)
23. Safe Modules — https://docs.safe.global/advanced/smart-account-modules — (docs, high)
24. Tenderly x Rabby transaction preview — https://github.com/Tenderly/tenderly-rabby-transaction-preview — (github, high)
25. About wallet recovery — Argent — https://support.argent.xyz/hc/en-us/articles/360022631412-About-wallet-recovery — (docs, high)
26. Paying Gas with ERC20s / 7702 quickstart — ZeroDev — https://docs.zerodev.app/sdk/core-api/pay-gas-with-erc20s — (docs, high)
27. ERC-4337 Paymasters: Better UX, Hidden Risks — OtterSec — https://osec.io/blog/2025-12-02-paymasters-evm/ — (blog, high)
28. ERC-7677: Paymaster Web Service Capability — https://github.com/ethereum/ERCs/blob/master/ERCS/erc-7677.md — (spec, high)
29. The Evolution of Clear Signing — Ledger — https://www.ledger.com/blog-the-evolution-of-clear-signing — (blog, high)
30. EF Kohaku SDK for wallet-level privacy — The Defiant — https://thedefiant.io/news/blockchains/ethereum-foundation-kohaku-sdk-privacy-wallet-integration-bb4t52 — (news, medium)
31. ethereum/RIPs — https://github.com/ethereum/RIPs — (github, high)
32. fireblocks-labs/awesome-eip-7702 — https://github.com/fireblocks-labs/awesome-eip-7702 — (github, medium)
33. ethereum/kohaku — https://github.com/ethereum/kohaku — (github, medium)
34. Smart wallet adoption surges after Pectra — The Block — https://www.theblock.co/post/354414/smart-wallet-adoption-surges-after-pectra-upgrade — (news, medium)
35. ERC-7683 in Production — Across docs — https://docs.across.to/developer-quickstart/erc-7683-in-production — (docs/vendor self-report, medium)
36. ERC-7811: Wallet Asset Discovery (wallet_getAssets) — https://eips.ethereum.org/EIPS/eip-7811 — (spec, high)
