# Agentic & LLM-driven Wallets

> Landscape of LLM-driven crypto wallets — agent-wallet frameworks, MCP integration surfaces, payment/identity standards, and safe-signing architecture. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- The agentic-crypto stack has consolidated around one safety axiom: **the agent never sees the seed**. The LLM is a scoped signer; the key stays isolated (TEE or a custody/signing service) behind a policy gate it cannot bypass [1][2].
- **MCP (Model Context Protocol) is the dominant integration surface.** A local stdio/HTTP daemon that exposes wallet ops (`simulate`, `sign`, `transfer`, `set spend limit`) as LLM tools is now the standard pattern — Coinbase Payments MCP, the relaunched Base MCP, GOAT MCP, standalone EVM MCP servers, and Alchemy MCP all follow it [3][4][9][10].
- **Coinbase shipped this as product.** Agentic Wallets (Feb 11, 2026) put agent keys in Trusted Execution Environments with three named guardrails: session caps, transaction limits, and enclave isolation [1].
- **The canonical safe-signing toolkit:** simulate-before-sign, scoped + expiring session permissions (ERC-7715/7710), onchain-enforced policy (limits/allowlists), human-in-the-loop approval for write actions, and key isolation [1][2][12][13].
- **Dual-key architecture** is the recurring blueprint: an operational *agent key* (scoped, often TEE-sealed) plus a non-custodial *owner key* that retains override (halt, withdraw, modify permissions), both gated by a smart-contract wallet [2].
- **Payment standards shipped fast.** x402 (HTTP-402 stablecoin payments) was donated to a Linux Foundation **x402 Foundation launched April 2, 2026** with Google/Microsoft/AWS/Visa/Mastercard/Amex/Stripe/Cloudflare/Circle/Shopify backing [5][6].
- **Google AP2** (Sept 16, 2025) adds cryptographically signed "Mandates"; its A2A-x402 extension is the production crypto path. AP2 itself is payment-agnostic — it does **not** mandate crypto [7][8].
- **ERC-8004 "Trustless Agents"** (Draft, Aug 13, 2025) defines onchain Identity/Reputation/Validation registries; reference contracts (CC0) are deployed on 30+ chains, though the Validation Registry is still in flux [11][12].
- Bounded session-signing (ERC-7715/7710) is shipping in MetaMask today — but the **granting** wallet must be a smart account (the session/agent account can be an EOA or smart account). Deckard is a plain EOA, so it can't grant 7715 permissions the way a MetaMask Smart Account can; adopting the semantics means either a smart-account layer or replicating scope/expiry/limit checks locally [14].

## Agent-wallet frameworks: convergence on the scoped-signer model

Across vendors, the architecture is strikingly uniform. The agent is given a *scoped signer*, not the master key, and a policy layer (ideally onchain) caps what that signer can do.

**Coinbase Agentic Wallets** (shipped Feb 11, 2026) is the canonical product expression of the "agent never sees the seed" model. Agents operate non-custodial wallets whose keys live inside Trusted Execution Environments, with three named security pillars — *session caps* (max spend per session), *transaction limits* (per-tx size), and *enclave isolation* (private keys in secure Coinbase infrastructure, never exposed to the prompt or LLM). It includes gasless settlement on Base and native x402 support [1]. Architecturally these are CDP Server Wallet v2: the key is split via Coinbase's `cb-mpc` library (threshold keyshares held between Coinbase and the operator) and the MPC operation runs inside an AWS Nitro Enclave — so it is MPC + TEE. "Enclave isolation" is the branded third pillar, not a denial that MPC is used [29].

**Crossmint** articulates the safety blueprint most explicitly as a **dual-key, two-layer** model [2]:

| Layer | Key | Role | Property |
|---|---|---|---|
| Operational | Agent Key | Signs only criteria-meeting txs, deployed in its own TEE | "Encrypted in memory, inaccessible to host"; can't be leaked |
| Override | Owner Key | Master "emergency brake" — halt agent, withdraw, modify perms | Non-custodial with user (passkey/MetaMask/embedded) |

Both interact with a smart-contract wallet (ERC-4337 on EVM, Squads on Solana) whose modules enforce guardrails. The stated rationale: the agent never holds full custody, eliminating the "honeypot" risk where one compromised agent key drains everything [2].

**Coinbase AgentKit** (`coinbase/agentkit`, tagline "Every AI Agent deserves a wallet") is the most mature open-source toolkit: framework-agnostic (LangChain, Vercel AI SDK, MCP, OpenAI Agents SDK, Pydantic AI, Strands, AutoGen, Eliza) and wallet-agnostic (CDP, Privy, and Viem — the Viem path being plain self-custodial EOA signing). It ships 50+ TS / 30+ Python action providers across Base/Ethereum/Solana, and "spend permissions" that limit token, amount, and time period [15][16]. Architecturally it separates the *agent skill modules* (authenticate/fund/send/trade) from the *wallet layer* (who can sign what) — a clean seam Deckard can mirror. Its public `WISHLIST.md` signals roadmap: Claude MCP support, more frameworks (CrewAI, Mastra, AutoGPT), Turnkey/Lit wallet providers, XMTP agent comms, and smart-wallet spend-permission actions [17]. Current versions: TS `@coinbase/agentkit` 0.10.x, Python `coinbase-agentkit` 0.7.x [27][28].

**GOAT SDK** (`goat-sdk/goat`, by Crossmint, MIT) is the broadest open-source onchain-actions library — its current README claims 200+ tools across 30+ chains and 10 framework adapters (including MCP) [18]. It is wallet-architecture-agnostic (self-custodial via Viem/Web3, smart-wallet via Safe/Lit, custodial via Crossmint) and deliberately minimal-core: install only the tools you need — a good model for a Rust port. Note: GOAT does **not** manage keys; it plugs into whatever wallet you give it, which is precisely Deckard's seam [18]. ⚠ unverified: the "200+ integrations / 10 framework adapters" figures come from the evolved current README, not the originally cited launch blog (which states only 30+ chains / 5 frameworks); the community `goat-mcp` wrapper that demonstrates exposing GOAT to Claude Desktop is a **0-star, single-commit demo**, not a mature project (the 993-star count belongs to the main `goat-sdk/goat` repo) [19].

**Thirdweb AI / Nebula** is a competing service-backed approach: a proprietary blockchain model ("t1") that reads/writes/reasons onchain across 2500+ EVM chains, exposed via an MCP server [20]. Relevant as an "AI that transacts onchain" reference point, though it is service-backed rather than local-first.

## MCP as the integration surface

Every major player now exposes wallet operations to an LLM over MCP — a local or hosted daemon presenting wallet ops as callable tools. Two distinct security postures emerge:

**Key-isolated (the safe pattern).** *Coinbase Payments MCP* runs locally on desktop, needs no API key, and lets Claude/Gemini/Codex create a wallet by email, onramp, pay via x402 (with a "Bazaar Explorer" to discover payable APIs), and set user-approved spend limits and approval thresholds [3]. *Base MCP* is the strongest human-in-the-loop example: the original `coinbase/base-mcp` repo is **archived/deprecated** (the org moved from "coinbase" to "base"; it now lives at `base/base-mcp-legacy`), and Base **relaunched** "Base MCP" (~May 26, 2026) connecting any AI to a Base Account where **every write action requires explicit user approval** — the MCP returns an `approvalUrl` + `requestId`, the user reviews a simulation of asset changes, and the assistant polls `get_request_status()` until confirmed. The smart wallet signs server-side; private keys are never exposed to the AI layer [4]. ⚠ unverified: the literal "OAuth 2.1" wording and "private keys never exposed / smart wallet signs server-side" phrasing come from the Base blog and Fortune coverage, not the `docs.base.org/ai-agents` page itself, which confirms the approval flow but not those exact terms.

**Direct-key (the anti-pattern to improve on).** Standalone local EVM MCP servers handle keys directly via env vars and run over stdio — the closest open analog to a raw self-custodial signing sidecar. `mcpdotdirect/evm-mcp-server` exposes 22 tools + 10 prompts across 60+ chains (`transfer_native`, `transfer_erc20`, `approve_token_spending`, `write_contract`, `sign_message`, `sign_typed_data`), keyed by `EVM_PRIVATE_KEY` / `EVM_MNEMONIC`, over stdio (default) or HTTP/SSE on port 3001 [10]. `dcSpark/mcp-cryptowallet-evm` (ethers v5) supports wallet create/import (private-key/mnemonic/encrypted-JSON), send/sign, EIP-712, and ENS [10]. Both warn never to commit keys and say keys are used only for signing, never stored — but the raw key still sits in env/process memory accessible to the LLM tool layer, the opposite of the TEE models [10].

**Read/observe half.** *Alchemy MCP* (`alchemyplatform/alchemy-mcp-server`, released May 10, 2025) is the data side of an operator loop: ~159 tools across 100+ networks (prices, NFT metadata, tx history, holdings, contract simulation, tracing, account-abstraction), hosted via OAuth or local via API key, and can drive webhooks so agents react to onchain events without polling [21]. Pairing read-heavy data tools with a tightly-scoped signing tool is the natural decomposition.

## Payment & identity standards

**x402** embeds stablecoin payments into HTTP. Flow: client requests a resource → server returns `402` with payment details → client builds a `PaymentPayload`, re-sends with a signature → a *Facilitator* verifies and settles → server returns `200` + resource. It supports EVM, Solana, and Stellar (USDC on Base most common), with SDKs in TS, Python, and Go. Open-sourced by Coinbase May 2025, it was donated to the Linux Foundation **x402 Foundation, launched April 2, 2026**, with members spanning Google, Microsoft, AWS, Visa, Mastercard, Amex, Stripe, Cloudflare, Circle, Shopify, and others; the canonical repo moved to `github.com/x402-foundation/x402` (`coinbase/x402` is now a development fork) [5][6][22]. ⚠ unverified: vendor/aggregator figures of ~69k active agents / 165M transactions / ~$50M cumulative volume (late Apr 2026) are **not traceable to an official x402 dashboard** and the originally cited source reports different numbers. On trajectory, the Chainalysis report says growth "moderated" and reports 100M+ cumulative transactions [23]. The sharper claim of a **~92% drop in *daily* x402 transactions** from Dec 2025 (~731k/day) to Feb 2026 (~57k/day) comes only from the **low-reliability** blockeden.xyz blog, not Chainalysis — treat those daily figures with caution even as cumulative totals grew [23].

**Google AP2 (Agent Payments Protocol)** launched Sept 16, 2025 with 60+ partners (Mastercard, Amex, PayPal, Coinbase, Mysten Labs, et al.). It is payment-agnostic and addresses three trust gaps — Authorization, Authenticity, Accountability — via **Mandates**, tamper-proof cryptographically-signed contracts signed by verifiable credentials: an *Intent Mandate* (captures user intent and delegation rules: price limits, timing, conditions) and a *Cart Mandate* (signed after exact items + price, creating an unchangeable record). The crypto path is the A2A-x402 extension (`google-agentic-commerce/a2a-x402`, built with Coinbase/EF/MetaMask), described as production-ready. AP2 extends A2A and MCP and does **not** mandate crypto — x402 is an optional extension [7][8]. The Mandate concept maps directly onto Deckard: a signed, scoped pre-authorization the LLM operates under.

**ERC-8004 "Trustless Agents"** (Draft ERC, created Aug 13, 2025; authors from MetaMask, EF, Google, Coinbase) defines a minimal onchain trust layer via three registries: **Identity** (ERC-721, portable agent ID; registration file lists A2A cards, MCP endpoints, ENS, DIDs, wallet addresses — so MCP/A2A endpoints are first-class), **Reputation** (signed feedback), and **Validation** (0–100 scores via stake-secured re-execution, ZK proofs, or TEE oracles) [11]. Reference contracts (CC0) are deployed across 30+ EVM networks with `0x8004…` vanity addresses, but the Validation Registry is "still under active update and discussion with the TEE community" and there are **no formal releases** [12].

## Safe-signing architecture

The consensus stack, synthesized from primary vendor guidance:

1. **Simulate everything.** Dry-run each action in a forked/simulated environment to compute asset/balance changes (with USD values), gas, decoded traces, and human-readable warnings; block execution if slippage, approvals, or calldata deviate from intent. Tenderly's Simulation API provides this and is already wired into MetaMask Snaps and Rabby's sign modal [13].
2. **Execute validated intents, not raw LLM suggestions.**
3. **Scoped, expiring permissions (ERC-7715/7710).** ERC-7715 (Draft, May 2024) adds a wallet-side JSON-RPC method (`wallet_requestExecutionPermissions`, earlier `wallet_grantPermissions`) to grant a session account scoped permissions — `native-token-allowance`, `erc20-token-allowance` (spend limits), `ExpiryRule` (unix timestamp). It pairs with ERC-7710 delegation: the grant returns a `delegationManager` + context blob, and the agent redeems via `redeemDelegation` to execute within bounds, offline, without exposing the main wallet. Canonical example: authorize an agent to spend up to 10 USDC/day to DCA into ETH for 30 days with one signed permission. MetaMask ships this as "Advanced Permissions." The **granting** wallet must be a smart account (per MetaMask docs the session/agent account can be an EOA or smart account) [14].
4. **Human-in-the-loop for writes** — but with a known tension: agents make hundreds of decisions/minute, so per-signature prompts collapse agent speed to human speed. The resolution is a "fenced area" where the agent acts freely inside scoped bounds while the human retains override/revocation outside [13].
5. **Key isolation** — the agent never sees the seed; signing happens behind a TEE or a policy-checking signing API.

An emerging local pattern packages exactly this: `1lystore/dcp` describes itself as a "permission layer for AI agents — wallet signing, vault access, budgets, human approvals" [13]. It is now at v2.0.4 (May 2026) with a desktop app + CLI and active maintenance, though still small/niche (single-digit stars).

**Identity & metering frontier.** Skyfire offers a "Know Your Agent" (KYA) framework plus KYAPay USDC settlement, and Nevermined adds metering/business-logic layers atop x402/A2A/MCP/AP2 [24][25]. ⚠ unverified: the claim that Skyfire records KYA IDs as "ERC-8004-compliant onchain attributes" rests on **secondary** sources, not a Skyfire primary source — the cited Skyfire page describes JWT/OAuth2 identity, not blockchain attributes. KYAPay/USDC settlement is confirmed by Skyfire's own June 2025 release [26].

## What this means for Deckard

- The **local-MCP-sidecar + simulate + scoped-policy + key-isolation** pattern is proven and shipping today (Coinbase Payments MCP, Base MCP v2). Deckard's "local CLI/sidecar driving the wallet" idea is the same shape multiple vendors converged on independently [1][3][4].
- The industry's load-bearing safety axiom — **the LLM never touches the seed** — is directly at odds with the simplest open MCP servers, which place the raw private key in env/process memory reachable by the tool layer [2][10]. Deckard's planned encrypted keystore (Argon2id + XChaCha20-Poly1305) gives it a key-isolation boundary those servers lack.
- Deckard is an **EOA**, so the onchain-enforcement primitives (ERC-4337 spend caps, ERC-7715/7710 session keys) are unavailable without a smart-account layer. The same scope/expiry/limit *semantics* can be replicated locally in a policy engine between the LLM tools and the secp256k1 key — at the cost of being software-enforced rather than chain-enforced [2][14].
- The **dual-key split** (operational signer vs. master override) is a language-agnostic blueprint: it maps onto a Rust design where a bounded, policy-gated signing path is separate from the master seed, and a human-held override can halt or revoke [2].
- A native operator wallet could **pay x402 endpoints directly** for data/compute, and consume Mandate-style signed pre-authorizations (AP2) as the local policy object the LLM operates under [5][7].
- An **observe/act decomposition** fits the operator loop: read-only state/portfolio/simulation tools (an Alchemy-MCP-style data layer) paired with a separate, tightly-scoped local signing tool for writes [21][13].
- **Simulate-before-sign** is a self-contained safety primitive Deckard can adopt regardless of account type — compute expected asset changes, then block on deviation [13].
- The identity/reputation layer (ERC-8004, KYA) and fully-autonomous unattended signing remain **frontier**, not settled — production products still fence autonomy with human-in-the-loop and override keys [11][12][13].

## Open questions

- For an EOA with a software policy gate (no onchain enforcement), what threat model is acceptable — i.e., what can a compromised LLM tool layer still do if the seed is encrypted at rest but must be decrypted to sign?
- Is a smart-account layer (ERC-4337) worth adopting purely to gain chain-enforced spend caps and ERC-7715 session keys, given Deckard's EOA-today stance?
- How should the "fenced area" autonomy boundary be configured — per-transaction approval, daily budgets, allowlists, or a hybrid — without collapsing agent speed to human speed [13]?
- Does Deckard's operator vision need a verifiable onchain identity (ERC-8004) at all, or only when transacting with *other* agents/services?
- Which MCP transport (stdio vs. local HTTP) best fits a native GPUI desktop app, and how should the policy gate be process-isolated from the model context?
- What is x402's real adoption trajectory, given the reported daily-transaction decline in early 2026 despite cumulative growth [23]?

## Sources

1. Introducing Agentic Wallets — https://www.coinbase.com/developer-platform/discover/launches/agentic-wallets — (docs, high)
2. The AI Agent Wallet Problem: Why Your Architecture Needs Dual Keys — https://www.crossmint.com/learn/ai-agent-wallet-architecture — (blog, medium)
3. Payments MCP: Bringing Wallets, Onramps, and Payments to Every Agent — https://www.coinbase.com/developer-platform/discover/launches/payments-mcp — (docs, high)
4. Base AI Agents / Base MCP — https://docs.base.org/ai-agents — (docs, high); relaunch detail: https://blog.base.org/base-mcp and https://fortune.com/2026/05/26/coinbase-pushes-further-into-ai-payments-with-new-mcp-for-base-network/ — (blog/news, high/medium)
5. Linux Foundation launching the x402 Foundation — https://www.linuxfoundation.org/press/linux-foundation-is-launching-the-x402-foundation-and-welcoming-the-contribution-of-the-x402-protocol — (news, high)
6. coinbase/x402 (now a dev fork of x402-foundation/x402) — https://github.com/coinbase/x402 — (github, high)
7. Announcing Agent Payments Protocol (AP2) — https://cloud.google.com/blog/products/ai-machine-learning/announcing-agents-to-payments-ap2-protocol — (blog, high)
8. google-agentic-commerce/a2a-x402 (A2A x402 extension) — https://github.com/google-agentic-commerce/a2a-x402 — (github, high)
9. cryptoleek-team/goat-mcp (GOAT as a Claude-Desktop MCP server; 0-star demo) — https://github.com/cryptoleek-team/goat-mcp — (github, medium)
10. mcpdotdirect/evm-mcp-server — https://github.com/mcpdotdirect/evm-mcp-server — (github, high); dcSpark/mcp-cryptowallet-evm — https://github.com/dcSpark/mcp-cryptowallet-evm — (github, high)
11. ERC-8004: Trustless Agents (spec) — https://eips.ethereum.org/EIPS/eip-8004 — (spec, high)
12. erc-8004/erc-8004-contracts (reference registries, CC0) — https://github.com/erc-8004/erc-8004-contracts — (github, high)
13. Transaction Preview — Tenderly Documentation — https://docs.tenderly.co/simulations/transaction-preview — (docs, high); How to Build Onchain Agents — https://www.alchemy.com/blog/how-to-build-onchain-agents — (blog, high); 1lystore/dcp — https://github.com/1lystore/dcp — (github, medium)
14. ERC-7715: Request/Grant Permissions from Wallets (spec) — https://eips.ethereum.org/EIPS/eip-7715 — (spec, high); Advanced Permissions (ERC-7715) — https://docs.metamask.io/smart-accounts-kit/concepts/advanced-permissions/ — (docs, high)
15. coinbase/agentkit — https://github.com/coinbase/agentkit — (github, high)
16. AgentKit Overview — Coinbase Developer Documentation — https://docs.cdp.coinbase.com/agent-kit/welcome — (docs, high)
17. AgentKit WISHLIST.md (roadmap signals) — https://github.com/coinbase/agentkit/blob/master/WISHLIST.md — (github, high)
18. goat-sdk/goat — Great Onchain Agent Toolkit — https://github.com/goat-sdk/goat — (github, high)
19. Introducing GOAT — Crossmint blog — https://blog.crossmint.com/introducing-goat-great-onchain-agent-toolkit/ — (blog, high)
20. thirdweb-dev/ai (Nebula model "t1") — https://github.com/thirdweb-dev/ai — (github, high); thirdweb MCP Server docs — https://portal.thirdweb.com/ai/mcp — (docs, high)
21. alchemyplatform/alchemy-mcp-server — https://github.com/alchemyplatform/alchemy-mcp-server — (github, high); Alchemy MCP Server docs — https://www.alchemy.com/docs/alchemy-mcp-server — (docs, high)
22. Launching the x402 Foundation with Coinbase — Cloudflare blog — https://blog.cloudflare.com/x402/ — (blog, high)
23. Inside x402 — Agentic Payments on Base — https://www.chainalysis.com/blog/x402-agentic-payments-adoption/ — (analysis, high); x402 Foundation: payment layer for the AI internet — https://blockeden.xyz/blog/2026/03/05/x402-foundation-ai-payment-internet/ — (blog, low)
24. Skyfire KYA Protocol as identity layer for Experian's KYA framework — https://skyfire.xyz/skyfires-kya-protocol-is-now-the-identity-layer-for-experians-know-your-agent-framework/ — (blog, medium)
25. AI Agent Payment Systems — Nevermined — https://nevermined.ai/blog/ai-agent-payment-systems — (blog, low)
26. Skyfire Launches Open KYAPay Protocol With Agent Checkout — BusinessWire — https://www.businesswire.com/news/home/20250626772489/en/Skyfire-Launches-Open-KYAPay-Protocol-With-Agent-Checkout — (news, medium)
27. @coinbase/agentkit (npm latest) — https://registry.npmjs.org/@coinbase/agentkit/latest — (registry, high)
28. coinbase-agentkit (PyPI) — https://pypi.org/pypi/coinbase-agentkit/json — (registry, high)
29. coinbase/cb-mpc (MPC library for CDP wallets) — https://github.com/coinbase/cb-mpc — (github, high)
