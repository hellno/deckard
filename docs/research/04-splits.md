# Splits — agentic, smart-account-native

> How Splits (formerly 0xSplits) turned onchain payment-splitting into a self-custodial, ERC-4337 smart-account "onchain bank" with agents as first-class signers. Part of the Deckard wallet research KB. Researched 2026-06-05.

## TL;DR

- Splits evolved from a non-upgradable onchain payment-splitting protocol into "Splits Teams" — self-custodial onchain banking built on a **custom ERC-4337 smart account** they call **Smart Vaults** (EntryPoint v0.7), a tiered system of multi-chain m-of-n multisigs supporting both passkeys (WebAuthn/secp256r1) and EOAs as signers, with ERC-1271 verification [2][9].
- The agentic surface is **shipped, not aspirational**: a single binary, `@splits/splits-cli`, is *both* a CLI and an MCP server — running it with `--mcp` auto-exposes every command as an MCP tool with no separate registration step [1].
- On **2026-05-28** Splits shipped "agents as signers on accounts, enabling server keys and agents to transact on behalf of teams," plus custom JSON transaction metadata. The CLI/MCP + scoped-keys path shipped **2026-04-14**; subaccount management + signing shipped **2026-04-28** [4].
- An agent becomes a signer by **registering its EOA** (`splits auth register-signer`) and **attaching it to a specific subaccount** (`splits accounts update-signers`), then signing pending multisig UserOps locally (`splits transactions sign`). No seed phrase is ever handed over [3].
- The security model is **three layered controls**: scoped API keys (read-only vs owner-scoped), per-account signer membership, and the multisig threshold. In MCP mode the CLI **refuses flag-based secrets** so keys never appear in tool-call transcripts [5].
- There are **no client-exposed spend limits, allowlists, or session keys** in the CLI/MCP today. The only allowlist-like surfaces (`tokens whitelist`, `tokens blocklist`) and `automations list` are **read-only** [6].
- Smart Vaults support **Merkelized UserOps** (sign one Merkle root to authorize many UserOps across networks/accounts) and **Light UserOps** (early signers sign a reduced op so the final signer prices gas at execution) [7].
- The protocol layer — Split/SplitV2, Warehouse, Waterfall, Swapper, Vesting, plus templates (Liquid Split, Recoup, Diversifier) — is **non-upgradable, fee-free** (gas-cost-only) and deployed across Ethereum, Optimism, Base, Zora, Polygon, Arbitrum and more [5][8].
- Forward signals: Splits maintains a **fork of Ithaca's Porto** (EIP-7702 + passkeys account stack, last pushed 2026-04-09) [10], and runs forks of two agent-infra projects — `centaur` (credential-bounded team agents) and `iron-proxy` (egress firewall with boundary secret injection) [9].
- For Deckard: an existing alloy EOA can become a Splits signer with almost no new crypto, and the v2 distribution contracts are directly callable — but *natively owning* a smart account means adopting a 4337 stack and a bundler/paymaster pipeline.

## From splitting protocol to onchain bank

The original **0xSplits** is a set of non-upgradable, fee-free "hyperstructure" contracts (it "runs exactly at gas cost"; "non-upgradable contracts run as long as the underlying network exists") with full/direct support on ~13 chains (Ethereum, Base, Optimism, Arbitrum, Celo, World Chain, and more), plus partial support on 70+ EVM networks via bridging (relay.link) [8]. The core primitives (per docs.splits.org) are **Split/SplitV2** (distribute incoming tokens by fixed percentage shares), **Warehouse** (a central balance/distribution hub with ERC-6909-style internal accounting that batches and reduces gas), **Waterfall** (tranched/priority payouts), **Swapper** (accept any input token, pay out a chosen output token), **Vesting** (time-locked release), and an **Oracle** primitive; **Liquid Split**, **Recoup**, and **Diversifier** are classified as *templates* built on those primitives rather than standalone primitives [8]. Splits v2 shipped 2024-05-09; the latest contracts release is **Splits v2.2 (2025-07-14)** [9].

In 2026 Splits repositioned as **"Modern banking for onchain startups"** (verbatim on splits.org/treasury/) with the pitch "the speed and workflows of Mercury, the security and peace of mind of multisigs" (verbatim on splits.org/treasury/); the agent-positioning quote "Agents can do everything people can, without the insecurity of handing over a seed phrase" is verbatim on the splits.org homepage; by end-2025, 100+ teams had processed over $50M through the product [12]. ⚠ unverified: only the exact string "onchain banking for startups and solo builders" remains unconfirmed verbatim (primary instead says "Modern banking for onchain startups"); the load-bearing technical claims below are confirmed from CLI source and the changelog.

## Smart-account architecture: custom ERC-4337 "Smart Vaults"

Splits does **not** use Safe. The account is a bespoke contract suite, **Smart Vaults**, in `packages/smart-vaults` of `splits-contracts-monorepo` (Solidity, GPL-3.0, Foundry/turborepo/pnpm), released as `smart-vaults-v1.0` on **2025-04-08** [9]. The README states they are "Splits 4337 smart accounts ... a tiered system of multi-chain multi-sigs" and "currently ... compatible with entry point v0.7" [2].

Source layout and features [2]:

| Component | Role |
|---|---|
| `src/vault/SmartVault.sol`, `SmartVaultFactory.sol` | the account + its factory |
| `src/signers/MultiSigner.sol` | m-of-n signer set |
| `src/signers/PasskeySigner.sol` | WebAuthn / secp256r1 (P-256) signer |
| `src/signers/AccountSigner.sol` | "a signer backed by an EOA or ERC-1271 smart account" |
| `src/utils/ModuleManager.sol` | add trusted modules that act on behalf of the account |
| `src/utils/FallbackManager.sol` | extensible callbacks; ERC-721/1155 receiving |

The **Module Manager** is the extensibility hook where policy/automation modules could live — conceptually analogous to ERC-7579 modular accounts, but it is **their own design, not an off-the-shelf ERC-7579 account**. The account also supports contract deployment via `CREATE` inside a UserOp.

## Cross-chain signing: Merkelized and Light UserOps

Two mechanisms reduce friction for multi-chain multisigs [7]:

- **Merkelized User Operations** — the signer builds a Merkle tree of all intended UserOps (across any number of networks and accounts), signs the single Merkle root **once**, and each submitted UserOp carries a Merkle proof for verification. There is "no strict limit on the number of operations." This is how a human or agent authorizes a batch of cross-chain actions with one signature.
- **Light User Operations** — when threshold > 1, the first *threshold − 1* signers sign over a reduced UserOp (only `sender`, `nonce`, `calldata`; excluding `initCode`, gas limits, `preVerificationGas`, `gasFees`, `paymasterAndData`, `signature`), so the **final signer prices gas at current market conditions**.

Together these directly serve an operator-wallet pattern: a human pre-authorizes intent and a later signer (or agent) finalizes execution and gas.

## The agentic surface: one binary, CLI + MCP

`@splits/splits-cli` (v0.2.9, last published 2026-05-22) is a single-file (`src/cli.ts`) Node 22+/TypeScript ESM tool built on the **`incur`** framework (wevm/incur). `cli.serve()` runs the CLI, and invoking it with `--mcp` exposes **every command as an MCP tool automatically** — incur documents "no manual config, no copy-pasting tool definitions" [1].

Command namespaces map 1:1 to backend resources: **`accounts`, `transactions`, `contacts`, `tokens`, `chains`, `members`, `settings`, `automations`, `auth`, and `org`** (the `org create` flow is an unauthenticated email-link org setup) [1]. Key commands include `auth login/whoami/create-key/register-signer/signers`, `accounts list/get/signers/create/rename/archive/update-signers`, `transactions list/get/sign/properties`, and `mcp add` (auto-detects Claude Code / Cursor). MCP tools mirror these with underscore names (`transactions_sign`, `accounts_create`, `auth_register_signer`). Registration: `claude mcp add splits -e SPLITS_API_KEY=sk_... -- npx @splits/splits-cli --mcp` [1].

The public API is reached at `SPLITS_API_URL + /public/v1` with a Bearer token (default base `https://server.production.splits.org`); API keys (`sk_...`) are issued from Teams Settings. Transaction rows expose `direction`, `transactionHash`, and `userOpHash` (both nullable) so callers can correlate Splits records with explorers and bundler webhooks — a clean REST surface an external wallet could call directly without the CLI [14].

> Note: the *published README* documents only the `auth`/`accounts`/`transactions`/`members` namespaces. Evidence for `tokens`/`chains`/`contacts`/`settings`/`automations`/`org` and the production hostname lives only in `src/` (`cli.ts`, `config.ts`, `http.ts`).

## How signing authority is delegated safely

Delegation is **layered, and notably does not yet use onchain session keys or client-exposed spend-limit modules** [3][5]:

1. **Scoped API keys** — issued per-team; some read-only, some owner-scoped (subaccount create/archive/rename require an owner-scoped key) [5].
2. **Signer membership** — the agent's EOA must be registered (`auth register-signer`, idempotent, returns an id) *and* attached to a specific subaccount (`accounts update-signers --add-eoa-signer-ids`). An unattached key can sign nothing [3].
3. **Multisig threshold** — an account can require m-of-n, so an agent can be configured to merely *propose* / partially-sign while a human provides the final signature (`--no-submit` records a signature without submitting) [3].

Hygiene controls: the private key lives only in `~/.splits/config.json` (mode 0600, auto-gitignored) and "never appears in any command's response — only the derived address." Under `SPLITS_MCP_MODE=1` (or `--mcp`) the CLI **refuses `--api-key`/`--private-key` flags** so secrets never leak into MCP tool-call transcripts; stdin is preferred for key entry [3][5].

## What's not there yet (spend limits, session keys, allowlists)

Despite a smart-account foundation that could support it, the agent-facing surface has **no spend-limit, allowlist-enforcement, or session-key commands** — a source review of `src/cli.ts` (v0.2.9) finds none [6]. The only allowlist-like surfaces are read-only `tokens whitelist` (GET `/tokens/whitelist`; described as "allowlisted tokens") and `tokens blocklist`, plus a read-only `automations list` (GET `/automations`) [6]. Today the granularity comes from **API-key scope + signer attachment + multisig threshold**, not from per-action onchain policy. Treat per-transaction spend policy as server/account-side and roadmap-level.

## Forward signals: 7702 and agent infrastructure

- **Porto (EIP-7702):** Splits maintains a **fork of Ithaca's Porto** ("Porto — Next-gen Account for Ethereum"), last pushed 2026-04-09. Porto is the EIP-7702 + WebAuthn/passkeys account stack (RIP-7212 P256 precompile, app sessions / permissions) [10]. Maintaining a 7702 fork alongside their bespoke 4337 Smart Vaults suggests they are *evaluating* a 7702-based path (upgrade an EOA in place) — this is a **fork, not a shipped product line** (experimental). (Attribution note: Porto is Ithaca's; "Reth" is a separate Paradigm execution client.)
- **Agent infrastructure (forks, not homegrown):** `centaur` — "Shared AI agents for teams" with **credential boundaries** ("agents can use approved services without receiving raw API keys"), isolated Kubernetes sandboxes, bring-your-own-harness (Claude Code/Codex/Amp), durable sleep/resume/spawn workflows. `iron-proxy` — a default-deny MITM egress firewall that injects **real secrets at the network boundary** ("workloads use proxy tokens ... a compromised workload can exfiltrate a token that's worthless outside the proxy"), blocking SSRF/DNS-rebinding to `169.254.169.254`/loopback, with per-request JSON audit [9]. ⚠ correction: both repos are **forks** (`centaur` from `paradigmxyz/centaur`, `iron-proxy` from `ironsh/iron-proxy`), not Splits-built; Splits adopting them signals its agent-security worldview, but does not imply authorship. (`centaur` last pushed 2026-06-04; `iron-proxy` last pushed 2026-05-29.)

## Adjacent surfaces

The **TypeScript SDK** (`@0xsplits/splits-sdk` core + `splits-sdk-react` + `splits-kit` components) is at **v6.4.1 (2026-01-22)** as the latest GitHub release tag (npm latest is **6.5.0**, published 2026-03-19) across 65 releases, but targets the original 0xSplits **protocol contracts + subgraph**, *not* the new Teams smart-account/agent API (which lives behind `/public/v1` and the CLI) — for a Rust consumer like Deckard it is reference material, and the REST API is the integration point [11]. **Splits Connect** (`splits-connect`, shipped per the 2026-04-28 changelog) is a browser extension that lets a self-custodied Teams smart account act as a wallet in external dapps via WalletConnect + injected provider, with batch transactions [13]. Recovery is framed as re-establishing a sufficient signer set (passkey + EOA), with email-based recovery for team accounts shipped 2026-04-14 — a multisig-smart-account custody model rather than single-seed restore [13].

## What this means for Deckard

- **Deckard's existing alloy secp256k1 EOA can become a Splits signer with almost no new crypto** — register it via the public API/CLI flow and co-sign multisig UserOps; Deckard only needs an API token plus the ability to produce ERC-1271/EOA signatures over a UserOp or Merkle root it can already sign [14][3].
- **The v2 distribution contracts (Split, Warehouse, Waterfall, Swapper, Diversifier, Vesting) are open-source, non-upgradable, with full/direct support on ~13 chains (Ethereum, Base, Optimism, Arbitrum, Celo, World Chain, and more) plus partial support on 70+ EVM networks via bridging** — Deckard could *call* them directly to split revenue without any account migration [8][3].
- **The one-binary CLI-and-MCP pattern is a directly copyable design** for Deckard's own LLM-operator surface: scoped keys, MCP-mode secret refusal, keys never in transcripts, secrets only in a 0600 config file [1][5].
- **Natively owning a smart account is the heavy path:** it requires adopting a 4337 account stack (Smart-Vaults-like contracts, or a 7702 path à la their Porto fork), running or renting a bundler + paymaster, and implementing UserOp construction, Merkelized/Light-UserOp signing, and EntryPoint v0.7 packing [2][7][10].
- **Merkelized + Light UserOps map cleanly onto the operator-wallet thesis** — a human can pre-authorize a batch of cross-chain intent with one signature and let a later signer (or agent) finalize gas/execution [7].
- **Splits does not give you operator-LLM spend-policy off the shelf** — there are no client-exposed spend limits, allowlists, or session keys today; a policy layer would be Deckard's to build [6].
- **The `iron-proxy` / `centaur` "revocable, low-blast-radius credential" pattern is the security model to study** for an autonomous local operator — applied to signing authority, the agent holds a scoped, attachable, revocable signer key rather than a master seed [9].
- **Off-chain treasury/fiat services Splits operates** — splits.org/treasury/ confirms invoicing plus generic bank transfers / fiat ramps / yield on idle cash (specific product names like ACH/SEPA rails, 1099 tax forms, or a "USDC Earn" product are not stated verbatim) — they are not reusable as contracts; consuming them would mean integrating Splits' API or comparable ramps [12][13].

## Open questions

- Will Splits expose onchain spend-limit / session-key / allowlist primitives to clients (closing the gap between the "granular cryptographic approvals" framing and today's API-key-scope reality), and via the Module Manager or a 7702/Porto path?
- Is the Porto fork headed for production as a 7702 in-place EOA-to-smart-account upgrade, or is it pure evaluation?
- Does the public `/public/v1` API expose enough (UserOp construction, signature submission, Merkle-root retrieval) for a non-TS client to act as a signer without the CLI, or is the CLI the de facto SDK?
- How are recurring/scheduled transactions and Automations actually executed (relayer/bundler cadence, who pays gas), and can an external signer participate?
- What are the licensing implications for Deckard of the GPL-3.0 Smart Vaults contracts versus calling them as deployed bytecode?

## Sources

[1] splits-cli README + `src/cli.ts` (CLI + MCP server) — https://github.com/0xSplits/splits-cli — (github, high). Framework: https://github.com/wevm/incur
[2] Smart Vaults README (ERC-4337 v0.7 account architecture) — https://github.com/0xSplits/splits-contracts-monorepo/blob/main/packages/smart-vaults/README.md — (github, high)
[3] Splits Teams — onchain banking (signer-delegation flow) — https://splits.org/teams/ — (docs, high)
[4] Splits Changelog (2023-01 → 2026-05) — https://splits.org/changelog/ — (docs, high)
[5] splits-cli `src/http.ts` / `src/config.ts` (scoped keys, MCP secret refusal) — https://github.com/0xSplits/splits-cli — (github, high)
[6] splits-cli `src/cli.ts` command inventory (no spend-limit/session-key commands) — https://github.com/0xSplits/splits-cli — (github, high)
[7] Smart Vaults README — Merkelized + Light UserOps — https://github.com/0xSplits/splits-contracts-monorepo/blob/main/packages/smart-vaults/README.md — (github, high)
[8] Splits protocol documentation (primitives, Warehouse, fee-free/non-upgradable) — https://docs.splits.org/ — (docs, high)
[9] splits-contracts-monorepo (Splits v2 + Smart Vaults) — https://github.com/0xSplits/splits-contracts-monorepo — (github, high). Agent-infra forks: centaur — https://github.com/0xSplits/centaur (upstream https://github.com/paradigmxyz/centaur); iron-proxy — https://github.com/0xSplits/iron-proxy (upstream https://github.com/ironsh/iron-proxy)
[10] 0xSplits fork of Ithaca Porto (EIP-7702 account stack) — https://github.com/0xSplits/porto — (github, high). Upstream context: https://ithaca.xyz/updates/porto
[11] splits-sdk (TypeScript protocol SDK; v6.4.1 is the latest GitHub release tag, npm latest is 6.5.0 published 2026-03-19) — https://github.com/0xSplits/splits-sdk — (github, high)
[12] Splits Treasury — "Modern banking for onchain startups" + 2025 in review — https://splits.org/treasury/ — (docs, high). Traction: https://splits.org/blog/2025-in-review/
[13] splits-connect (browser extension for Teams accounts) — https://github.com/0xSplits/splits-connect — (github, high)
[14] splits-cli `src/config.ts` + `src/http.ts` (public API shape: `/public/v1`, Bearer, production host) — https://github.com/0xSplits/splits-cli — (github, high)
[15] ERC-4337 account-abstraction EntryPoint v0.7 — https://github.com/eth-infinitism/account-abstraction/tree/releases/v0.7 — (github, high)
[16] ERC-1271: Standard Signature Validation for Contracts — https://eips.ethereum.org/EIPS/eip-1271 — (spec, high)
