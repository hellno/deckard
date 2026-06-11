# Deckard

**Your money, on your rules, that no one can quietly switch off.**

Deckard is a native, self-custodial money agent for people who live onchain. You set
financial policies, an AI agent proposes the moves, and a **process-isolated signer your
agent cannot bypass** carries them out within those rules — so your money manages itself
and stays yours. No bank, no exchange, no company sits in the middle, and the agent surface
can never sign on its own.

> Name scanned clear in crypto/web3 (a full domain/handle/trademark check is still queued).
> Early and built in public, `0.0.1-alpha` — testnet/throwaway keys only, no audit yet. The
> repo is the project, and a 6-tool agent (MCP) surface plus a one-command `just demo`
> already run today.

## Who it's for

People paid in crypto who live onchain and refuse custodians: DAO contributors, crypto
founders, onchain freelancers. If keeping your money sovereign has become a weekly chore
(get it off the exchange, set aside runway, put the rest to work, cover what's recurring)
and one wrong signature is a disaster you think about, Deckard is for you.

## Why it's different

This is not "a privacy company you trust." The trust boundary is structural, and it's the
whole point.

- **A signer your agent cannot bypass.** The private key lives in a **separate, process-
  isolated daemon** (`deckard-signerd`), not in the app and not in the agent. The agent can
  only *propose* an intent over a local Unix socket; the daemon runs every state-changing
  request through a **local policy gate** (allowlist, per-tx and daily caps, approval mode)
  and signs in its own address space. On STOP it **zeroizes** the in-memory key. The UI and
  the agent literally have no signing path.
- **The human approves every mainnet move — structurally.** On chain 1, the daemon converts
  every auto-allow into *needs-approval* by default; there is no hands-free agent spend on
  mainnet while the autonomy surface is still being earned. It constrains the agent, not your
  own machine — honest limits in [THREAT-MODEL.md](../THREAT-MODEL.md).
- **A key-less agent surface.** The agent is Claude (Desktop) wired through a **key-less MCP
  sidecar** — the sidecar holds no seed, no spending key, no passphrase, and has no code path
  that could sign. Error reasons that cross back into the transcript are **URL-redacted at the
  daemon boundary**, so an RPC API key in an error string never reaches the model.
- **Verified reads, not RPC trust.** Balances and on-chain state are validated against an
  embedded **Helios light client**, and the surface shows whether a read is light-client-
  verified. A lying or compromised RPC can't quietly feed you a false balance.
- **Privacy by default.** Shielding is built on **Railgun** (the same shielded-pool stack the
  Ethereum Foundation's Kohaku initiative builds on), so balances and flows aren't an open
  book to every observer.
- **Native and open.** A native **GPUI** desktop surface — not a browser extension — and the
  whole thing is **AGPL-3.0** open source. You can't trust what you can't read.

The model is the cheap part and it's getting cheaper. The value is the trust boundary, the
native surface, and the craft that make self-custody safe and effortless.

## How it works

```
  your accounts ──▶ Deckard app (native) ──▶ agent (MCP) proposes a move ──▶ you approve
        ▲                  │                          │                          │
        │                  ▼                          ▼                          ▼
   Helios light       app reads YOUR          key-less MCP sidecar       process-isolated
   client verifies    on-chain state          (holds no key)             signer daemon:
   the reads          (verified)              proposes an Intent ────────▶ policy gate +
                                                                           sign, or deny
```

The signer daemon holds the key and is the only thing that can sign — behind the policy gate,
in its own process. On mainnet, "you approve" is enforced by the guardrail, not left to good
intentions.

## How it compares

Agent wallets shipped fast in 2026; the honest comparison isn't against toy raw-key MCP
servers, it's against the real field. (Facts below as of mid-2026.)

- **Coinbase Agentic Wallets (CDP).** Coinbase gives an agent an **MPC wallet whose keys
  live in Coinbase's own TEE infrastructure** — non-custodial in the TEE sense, but the keys
  sit in a platform you link to and depend on, with KYT screening Coinbase runs on your
  behalf. Deckard's keys never leave a daemon on *your* machine; there is no platform account
  to link, freeze, or screen through.
- **MetaMask Agent Wallet / Smart Accounts (ERC-7715 delegation).** MetaMask grants agents
  scoped, self-custodial permissions, but the trust surface is the **browser extension** and
  the permission UI: the policy and the signer live inside the same wallet process the agent
  talks to. Deckard's policy lives in a **separate daemon process the UI and agent cannot
  reach into** — native process isolation, not an in-extension permission prompt.
- **Kohaku (Ethereum Foundation privacy SDK).** We build **on the Railgun/Kohaku shielded-
  pool stack** — we say that plainly rather than pretend to reinvent it. What's *ours* is the
  part above the privacy primitive: the **process-isolated signer + local policy boundary**,
  **Helios-verified reads**, the **key-less MCP agent surface**, and a **native desktop app**.
  Kohaku is an SDK for wallets to embed; Deckard is the opinionated, agent-first wallet that
  embeds it.

Across all three: the differentiator is the *boundary the agent cannot cross*, on hardware you
control, in the open.

## On compliance and "mixing"

Shielding is not mixing, and it matters that the difference is structural. Railgun is a
**non-custodial smart-contract privacy pool** — it never takes custody of your funds — paired
with **Private Proofs of Innocence (PPOI)**. On each shield, a zero-knowledge proof is
generated against published lists of known-illicit actors and transactions; spends carry a
proof of spendability tied to the PPOI inclusion set, so flagged UTXOs are provably *excluded*
from the pool — without revealing your balance, amounts, or viewing keys. The result is privacy
from **surveillance** (chain analysts and onlookers reading your every move), not privacy from
**your own records**: you keep what you need to answer a lawful subpoena of your own activity.
Deckard's sovereign-asset default and labeled stablecoin opt-in sit on top of that posture.

## Status and roadmap

Working today: the encrypted keystore + onboarding, live on-chain balances, receive
(address + QR), the command palette, Helios-verified reads, the process-isolated signer
daemon, the **6-tool MCP agent surface** (`deckard-mcp`), and a one-command **`just demo`**
(local Sepolia fork, isolated config, shield the hero amount end-to-end). The **Shield** hero
is wired and black-box tested on an Anvil fork.

- [ ] **Milestone 1 (advisory):** the runway loop. Deckard watches your accounts and, when
      income lands or your runway drifts, the agent proposes the moves (off the CEX, top
      runway in a sovereign stable, sweep the excess to yield). You approve with one tap.
      Testnet first.
- [ ] Get-off-CEX guardrail, auto-pay, rebalance. Send is gated ("next release"); Swap is a TODO.
- [ ] **Secure-enclave / Touch ID unlock** (Phase 2). v0 ships passphrase-only (Argon2id);
      biometric unlock is blocked on the macOS codesign/notarize pipeline.
- [ ] **Local-first / bring-your-own-model.** Today the agent surface is a cloud model (Claude)
      reached through the key-less sidecar; a local/BYO model is the destination.
- [ ] **On-chain policy enforcement** (scoped session keys, on-chain caps). Today enforcement
      is the local daemon + guardrail; pushing it on-chain is the next layer of the boundary.
- [ ] Security audit (funded as an explicit grant line item).
- [ ] Bounded hands-off autonomy, post-audit. The original promise.

Today the human approves every move (advisory-first), and on mainnet the guardrail makes that
structural, not optional. The destination is hands-off autonomy, shipped only after a third-
party security audit and a security co-maintainer are in place. The autonomy is the point; we
earn it.

## What I'm looking for

- **Onchain operators** who will try the early builds and tell me what actually hurts.
- **A security-minded co-maintainer** for the signer + policy daemon. This is money software;
  it should not be reviewed by one person. If reviewing self-custody policy enforcement is
  your thing, reach out.
- **Honest critique.** Especially on the trust model and the threat model.

## Funding

Deckard is a public good and is funded like one: open-source grants (NLnet/NGI Zero,
Gitcoin, EU sovereign-tech) and retroactive public-goods funding, with the security audit
as an explicit line item. No token, no custody of your money, no business model that
depends on watching you.

## License

**AGPL-3.0 (copyleft).** Nobody gets to take a sovereignty tool proprietary and close it
off. Copyleft forces reciprocity: build on Deckard, share your changes back. This matches
where open source in crypto is heading (Vitalik publicly shifted from permissive to copyleft
in 2025 for exactly this anti-capture reason) and it fits a commons-funded public good.

---

Built in public. Follow along and come argue with me.
