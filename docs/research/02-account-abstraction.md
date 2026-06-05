# Account Abstraction & Smart Accounts

> Where ERC-4337 and EIP-7702 stand post-Pectra, the canonical contract addresses, the Rust tooling, and the early threat data. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- EIP-7702 has status **Final** (Standards Track, Core) and shipped on Ethereum mainnet in the **Pectra** hard fork, which activated **May 7, 2025**. [1][2][3]
- The 7702 spec document itself does not name Pectra; the hard-fork attribution comes from the Pectra meta-EIP (EIP-7600), not from the eip-7702.md text. [1][2][3]
- ERC-4337 EntryPoint **v0.8.0** added **native EIP-7702** authorization handling in the EntryPoint contract and introduced **Simple7702Account**, "a fully audited minimalist smart contract wallet." [4][5]
- EntryPoint addresses: **v0.7** `0x0000000071727De22E5E9d8BAf0edAc6f37da032`, **v0.8** `0x4337084d9e255ff0702461cf8895ce9e3b5ff108`, **v0.9** `0x433709009B8330FDa32311DF1C2AFA402eD8D009` (v0.9.0 release dated Nov 16, 2025, ABI-compatible with v0.8/v0.7). [5][6][7]
- Native 7702 support was added in **v0.8**, not v0.9. [4][5]
- The Rust **alloy** stack supports both 7702 and 4337: `TransactionBuilder7702`, the `alloy-eip7702` crate, and ERC-4337 types under `alloy_rpc_types_eth::erc4337`. [8][9]
- Two production-grade **ERC-4337 bundlers are written in Rust**: **Rundler** (Alchemy) and **Silius** (modular). [10][11]
- Early threat signal: a market-maker's research (Wintermute) found **>97% of 7702 delegations in the first month post-Pectra pointed to contracts running identical "sweeper" bytecode** — but these sweepers reportedly made essentially no money. Treat as point-in-time, not a standing fact. [12][13]

## EIP-7702: status and what it is

EIP-7702 is a Standards Track / Core EIP whose on-chain frontmatter shows status **Final**. [1][2] It shipped as part of Ethereum's **Pectra** upgrade, which the Ethereum Foundation confirms activated on mainnet on **May 7, 2025** (epoch 364032). [3] One nuance worth carrying forward: the EIP-7702 document does not itself reference Pectra — the inclusion in that fork is established by the Pectra meta-EIP (EIP-7600), not by the 7702 spec. [1][2][3]

7702 lets an existing externally-owned account (EOA) delegate its code to a smart-contract implementation without migrating to a new address, bridging plain keypairs into the smart-account world. The reference infrastructure for that smart-account behavior is ERC-4337.

## ERC-4337 EntryPoint: versions and addresses

The canonical implementation lives in `eth-infinitism/account-abstraction`. [5] The pivotal release for 7702 interop is **v0.8.0**: its official GitHub release states it adds "native support for EIP-7702 authorizations in the EntryPoint contract" and introduces **Simple7702Account**, described as "a fully audited minimalist smart contract wallet" at `contracts/accounts/Simple7702Account.sol`. [4][5] A later v0.9.0 (released Nov 16, 2025) is the latest tagged release; it is ABI-compatible with v0.8 and v0.7. Its release notes enumerate the delta over v0.8: parallelizable paymaster signing via a new `paymasterSignature` field, block-number-based validity ranges (`validAfter`/`validUntil`), silent `initCode` handling for already-deployed accounts, a `getCurrentUserOpHash` helper, an `EIP7702AccountInitialized` event, and a `BasePaymaster` constructor change. Native 7702 handling is a v0.8 feature, not a v0.9 one. [4][5][6]

| EntryPoint | Address | Notes |
| --- | --- | --- |
| v0.7 | `0x0000000071727De22E5E9d8BAf0edAc6f37da032` | Confirmed via Etherscan + v0.7.0 release [5][7] |
| v0.8 | `0x4337084d9e255ff0702461cf8895ce9e3b5ff108` | Added native EIP-7702 + Simple7702Account [4][5] |
| v0.9 | `0x433709009B8330FDa32311DF1C2AFA402eD8D009` | v0.9.0 release dated Nov 16, 2025, ABI-compatible w/ v0.8 & v0.7 [6] |

## Rust tooling for AA

For a Rust codebase, the AA ecosystem is more reachable than the language gap suggests:

- **alloy** provides `TransactionBuilder7702` for constructing 7702 transactions, plus the dedicated **`alloy-eip7702`** crate exposing EIP-7702 constants, helpers, and types — together these are the basis for general 7702 authorization signing. [8][14] alloy PR #2499 ("Adding support for signing 7702 authorizations") is specifically the Ledger hardware-signer 7702 path, not general signing. [9]
- alloy also ships ERC-4337 request/response types under `alloy_rpc_types_eth::erc4337`. [8]
- **Rundler** ([alchemyplatform/rundler](https://github.com/alchemyplatform/rundler)) is Alchemy's ERC-4337 bundler, written in Rust. [10]
- **Silius** ([silius-rs/silius](https://github.com/silius-rs/silius)) is a modular ERC-4337 (account abstraction) bundler, written in Rust. [11]

This means a Rust wallet can sign 7702 authorizations and assemble UserOperations against the standard ERC-4337 stack without leaving the alloy ecosystem.

## Early 7702 threat data: the "CrimeEnjoyor" sweepers

The most-cited early statistic about 7702 adoption is a security one, and it deserves careful framing. The figure originates with **Wintermute** — a market maker's research/Dune dashboard, **not a neutral protocol-level source** — measured over the **first month** after Pectra (mid-2025). [12][13]

Wintermute's actual finding: **>97% of all EIP-7702 delegations were authorized to multiple contracts using the same exact (sweeper) bytecode.** The **>97% figure itself is independently reported by CoinDesk**, not only by Wintermute. [12][13] "CrimeEnjoyor" is the name Wintermute gave to the single most-reused decompiled variant; the 97% reportedly spans a small family of identical-bytecode sweepers (CrimeEnjoyor, CrimeEnjoyor2, AdvancedCrimeEnjoyor, HardcodedCrimeEnjoyor). The specific variant-name list is **single-sourced to Wintermute's X post, which is now login-walled (HTTP 402) and could not be re-fetched for this verification**. So "are CrimeEnjoyor sweepers" is a slight simplification of "are copies of the same sweeper bytecode Wintermute named CrimeEnjoyor." [13]

Two corrections matter for anyone reusing this stat:

- **It measures delegation *count* dominance, not value stolen.** Per CoinDesk/Wintermute, the sweeper operators spent ~2.88 ETH to authorize ~79,000 addresses but made **essentially no money** — no observed inbound ETH to the destination wallets. Most of those delegations are automated/spam-like, not successful drains. [12][13]
- **It is point-in-time.** Tied to the first month post-Pectra, it should not be presented as a standing characterization of the 7702 ecosystem in 2026. [12][13]

The underlying mechanism is the real lesson: a sweeper preys on a 7702 authorization signed (often blindly, or for a compromised key) that delegates an EOA's execution to attacker-controlled code, which then drains incoming funds. The signing UX — what exactly a user authorizes when they sign a 7702 tuple — is the security surface.

## What this means for Deckard

Observations and opportunities only — not a roadmap.

- Deckard today is a bare EOA (single secp256k1 keypair via alloy). EIP-7702 is the standardized path for a *bare EOA to gain smart-account behavior without changing address* — directly relevant to a wallet that already has accounts in the field. [1][2]
- The needed Rust primitives already exist in the stack Deckard uses: alloy's `TransactionBuilder7702`, the `alloy-eip7702` crate, and `alloy_rpc_types_eth::erc4337` types — so AA exploration would not require leaving alloy or adding a non-Rust dependency. [8][9]
- Running a bundler in-process or alongside the desktop app is feasible in Rust today (Rundler, Silius are both Rust), which is notable for a local-first app that may prefer not to depend solely on hosted bundler services. [10][11]
- If Deckard ever adopts 7702, **Simple7702Account** is a pre-audited, minimal delegation target maintained in the canonical eth-infinitism repo — an off-the-shelf implementation rather than a bespoke contract. [4][5]
- The CrimeEnjoyor data is a concrete argument that **7702 authorization signing is a high-stakes UX surface**: the dominant real-world 7702 activity in its first month was malicious delegation. A wallet that makes the delegation target legible to the user (and to an operator LLM) is mitigating the exact attack class observed on-chain. [12][13]
- EntryPoint addresses are version-pinned and ABI-compatible across v0.7–v0.9; any integration must target a specific deployed singleton, and v0.8+ is the line where native 7702 handling exists. [4][5][6]
- For the operator-wallet vision, AA (4337 + 7702) is the substrate that makes session keys, batching, and sponsored/delegated execution possible — but none of that is reachable from a plain EOA without first adopting the smart-account or 7702 layer. (Observation; the operator-specific standards are out of scope for this file.)

## Open questions

- What is the 7702 delegation/sweeper picture in mid-2026? The 97% figure is first-month-post-Pectra (mid-2025) and from a single market-maker source; a current, neutral on-chain census was not verified here. [12][13]
- ~~What does v0.9.0 add beyond v0.8?~~ Resolved: the v0.9.0 release notes enumerate the delta — parallelizable paymaster signing (new `paymasterSignature` field), block-number-based validity ranges (`validAfter`/`validUntil`), silent `initCode` handling for existing accounts, `getCurrentUserOpHash`, an `EIP7702AccountInitialized` event, and a `BasePaymaster` constructor change. [6]
- How mature/audited are the Rust bundlers (Rundler, Silius) for production self-custody use, and what is their EntryPoint-version coverage? [10][11]
- What is the security review status of `alloy-eip7702` and alloy's 7702 signing path for a wallet that signs authorizations on behalf of a user? [8][9]
- Does running a bundler locally inside a desktop app change the trust/mempool assumptions versus using a hosted bundler? (Not addressed by the verified sources.)

## Sources

1. EIP-7702 (eips.ethereum.org) — https://eips.ethereum.org/EIPS/eip-7702 — (spec, high)
2. EIP-7702 markdown source (ethereum/EIPs, raw) — https://raw.githubusercontent.com/ethereum/EIPs/master/EIPS/eip-7702.md — (spec/source, high)
3. Ethereum Foundation: Pectra Mainnet Announcement — https://blog.ethereum.org/2025/04/23/pectra-mainnet — (primary blog, high)
4. EntryPoint v0.8.0 release (eth-infinitism/account-abstraction) — https://github.com/eth-infinitism/account-abstraction/releases/tag/v0.8.0 — (release notes, high)
5. eth-infinitism/account-abstraction (canonical ERC-4337 repo) — https://github.com/eth-infinitism/account-abstraction — (GitHub repo, high)
6. EntryPoint v0.9.0 release — https://github.com/eth-infinitism/account-abstraction/releases/tag/v0.9.0 — (release notes, high)
7. EntryPoint v0.7.0 release — https://github.com/eth-infinitism/account-abstraction/releases/tag/v0.7.0 — (release notes, high)
8. alloy-rs/alloy — https://github.com/alloy-rs/alloy — (GitHub repo, high)
9. alloy-rs/alloy PR #2499 "Adding support for signing 7702 authorizations" — https://github.com/alloy-rs/alloy/pull/2499 — (GitHub PR, high)
10. Rundler — Alchemy's ERC-4337 bundler in Rust — https://github.com/alchemyplatform/rundler — (GitHub repo, high)
11. Silius — modular ERC-4337 bundler in Rust — https://github.com/silius-rs/silius — (GitHub repo, high)
12. CoinDesk: Post-Pectra, malicious Ethereum contracts try to drain wallets but to no avail (Wintermute) — https://www.coindesk.com/tech/2025/06/02/post-pectra-upgrade-malicious-ethereum-contracts-are-trying-to-drain-wallets-but-to-no-avail-wintermute — (news, medium)
13. Wintermute research (X / Dune dashboard) — https://x.com/wintermute_t/status/1932101433916305743 — (market-maker research, medium; login-walled / HTTP 402)
14. alloy-eip7702 crate — https://crates.io/crates/alloy-eip7702 — (crate registry, high)
