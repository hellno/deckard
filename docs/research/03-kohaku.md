# Kohaku — the EF-maintained wallet

> The Ethereum Foundation's open-source privacy SDK and reference wallet: a Rust-to-WASM crypto core, an Ambire-forked browser extension, and a roadmap that explicitly names local-AI transaction scoring and post-quantum accounts. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- **Kohaku is two repos, not one.** `github.com/ethereum/kohaku` is the **SDK** (a Rust + TypeScript monorepo of privacy packages). The **reference wallet** is a separate repo, `github.com/ethereum/kohaku-extension`, whose README states explicitly that it is "a fork of Ambire Wallet" and is currently Sepolia-testnet-only and "under active development" [1][2][3].
- Framing: "Privacy-first tooling for the Ethereum ecosystem." The SDK ships packages including `@kohaku-eth/railgun`, `@kohaku-eth/privacy-pools`, `@kohaku-eth/provider`, and `@kohaku-eth/pq-account` [1].
- **The crypto core is Rust compiled to WebAssembly.** The root `Cargo.toml` defines a workspace of 8 crates, declares `wasm-bindgen` 0.2.108 plus `wasm-bindgen-futures` and `gloo-net`, and carries a dedicated `[profile.release-wasm]` (opt-level `'z'`, `lto=true`) tuned for WASM [4].
- The Rust→WASM→TypeScript binding is canonical: `crates/railgun-ts/Cargo.toml` sets `crate-type = ['cdylib','rlib']` and depends on the Rust `railgun` crate (with the `js` feature), `wasm-bindgen`, `wasm-bindgen-futures`, and `tsify` [5].
- **Railgun is the most mature integration, shipped as an alpha.** The published npm package `@kohaku-eth/railgun` reached `0.0.1-alpha.22` (latest May 26, 2026), following a continuous alpha.13→alpha.22 series [6].
- The `railgun` crate supports UTXO/TXID syncing, state querying, transaction construction, proof generation, POI (proof-of-innocence) generation/submission, and submission via broadcasters [7].
- The naming is a tell: **"Kohaku" is Japanese for amber**, chosen because the wallet forked from **Ambire** (amber) [2][10].
- **`@kohaku-eth/pq-account` is a post-quantum ERC-4337 account** — account abstraction via the *current* ERC-4337 path, distinct from the future L1 "native AA" roadmap item [1].
- **Roadmap items are planned/exploratory, not shipped.** The EF-hosted roadmap lists local-AI transaction scoring under "future directions we are exploring," and treats native account abstraction as an L1-advocacy goal for 2026 — not something Kohaku builds itself [8].
- **EF stewardship is formally announced.** The Ethereum Foundation named Kohaku on its official blog on 2025-10-08, as part of its Privacy Cluster / "Commitment to Privacy," describing "a new reference implementation of a privacy-preserving wallet and an open-source wallet SDK" and linking both repos [11]. This is corroborated by org ownership (both repos under `github.com/ethereum`) and the EF-hosted roadmap on `notes.ethereum.org` [1][8][11].

## What Kohaku actually is

Kohaku is the Ethereum Foundation's open-source privacy wallet stack, best understood as a **layered split**: a reusable SDK and a reference application that consumes it.

The **SDK** (`ethereum/kohaku`) bundles privacy primitives behind a package interface. Confirmed npm packages include `@kohaku-eth/railgun` (Railgun shielding), `@kohaku-eth/privacy-pools`, `@kohaku-eth/provider` (an RPC/provider abstraction), and `@kohaku-eth/pq-account` (a post-quantum ERC-4337 account) [1]. The README frames it as "Privacy-first tooling for the Ethereum ecosystem," with a blanket caveat that "some parts of this project are work in progress and not ready for production use" [1].

The **reference wallet** (`ethereum/kohaku-extension`) is a *separate* repository and is **a fork of Ambire Wallet** — stated verbatim in its README, and corroborated by the official docs noting it was "Forked from `@ambiretech/extension` & `@ambiretech/ambire-common`" [2][3][9]. It is currently a work-in-progress browser extension supporting only **Sepolia testnet** [2][9]. The split matters: the privacy logic is meant to be embeddable, while the Ambire-derived extension is just one consumer of it.

EF stewardship is anchored in a first-party announcement: the Ethereum Foundation's official blog post "The Ethereum Foundation's Commitment to Privacy" (2025-10-08) names Kohaku as part of its Privacy Cluster, describing "a new reference implementation of a privacy-preserving wallet and an open-source wallet SDK" and linking both repos [11]. This is reinforced by repository ownership under the official `github.com/ethereum` org, the EF-hosted roadmap on `notes.ethereum.org`, and the PSE (Privacy Stewardship of Ethereum) lineage. As corroboration, a QuickNode deep-dive states "the Ethereum Foundation leads the project with collaboration from teams like Ambire, Railgun, Helios, and PSE" [10].

## Architecture: a Rust core, compiled to WASM, bound to TypeScript

For a Rust shop, the key fact is that **Kohaku's cryptographic core is Rust**, compiled to WebAssembly and exposed to a TypeScript application layer.

- The root `Cargo.toml` defines a Rust workspace of **8 crates**: `common`, `crypto`, `eip-1193-provider`, `poseidon-rust`, `railgun`, `railgun-ts`, `userop-kit`, and `userop-kit-ts` [4].
- It declares `wasm-bindgen` 0.2.108, plus `wasm-bindgen-futures` and `gloo-net`, and adds a dedicated `[profile.release-wasm]` (`opt-level = 'z'`, `lto = true`) for size-optimized WASM builds [4].
- `crates/railgun-ts/Cargo.toml` is the canonical binding crate: `[lib] crate-type = ['cdylib','rlib']`, depending on the Rust `railgun` crate (with the `js` feature) plus `wasm-bindgen`, `wasm-bindgen-futures`, and `tsify` for TypeScript-type generation [5].

The `-ts` crate-naming convention (`railgun-ts`, `userop-kit-ts`) signals the pattern: a pure-Rust crate implements the protocol; a sibling `-ts` crate wraps it for WASM/TypeScript consumption — a clean reference for making a Rust crypto core portable without rewriting the cryptography.

| Layer | What it is | Evidence |
|---|---|---|
| Rust core crates | `crypto`, `poseidon-rust`, `railgun`, `userop-kit`, `common` | workspace `Cargo.toml` [4] |
| WASM bindings | `railgun-ts`, `userop-kit-ts` (`cdylib`, `wasm-bindgen`, `tsify`) | `railgun-ts/Cargo.toml` [5] |
| TS SDK packages | `@kohaku-eth/railgun`, `…/privacy-pools`, `…/provider`, `…/pq-account` | SDK README [1] |
| Reference wallet | `kohaku-extension` (Ambire fork, Sepolia-only) | extension README / docs [2][9] |

The SDK also exposes a documented **plugin interface**, and `@kohaku-eth/pq-account` is described as a "post-quantum 4337 account implementation" — PQ account abstraction over the **existing ERC-4337** path [1].

## Railgun: shipped as alpha, the most mature piece

Railgun is Kohaku's flagship integration and the clearest evidence of "shipped." The substantiation is **release versioning**, not a prose label: the published `@kohaku-eth/railgun` package reached `0.0.1-alpha.22` (latest May 26, 2026), with a continuous alpha.13→alpha.22 series of releases [6].

The Rust `railgun` crate's README enumerates supported capabilities: UTXO and TXID syncing, on-chain state querying, transaction construction, proof generation, POI (proof-of-innocence) proof generation and submission, and transaction submission via **broadcasters** [7]. That covers the full shield/transact/unshield lifecycle of a Railgun-style shielded pool.

⚠ Precision note: the SDK README does **not** literally label Railgun "alpha." It shows Railgun with a checkmark under the blanket "not ready for production" caveat, while the published docs (`llms-full.txt`) mark Privacy Pools and Tornado as "WIP" and leave Railgun unmarked [1][9]. The word "alpha" is justified purely by the npm semver (`0.0.1-alpha.x`), which is unambiguous. "Shipped" here means *published and usable in alpha*, not a stable release [6].

## Roadmap: AI scoring and account abstraction are aspirational

The EF-hosted roadmap (`notes.ethereum.org/@niard/KohakuRoadmap`) is explicit that several headline-grabbing items are **not yet built** [8]:

- **Local-AI transaction scoring** is listed under "future directions we are exploring": "develop transaction security scoring through local AI to help identify low-risk vs high-risk transactions without leaking private information." Exploratory, not shipped [8].
- **Native account abstraction** is an **L1-dependency advocacy item**, not a Kohaku feature: "we need the ethereum network to implement native account abstraction. We will be working in that direction over 2026." This is distinct from the *current* ERC-4337 path that `pq-account` already uses [8][1].
- The **plugin system** and a **post-quantum killswitch** (optimized Falcon/Dilithium Solidity verifiers, opt-in PQ accounts) are confirmed roadmap items [8].

Kohaku was showcased by Vitalik Buterin at **Devcon 2025** (Buenos Aires, Nov 16, 2025), which is part of the EF-stewardship evidence base [10].

## What this means for Deckard

Observations and opportunities only — no sequencing or priorities implied.

- **Kohaku is a same-language reference for a portable Rust crypto core.** Its `railgun`/`railgun-ts` split (pure-Rust protocol crate + `wasm-bindgen`/`tsify` binding crate) is a concrete pattern for keeping cryptography in Rust while exposing it elsewhere — directly relevant if Deckard ever needs a non-native surface, though Deckard's native GPUI app can consume the pure-Rust crates without the WASM layer at all [4][5][7].
- **The EF's reference wallet is browser-extension-shaped (Ambire fork, Sepolia-only).** Deckard occupies a different niche — a native desktop app — so the SDK packages are reusable, but the reference UX is not a template for Deckard's form factor [2][9].
- **Local-AI transaction scoring is on the EF's own exploratory roadmap**, framed as classifying low- vs high-risk transactions "without leaking private information." That is conceptually adjacent to Deckard's operator-wallet vision, and notably the EF frames it as *local* AI for privacy reasons [8].
- **Post-quantum account abstraction is treated as ERC-4337-based today** (`pq-account`), with L1 "native AA" positioned as a multi-year advocacy goal. For an EOA-today wallet, this signals that account abstraction remains an opt-in account-layer choice, not a settled L1 primitive [1][8].
- **The Railgun crate is an off-the-shelf Rust implementation of a shielded-pool lifecycle** (syncing, proof generation, POI, broadcaster submission) — a reference point if Deckard ever evaluates privacy features, with the caveat that it is alpha-versioned [6][7].
- **"EF-maintained" is backed by a first-party EF announcement** (the 2025-10-08 "Commitment to Privacy" blog post naming Kohaku), plus org ownership and the EF-hosted roadmap — useful context when weighing Kohaku's maturity and longevity as a dependency or design reference [8][10][11].

## Open questions

- Are any of the SDK's Rust crates (e.g. `crypto`, `poseidon-rust`, `railgun`) consumable as standalone Rust dependencies without the WASM/TS wrapper, with a stable enough API to depend on?
- What is the licensing of the SDK crates and of the Ambire-forked extension, and how does Ambire's upstream license flow through?
- How concrete is the "local AI transaction scoring" exploration — is there any prototype, threat model, or spec beyond the one-line roadmap entry? [8]
- Does Kohaku's "without leaking private information" local-AI framing imply on-device inference, and if so what model class/size is contemplated?
- What is the relationship and dependency direction between `userop-kit` (ERC-4337 tooling) and `pq-account`, and is the PQ account validated on any live network beyond Sepolia? [1][2]
- Beyond the EF's first-party announcement (the 2025-10-08 "Commitment to Privacy" blog post), what is the ongoing governance model — who formally owns Kohaku's roadmap and release decisions across the EF, PSE, and the named collaborating teams? [10][11]

## Sources

1. ethereum/kohaku — privacy SDK monorepo (README; packages `@kohaku-eth/railgun`, `privacy-pools`, `provider`, `pq-account`) — https://github.com/ethereum/kohaku — (GitHub repo, high)
2. ethereum/kohaku-extension — reference wallet, README states "a fork of Ambire Wallet," Sepolia-only, WIP — https://github.com/ethereum/kohaku-extension — (GitHub repo, high)
3. Kohaku official docs (full text) — confirms wallet "Forked from `@ambiretech/extension` & `@ambiretech/ambire-common`" — https://ethereum.github.io/kohaku/llms-full.txt — (project docs, high)
4. ethereum/kohaku root `Cargo.toml` — 8-crate Rust workspace, `wasm-bindgen` 0.2.108, `[profile.release-wasm]` — https://github.com/ethereum/kohaku/blob/master/Cargo.toml — (source file, high)
5. ethereum/kohaku `crates/railgun-ts/Cargo.toml` — `crate-type=['cdylib','rlib']`, depends on `railgun` + `wasm-bindgen` + `tsify` — https://github.com/ethereum/kohaku/blob/master/crates/railgun-ts/Cargo.toml — (source file, high)
6. Kohaku GitHub Releases — `@kohaku-eth/railgun@0.0.1-alpha.22` (latest May 26, 2026); alpha.13–alpha.22 series — https://github.com/ethereum/kohaku/releases — (release feed, high)
7. ethereum/kohaku `crates/railgun` — README enumerates UTXO/TXID sync, proof + POI generation, broadcaster submission — https://github.com/ethereum/kohaku/tree/master/crates/railgun — (source/README, high)
8. Kohaku Roadmap (EF-hosted) — plugin system, PQ killswitch (Falcon/Dilithium), local-AI tx scoring (exploratory), native AA (L1 advocacy over 2026) — https://notes.ethereum.org/@niard/KohakuRoadmap — (spec/roadmap, high)
9. Kohaku official docs — Introduction to Railgun and protocol status markers (Privacy Pools / Tornado "WIP") — https://ethereum.github.io/kohaku/railgun/intro/ — (project docs, high)
10. QuickNode — "Ethereum Foundation leads the project"; reference wallet "a browser extension forked from Ambire"; Devcon 2025 showcase — https://blog.quicknode.com/ethereum-kohaku-wallet-privacy-roadmap/ — (secondary deep-dive, medium)
11. Ethereum Foundation blog — "The Ethereum Foundation's Commitment to Privacy" (2025-10-08); names Kohaku as "a new reference implementation of a privacy-preserving wallet and an open-source wallet SDK," links both repos — https://blog.ethereum.org/2025/10/08/privacy-commitment — (first-party EF blog, high)
