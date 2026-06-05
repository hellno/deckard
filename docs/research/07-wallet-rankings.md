# Wallet Rankings & Scorecards (the 'L2BEAT for wallets')

> Survey of the credible, codified wallet-evaluation projects (Walletbeat, WalletScrutiny) and where a native self-custodial EOA desktop wallet lands against their rubrics. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- The "L2BEAT for wallets" exists: it is **Walletbeat**, whose GitHub README literally calls itself "the L2BEAT of wallets — an open repository of EVM-compatible wallets." Live beta at `beta.walletbeat.eth.limo`, code at `github.com/walletbeat/walletbeat` (active on the `beta` branch) [1][2][5].
- Walletbeat rates wallets on a fully codified pass/partial/fail rubric across five attribute groups — **Security, Privacy, Self-sovereignty, Transparency, Ecosystem** — plus a standalone **Maintenance** check (the latter applies to hardware/embedded wallets, not software) [6].
- Ratings map to numbers (`FAIL=0.0`, `UNRATED=-0.5`, `EXEMPT=null`) and are weighted-averaged into a score from **-0.5 to 1.0**; an unrated component appends an asterisk. The `PASS`/`PARTIAL` numeric defaults and the verifiability weighting live in `attributes.ts`, not `score.ts` (see correction below) [7][a].
- Walletbeat has a **Stages maturity ladder** (Stage 0 / 1 / 2 in code) borrowed from L2BEAT's rollup framework. ⚠ unverified: a "Stage 0.5" appears in EF/EthCC press coverage but **not** in the beta code file `software-wallet-stages.ts`, which defines only stages 0/1/2 [8][b].
- Stage 0 needs only **publicly available source code** to qualify for evaluation; Stage 1 adds recent audits, multi-vendor hardware support, private-by-default transfers, account portability, own-node use, a FOSS license, and ENS; Stage 2 adds a funded bug bounty, address non-correlation, account abstraction, and atomic batching [8].
- Created by **Moritz** (of Fluidkey, a Swiss company that also ships a wallet), revamped in 2025 by **polymutex**; funded by Ethereum Foundation grants and committed to **not rating Fluidkey's own wallet** for credible neutrality [c][d].
- **WalletScrutiny** (`walletscrutiny.com`) is the complementary project: it verifies wallets by **reproducible builds** (does the shipped binary match public source?) and gives **categorical verdicts, no numeric score** [9][10].
- **L2BEAT itself** (`l2beat.com`) does **not** rank wallets — it covers L2 rollups. It is relevant only as the methodological template Walletbeat copied [11][12].
- **ethereum.org's wallet finder** is a curated, filterable **directory** (~52 wallets), explicitly "not official endorsements" — not a scorecard [13].
- A native self-custodial EOA desktop wallet like Deckard scores well on the **self-sovereignty/ownership** and **license/source-visibility** axes, but goes FAIL/unrated on audits, bug bounty, default-private RPC, hardware support, privacy non-correlation, and most Ecosystem items (account abstraction, batching, ENS, WalletConnect) [8][14][15][16].

## Walletbeat — the answer to "is there an L2BEAT for wallets?"

Yes. **Walletbeat** is an open repository of EVM-compatible wallets that rates them, and it explicitly brands itself "the L2BEAT of wallets" in its GitHub README [1]. The canonical live surface is the ENS/IPFS-hosted beta at `beta.walletbeat.eth.limo` [2]; the legacy site `walletbeat.fyi` reflects an older, simpler feature-matrix schema with the disclaimer that "a high score does not necessarily mean better performance, it just means more available features" [4]. Active development happens on the `beta` branch (TypeScript ~76%, Svelte ~16%; ~2,481 commits, 112 stars, 83 forks as observed mid-2026) [1]. The repo was historically under `github.com/fluidkey/walletbeat` and now lives at `walletbeat/walletbeat`. The project's About page frames the mission directly: "As L2Beat has done for Ethereum Layer 2s, Walletbeat aims to do the same for Ethereum wallets" [e]. Anyone can add a wallet by dropping a data file in the wallet-data folder and opening a PR [1]. The Walletbeat repo itself is **MIT-licensed** [a].

## The codified rubric — a ready-made "what a good wallet has" checklist

`src/schema/attribute-groups.ts` enumerates the exact scored attributes [6]:

| Group | Attributes |
|---|---|
| **Security** | securityAudits, scamPrevention, chainVerification, transactionLegibility, hardwareWalletSupport, securityBestPractices, bugBountyProgram, supplyChainDIY, supplyChainFactory, firmware, userSafety, accountRecovery, duressResistance |
| **Privacy** | addressCorrelation, multiAddressCorrelation, privateTransfers, hardwarePrivacy, appIsolation, privacyHygiene |
| **Self-sovereignty** | l1ProviderIndependence, accountPortability, permissionsManagement, transactionInclusion, accountUnruggability |
| **Transparency** | openSource, sourceVisibility, funding, feeTransparency, releaseProcess, reputation |
| **Ecosystem** | accountAbstraction, addressResolution, browserIntegration, chainAbstraction, transactionBatching, hardwareWalletInteroperability, interoperability, appConnectionSupport |
| **Maintenance** | standalone group; software wallets omit it (applies to hardware/embedded wallets) |

The attribute folders mirror this layout under `src/schema/attributes/{security,privacy,self-sovereignty,transparency,ecosystem}/`, with a shared `common.ts` [6].

### How grading works

Each attribute is rated by an objectively-measurable, mostly pass/partial/fail rule. `src/schema/score.ts` defines `FAIL=0.0`, `UNRATED=-0.5`, and `EXEMPT=null` (excluded entirely — e.g. hardware-only attributes are EXEMPT for software wallets), plus a `weightedScore()` that sums `score × weight` over non-null scores and divides by summed weights; the final score runs from **-0.5 (fully unrated, worst) to 1.0 (best)**, and a `hasUnratedComponent` flag appends an asterisk [7]. ⚠ correction: the `PASS=1.0` / `PARTIAL=0.5` mapping is **not** in `score.ts` — it lives in `src/schema/attributes.ts`'s `defaultRatingScore()`, and is **verifiability-weighted, not flat**: `PASS` is 1.0 when self-evident but drops to 0.7 if independently audited and 0.1 if unverifiable; `PARTIAL` is 0.5 default, 0.2 if audited, 0.05 if unverifiable [a]. For multi-version wallets the system floors each attribute at its worst rating across versions [c].

## Walletbeat Stages — an L2BEAT-style maturity ladder for wallets

`src/schema/stages/software-wallet-stages.ts` defines a maturity ladder analogous to L2BEAT's rollup Stages [8]:

- **Stage 0** — "meets the minimum criteria for evaluation": the single criterion is publicly available source code (assessed via `sourceVisibility`).
- **Stage 1** — recent audit (within 1 year), hardware-wallet support across 3+ manufacturers, L1 chain verification, private-by-default token transfers, account portability/export, ability to use your own Ethereum node, a FOSS license, ENS human-readable addresses, and browser-integration standards compliance.
- **Stage 2** — funded bug bounty, address & multi-address non-correlation, permissionless L2→L1 withdrawals, custom RPC for all chains, public funding/revenue disclosure, fee transparency, chain-specific address resolution (ERC-7828/7831), Account Abstraction support, and atomic transaction batching.

⚠ unverified: A **Stage 0.5** is *not* present in the cited beta code file (the `stages` array is `[softwareWalletStageZero, softwareWalletStageOne, softwareWalletStageTwo]`), and the file contains no internal L2BEAT reference [8][b]. The Stage 0.5 concept and the explicit L2BEAT analogy come from EF/EthCC press coverage of the maturity model (described as unveiled by EF's Hester Bruikman at EthCC, ~April 2026), not from the code; the secondary news source for it is low-reliability [g].

## The attribute rules that matter most for an EOA desktop wallet

- **Account Portability** (`self-sovereignty`): for an EOA, `PASS` requires standards-compliant **BIP-39 + BIP-32 + BIP-44** derivation with an exportable seed phrase or private key; non-standard derivation but exportable key = `PARTIAL`; no key export = `FAIL` [14].
- **Security Best Practices** (`security`): key storage in a secure enclave / HSM = `PASS`; **standardized-KDF-encrypted or OS-sandboxed storage = `PARTIAL`**; weak/non-standard KDF, off-device key generation, MPC reconstruction that bypasses the user device, or closed source = `FAIL`. RNG: OS CSPRNG = `PASS`, unverified library RNG = `PARTIAL`. It hard-requires key material to be generated/reconstructed on the user's device [15].
- **Source visibility vs license** (`transparency`): these are two distinct attributes. `sourceVisibility` asks only whether code is public (irrespective of license): `PASS` if all repos are viewable, `PARTIAL` if only some components, `FAIL` if private. `openSource` (license) is stricter: `PASS` for OSI-definition FOSS (MIT/Apache/BSD/GPL), `PARTIAL`/`FUTURE_FOSS` for a delayed-FOSS license like BUSL, `FAIL` for proprietary, mixed, or **unlicensed** (conservatively treated as NOT_FOSS) [16][f].
- **L1 Provider Independence** (`self-sovereignty`): `PASS` only if a self-hosted node can be configured **before any request hits the default RPC** and all basic ops work through it; configurable-but-default-used-first = `PARTIAL`; no config / hard external dependency = `FAIL`. Motivation: don't leak address/IP to a default RPC [17].
- **Account Unruggability** (`self-sovereignty`): `FAIL` if the provider or any single external party can unilaterally take over/reconstruct the account, if keys live on external servers, or if the developer offers unencrypted seed backup on their own platform; `PASS` requires on-device key control [18].
- **Account Recovery** (`security`): evaluates **only guardian-based ("social") recovery — explicitly NOT seed-phrase backup**. `PASS` requires the recovery secret split across 3+ independent external services with 2+ different shares needed, no single party (including the provider) able to recover alone, and reconstitution on the user's device. It is fail/pass with no `PARTIAL` [18].
- **Security Audits** (`security`): `PASS` = audited within the last 365 days with all medium+ findings fixed; `PARTIAL` = stale (>1yr) or recent-but-unresolved findings; `FAIL` = never audited or stale with unresolved findings; no audit data => unrated, not auto-fail [c].

The rule-selection philosophy (per the FAQ): attributes are chosen for Ethereum/cypherpunk alignment, shared ecosystem goals, and *not-already-market-driven* gaps (e.g. supply-chain security, data privacy). The scoring rules must be objectively measurable, technology-neutral, immediately feasible, pragmatic, and designed to **raise the bar over time** [3].

## WalletScrutiny — the complementary "can you trust the binary" check

WalletScrutiny (`walletscrutiny.com`) answers a different question than Walletbeat: does the binary users run actually match the published source (a **reproducible build**)? It targets the exit-scam / bait-and-switch attack [9][10]. It assigns **categorical verdicts, no numeric score** — e.g. positive: "Source code is available", "Do-It-Yourself Project"; negative: "Custodial: The provider holds the keys", "No source for current release found", "Obfuscated", "Provided private keys", "Leaks Keys" — plus status verdicts ("Review is Work in Progress", "Discontinued") [9]. Android/desktop evaluation runs review-status → authenticity → is-it-a-wallet → custody → source availability → obfuscation → reproducibility → maintenance. **No iPhone app has been reproducible** because Apple restricts the needed access, so the burden of proof is shifted onto providers/Apple [9]. The project stresses reproducibility verifies a point-in-time match, not the absence of malware or a future bait-and-switch [9].

The canonical source is **GitLab** (`gitlab.com/walletscrutiny/walletScrutinyCom`); the GitHub repo (`github.com/WalletScrutiny/WalletScrutinyCom`) is a mirror (~8,989 commits, JS-heavy, actively maintained) [10][h]. Originally Bitcoin-focused, it now covers mobile/desktop/hardware across multiple asset classes, runs a community "Verifications" model with an automated build server that re-runs reproducibility scripts on new releases, and is decentralizing verdict data via **Nostr event specifications** so other apps can consume verdicts [10][i].

## What's *not* a ranking

- **L2BEAT** (`l2beat.com`) tracks L2 rollups — TVS, activity, risk, and a Stages framework introduced **June 19, 2023** (Stage 0 "Full Training Wheels" → Stage 1 "Limited Training Wheels" → Stage 2 "No Training Wheels") that rates rollup decentralization/trust-minimization. It does not rank wallets; it is purely the template Walletbeat borrowed [11][12].
- **ethereum.org wallet finder** is a curated, filterable directory (~52 wallets, "not official endorsements ... for informational purposes only") with filters for non-custody, open source, hardware, multisig, social recovery, privacy, smart accounts, account upgrades, custom RPC import, gas customization, ENS, etc. Listing requires EIP-1559 (type-2) support, an Ethereum/L2 default network, 6+ months live (or an established team), and one of an audit / internal security team / open-source review — not strictly an audit [13][j].
- DeFiLlama and "top 10 wallets" pages are SEO listicles, not codified rubrics — treat as low-reliability. The credible, codified options are **Walletbeat** (values/feature scorecard + Stages) and **WalletScrutiny** (reproducibility verdicts).

## What this means for Deckard

Observations and opportunities only — not a roadmap.

- **A codified, open checklist already exists.** Walletbeat's attribute groups and Stage criteria are a public, machine-readable spec of "what a good Ethereum wallet has," and any wallet can self-assess against it without permission [6][8].
- **Source visibility gates everything.** Walletbeat Stage 0 requires public source code merely to *qualify for evaluation*; a closed-source wallet is effectively below Stage 0 and unrated [8].
- **Deckard's 0BSD license clears the license bar.** 0BSD is OSI-approved/FOSS, so `transparency.openSource` would be a `PASS` — though any unlicensed component would conservatively be treated as NOT_FOSS and could drag it to FAIL [16][f].
- **The self-custodial EOA structurally aligns with the highest-leverage attributes.** Keys generated and held on-device (no provider able to take over) is a strong `accountUnruggability` candidate, and the planned BIP-39/BIP-32/BIP-44 seed backup with exportable keys maps directly onto `accountPortability`'s `PASS` rule [14][18].
- **The planned keystore lands at `PARTIAL`, not `PASS`.** An Argon2id + XChaCha20-Poly1305 encrypted keystore reads as "standardized-KDF-encrypted / OS-sandboxed" storage = `PARTIAL` under `securityBestPractices`; a `PASS` requires a hardware/secure-enclave path. Persisting an unencrypted key to the OS config dir (v0) sits at the `PARTIAL`/`FAIL` boundary [15].
- **RNG is likely already a `PASS`** if key generation uses an OS CSPRNG (alloy/getrandom draws from the OS CSPRNG) [15].
- **Several attributes are FAIL/unrated until external milestones land**, independent of code quality: `securityAudits` (no independent audit), `bugBountyProgram` (no funded program), `l1ProviderIndependence` (PASS needs user-set self-hosted RPC before first request), plus privacy non-correlation, hardware support, and Ecosystem items (account abstraction, batching, ENS, WalletConnect/EIP-6963) [8][14][15].
- **The operator-wallet vision intersects directly with `accountUnruggability` and `securityBestPractices`.** A *local* or self-custodial LLM agent keeps keys on-device and aligns with the rubric; any cloud component that could move funds without on-device key control would jeopardize those PASS ratings — and note that `accountRecovery` credits only 3+-guardian social recovery, so seed backup alone does not score there [15][18].

## Open questions

- The EF ESP grant proposal **requested** $106,100 and its front-matter is marked **"Status: Funded"**; was the full sum actually disbursed? (Primary source confirms "Amount: 106100 USD" and "Status: Funded", but disbursement of the full amount is not separately proven) [d].
- Where does the canonical "Stage 0.5" definition live, given it is absent from the beta code file? Is it slated to land in code, or is it press-only framing? [b][g].
- How does the verifiability-weighting in `defaultRatingScore()` change real-world rankings versus a flat pass/partial/fail — i.e. how much does "independently audited" vs "self-evident" move a score? [a].
- Does Walletbeat currently list any native Rust / GPUI / desktop EOA wallets, and how are pure desktop (non-extension, non-mobile) wallets scored on browser-integration and app-connection attributes?
- Would a desktop wallet that defaults to a bundled RPC but exposes a pre-first-request custom-RPC setting clear `l1ProviderIndependence`'s `YES_BEFORE_ANY_REQUEST` bar? [17].

## Sources

[1] walletbeat/walletbeat — "the L2BEAT of wallets" repo — https://github.com/walletbeat/walletbeat — (github, high)
[2] Walletbeat (live beta site) — https://beta.walletbeat.eth.limo/ — (docs, high)
[3] Walletbeat FAQ — rubric philosophy, scoring, governance — https://beta.walletbeat.eth.limo/faq/ — (docs, high)
[4] Walletbeat legacy site (older feature-matrix schema) — https://www.walletbeat.fyi/ — (docs, medium)
[5] Walletbeat README — https://github.com/walletbeat/walletbeat/blob/main/README.md — (github, high)
[6] attribute-groups.ts — full list of scored attributes by group — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attribute-groups.ts — (github, high)
[7] score.ts — FAIL=0.0/UNRATED=-0.5/EXEMPT=null & weighted average — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/score.ts — (github, high)
[8] software-wallet-stages.ts — Stage 0/1/2 ladder — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/stages/software-wallet-stages.ts — (github, high)
[9] WalletScrutiny methodology — reproducible builds, verdicts — https://walletscrutiny.com/methodology/ — (docs, high)
[10] WalletScrutiny GitHub mirror — https://github.com/WalletScrutiny/WalletScrutinyCom — (github, high)
[11] L2BEAT — L2 ecosystem summary (no wallet ranking) — https://l2beat.com/scaling/summary — (docs, high)
[12] L2BEAT — Introducing Stages (June 19, 2023) — https://medium.com/l2beat/introducing-stages-a-framework-to-evaluate-rollups-maturity-d290bb22befe — (blog, high)
[13] ethereum.org wallet finder (filterable directory) — https://ethereum.org/en/wallets/find-wallet/ — (docs, high)
[14] account-portability.ts — BIP-39/32/44 export rating — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/self-sovereignty/account-portability.ts — (github, high)
[15] security-best-practices.ts — key storage, RNG, hardening — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/security/security-best-practices.ts — (github, high)
[16] open-source.ts — license rating (FOSS/FUTURE_FOSS/NOT_FOSS) — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/transparency/open-source.ts — (github, high)
[17] l1-provider-independence.ts — own-node/RPC rating — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/self-sovereignty/l1-provider-independence.ts — (github, high)
[18] account-unruggability.ts & account-recovery.ts — provider-takeover and social-recovery rules — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/self-sovereignty/account-unruggability.ts — (github, high)
[a] attributes.ts — defaultRatingScore(): PASS/PARTIAL→number with verifiability adjustments; repo is MIT-licensed — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes.ts — (github, high)
[b] stages.ts — StageCriterionRating enum & WalletStage type; confirms stages 0/1/2, no 0.5 in code — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/stages.ts — (github, high)
[c] Walletbeat FAQ — origin (Moritz created it; 2025 revamp by polymutex), scoring philosophy, DAO goal — https://beta.walletbeat.eth.limo/faq/ — (docs, high)
[d] Walletbeat ESP grant proposal — "Amount: 106100 USD", "Status: Funded", Fluidkey-ineligibility, separate "Pectra Proactive Grant" = $577.02 — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/governance/grants/2025-07-ethereum-foundation-esp-grant-proposal/proposal.md — (github, high)
[e] Walletbeat About page — "As L2Beat has done for Ethereum Layer 2s, Walletbeat aims to do the same"; MIT-licensed; affiliation disclosure — https://beta.walletbeat.eth.limo/about/ — (docs, high)
[f] source-visibility.ts — public-code rating (irrespective of license) — https://raw.githubusercontent.com/walletbeat/walletbeat/beta/src/schema/attributes/transparency/source-visibility.ts — (github, high)
[g] EF/EthCC coverage of the wallet security maturity model (Stage 0.5 framing) — https://www.binance.com/en/square/post/308159202760305 — (news, low)
[h] WalletScrutiny canonical repo on GitLab (GitHub is the mirror) — https://gitlab.com/walletscrutiny/walletScrutinyCom — (gitlab, high)
[i] WalletScrutiny — User-Created Verifications on Nostr (decentralized verdict-sharing) — https://walletscrutiny.com/verifications/ — (docs, high)
[j] (covered under [13]) ethereum.org listing criteria — audit OR internal security team OR open-source review — https://ethereum.org/en/wallets/find-wallet/ — (docs, high)
