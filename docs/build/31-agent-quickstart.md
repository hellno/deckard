# Agent Quickstart — drive Deckard through the MCP sidecar

> **You are probably an agent.** This page is written for the LLM operating Deckard's agent
> surface (and for the human wiring one up). It is the canonical zero-to-working path: register
> the sidecar, read the policy, propose a shield. Everything here describes the shipped
> `mcp.v0.1` profile in `crates/deckard-mcp`; the frozen wire contract behind it lives in
> [`30-mcp-shape.md`](30-mcp-shape.md). The tool list on this page is **drift-guarded by a
> test** (`crates/deckard-mcp/src/server.rs`, `quickstart_doc_lists_exactly_the_registered_tools`):
> if a tool is added or renamed without updating this page, the build fails.

## What you are talking to (30 seconds)

Deckard is a self-custodial Ethereum wallet — the user holds their own keys; no company can
move or freeze the funds. You talk to `deckard-mcp`, a **key-less** sidecar: it holds no keys
and cannot sign anything. Every write you propose is checked by a separate signer-daemon
process against a **policy** (spending limits a human owns and you can only read). Stay inside
the policy and writes auto-allow; step outside it and you get a refusal or a request that a
human must approve in the Deckard app.

Terms used below, explained once:

- **shield** — move funds from the wallet's normal, publicly visible balance into a private
  balance (via Railgun), so onlookers can no longer trace them. This is the hero action.
- **wei** — the smallest unit of ETH: 1 ETH = 10^18 wei. Policy caps are wei strings; tool
  *inputs* are decimal ETH strings like `"0.02"`.
- **request_id** — a 32-byte `0x`-hex ticket a proposal returns. It is the only thing that can
  be executed; you never send raw transactions.

## Register the sidecar (the one-liner)

```sh
deckard-mcp install --demo   # prints the Claude Desktop registration; add --write to merge it in
```

`--demo` points the sidecar at the isolated demo world (constants below) and is the right
choice for a first run. The printed JSON embeds the absolute path of the binary plus a
key-less env block — no secret ever enters the client config. Restart Claude Desktop after
writing it.

Preconditions before any tool works: `just demo` is running (forked chain + the Deckard app,
which spawns the signer daemon), a throwaway wallet is **created and unlocked** in that app,
and `just demo-fund` has funded it. `just demo-check` diagnoses a broken setup and prints the
exact fix for each failure.

**Quick prompt** (what a human pastes into a fresh agent session):

> Read https://github.com/hellno/deckard/blob/main/docs/build/31-agent-quickstart.md — then
> check my Deckard wallet policy and shield 0.02 ETH, staying inside the policy caps.

### Other MCP clients (Cursor, Codex, anything stdio)

The registration is the generic MCP stdio shape — map the printed JSON into your client's own
config format. There is nothing Claude-specific about it:

- **command:** the absolute path to the `deckard-mcp` binary
- **args:** `["--mcp"]`
- **env (demo mode only):** `DECKARD_SOCKET_PATH`, `DECKARD_CONFIG_DIR`, `DECKARD_CHAIN_ID`,
  `DECKARD_RPC_URL` — exactly as printed by `install --demo`; no secrets.

## The 6 tools

This is the complete `mcp.v0.1` surface. There is deliberately no raw "propose" and no
"approve" tool — you cannot submit an arbitrary transaction or approve your own request.

| Tool | What it does | Side effects |
|---|---|---|
| `deckard_wallet_address` | Read the wallet's public `0x` address. | none |
| `deckard_wallet_balance` | Read the **public** balance (`public_wei`, `public_eth`) plus a `read_status` trust label. | none |
| `deckard_policy_get` | Read the policy fence (fields below). **Call this first.** | none |
| `deckard_shield` | **Propose** shielding `amount_eth` (a decimal ETH **string**, e.g. `"0.02"`) to the wallet's own private address. Signs nothing. | creates a pending request |
| `deckard_execute` | Sign + broadcast a previously allowed `request_id`. Policy is re-checked at sign time. | broadcasts a transaction |
| `deckard_revoke_all` | **STOP — the panic brake.** Zeroizes the signing key, locks the daemon, denies every in-flight request. | irreversible for the session |

Semantics that matter:

- **Balance is public-only in v0.1.** The `shielded` field is the honest string
  `"unavailable — read it in the Deckard app (v1 limitation)"` — never report the private
  balance as 0.
- **Shield returns a decision, not a transaction.** `"decision": "allow"` comes with a
  `request_id` → call `deckard_execute` with it. `"decision": "needs_approval"` means a human
  must approve in the Deckard app; this alpha has no approval card yet, so the practical fix is
  a smaller amount under the per-tx cap (or the human edits `policy.json`).
- **Execute is the one call you must never retry blind.** If it times out or the connection
  drops, the broadcast status is UNKNOWN — a retry could double-spend. Check the Deckard app
  first. An identical re-shield in the same session is refused as `already_executed`; vary the
  amount to run the flow again.
- **STOP is always available** and needs no approval. Use `deckard_revoke_all` immediately if
  anything looks wrong; only a human unlocking the wallet re-arms signing.

The happy path, in order:

```
deckard_policy_get  →  deckard_wallet_balance  →  deckard_shield("0.02")  →  deckard_execute(request_id)
   know the fence        know the funds            decision: allow            status: broadcast + tx_hash
```

## The policy fields you will see

`deckard_policy_get` returns the fence as JSON — wei values as decimal strings, each cap also
rendered as ETH for convenience. You can read it, never write it (a human edits `policy.json`
in the Deckard config dir).

| Field | Meaning | What it means for you |
|---|---|---|
| `per_tx_cap_wei` | Max wei a single write may move. | Propose amounts at or under this and they can auto-allow. |
| `daily_cap_wei` | Max total wei per UTC day (see `spent_today_wei` for the running count). | Even within-cap writes are refused once the day's total would pass this; it rolls over at UTC midnight. |
| `allow_to` | Recipient allowlist; **empty = any recipient**. | Shield targets the wallet's own private address, so this rarely blocks you. |
| `require_approval` | `never` / `over_cap` / `always`. | `over_cap` (the demo default): within-cap auto-allows, over-cap needs a human. `always`: every write needs a human. |
| `revoked` | `true` once STOP is engaged. | Nothing will sign; a human must re-unlock in the app. |
| `auto_shield_min_wei` | Advisory threshold: inbound amounts at or above it are worth proposing a shield for. | Guidance for you; the policy gate does not enforce it. |

## When you are refused — every deny tag, with the fix

Every failure is structured JSON: `{"error": {"problem", "cause", "fix"}}` — deterministic and
secret-free. The `fix` line is authoritative; this table is the summary. Default instinct on an
error is to retry — for two of these (marked **do NOT retry**) that instinct is wrong.

| Tag | Meaning | What to do |
|---|---|---|
| `locked` | The daemon holds no key (it starts locked; lock/STOP zeroize it). If no wallet exists yet, the error says so — that's onboarding, not unlocking. | A human unlocks (or creates) the wallet in the Deckard app, then retry. |
| `revoked` | STOP is engaged; every in-flight request was denied. | Irreversible for the session — a human must re-unlock, then start over from `deckard_shield`. |
| `expired` | The request outlived its TTL. | Re-run the flow from `deckard_shield` for a fresh `request_id`. |
| `unknown_request` | The daemon restarted or re-unlocked — a clean session, old requests gone. | Re-run the flow from `deckard_shield`; never reuse old request ids. |
| `already_executed` | This exact request already broadcast (ids are deterministic per intent). | **Do NOT retry.** Vary the amount to demo again, or a human re-unlocks for a fresh session. |
| `broadcast_timeout` | The RPC didn't answer in time — the transaction MAY be on-chain. | **Do NOT retry** (double-spend risk). Check the Deckard app / `just demo-check` and act only once status is known. |
| `broadcast_failed: …` | The RPC refused the transaction; nothing was consumed. | Check the chain/RPC is up (`just demo-check`), then re-run from `deckard_shield`. |
| `not_approved` | The request needs a human approval that hasn't happened. | Lower the amount under the per-tx cap, or a human edits `policy.json` (no approval card in this alpha). |
| `user_denied` | A human said no. | Respect it; propose something different only if asked. |
| `resolve_not_authorized` | A `Resolve` (approval) was sent on the public proposer socket, which can't approve — only the Deckard app, over its private channel, can. | Don't try to self-approve. A human approves in the Deckard app (hold-to-confirm); the sidecar never gets a `resolve` tool. |
| `chain_mismatch` | Sidecar and daemon disagree on the chain (e.g. demo sidecar → real daemon). | Re-run `deckard-mcp install --demo` and make sure `just demo` is what's running. |
| `over_cap` | Over the cap with `require_approval = never` — nothing can authorize it. | Lower the amount under `per_tx_cap_wei` (read it with `deckard_policy_get`). |
| `cap_exceeded` | Executing would pass the spending caps as re-checked at sign time. | Lower the amount or wait for the UTC-midnight rollover; re-read the policy for current numbers. |
| `off_allowlist` | The recipient isn't in `allow_to`. | Use an allowed recipient, or a human edits `policy.json`. |
| `undecodable` | The intent's calldata doesn't match its kind (client-side bug if it recurs). | Re-run the flow from `deckard_shield`. |
| `shield_to_mismatch` | The shield doesn't target the official Railgun contract for this chain. | Re-run from `deckard_shield` (it builds the right target); recurring means the chain is unsupported. |
| `unsupported_v1` / `erc20_unsupported_v1` | v0.1 supports native-ETH shield/send only. | Stay with native-ETH `deckard_shield` / `deckard_execute`. |
| `malformed_request` | The daemon couldn't decode the request frame at all (wire-level). | Client/version bug — re-run from `deckard_shield`; make sure the sidecar and app versions match. |
| `off_swap_list` | A swap's sell or buy token isn't in `allow_swap_tokens`. | Use an allowed token, or a human edits `policy.json`. |
| `receiver_not_wallet` | The swap order would pay out to an address other than your wallet. | Re-run the swap flow — it binds the receiver to the operator wallet. |
| `receiver_zero` | The swap order receiver is the zero address. | Re-run the swap flow; a recurring case is a client bug. |
| `zero_amount` | The swap order's sell amount is zero. | Re-quote with a non-zero sell amount. |
| `valid_to_too_far` | The swap order's `valid_to` is more than 24h out. | Re-quote with a `valid_to` inside 24 hours. |
| `not_an_order` | The `request_id` points at a transaction where an order was expected (or vice versa). | Use the id returned by the matching propose call. |
| `already_signed` | The swap order was already signed. | Don't re-sign; cancel via the swap-cancel flow if you need to abort. |
| `approve_no_matching_order` | An `approve` arrived with no stored order matching its token + amount. | Propose the swap order first; the approve must match it exactly. |
| `approve_with_value` | A swap `approve` carried ETH value (would move ETH invisibly). | Re-issue a value-0 approve (the swap flow does this). |
| `approve_wrong_spender` | A swap `approve`'s spender isn't the CoW vault relayer. | Re-issue the approve to the correct spender (the swap flow does this). |
| `derivation_unverified` | The Railgun derivation self-check failed; a view grant was refused. | A bug — restart the app; don't trust a private balance until it clears. |
| `shield_unavailable` | This build has no shielding support. | Use a build with the `shield` feature enabled. |
| `railgun_keys: …` | A Railgun key/grant error (redacted detail appended). | Restart the app; if it recurs, the chain may be unsupported for shielding. |
| `signer_error: …` | The daemon couldn't get an account signer (redacted detail appended). | A human re-unlocks the wallet in the app, then retry. |
| `sign_failed: …` | Offline order-digest signing failed (redacted detail appended). | Re-run the swap flow; a recurring case is a client/daemon bug. |

Two transport-level failures carry the same three-part shape: **socket missing** (the daemon
isn't running — start the Deckard app, or `just demo`) and **connection lost during execute**
(status UNKNOWN — same do-NOT-retry rule as `broadcast_timeout`).

## Demo-world constants

`install --demo` pins the sidecar (and `just demo` pins the daemon) to one isolated world — no
real funds can be touched from it:

| Constant | Value |
|---|---|
| Chain | `11155111` (Sepolia, forked locally by anvil) |
| Config dir | `~/.deckard/demo` |
| Daemon socket | `~/.deckard/demo/signerd.sock` |
| RPC | `http://127.0.0.1:8545` (the local fork) |
| Demo policy | per-tx cap **0.1 ETH** · daily cap **0.5 ETH** · `allow_to` empty (any) · `require_approval: over_cap` · advisory auto-shield min **0.01 ETH** (`policy.demo.json`) |

So the canonical demo ask — shield **0.02 ETH** — is within cap and auto-allows; anything over
0.1 ETH comes back `needs_approval`. Each `just demo` is a fresh fork: re-run `just demo-fund`
and re-shield every time.

## Hard rules (worth restating)

- Never ask the user for a seed phrase, private key, or passphrase — **no tool here accepts
  them**, and nothing you can call will ever return one.
- You cannot change the policy, approve your own requests, or sign anything yourself. That is
  the design, not a missing feature.
- When in doubt, `deckard_revoke_all` is always safe to call.
