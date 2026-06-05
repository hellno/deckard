# Deckard

**Your money, on your rules, that no one can quietly switch off.**

Deckard is a native, self-custodial money agent for people who live onchain. You set
financial policies that only you can change. A local AI proposes the moves and an
on-chain policy engine carries them out, so your money manages itself and stays yours.
No bank, no exchange, no company sits in the middle and no one can freeze it, debank it,
or change the rules on you.

> Name scanned clear in crypto/web3 (a full domain/handle/trademark check is still queued).
> Early and built in public. The repo is the project; the demo comes next.

## Who it's for

People paid in crypto who live onchain and refuse custodians: DAO contributors, crypto
founders, onchain freelancers. If keeping your money sovereign has become a weekly chore
(get it off the exchange, set aside runway, put the rest to work, cover what's recurring)
and one wrong signature is a disaster you think about, Deckard is for you.

## Why it's different

This is not "a privacy company you trust." It is trustless by construction.

- **Your keys never leave your device.** Root key in the secure enclave.
- **The AI runs locally.** Your data and your finances are not sent to anyone's server.
- **The AI proposes, cryptography enforces.** The agent can never move funds outside the
  rules you set. Enforcement is on-chain policy and scoped session keys, not a promise.
- **Open source.** You cannot trust what you cannot read. Everything is auditable.
- **Sovereign assets by default.** Deckard defaults to assets that cannot be unilaterally
  frozen or blacklisted. Centralized stablecoins are an explicit, labeled opt-in, never a
  silent default.

The local model is the cheap part and it is getting cheaper. The value is the trust
model, the native surface, and the craft that make self-custody safe and effortless.

## How it works

```
  your accounts ──▶ Deckard (local) ──▶ proposes a move ──▶ you approve (1 tap)
        ▲                │                                        │
        │                ▼                                        ▼
   secure enclave   local AI reads             on-chain policy engine + scoped
   holds root key   YOUR state, on-device      session keys execute within bounds
```

Today the human approves every move (advisory-first). The destination is hands-off
autonomy, shipped only after a third-party security audit and a security co-maintainer
are in place. The autonomy is the point; we earn it.

## Status and roadmap

- [ ] **Milestone 1 (advisory):** the runway loop. Deckard watches your accounts and, when
      income lands or your runway drifts, proposes the moves (off the CEX, top runway in a
      sovereign stable, sweep the excess to yield). You approve with one tap. Testnet first.
- [ ] Get-off-CEX guardrail, auto-pay, rebalance.
- [ ] Security audit (funded as an explicit grant line item).
- [ ] Bounded hands-off autonomy, post-audit. The original promise.

## What I'm looking for

- **Onchain operators** who will try the early builds and tell me what actually hurts.
- **A security-minded co-maintainer** for the policy engine. This is money software; it
  should not be reviewed by one person. If reviewing self-custody policy enforcement is
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
