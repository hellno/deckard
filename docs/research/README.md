# Deckard Wallet Research Knowledge Base

> The 2026 state of the art for crypto wallets — capabilities, account abstraction, the EF's
> Kohaku, Splits' agentic/smart-account model, AI-driven "operator" wallets, privacy, the
> "L2BEAT-for-wallets" scorecard, and key-management security — assembled as a reference for
> building **Deckard** (a native Rust/GPUI, cross-platform, self-custodial desktop wallet with an
> LLM operator-wallet vision). Researched **2026-06-05**.

This is a **research knowledge base, not a product plan.** Every file ends with a neutral
"What this means for Deckard" section of *observations and opportunities only* — no sequencing,
priorities, or roadmap. Product planning is a deliberate next step done against this material.

## How this was built

Each file was produced by an independent three-stage pipeline: **deep research** (many web +
GitHub-repo searches, reading primary pages) → **adversarial verification** (a skeptic re-checked
every load-bearing claim against a primary source, defaulting to "unverifiable" when it couldn't be
confirmed) → **write** (only confirmed/partial claims asserted; refuted or unconfirmable ones either
dropped or flagged inline with `⚠ unverified`). Verifier corrections are baked in throughout
(e.g. EntryPoint version attribution, MetaMask's PBKDF2 iteration count, the Wintermute 7702 stat's
framing, AgentKit's real version, x402's daily-volume decline).

## Source-reliability legend

Sources are tagged `(kind, reliability)`. Prefer **high** when acting on a claim.

- **high** — official docs, GitHub repos/releases/source, EIP/ERC/RIP specs, EF blog, primary forum
  threads (ethereum-magicians / ethresear.ch), Linux Foundation / standards bodies.
- **medium** — reputable secondary deep-dives, vendor blogs making first-party claims, market-maker
  research, well-sourced trade press.
- **low** — single-source aggregators or promotional posts; used only where no primary source exists,
  and flagged.

Inline `[n]` citations in each file resolve to its own numbered **Sources** section. (Files 04 and 07
use `[n]`-style source numbering; the rest use `n.` — internally consistent within each file.)

## The files

| # | File | What's inside | Anchor facts (verified) |
|---|------|---------------|-------------------------|
| 01 | [`01-landscape-2026.md`](01-landscape-2026.md) | SOTA capability map: AA in practice, the standards mesh, recovery/gas/batching, embedded vs local, security baseline, agentic primitives, where most wallets still fall short | 4337+7702 *compose* (the shipped pattern); MetaMask shipped ERC-7715/7710 "Advanced Permissions" Apr 6 2026; native-AA EIP-8141 only "Considered for Inclusion" for late-2026 Hegota |
| 02 | [`02-account-abstraction.md`](02-account-abstraction.md) | The technical AA substrate + **the EOA→smart-account migration path** + Rust tooling reality | EIP-7702 Final, live in Pectra May 7 2025; EntryPoint v0.8 added native 7702 + audited Simple7702Account; alloy `TransactionBuilder7702` / `alloy-eip7702` + Rust bundlers Rundler & Silius exist |
| 03 | [`03-kohaku.md`](03-kohaku.md) | The EF wallet: it's **two repos** — a Rust+TS SDK and an Ambire-fork extension; architecture, privacy stack, roadmap | Crypto core is **Rust → WASM → TS**; `@kohaku-eth/railgun` shipped at alpha; "local-AI tx scoring" is exploratory, "native AA" is L1 advocacy |
| 04 | [`04-splits.md`](04-splits.md) | Splits' agentic, smart-account-native model: how agents become signers, the CLI/MCP surface, what's missing | Custom 4337 "Smart Vaults" (not Safe); `@splits/splits-cli` is **one binary = CLI + MCP server**; "agents as signers" shipped 2026-05-28; **no** client-exposed spend limits / session keys yet |
| 05 | [`05-agentic-wallets.md`](05-agentic-wallets.md) | **The core dimension** — AgentKit/GOAT/MCP servers, x402/AP2/ERC-8004, and the safe-signing architecture for an LLM operator | Convergent axiom: **the agent never sees the seed**; MCP is the integration surface; dual-key (scoped signer + master override); simulate-before-sign; x402 Foundation launched at the Linux Foundation Apr 2 2026 |
| 06 | [`06-privacy.md`](06-privacy.md) | Privacy as a stack: stealth addresses, shielded pools, FHE, the metadata/RPC layer, regulatory backdrop | Vitalik's 4-pillar L1 privacy roadmap; PSE rebrand + ~47-person Privacy Cluster; opposite compliance models (Privacy Pools allowlist vs Railgun PPOI blocklist); EIP-8182 protocol-native shielded pool proposed for Hegota; Helios is a Rust embeddable light client |
| 07 | [`07-wallet-rankings.md`](07-wallet-rankings.md) | The "L2BEAT for wallets" and its **codified rubric** (a ready-made checklist) | **Walletbeat** (`beta.walletbeat.eth.limo`, MIT repo) rates 5 attribute groups + a Stages ladder; WalletScrutiny does reproducible-build verdicts; L2BEAT itself does **not** rank wallets |
| 08 | [`08-security-keystores.md`](08-security-keystores.md) | Key-management field standards; validates Deckard's locked keystore; flags the v0 risk | Web3 Secret Storage v3 is the floor; v0 **plaintext key on disk is below it**; Argon2id+XChaCha20 is stronger but non-interoperable; Secure Enclave is **secp256r1-only**; EIP-7951 put P-256 on mainnet (Fusaka, Dec 3 2025) |
| 09 | [`09-deckard-relevance.md`](09-deckard-relevance.md) | **Cross-cutting synthesis** — the recurring threads across all eight files and the opportunity surface (observations only) | — |

## How to use it

- **Orienting / sharing context?** Read this README + the TL;DR of each file.
- **Planning a feature area?** Open the matching file; the "What this means for Deckard" and
  "Open questions" sections are the seams into product work.
- **Want the big picture?** Read [`09-deckard-relevance.md`](09-deckard-relevance.md) — it threads the
  themes that recur across files (the EOA→smart-account hinge, the operator-wallet blueprint, the
  privacy stack, the public scorecard, the security floor) and the genuine white space.
- **Acting on a claim?** Check its `[n]` source and reliability tag first; treat `⚠ unverified`
  notes as open items, not facts.

## Recurring threads (one line each — detail in file 09)

1. **The EOA→smart-account hinge.** Almost every advanced capability needs a smart account or a
   **7702-delegated EOA**; 7702 is the address-preserving bridge, and the Rust tooling already exists.
2. **The operator-wallet blueprint has converged** — agent-never-sees-the-seed, dual-key, local MCP
   sidecar, simulate-before-sign — but scoped on-chain permissions need a smart account; on a bare
   EOA the same limits must live in a local policy gate.
3. **Privacy is a stack of independent properties**, and the most wallet-controllable layer
   (operational/RPC/metadata) is the most neglected — and is Rust-native (Helios).
4. **A public, codified scorecard exists** (Walletbeat) — a permissionless checklist of "what a good
   wallet has," with Deckard's structural strengths and externally-gated gaps both legible.
5. **The security floor is non-negotiable and Rust-served** — the locked encrypted keystore clears it;
   v0's plaintext key does not. Hardware/enclave and passkeys are smart-account-gated.
6. **The white space:** no shipping consumer wallet offers safe, scoped, revocable end-to-end
   LLM-operator control as a product — it exists today only as infra plumbing plus MetaMask's
   just-launched permissions.
