# Privacy in Wallets, 2026

> How Ethereum privacy went from fringe "mixer" tooling to an Ethereum-Foundation-led roadmap, the shipping primitives (stealth addresses, shielded pools, FHE tokens), the near-horizon protocol-native bet (EIP-8182), and the operational/metadata layer a native wallet can own. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- The canonical framing is Vitalik Buterin's **"maximally simple L1 privacy roadmap"** (ethereum-magicians, April 2025): four pillars — on-chain payment privacy (shielded balances, ideally on by default), in-app activity anonymization via **one address per application**, **private reads** (RPC/metadata), and **network-level obfuscation** — all "very light on Ethereum consensus changes" [1].
- The Ethereum Foundation reorganized: "Privacy & Scaling Explorations" rebranded to **Privacy Stewards of Ethereum (PSE)** (roadmap Sept 2025) and stood up a **~47-person Privacy Cluster** (Oct 2025) whose reference deliverable is **Kohaku**, an open-source privacy wallet SDK [2][3][4].
- **Kohaku** (`github.com/ethereum/kohaku`) is a TypeScript+Rust monorepo (≈TS 44% / Rust 38% / Solidity 7%) bundling Railgun, Privacy Pools (WIP), Tornado (WIP), a provider abstraction (ethers/viem/Helios/Colibri), and a post-quantum ERC-4337 account; the repo itself carries a "not ready for production" disclaimer [5][6].
- Shipping primitives: **stealth addresses** (ERC-5564 / ERC-6538, canonical contracts deployed at deterministic vanity addresses across ~16 networks); **shielded pools** (Railgun with Private Proof of Innocence; 0xbow's compliant Privacy Pools live on mainnet since March 2025) [7][8][9][10].
- The two leading shielded-pool **compliance models are opposites**: Privacy Pools proves *inclusion* in an allowlist association set; Railgun PPOI proves *non-membership* in blocklists [10][11].
- **FHE confidential tokens** reached mainnet: Zama's ERC-7984 (encrypted balances/amounts via fhEVM) went live Dec 30 2025 — complementary to shielded pools (hides amounts, not the address graph) [12][13].
- Big near-horizon bet: **EIP-8182** (Draft, March 2026) proposes a *protocol-native* shielded pool as a no-admin system contract for the H2-2026 **Hegota** upgrade — one shared chain-wide anonymity set, any wallet, no special address format [14][15].
- The **operational/metadata layer** (private RPC, light clients, address-per-dapp, broadcast privacy) is now an explicit EF workstream ("Private Reads") but remains largely unshipped in mainstream wallets, which still default to IP-leaking RPC like Infura [16][17].
- Regulatory backdrop is more favorable: Tornado Cash sanctions were vacated and OFAC delisted the contracts (March 2025); but developer Roman Storm was convicted (Aug 2025) on one money-transmission count — operators face exposure, self-custodial integrators much less so [18][19].

## Vitalik's "maximally simple L1 privacy roadmap" — the strategic spine

On **April 2025** Vitalik Buterin posted a four-pillar roadmap to "practically improve the state of privacy experienced by Ethereum's users in a way that is very light on Ethereum consensus changes" [1]. The pillars: (1) **privacy of on-chain payments** — wallets "should have a notion of a shielded balance, and when you send to someone else, there should be a 'send from shielded balance' option, ideally turned on by default," integrating Privacy Pools and Railgun; (2) **anonymizing in-app activity** via a "one address per application" default (he conceded "significant convenience sacrifices" but called it the most practical way to break public cross-app links); (3) **private reads** — protecting RPC calls so reading the chain doesn't leak which addresses you care about (the post emphasizes a near-term TEE-based RPC mitigation); (4) **network-level obfuscation** — hiding IP/metadata at the transport layer [1]. This document is the spine PSE, the Privacy Cluster, and Kohaku all execute against, and it maps closely onto the operational-privacy concerns of a desktop wallet.

By **May 26, 2026** Vitalik reframed the goal as shipping over rhetoric — "We've accelerated narratives enough. Let's accelerate the cypherpunk privacy reality" — and described planned Kohaku support for browser extensions, **CLI wallets**, post-quantum accounts, multisigs, and hardware wallets, i.e. the stack is explicitly meant to reach native/CLI wallets, not only browser extensions [6].

## The EF reorganization: PSE rebrand + Privacy Cluster + Kohaku

PSE published its roadmap in **Sept 2025**, shifting from cryptography exploration to "problem-first" work and warning that without privacy Ethereum "risks becoming the backbone of global surveillance rather than global freedom" [2]. On **Oct 8, 2025** the EF published a formal privacy commitment and unveiled a **~47-member Privacy Cluster** organized into five initiatives: Private Reads & Writes, Private Proving, Private Identities, Privacy Experience, and an Institutional Privacy Task Force [3][4]. Kohaku is named as the cluster's reference privacy wallet + SDK.

**Kohaku** (`github.com/ethereum/kohaku`) is the most important artifact here. Confirmed packages: `@kohaku-eth/railgun` (Railgun shielding lib), `@kohaku-eth/privacy-pools` (WIP), `@kohaku-eth/tornado` (WIP), `@kohaku-eth/provider` (abstraction over ethers/viem/Helios/Colibri), and `@kohaku-eth/pq-account` (post-quantum ERC-4337 account) [5]. The SDK leaves railgun **unmarked** (only privacy-pools and tornado carry "WIP" labels), and railgun is published as an alpha (npm `0.0.1-alpha.x`), not a stable/production release — it is the most mature integration, but not "production-ready." Architecturally it pushes "privacy by default" through per-dapp account creation, user-defined/private RPC, light-client verification via Helios, and Tor routing for extreme cases [5][20].

⚠ unverified: the precise claim that "ERC-4337 mempool relaying shipped at `@kohaku-eth/railgun@0.0.1-alpha.21`" rests on secondary press, not the primary changelog — the alpha.21 release (published May 23, 2026) note reads only "fix: account for railgun fee" [6][21]. The version exists and railgun is the most mature integration (though still alpha, not production-ready; the latest is `0.0.1-alpha.22`, published May 26, 2026, with alpha.21 the relaying release); the 4337-relaying feature attribution is secondary-sourced. Open issues signal the roadmap: Tornado/Railgun v0.1.0 + Snap Sync, direct devp2p sync, an ERC-7579 PQValidator module, Tx-Shield modules, and explicit discussion of whether libraries in "languages like Rust, Swift" are in scope — directly relevant to a Rust wallet [5].

## Shipping primitive 1: stealth addresses (ERC-5564 / ERC-6538)

ERC-5564 standardizes a non-interactive stealth-address scheme on secp256k1: a sender generates an ephemeral keypair, derives a shared secret with the recipient's published viewing key, and computes a fresh stealth address only the recipient can spend; a one-byte **view tag** lets recipients filter announcements ~6x faster. ERC-6538 is the Stealth Meta-Address Registry [7]. **Canonical contracts** (ScopeLift) are deployed via CREATE2 at the same deterministic vanity addresses on every chain — Announcer `0x55649E01B5Df198D18D95b5cc5051630cfD45564`, Registry `0x6538E6bf4B0eBd30A8Ea093027Ac2422ce5d6538` — live on Ethereum, Arbitrum, Base, Optimism, Polygon, Gnosis (and Scroll) mainnets plus testnets [8].

⚠ unverified: the exact "16 networks" count is close but not cleanly reproducible from the ScopeLift README table (renders as ~15–16 depending on counting, and includes Scroll mainnet); the addresses, CREATE2 sameness, and the named mainnets are confirmed [8]. Audit/security docs were listed "coming soon."

Production wallets: **Fluidkey** is a live, non-custodial ERC-5564 wallet that derives viewing/spending keys from a signed message (path `m/5564'/0'/8'/0'/0'/p'/n'`), uses 1-of-1 Safe smart accounts as counterfactually-deployed stealth accounts, and — notably — does *not* rely on scanning announcements: its `.fkey.id`/`.fkey.eth` ENS offchain resolver returns a fresh stealth address per query, so senders just send to a normal-looking address. Live on ~6 EVM chains (Base, Optimism, Arbitrum, Polygon, Gnosis, Ethereum); multisigs not yet supported; audited by Dedaub (May 2024) [22][23]. **Umbra** (ScopeLift) is the original ERC-5564-aligned protocol (live since 2021, ~$500M volume); v2 is ~90% complete, targeting a summer-2026 stablecoin-focused launch, and is now a self-funded public good (no token/VC) — a useful signal that even successful privacy infra struggles to fund itself post-Tornado [24].

**Inherent limitations a wallet must handle:** the funding wallet can de-anonymize the recipient if linkable; announcement spam is an un-compensated DoS on scanners; the view tag trades ~4 bits of margin (128→124-bit) for faster scanning; and stealth addresses alone do **not** break on-chain transaction-graph traceability — coin-selection at withdrawal still matters [7][23].

## Shipping primitive 2: shielded pools and their compliance models

**Privacy Pools** (0xbow) is the production implementation of the 2023 Buterin/Soleimani/Illum/Nadler/Schär paper "Blockchain Privacy and Regulatory Compliance" [25][26]. Users deposit then withdraw with no on-chain link, proving via ZK that they belong to a chosen **association set** maintained by an Association Set Provider (ASP); a **ragequit** lets an un-approved depositor publicly exit. Live on Ethereum mainnet since **March 2025**, multi-asset since July 2025; by late 2025 it had processed **~$6M from 1,500+ users**, raised a **$3.5M seed (Nov 2025)**, and was integrated into Kohaku [10][9]. ⚠ unverified: the finer "1,186 withdrawals / 16,000+ flagged addresses" figures come from 0xbow's own materials and weren't independently re-confirmed; the headline stats are confirmed.

**Railgun** gives users encrypted `0zk` addresses where balances/history are visible only to them, via zk-SNARKs, with private DeFi swaps. Transactions are submitted by **Broadcasters** (relayers, ~10% gas premium) so activity appears to originate from the Broadcaster, not the shielding address. Its compliance answer is **Private Proof of Innocence (PPOI)**: a recursive-SNARK proof of *non-membership* in blocklists from five list providers (Elliptic, ScamSniffer, PureFi, SlowMist, Chainalysis Sanctions Oracle), plus a **1-hour unshield-only standby period** so bad actors can't hop addresses faster than lists update [11][27]. Railgun is the privacy tool Vitalik explicitly cited and the most-shipped Kohaku integration.

| Model | Proof | Default posture | Tradeoff |
|---|---|---|---|
| **Privacy Pools** (0xbow) | *Inclusion* in a curated allowlist (association set) | Opt-in; an excluded user is de-anonymized via ragequit | Regulator-friendly but exclusionary |
| **Railgun PPOI** | *Non-membership* in known-bad blocklists | Private-by-default for anyone not on a list | Depends on list quality; harder to make airtight |
| **Labyrinth** (testnet→mainnet) | Threshold/selective reveal ("Decom") | Hidden by default, selective de-anon via threshold decryption | Third model; user-downloadable data for selective disclosure |

Both ZK approaches reveal nothing beyond the single membership/non-membership bit [10][11]. ⚠ unverified (medium-reliability sources): Labyrinth's gas figures and Optimism/testnet status [28].

## Shipping primitive 3: FHE confidential tokens (Zama, ERC-7984)

Zama's FHE confidentiality layer reached Ethereum mainnet on **Dec 30 2025** with the **ERC-7984** confidential token standard — encrypted balances and transfer amounts via fhEVM, with OpenZeppelin confidential-contract libraries and a Confidential Token Wrappers Registry to shield/unshield any ERC-20 at **~$0.13 per transfer**; the launch operator set includes Ledger and Fireblocks [12][13]. A **Jan 2026** sealed-bid token auction drew **~$118–121M** committed (value shielded in bidding, not strictly net proceeds) [29]. For a wallet, FHE tokens hide balances/amounts but **not** the sender/recipient address graph the way Railgun/Privacy Pools do — complementary, not a replacement. ⚠ note: an earlier-cited Zama URL was a deprecated 2023 post; the ERC-7984/registry/operators claims are nonetheless confirmed via Zama's 2025–2026 materials.

## The near-horizon bet: EIP-8182 (protocol-native shielded pool)

**EIP-8182 "Private ETH and ERC-20 Transfers"** (Draft, Standards Track/Core, created **March 2026**, author **Tom Lehman** of Facet) would embed a shielded pool as a **system contract at `0x...081820`** — no proxy, no admin, no pause, upgradeable only via hard fork — to solve the pool-bootstrapping problem ("a small pool offers weak privacy even for a superior product") [14]. Design: a UTXO/note model with a depth-32 commitment tree, and a **split-proof architecture** — a fork-managed Groth16/BN254 "pool proof" (value conservation, nullifiers, Merkle membership) plus a permissionless, user-selected **"auth proof"** enabling ECDSA, passkeys, hardware wallets, and delegated proving. Three functions: `deposit()`, `transact()`, `setAuthPolicy()`. It deliberately ships **no** in-protocol compliance. Lehman pitched it (late May 2026) for Ethereum's H2-2026 **Hegota** upgrade [15]. If it lands, every wallet — including a native EOA-style one — could offer "send private ETH/ERC-20 to any address or ENS" from existing accounts, with no special address format, sharing one chain-wide anonymity set. This is the single most strategically important near-horizon item. ⚠ unverified: the exact pitch date ("May 25, 2026" vs "pitched Friday" in coverage clustered May 22–25); the H2-2026 targeting and technical specifics are confirmed.

## Operational / metadata privacy: RPC, IP, light clients

The privacy mainstream wallets routinely ignore is operational/metadata privacy. The default leak: MetaMask's default RPC, **Infura** (ConsenSys), collects users' IP + Ethereum addresses on transactions (ConsenSys' policy update, Nov 2022, made this explicit); any third-party RPC sees your IP+address, and dApp connections add network-level data that combines with on-chain history into a behavioral fingerprint [16]. EF-recommended mitigations: (1) **private/self-hosted RPC**; (2) **light clients** — a16z's **Helios**, a Rust Ethereum + OP-stack light client that "converts an untrusted centralized RPC endpoint into a safe unmanipulable local RPC" and compiles to WebAssembly to embed inside wallets (Kohaku plans a WASM build); the Colibri-Stateless provider is an EIP-1193-compatible alternative; (3) **network-layer obfuscation** via Tor/mixnets [16][17].

PSE's **"Private Reads"** workstream codifies the metadata roadmap: launching a **Private RPC working group**; integrating **ORAM** into Kohaku for privacy-preserving state reads from remote RPC; implementing a **Sphinx-protocol mixnet** for transaction-broadcast privacy; and TLSNotary/zkTLS for production [2][30]. Community critique on the Magicians thread: the roadmap is heavy on research and light on a clear line to concrete user-visible improvements, and overlaps with account-abstraction concerns [30].

## Regulatory context

The post-Tornado-Cash picture clarified and de-risked self-custodial privacy tooling. **Van Loon v. Treasury** (5th Circuit, Nov 2024) held that Tornado Cash's immutable smart contracts aren't "property" under IEEPA (no ownership/control/exclusivity); **OFAC formally delisted** the contracts on **March 21, 2025**; a W.D. Texas court (April 2025) permanently enjoined re-sanctioning [18][31]. Separately, developer **Roman Storm was convicted (Aug 6, 2025)** on one count — conspiracy to operate an unlicensed money-transmitting business (18 U.S.C. §1960) — while the jury deadlocked on the heavier money-laundering and sanctions counts [19]. Net signal: immutable privacy smart contracts are much harder to sanction, but operators/developers running active money-transmission services still face criminal exposure — which is why production protocols lead with compliance-by-design and self-custodial, non-operator architectures. A self-custodial wallet that merely *integrates* these protocols sits on the favorable side of that line.

## Adjacent: Aztec (privacy at the execution layer)

Aztec launched its **"Ignition" chain** on Ethereum mainnet (Nov 2025), billed as the first fully decentralized privacy L2 — producing consensus blocks but **without the smart-contract execution layer** initially; private contract execution and live transactions targeted for early 2026, with earliest TGE Feb 11 2026 [32][33]. Relevant as a destination chain where privacy is native at the execution layer (vs. bolt-on L1 shielded pools), but it requires a chain-specific account/wallet model and is not a drop-in for an EOA-style wallet.

## What this means for Deckard

- The four-pillar roadmap maps closely onto a desktop wallet's surfaces, and the **operational/metadata pillars** (private reads, network-level obfuscation) are largely unshipped in mainstream wallets — a gap a native client controls directly rather than depending on protocol upgrades [1][16].
- **Helios is a Rust light client built to embed in wallets**, and Deckard's runtime is already Rust — so the light-client / "untrusted-RPC-into-verifiable-local-RPC" path involves no language bridge, unlike the TS-first Kohaku SDK [17][5].
- Kohaku contributors are **explicitly discussing whether Rust/Swift libraries are in scope**, and the EF roadmap names **CLI/native wallets** as intended Kohaku targets — so a native desktop wallet is within the stated audience, not outside it [5][6].
- The shielded-pool **compliance models are mutually exclusive design choices** (allowlist inclusion vs blocklist non-membership vs threshold reveal); an operator-wallet that lets the user/agent pick per-transaction would span all three rather than hard-coding one posture [10][11][28].
- **EIP-8182, if it lands in Hegota (H2-2026), would give an EOA-today wallet private transfers with no new address format and a shared anonymity set** — i.e. payment privacy without first migrating to smart accounts; its split auth-proof design already contemplates ECDSA and hardware-wallet signing [14][15].
- **FHE confidential tokens and shielded pools are complementary, not substitutes** — FHE hides amounts/balances, shielded pools break the address graph — so "privacy" is not one toggle but a stack of independent properties a wallet exposes separately [12][11].
- The regulatory line currently favors **self-custodial integrators over service operators**, which matches Deckard's self-custodial, non-custodial-relayer posture; integrating compliance-by-design protocols (PPOI, association sets) keeps a wallet on that side [18][19].
- **Address-per-dapp and coin-selection-at-withdrawal are wallet-side responsibilities**, not protocol features — stealth addresses and shielded pools don't deliver unlinkability on their own, so account-management UX inside the wallet is load-bearing for the privacy actually achieved [1][23].

## Open questions

- Is the Kohaku SDK consumable from Rust, or does its TS-first design force a native wallet to reimplement primitives (Railgun proving, provider abstraction) rather than bind to it?
- Will EIP-8182 actually make the Hegota (H2-2026) cut, and does its permissionless "auth proof" verifier admit a plain-EOA ECDSA path with acceptable proving cost on a desktop machine?
- For an LLM-driven operator wallet, what is the right default privacy posture (which compliance model, shielded-by-default vs opt-in), and how is that decision surfaced to or delegated by the user?
- What is the desktop UX/perf cost of running Helios as an embedded light client (sync time, resource use) versus a privacy-respecting hosted RPC?
- How mature is the broadcast-privacy layer (Sphinx mixnet, Broadcasters) for a wallet that wants to avoid linking IP↔address at transaction submission, and what latency does it add?
- Does delegated/remote proving (for shielded transfers or EIP-8182 auth proofs) reintroduce a metadata leak or trust dependency that undercuts the local-first model?

## Sources

1. A maximally simple L1 privacy roadmap (Vitalik Buterin, Apr 2025) — https://ethereum-magicians.org/t/a-maximally-simple-l1-privacy-roadmap/23459 — (forum, high)
2. PSE Roadmap: 2025 and Beyond — https://pse.dev/blog/pse-roadmap-2025 — (blog, high)
3. The Ethereum Foundation's Commitment to Privacy — https://blog.ethereum.org/2025/10/08/privacy-commitment — (blog, high)
4. EF Expands Privacy Push With Dedicated Research Cluster — https://www.coindesk.com/tech/2025/10/09/ethereum-foundation-expands-privacy-push-with-dedicated-research-cluster — (news, medium)
5. ethereum/kohaku — Privacy-first tooling for Ethereum (SDK monorepo) — https://github.com/ethereum/kohaku — (github, high)
6. Vitalik: Ethereum Has Enough Privacy Narratives as Kohaku SDK Advances — https://www.cryptotimes.io/2026/05/26/vitalik-ethereum-has-enough-privacy-narratives-as-kohaku-sdk-advances/ — (news, medium)
7. ERC-5564: Stealth Addresses (with ERC-6538 Registry) — https://eips.ethereum.org/EIPS/eip-5564 — (spec, high)
8. ScopeLift/stealth-address-erc-contracts (canonical 5564/6538 deployments) — https://github.com/ScopeLift/stealth-address-erc-contracts — (github, high)
9. 0xbow Closes $3.5M Round Following Ethereum Foundation Integration — https://www.globenewswire.com/news-release/2025/11/18/3190435/0/en/0xbow-Closes-3-5M-Round-for-Compliant-Crypto-Privacy-Technology-Following-Ethereum-Foundation-Integration.html — (news, medium)
10. Privacy Pools documentation — https://docs.privacypools.com/ — (docs, high)
11. RAILGUN Private Proofs of Innocence — https://docs.railgun.org/wiki/assurance/private-proofs-of-innocence — (docs, high)
12. ERC-7984 Standard (Zama/OpenZeppelin) — https://docs.zama.org/protocol/examples/openzeppelin-confidential-contracts/erc7984 — (docs, high)
13. Confidentiality Layer: Zama Wraps Blockchains in Privacy — https://www.bankless.com/read/confidentiality-layer-zama-wraps-blockchains-in-privacy — (news, medium)
14. EIP-8182: Private ETH and ERC-20 Transfers — https://eips.ethereum.org/EIPS/eip-8182 — (spec, high)
15. Facet's Tom Lehman Pitches EIP-8182 for Hegota — https://unchainedcrypto.com/facets-tom-lehman-pitches-eip-8182-to-bring-native-private-transfers-to-ethereums-hegota-upgrade/ — (news, medium)
16. Infura to Collect MetaMask Users' IP + Ethereum Addresses (policy update) — https://decrypt.co/115486/infura-collect-metamask-users-ip-ethereum-addresses-after-privacy-policy-update — (news, medium)
17. a16z/helios — Rust Ethereum + OP-stack light client — https://github.com/a16z/helios — (github, high)
18. Why OFAC Delisted Tornado Cash — https://www.coindesk.com/policy/2025/04/05/why-ofac-delisted-tornado-cash — (news, medium)
19. US v. Storm: Background & Timeline — https://www.defieducationfund.org/us-v-storm-background-timeline/ — (other, high)
20. Kohaku documentation (llms-full) — https://ethereum.github.io/kohaku/llms-full.txt — (docs, high)
21. Kohaku GitHub releases page — https://github.com/ethereum/kohaku/releases — (github, high)
22. Fluidkey Technical Walkthrough — https://docs.fluidkey.com/technical-documentation/technical-walkthrough/ — (docs, high)
23. Fluidkey FAQ — https://docs.fluidkey.com/readme/frequently-asked-questions/ — (docs, high)
24. ScopeLift/umbra-protocol (Umbra stealth-payment protocol) — https://github.com/ScopeLift/umbra-protocol — (github, high)
25. Blockchain Privacy and Regulatory Compliance: Towards a Practical Equilibrium (Buterin et al., 2023) — https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4563364 — (spec, high)
26. 0xbow: Unlocking Privacy-Preserving Compliance with Association Sets — https://0xbow.io/blog/unlocking-privacy-preserving-compliance-with-association-sets — (blog, high)
27. RAILGUN Privacy System (docs) — https://docs.railgun.org/wiki/learn/privacy-system — (docs, high)
28. Labyrinth's journey to private and compliant DeFi — https://labyrinthprotocol.tech/blog/labyrinths-journey-to-private-and-compliant-defi-milestones-integrations-and-the-road-to-mainnet-2/ — (blog, medium)
29. $118M Committed for the First Encrypted ICO on Ethereum (Zama) — https://www.zama.org/post/118m-committed-for-the-first-encrypted-ico-on-ethereum — (blog, high)
30. PSE Roadmap: 2025 and Beyond (Magicians discussion) — https://ethereum-magicians.org/t/pse-roadmap-2025-and-beyond/25423 — (forum, high)
31. Fifth Circuit Tosses OFAC Sanctions on Tornado Cash (Mayer Brown) — https://www.mayerbrown.com/en/insights/publications/2024/12/federal-appeals-court-tosses-ofac-sanctions-on-tornado-cash-and-limits-federal-governments-ability-to-police-crypto-transactions — (other, high)
32. Privacy-Focused Aztec Network's Ignition Chain Lights Up on Ethereum (CoinDesk) — https://www.coindesk.com/markets/2025/11/20/privacy-focused-aztec-network-s-ignition-chain-lights-up-on-ethereum — (news, medium)
33. Aztec — Roadmap for Decentralized Privacy On-Chain — https://aztec.network/roadmap — (docs, high)
