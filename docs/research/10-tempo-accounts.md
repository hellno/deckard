# Tempo Accounts SDK — agent-first devex on a payments chain

> What `tempoxyz/accounts` (the Accounts SDK for apps/wallets building stablecoin experiences on
> Tempo, the payments-focused L1) does well in devex, documentation, and agentic experience — and
> which of those patterns transfer to a self-custody desktop wallet. Part of the Deckard wallet
> research KB. Researched **2026-06-11**.
>
> ⚠ **Verification level:** lighter than files 01–09 — single-pass research (repo source via raw
> GitHub + web search), no adversarial-verification stage. `accounts.tempo.xyz`, `docs.tempo.xyz`,
> and `tempo.xyz/blog` were bot-blocked (HTTP 403) at research time, so protocol-level claims rest
> on search-result excerpts of those docs plus secondary analyses; they are tagged accordingly.

## TL;DR

- `tempoxyz/accounts` is a **TypeScript SDK (npm package `accounts`)** for integrating accounts
  into apps on Tempo. Monorepo: `src/`, **13 runnable examples**, 3 playgrounds (web/wagmi/CLI),
  reference implementations (embed dialog, CLI device-code auth), and a Vocs docs site. Dual
  Apache-2.0/MIT, **81 releases** (latest v0.14.9 at research time) — high cadence on a pre-1.0
  surface [1].
- The headline features lean on **protocol-native account abstraction**: Tempo Transactions (an
  EIP-2718 tx type) natively support WebAuthn/P256 signature validation, **access keys** (root key
  provisions scoped secondary keys with expiry + per-token spend limits), **fee sponsorship** via a
  dual-signature "fee payer envelope," 2D parallelizable nonces, call batching, and scheduled
  transactions — no ERC-4337 stack required [6][7].
- **Access keys are the session-key UX bar:** authorize once on connect, then "subsequent matching
  transfers are signed locally without the wallet popup"; default expiry 24 h; the delegate can be
  a device-bound non-extractable WebCrypto key [4][6].
- **MPP (Machine Payment Protocol)** ships as a working example: the SDK auto-intercepts HTTP 402
  responses, signs a payment, and retries. Two intents: a **zero-charge "auth" intent**
  (replay-protected proof of account ownership, no on-chain tx) and a **charge intent** ($0.01
  settled on-chain before the response is served). Server side is a Hono middleware (`mppx/hono`) [5].
- The **agentic onboarding is one sentence**: the README's quick prompt is *"Read
  accounts.tempo.xyz/docs and integrate Tempo Wallet into my application."* An `AGENTS.md` carries
  repo conventions (including a docs-style rule: plain language, "feeless" over "gasless"), and the
  docs site's Vocs config enables **MCP sources** over `tempoxyz/accounts`, `tempoxyz/docs`, and the
  wevm repos (viem/wagmi/ox) — the docs are queryable by agents, not just readable [2][3].
- **Devex is one-command and example-first:** `pnpm dev` boots all services (Docker/OrbStack with
  automatic local HTTPS); every example is fetchable standalone via
  `npx gitpick tempoxyz/accounts/examples/<name>` [1][4].
- An **adapter model** decouples the account interface from the signer backend: Tempo Wallet
  dialog, domain-bound WebAuthn passkeys, Turnkey, Privy, raw secp256k1 private key, or custom auth
  infrastructure — one integration surface, swappable custody [3][6].
- For Deckard the transferable material is **devex/docs patterns and the agent-facing surface**
  (quick prompt, docs-as-MCP, examples-as-product, 402 auth intent, session-key UX), not the
  account model: the SDK's embedded/dialog wallet, custodial adapters (Turnkey/Privy), and hosted
  sponsor service sit on the other side of Deckard's self-custody NEVER lines.

## What it is

Tempo is the payments/stablecoin-focused L1 (Stripe/Paradigm lineage); `accounts` is its SDK "for
Apps and Wallets building stablecoin experiences on Tempo" [1]. It is wagmi/viem-adjacent (the
connector is wagmi-based) and ships as a single npm package. The repo layout is itself a devex
statement [1]:

| Directory | Role |
|---|---|
| `src/` | the library |
| `examples/` | 13 standalone examples: `basic`, `authentication`, `webauthn`, `access-key`, `access-key-and-webauthn`, `fee-payer`, `fee-payer-and-webauthn`, `deposits`, `transfers`, `swaps`, `subscriptions`, `mpp`, `cli` |
| `playgrounds/` | interactive web / wagmi / CLI environments |
| `ref-impls/` | reference implementations: custom embed dialog, CLI device-code auth |
| `site/` | Vocs docs site (MDX is ~17% of the repo by language) |

Each example demonstrates exactly one concept and is independently runnable
(`npx gitpick tempoxyz/accounts/examples/access-key && npm i && npm dev`) [4]. The progressive
pairs (`access-key` → `access-key-and-webauthn`, `fee-payer` → `fee-payer-and-webauthn`) compose
concepts two at a time rather than one mega-demo.

## The protocol substrate: native AA instead of 4337

Per the Tempo docs (via search excerpts — ⚠ not fetched directly), Tempo Transactions are an
EIP-2718 transaction type with native support for: WebAuthn/P256 signature validation,
parallelizable **2D nonces** (protocol nonce key 0 sequential; user nonce keys 1+ independent, with
guidance to reuse a small set of keys since new nonce keys incur state-creation cost), **gas
sponsorship**, **call batching**, **scheduled transactions**, and **access keys** [6][7]. Fee
sponsorship uses dual signature domains — sender signs the tx, the fee payer signs a "fee payer
envelope" committing to pay for that specific sender — and Tempo runs a **public testnet fee-payer
service** (`sponsor.moderato.tempo.xyz`) so developers get sponsorship in dev with zero setup [6].

Access keys are the protocol-level delegation primitive: a root key provisions scoped access keys
with **expiry timestamps and per-TIP20-token spending limits**, enforced by the chain [6][7]. The
SDK's `authorizeAccessKey` flow authorizes on connect; thereafter matching transfers sign locally
with no wallet prompt, default expiry 24 h [4].

This is the same capability set Ethereum assembles from 4337 + 7702 + paymasters + session-key
validators — delivered as chain primitives. The integration cost difference is visible in the SDK:
no bundler, no UserOp packing, no EntryPoint versioning.

## The agentic experience

Three distinct layers, worth keeping separate:

1. **Coding-agent onboarding (integration time).** The README leads with a quick prompt — *"Read
   accounts.tempo.xyz/docs and integrate Tempo Wallet into my application"* — betting that the
   first consumer of the docs is an LLM, not a human [1]. `AGENTS.md` encodes repo conventions
   (TypeScript style, type-inference rules, testing conventions, a docs-language rule banning
   crypto jargon — "feeless" over "gasless" — plus learned workspace facts) so agents converge on
   house style [2]. The Vocs config enables **MCP sources** spanning their own repos *and* their
   dependencies' repos (viem/wagmi/ox), making the whole dependency stack queryable from an agent
   session [3]. Docs pages carry "suggest changes" edit links back to the MDX source [3].
2. **Headless/CLI auth (agent runtime).** A CLI example and a device-code-auth reference
   implementation give non-browser processes a sanctioned way to obtain account authority —
   the shape an autonomous agent needs [1].
3. **Machine payments (agent economy).** MPP's HTTP 402 loop lets a client pay per-request with no
   human in the loop. The **zero-charge auth intent** is the notable design: a replay-protected
   proof of account ownership with no on-chain transaction — authentication and payment share one
   wire protocol, and the free case costs nothing [5].

## Documentation shape

The Vocs sidebar (from `site/vocs.config.ts`) [3]: **Getting Started → Deploying to Production →
FAQ**, then task-oriented **Guides** (Connect Accounts, Authentication, Identity, Transfers, Spend
Permissions, React Native, Subscriptions, Fee Sponsorship, Deposits, Swaps, Theming, CLI), an
**Adapters** section (one page per custody backend), and a **Reference** section that includes a
complete **JSON-RPC reference (26 methods)** alongside per-module API docs. Notable properties:

- **Guides are named for user tasks, not internals** ("Spend Permissions," "Fee Sponsorship") and
  the language rule is enforced in `AGENTS.md`, so the no-jargon stance survives contributions [2][3].
- **"Deploying to Production" is a top-level page** — the gap between demo and production is part
  of the documented path, not an exercise for the reader [3].
- **Reference completeness extends to the wire protocol** (the JSON-RPC list), so an integrator —
  or an agent — can work below the SDK if needed [3].

## What this means for Deckard

Different animal, first: Deckard is a self-custody desktop wallet where the agent is an *operator
inside the trust boundary* (key-less MCP sidecar → policy-gated daemon); `accounts` is an
*embedding SDK* where the wallet lives inside someone else's app and the "agent" is mostly the
integrator's coding assistant plus machine-payment clients. The custody-model pieces (dialog/iframe
wallet, Turnkey/Privy adapters, hosted fee payer) are explicitly on the wrong side of Deckard's
NEVER list (`roadmap.md`: no custodial/WaaS, no hosted services that move funds). What transfers is
the **experience engineering around the edges**. Observations:

- **The one-sentence agent quickstart is directly copyable.** Deckard's MCP profile exists and is
  spec'd (`docs/build/30-mcp-shape.md`), but there is no equivalent of *"point your agent here and
  it works."* A post-demo analog: a README quick prompt ("Add the Deckard MCP server to your agent
  and ask it to check your policy"), a one-line registration command (`claude mcp add deckard …`),
  and a single canonical agent-facing page describing the 6-tool profile, the policy semantics, and
  the typed refusals an agent will see. Tempo treats the agent as the first reader; Deckard's
  equivalent first reader is the *operator LLM*, which is even more reason to write for it.
- **Docs-as-MCP is cheap leverage.** Vocs ships it as config. Whatever docs stack Deckard adopts,
  exposing the docs (and possibly the wire-contract spec) as an MCP source means the same agent
  that operates the wallet can also *read the manual* in-session [3].
- **Examples-as-product, one concept each.** The 13-example library with `gitpick` standalone
  fetching is the strongest devex pattern in the repo. Deckard's analog isn't web apps — it's
  **scenario recipes**: variant `policy.json` files (per-tx caps, allowlists, expiry), transcript-style
  walkthroughs of agent sessions (propose → policy deny → typed reason → human approve), and
  `just demo`-style one-command scenarios per concept. `just demo` already proves the pattern;
  the gap is breadth and composability (their `access-key-and-webauthn` pairing → e.g.
  "shield + mainnet-guardrail" as a composed scenario).
- **The access-key UX is the bar for Deckard's 7702 session keys.** "Authorize once on connect;
  matching transfers sign without prompts; expiry defaults to 24 h; limits are per-token and
  chain-enforced" [4][6] is precisely the experience the roadmap's minimal-7702 NOW item must
  deliver on Ethereum, where it costs a delegation tx + validator instead of being native. Deckard
  already has the *semantics* in the daemon policy gate (caps/budgets/allowlists/expiry); the
  lesson is the *ergonomics*: scoped authority should feel like one legible grant, not a settings
  page. The root-key/access-key split also maps cleanly onto the amber/cyan actor model — the grant
  card is an amber moment, the spend under it is cyan.
- **The zero-charge 402 auth intent is a low-risk, high-value future MCP capability.** A
  replay-protected "prove control of this wallet" signature with no value transfer [5] would let
  Deckard's operator authenticate to external services without touching funds — policy-gateable as
  its own intent class, far below `deckard_execute` in risk. It is also the natural on-ramp to the
  x402 LATER item, which Tempo's MPP traction further validates (x402 was already noted
  EOA-reachable in `roadmap.md`).
- **Fee sponsorship: rent, don't run.** Tempo's public testnet sponsor shows how much friction a
  fee payer removes from demos and onboarding. Deckard's NEVER list rightly excludes *operating*
  such a service, but the roadmap already allows calling/renting one — worth remembering when the
  LATER gas-abstraction item (paymasters) comes up, and reinforced by how much of Tempo's "it just
  works" feel traces to sponsorship [6].
- **The adapter pattern argues for a signer trait in the daemon now.** One account interface over
  passkey/Turnkey/Privy/private-key/custom backends [3][6] is, translated to Deckard, a clean
  signer abstraction inside `deckard-signerd` so Ledger/Trezor (LATER) and a possible
  enclave/passkey path slot in without disturbing the policy gate or the wire contract. Cheap to
  shape now, expensive to retrofit.
- **Two docs habits worth codifying:** (1) the plain-language rule, enforced via `AGENTS.md`, which
  matches Deckard's clear-signing ethos and should extend to UI copy and typed refusal strings;
  (2) a top-level "production" page — for Deckard, the honest mainnet story (guardrail behavior,
  what's demo-grade vs. hardened), which `THREAT-MODEL.md` already seeds.
- **Release cadence as a trust signal:** 81 small releases on a pre-1.0 SDK with a frozen-feeling
  surface [1] rhymes with Deckard's freeze-first wire contract; versioned, changelogged releases of
  the MCP profile/wire contract would give agent integrators the same stability signal.

## Open questions

- Is the MPP/402 protocol (`mppx`) Tempo-specific, or specified portably enough that a non-Tempo
  wallet could implement the client side against x402-style servers? (Relevant to whether Deckard's
  402 path targets x402, MPP, or both.)
- What exactly does Vocs's "MCP sources" feature expose (search? full pages? code from the listed
  repos?), and what would the equivalent be for a Rust/mdBook-or-similar docs stack?
- How are Tempo access keys revoked/rotated at the protocol level, and what does the wallet UX for
  revocation look like? (Deckard's `deckard_revoke_all` kill-switch wants a chain-enforced
  counterpart in the 7702 path.)
- Does the CLI device-code auth flow grant a full account credential or a scoped access key — i.e.,
  is headless authority bounded by default?
- The repo's "spend permissions" guide vs. protocol "access keys": one concept or two layers?

## Sources

[1] tempoxyz/accounts repo + README (structure, examples list, quick prompt, releases, licensing) — https://github.com/tempoxyz/accounts — (github, high)
[2] AGENTS.md (conventions incl. docs-language rule) — https://github.com/tempoxyz/accounts/blob/main/AGENTS.md — (github, high)
[3] site/vocs.config.ts (sidebar structure, MCP sources, edit links, JSON-RPC reference) — https://github.com/tempoxyz/accounts/blob/main/site/vocs.config.ts — (github, high)
[4] examples/access-key README (authorize-once UX, gitpick flow, 24 h default expiry) — https://github.com/tempoxyz/accounts/tree/main/examples/access-key — (github, high)
[5] examples/mpp (HTTP 402 flow, zero-charge auth intent, charge intent, mppx/hono) — https://github.com/tempoxyz/accounts/tree/main/examples/mpp — (github, high)
[6] Tempo docs: accounts getting started, access keys, fee sponsorship, transactions — https://docs.tempo.xyz/accounts , https://docs.tempo.xyz/guide/tempo-transaction — (docs, high; ⚠ accessed via search excerpts only, site bot-blocked at research time)
[7] Tempo Transactions spec/intro (EIP-2718 type, native P256, 2D nonces, scheduled txs, access keys) — https://docs.tempo.xyz/protocol/transactions/spec-tempo-transaction , https://tempo.xyz/blog/tempo-transactions/ — (docs/blog, high; ⚠ via search excerpts). Secondary: https://medium.com/@organmo/tempo-architecture-analysis-1-tempos-account-abstraction-6babdeabc93e — (blog, medium)
