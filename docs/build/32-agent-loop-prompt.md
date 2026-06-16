# Agent Loop — watch the wallet, shield new deposits (the first agent demo)

> **You are probably an agent.** This page turns the one-shot quickstart
> ([`31-agent-quickstart.md`](31-agent-quickstart.md)) into a **loop**: notice a deposit, shield
> it within the policy, and stop cleanly when told. It is the runner for the first agent demo
> (#61) — *"Claude watches the wallet, shields incoming funds within the cap, and a human can
> see-and-stop it in the activity feed (#60)."*
>
> No new authority. You use the same six `mcp.v0.1` tools and the same key-less path. The daemon
> still holds the key, the policy gate still decides, and a human can still approve or STOP. The
> only new thing is *you, in a loop*.

## What the loop is

A deposit lands in the wallet's public balance. You notice the increase, propose a shield for it,
and — if it is within the policy caps — execute it. The human watches every step in the Deckard
app's **Activity feed** and can hit **STOP** at any moment. That is the whole demo.

```
poll balance ─▶ new ETH arrived? ─▶ shield(delta) ─▶ decision?
   every          (delta ≥             propose            ├─ allow         ─▶ execute ─▶ broadcast ✓
   ~10s           auto_shield_min)                        └─ needs_approval ─▶ wait for the human
     ▲                                                                          in the Activity feed
     └──────────────────────────────────────────────────────────────────────────────────────────┘
```

You run this with the six tools from the quickstart — read them first if you have not:
`deckard_policy_get`, `deckard_wallet_balance`, `deckard_shield`, `deckard_execute`,
`deckard_wallet_address`, `deckard_revoke_all`.

## The prompt (paste this into a fresh agent session)

> You are operating a self-custodial Ethereum wallet through the Deckard MCP sidecar. First read
> https://github.com/hellno/deckard/blob/main/docs/build/32-agent-loop-prompt.md and
> https://github.com/hellno/deckard/blob/main/docs/build/31-agent-quickstart.md.
>
> Your job: **watch the wallet and shield new deposits, staying inside the policy.**
>
> 1. Call `deckard_policy_get` once and note `per_tx_cap_wei`, `daily_cap_wei`, and
>    `auto_shield_min_wei`. Call `deckard_wallet_balance` once and remember `public_wei` as the
>    **baseline** — do not shield the funds that were already there.
> 2. Then loop: every ~10 seconds call `deckard_wallet_balance`. First, if `public_wei ≤
>    last_seen_wei` (a prior shield settled and lowered the public balance), set `last_seen_wei =
>    public_wei` — this ratchets the baseline DOWN so a later deposit is still detected. Then
>    compute `delta = public_wei − last_seen_wei`. If `delta` is positive and `delta ≥
>    auto_shield_min_wei`, a new deposit arrived worth shielding.
> 3. For a new deposit: convert `delta` to a decimal ETH string and call `deckard_shield` with it
>    (leave a little headroom for gas — shield slightly less than the full delta if the deposit is
>    close to the whole balance). Set `last_seen_wei = public_wei` **immediately**, before you
>    execute, so the next poll does not re-shield the same funds.
>    - `decision: "allow"` → call `deckard_execute` with the `request_id`. Report the `tx_hash`.
>    - `decision: "needs_approval"` → the amount is over a cap (or you're on mainnet). **Do not
>      re-propose and do not lower the amount on your own.** Remember this `request_id`. Tell the
>      human it is waiting in the Deckard app's Activity feed — the **"Needs you"** band (⌘K →
>      Activity, or ⌘⇧A) — for them to approve. Keep polling, and on each poll retry
>      `deckard_execute` with that **saved request_id**: while it is still pending you'll get
>      `not_approved` (keep
>      waiting); once the human approves, `deckard_execute` broadcasts — report the `tx_hash` and
>      drop the saved id.
> 4. If `deckard_shield` returns `locked`, or `deckard_execute` returns `revoked`, STOP was
>    pressed (a propose after STOP reads `locked`; an execute of an already-approved request reads
>    `revoked`). Either way the key is zeroized: report that the loop was stopped and that a human
>    must unlock the wallet in the app to re-arm, then exit the loop — do not retry.
> 5. If anything looks wrong, call `deckard_revoke_all` yourself. It is always safe.
>
> Narrate each step briefly so the human can follow along while watching the Activity feed.

## How each rule works (and why)

### Detecting a deposit — the balance delta

There is no daemon-side receive-watcher in this demo (a `get_logs` watcher is the later
hands-free version). You poll. Hold a single number in your context — `last_seen_wei` — and on
each poll compare it to the fresh `public_wei`. A positive `delta` is new inbound ETH.

`auto_shield_min_wei` is **advisory**: the policy gate does not switch on it (`deckard_policy_get`
returns it for you to read). Treat it as your own floor so dust deposits and gas-refund noise do
not trigger a shield. In the demo world it is `0.01 ETH`.

### Idempotency — never shield the same funds twice

This is the one rule that makes a *loop* safe. The trap: you shield a deposit, but on the next
poll the public balance has not dropped yet (the shield is still confirming), so a naive loop
sees the "same" balance as still-new and shields it again.

Two layers stop that:

1. **You track `last_seen_wei` in your own context** and advance it the moment you propose — so
   the next poll's `delta` is computed against the balance you already acted on, not the baseline.
   In the prompt form this is the primary mechanism: the conversation *is* the state. Crucially,
   `last_seen_wei` is a high-water mark you must also **ratchet DOWN**: once a shield settles the
   public balance drops, so if you never lower `last_seen_wei` a smaller later deposit can leave
   the balance still below the old mark and be missed forever. The poll's first step (`if
   public_wei ≤ last_seen_wei: last_seen_wei = public_wei`) handles this.
2. **The daemon is the backstop.** `deckard_shield` builds a deterministic `request_id` from the
   intent, so an identical re-propose returns the same id; once it has broadcast, a re-execute is
   refused with `already_executed` — **do not retry that**. (An in-app loop, if one is built
   later, would track the in-flight `request_id` explicitly instead of relying on conversation
   context — see #61.)

### Backpressure — wait, do not spam

When `deckard_shield` returns `needs_approval`, the deposit was over a cap, or the daemon is on
mainnet (where the guardrail downgrades every auto-allow to a human approval). The right move is
to **wait**, not retry and not quietly shrink the amount:

- The proposal is now a pending card. The human sees it in the Deckard app's **Activity feed**
  (#60) — in the **"Needs you"** band, with the *actual* breached cap cited (per-tx vs daily) —
  and approves or denies it there. You never approve your own request — there is no `resolve`
  tool, by design (`resolve_not_authorized`).
- **Save the `request_id` and keep trying to finish it.** Do **not** re-propose the same deposit.
  Instead, on each poll, call `deckard_execute(request_id)` with the saved id: while the human
  hasn't acted it returns `not_approved` (keep waiting); once they approve in the feed, the very
  same `deckard_execute` broadcasts and you report the `tx_hash`. (Without this retry the approved
  request would just sit there — approval alone doesn't broadcast; execute does.) If they deny it,
  `deckard_execute` returns `user_denied` — respect it and drop the id.

This is the seam the demo is about: an over-cap deposit is exactly when the human is pulled into
the loop, and the feed is where they do it.

### STOP — the kill switch, mid-loop

STOP is always reachable — the human presses it in the feed header (or runs the ⌘K *STOP — lock &
revoke all* command), or you call `deckard_revoke_all` yourself if something looks wrong. After
STOP:

- The signing key is zeroized and every in-flight request is denied. Your next call returns one of
  two STOP tags depending on which call you make: a **`deckard_shield`/propose** after STOP reads
  **`locked`** (the daemon holds no key — the lock gate runs before the policy gate), while a
  **`deckard_execute`** of an already-approved request reads **`revoked`** (the execute-time STOP
  re-check). Treat **both** as the same terminal signal: STOP was pressed.
- Report the kill cleanly and **exit the loop**. Do not retry — STOP is irreversible for the
  session; only a human unlocking the wallet in the app re-arms signing.

## Capture the run (for the learning writeup)

Time the path the way the quickstart's happy path runs — read-policy → propose → execute — and
record it TTHW-style ("time to here's-what-happened"). Append the measured timing to `STATUS.md`'s
TTHW line, or drop a short run log under `docs/build/` and name it in the PR. Capture at least:

- baseline-read → first deposit noticed (your poll cadence bounds this),
- `deckard_shield` propose → `decision`,
- `deckard_execute` → `broadcast` + `tx_hash`,
- and, for an over-cap deposit: propose → the human approving in the feed → execute.

## Try it end-to-end (demo world)

Preconditions are the quickstart's: `just demo` running, a throwaway wallet **created and
unlocked** in the app, `just demo-fund` funded. Then:

1. Start the loop with the prompt above. It reads the policy, takes a baseline, and idles.
2. Send the wallet a fresh deposit — e.g. `just demo-fund` again, or any transfer to the address
   `deckard_wallet_address` reports. In the demo world the per-tx cap is **0.1 ETH**.
3. **Within-cap deposit** (≤ 0.1 ETH): the loop proposes, auto-allows, executes, and the Activity
   feed shows `Atlas · shield … · auto-approved within cap` with the real tx hash and a timestamp.
4. **Over-cap deposit** (> 0.1 ETH): the loop proposes and waits; the feed shows it pending with
   the per-tx-cap cite. Approve it from the feed (select the row, ⌘Enter, ⌘Enter) and the loop
   executes on the next poll.
5. **STOP**: press STOP in the feed header mid-run; the loop's next `deckard_shield` returns
   `locked` (or a pending `deckard_execute` returns `revoked`) and it reports the kill. Unlock the
   wallet in the app to run again.

## Hard rules (restated)

- The thinking agent stays **key-less**. You propose; the daemon signs. You never hold a key.
- v1 limits are **software-enforced** by the policy gate + human approval. On a fork/testnet a
  within-cap shield auto-allows hands-free so the demo can run; on mainnet the guardrail forces a
  human approval for *every* write. The limits are real but enforced by software, not by the
  chain — never tell the user a cap "cannot be exceeded."
- Never ask for a seed phrase, private key, or passphrase — no tool accepts one.
- When in doubt, `deckard_revoke_all` is always safe.
