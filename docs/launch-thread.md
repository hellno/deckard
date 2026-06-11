# Deckard — build-in-public launch thread (Farcaster)

Draft in your voice, first person. Post as a thread. Tune the tone to sound like you,
swap in your repo link. Each cast is kept short enough for a standard cast; merge into
longcasts if you prefer.

---

**1/**
i'm starting something and building it in public from day zero.

Deckard: a native, self-custodial money agent that runs your onchain finances on rules
only you can change. no bank, no exchange, no company that can freeze it. 🧵

**2/**
the itch: being your own bank is a part-time job.

get income off the CEX. park runway in something that can't be switched off. put the rest
to work. don't fat-finger an approval and get drained. every week.

so most people give up and hand their money back to a custodian.

**3/**
that trade — sovereign vs usable — is the thing i want to kill.

you shouldn't have to choose between owning your money and being able to live with it.

**4/**
how it works:

an AI agent proposes a move. it can't sign. the private key lives in a separate, process-
isolated daemon that checks your policy (caps, allowlist, approval) and signs on its own.

the agent surface is key-less. no seed, no spending path. it proposes; the daemon decides.

**5/**
the key one: on mainnet, the human approves EVERY move — structurally.

the daemon turns every auto-allow into needs-approval on chain 1 by default. no hands-free
agent spend while that autonomy is still being earned. a control, not a printed warning.

**6/**
why this isn't just "AI + a wallet":

the model is the cheap part and getting cheaper. the point is the boundary the agent can't
cross — on your hardware, in the open.

reads are verified against a Helios light client (no blind trust in an RPC). privacy is on
by default via Railgun. native GPUI app, not a browser extension. AGPL.

**7/**
honest about the field, as of mid-2026:

Coinbase Agentic Wallets keep keys in Coinbase's TEE — a platform you link to. MetaMask's
agent wallet lives in the browser extension, policy + signer in one process.

Deckard's keys never leave a daemon on YOUR machine, and the policy lives in a separate
process the UI and agent can't reach into.

**8/**
on Kohaku / privacy: i build ON the Railgun + Kohaku shielded-pool stack and say so plainly.

what's mine is the layer above: the signer + policy boundary, verified reads, the key-less
agent (MCP) surface, the native desktop app.

and shielding ≠ mixing — Railgun is a non-custodial pool with Private Proofs of Innocence:
flagged funds are provably excluded. privacy from surveillance, not from your own records.

**9/**
honest about stage: this is alpha, testnet/throwaway keys only, no audit yet.

but it's not vaporware — a 6-tool agent (MCP) surface and a one-command `just demo` run
today. starting advisory-first; hands-off autonomy ships only after an audit.

**10/**
who it's for:

people paid in crypto who live onchain and won't touch a custodian. if keeping your money
sovereign is a weekly chore and one wrong signature is a fear you carry, this is for you.

**11/**
what i'm looking for:

→ onchain operators who'll try early builds and tell me what actually hurts
→ a security-minded co-maintainer for the signer + policy daemon (money software shouldn't
  be one person's review)
→ brutal critique on the trust + threat model

**12/**
it's a public good and funded like one: open-source + sovereignty grants, with a security
audit as an explicit line item. no token, no custody of your money, no watching you.

repo + manifesto: [link]

follow along and come argue with me.
