# Deckard v1 — Demo-Driven Build Plan

> One goal: **ship a reliable, exciting 90-second video that pitches the EF CROPS direction + community.**
> Everything here serves it. Supersedes the Now/Later framing in `roadmap.md` for build purposes.
> Settled via 3 rounds of requirements Q&A, 2026-06-05.

## The spirit

A native, **open-source, self-custodial** wallet where an AI handles your money **privately** and you
**can't be switched off**. CROPS in one product: **P**rivacy (shielded), **S**elf-sovereign / **S**ecurity
(local keys, bounded agent), **C**ensorship- & capture-**R**esistance + the **walkaway test** (Helios — no
Infura), **O**pen-source (0BSD). MPP/x402 are deliberately *not* here — they come later as **plugins**.

## The video (the only spec that matters)

**Scene: "receive → instantly private → can't be switched off."** ~90s, one continuous mainnet recording:

1. **Real, self-custodial wallet.** Deckard opens — native, fast, real mainnet balance; on-screen: *keys never leave this device, open-source.* *(0:10)*
2. **🎯 Live receive → instant auto-shield (HERO).** A payment lands in the wallet *live*. The agent (Claude Desktop, via Deckard's MCP sidecar) immediately shields it: public balance drops, **private balance rises, the trail is broken** — all on mainnet. *(0:30)*
3. **🎯 Walkaway test (HERO).** Cut / block the centralized RPC on camera. Deckard keeps showing **verified** balances because **Helios** checks the chain itself. "Works even if Infura — or the EF — disappears." *(0:30)*
4. **Trust close.** Quick pan: it's open-source, the key lives in an isolated signer process the AI can't reach, there's a STOP button. *(0:20)*

Beats 2 and 3 are the must-haves (locked). STOP-on-camera and an allocate/donate slice are **fast-follow**.

## The two risky hero beats — spike FIRST, in parallel, before committing the shot

Both heroes rest on immature pieces. De-risk them on day one; only attempt the mainnet hero once both are green.

- **R1 · Shield on mainnet via Kohaku's *alpha* Railgun crate.** Open question from the KB: is the crate consumable standalone from Rust with a stable API? **Spike:** shield+unshield a test amount on a fork/Sepolia from Rust. *Fallback if it's flaky:* shield on **Sepolia** for the video (keep Helios-walkaway on mainnet), or swap the shielded-pool path (Privacy Pools). 
- **R2 · Helios "cut the RPC and keep working."** The walkaway beat must be *real*, not cached. **Spike:** run Helios on mainnet, verify reads, then kill the primary RPC and confirm it continues from a second source / light-client peer. *Fallback:* if continuation is hard, the beat becomes "Helios *verified locally* (no trusted server)" with a visible proof, minus the live cut.

## Deliverables, ranked by demo impact

| # | Deliverable | Done when (concrete, testable) | Proven by (agent/automated) | Beat | Track | Size |
|---|---|---|---|---|---|---|
| 1 | **Shield-on-receive (Railgun via Kohaku Rust crate)** | a received deposit is shielded into an owner-only private balance; public trail broken | fork/Sepolia: deposit→shield→assert private balance up, public down, link broken; mainnet rehearsal | 2 | T-Privacy | L ⚠R1 |
| 2 | **Embedded Helios + walkaway** | balances/state verified via Helios vs an untrusted RPC; cutting the primary RPC keeps verified reads working | integration: verify reads; kill RPC→assert continued verified reads (or graceful proof) | 3 | T-Trustless | L ⚠R2 |
| 3 | **Receive watcher** | wallet detects an inbound tx within seconds (via Helios-verified logs) and fires the agent | send→assert event < N s, sourced from verified logs | 2 | T-Core | S–M |
| 4 | **Process-isolated signer daemon + STOP/revoke** | key in a separate process; `propose`/`execute` only, no raw-bytes; STOP revokes agent authority | red-team script: agent process can't read key / raw-sign; STOP→next execute denied | 4 | T-Custody | M–L |
| 5 | **Encrypted keystore + unlock** (Argon2id+XChaCha20, atomic write) | no plaintext key on disk; passphrase unlock; survives crash mid-write | round-trip; kill-during-write→key intact; `grep` disk→no plaintext | 1 | T-Custody | M |
| 6 | **MCP sidecar** (key-less client of the daemon) for Claude Desktop/Cursor | external client registers + calls `balance`/`simulate`/`shield`/`execute`; secrets never in transcript | MCP test-client + Claude Desktop dry-run; assert policy enforced + no key leak | 2 | T-Agent | M |
| 7 | **Private RPC by default** | app talks to a privacy-respecting/proxied RPC (no IP+address leak to a default vendor); Helios on top | assert no address-bearing calls to a default centralized vendor | 1,3 | T-Trustless | S–M |
| 8 | **Mainnet balances + shield-deposit tx** (alloy) | shows ETH/ERC-20; constructs + sends the shield deposit on mainnet | fork/mainnet rehearsal: send, confirm receipt | 2 | T-Core | S–M |
| 9 | **Native "what just happened" surface** (GPUI) | shows live receive → shielding → private (before/after balances, trail broken) + a "verified by Helios — no Infura" indicator | UI test: states render; indicator reflects Helios status | 2,3 | T-UX | M |

## Parallel tracks (freeze one contract, then go wide)

**Freeze first (½ day):** the MCP tool surface + signer-daemon `Intent`/`Decision` API + the `shield(amount)` intent shape. Everyone codes against it.

- **T-Privacy** (#1) — *start immediately, it's R1.* Independent: needs only an EOA + fork/mainnet.
- **T-Trustless** (#2, #7) — *start immediately, it's R2.* Independent.
- **T-Custody** (#5 → #4) — keystore then the daemon (the integration point).
- **T-Agent** (#6) — mocks the daemon via the frozen contract; integrates when #4 lands.
- **T-Core** (#3, #8) — receive watcher + send; starts on plain RPC, swaps to Helios-verified.
- **T-UX** (#9) — builds against mocked states; this is what the camera sees.

T-Privacy and T-Trustless are both the **riskiest** and the **two hero beats** → they run first and in parallel; the rest can't make the video matter if those two don't land.

## Acceptance test = the shot list (one agent-runnable scenario)

Both the **CI gate** and the **storyboard**. If it passes on mainnet (or Sepolia per the fallback), shoot it.

```
Scenario "Shield-on-Receive, Trustless" (mainnet; Sepolia fallback for the shield):
  setup: encrypted wallet unlocked; Helios synced over private RPC; MCP sidecar registered
         in Claude Desktop; agent policy = "auto-shield inbound ETH above X".
  1. send a deposit to the wallet (live)                       assert: receive watcher fires < N s, from Helios-verified logs
  2. agent (Claude via MCP) calls shield(amount)               assert: private balance ↑, public ↓, link broken; tx confirms
  3. cut the primary RPC                                        assert: Deckard still shows VERIFIED balances via Helios (no crash)
  --- fast-follow asserts ---
  4. STOP / revoke                                             assert: agent's next execute is denied
  5. allocate/donate a slice                                   assert: rule honored
```

Steps 1–3 are the video. A coding agent runs this headless; the same run + GUI + screen recorder = the cut.

## Reliability plan (it cannot faceplant in front of EF)

Spike R1+R2 on Sepolia/fork → go mainnet only when both green → pre-fund the wallet, pre-sync Helios, do
multiple takes. Shield falls back to Sepolia if the alpha crate misbehaves on mainnet; the Helios walkaway
stays on mainnet regardless. **Backup driver:** if Claude Desktop (external MCP) flakes on stage, an in-app
agent loop can drive the same MCP tools — build the sidecar so either can call it.

## Fast-follow (right after the video — not in v1)

STOP-on-camera beat · allocate/donate slice · **7702 session keys** (with the plugin wave) ·
**x402 / MPP as wallet plugins** (+ the plugin architecture that hosts them) · stealth addresses ·
hardware-wallet signing · paid audit/bug-bounty. (`roadmap.md` holds the full Later/Never frame.)
