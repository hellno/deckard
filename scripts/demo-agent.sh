#!/usr/bin/env bash
# demo-agent.sh — the headless version of the "Atlas" agent loop (docs/build/32-agent-loop-prompt.md).
#
# A presenter runs this instead of pasting the prompt into Claude Desktop and hand-firing shields.
# It drives the SAME key-less MCP CLI (deckard-mcp) the prompt does — no new authority, no keys here.
# The daemon still holds the key, the policy gate still decides, and a human still approves or STOPs.
#
# What it does, every couple of seconds: read the public balance, and when fresh ETH lands, shield it.
# Within the cap it auto-allows and broadcasts; over the cap it waits for the human to approve in the
# Deckard app; on STOP it reports the kill and exits.
#
# Deps: bash, jq, cast (foundry), plus bc + perl (both default on macOS and Linux) for
#       arbitrary-precision wei math and millisecond timing — bash's 64-bit ints overflow
#       above ~9.2 ETH in wei, so the comparisons must not run through bash arithmetic.
# Env (defaults are the demo world, matching the Justfile demo_* variables):
#   DECKARD_SOCKET_PATH   default ~/.deckard/demo/signerd.sock
#   DECKARD_CONFIG_DIR    default ~/.deckard/demo
#   DECKARD_CHAIN_ID      default 11155111 (Sepolia fork)
#   DECKARD_MCP_BIN       the CLI to drive (else target/debug/deckard-mcp, else deckard-mcp on PATH)
#   DECKARD_AGENT_BASELINE_WEI  pin the starting baseline instead of reading the live balance.
#       Use it for the --once smoke: the wallet held ~0 before `just demo-deposit`, so pinning
#       the baseline to 0 makes the just-deposited ETH the delta this one iteration shields.
# Flags: --once (one iteration, CI smoke), --capture (print propose→decision→broadcast millis),
#        --interval N (override the 2s poll).
set -euo pipefail

# ─── Configuration ───────────────────────────────────────────────────────────
HOME_DEMO="${HOME}/.deckard/demo"
export DECKARD_SOCKET_PATH="${DECKARD_SOCKET_PATH:-${HOME_DEMO}/signerd.sock}"
export DECKARD_CONFIG_DIR="${DECKARD_CONFIG_DIR:-${HOME_DEMO}}"
export DECKARD_CHAIN_ID="${DECKARD_CHAIN_ID:-11155111}"

ONCE=0
CAPTURE=0
INTERVAL=2
while [[ $# -gt 0 ]]; do
    case "$1" in
        --once)         ONCE=1; shift ;;
        --capture)      CAPTURE=1; shift ;;
        --interval)     INTERVAL="${2:?--interval needs a number}"; shift 2 ;;
        --interval=*)   INTERVAL="${1#*=}"; shift ;;
        -h|--help)      sed -n '2,30p' "$0"; exit 0 ;;
        *)              echo "demo-agent: unknown flag '$1'" >&2; exit 2 ;;
    esac
done

# Resolve the CLI: explicit override → built debug binary next to this repo → PATH.
resolve_mcp_bin() {
    if [[ -n "${DECKARD_MCP_BIN:-}" ]]; then printf '%s' "${DECKARD_MCP_BIN}"; return; fi
    local repo_bin; repo_bin="$(cd "$(dirname "$0")/.." && pwd)/target/debug/deckard-mcp"
    if [[ -x "${repo_bin}" ]]; then printf '%s' "${repo_bin}"; return; fi
    printf '%s' "deckard-mcp"
}
MCP_BIN="$(resolve_mcp_bin)"

for dep in jq cast bc perl; do
    command -v "$dep" >/dev/null 2>&1 || { echo "demo-agent: missing dependency '$dep' (foundry → cast; jq via your package manager; bc + perl ship with macOS/Linux)" >&2; exit 3; }
done

# ─── MCP CLI plumbing ────────────────────────────────────────────────────────
# Run one subcommand. stdout → JSON (success), stderr → human problem/cause/fix (failure).
# We capture BOTH so we can classify deny tags that live in the human copy, not just JSON.
MCP_OUT=""   # last stdout (the JSON on success)
MCP_ERR=""   # last stderr (the problem/cause/fix on failure)
MCP_RC=0
mcp() {
    local err_file; err_file="$(mktemp)"
    set +e
    MCP_OUT="$("${MCP_BIN}" "$@" 2>"${err_file}")"
    MCP_RC=$?
    set -e
    MCP_ERR="$(cat "${err_file}")"
    rm -f "${err_file}"
}

# Pull a string field out of the last successful JSON; empty if absent/not JSON.
mcp_field() { printf '%s' "${MCP_OUT}" | jq -r "$1 // empty" 2>/dev/null || true; }

# Does the last call's combined output mention a given (case-insensitive) phrase?
# Deny tags surface in the human error copy (failure.rs), so match BOTH streams.
mcp_says() { printf '%s\n%s' "${MCP_OUT}" "${MCP_ERR}" | grep -qiE "$1"; }

# STOP is the one terminal signal: a shield after STOP reads `locked`, an execute of an
# approved request reads `revoked` — both mean the key was zeroized.
mcp_is_stop() { mcp_says 'locked|revoked|signer is stopped|revoke_all'; }

# A LOCKED daemon does not FAIL `balance`/`policy` — it succeeds (exit 0) and renders the
# locked balance as `read_status: "unsynced (unverified read): locked"`, public_wei "0". So a
# clean exit code is NOT proof the wallet is armed; we have to inspect read_status for "locked".
# bash 3.2 has no ${var,,}, so lowercase with tr and match with grep (case-insensitive too).
balance_is_locked() {
    printf '%s' "$(mcp_field '.read_status')" | tr '[:upper:]' '[:lower:]' | grep -q 'locked'
}

# Wall-clock millis (cast/date have no portable %3N on macOS; perl is a built-in everywhere).
now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time()*1000'; }

# Arbitrary-precision wei helpers via bc (bash ints overflow above ~9.2 ETH in wei).
# wei_sub A B → A-B ; wei_ge/wei_gt/wei_le A B → "1" or "0".
wei_sub() { echo "$1 - $2" | bc; }
wei_ge()  { echo "$1 >= $2" | bc; }
wei_gt()  { echo "$1 > $2"  | bc; }
wei_le()  { echo "$1 <= $2" | bc; }

say() { printf 'Atlas: %s\n' "$1"; }

# ─── Baseline: read the policy once, take a balance baseline ──────────────────
mcp policy
if [[ ${MCP_RC} -ne 0 ]]; then
    echo "Atlas: could not read policy — is 'just demo' running with the wallet unlocked?" >&2
    printf '%s\n' "${MCP_ERR}" >&2
    exit 1
fi
AUTO_MIN_WEI="$(mcp_field '.auto_shield_min_wei')"
CAP_WEI="$(mcp_field '.per_tx_cap_wei')"
: "${AUTO_MIN_WEI:=0}" "${CAP_WEI:=0}"

if [[ -n "${DECKARD_AGENT_BASELINE_WEI:-}" ]]; then
    # Pinned baseline (the --once smoke): treat funds above this as a new deposit to shield.
    LAST_SEEN_WEI="${DECKARD_AGENT_BASELINE_WEI}"
else
    mcp balance
    if [[ ${MCP_RC} -ne 0 ]]; then
        echo "Atlas: could not read balance — is the wallet unlocked?" >&2
        printf '%s\n' "${MCP_ERR}" >&2
        exit 1
    fi
    # A locked daemon SUCCEEDS at `balance` (exit 0, public_wei "0", read_status "…locked"), so a
    # clean exit isn't proof it's armed. Without this guard the baseline would be 0 and the loop
    # would poll forever shielding nothing — fail fast with the fix instead.
    if balance_is_locked; then
        echo "demo-agent: the wallet is LOCKED — unlock it in the Deckard app first (the daemon holds no key until you do)." >&2
        exit 1
    fi
    LAST_SEEN_WEI="$(mcp_field '.public_wei')"
    : "${LAST_SEEN_WEI:=0}"
fi

say "read policy (cap $(cast from-wei "${CAP_WEI}") ETH, auto-shield floor $(cast from-wei "${AUTO_MIN_WEI}") ETH) · baseline $(cast from-wei "${LAST_SEEN_WEI}") ETH · watching…"

# Pending request_ids the human still has to approve. Parallel arrays: id + whether we've
# already narrated the "waiting" line (so we say it once, not every poll).
PENDING_IDS=()
PENDING_NARRATED=()
BROADCASTS=0   # how many shields actually broadcast this run (for --once exit code)

# Empty-array expansion under `set -u` is an error in bash 3.2 (the macOS default), so every
# access to these arrays guards on the count first.
pending_count() { printf '%s' "${#PENDING_IDS[@]}"; }
pending_index() {
    local target="$1" i
    [[ "$(pending_count)" -eq 0 ]] && return 1
    for i in "${!PENDING_IDS[@]}"; do
        [[ "${PENDING_IDS[$i]}" == "${target}" ]] && { printf '%s' "$i"; return 0; }
    done
    return 1
}
pending_add() { PENDING_IDS+=("$1"); PENDING_NARRATED+=(0); }
pending_drop() {
    local i; i="$(pending_index "$1")" || return 0
    unset 'PENDING_IDS[i]' 'PENDING_NARRATED[i]'
    # Re-pack to close the index gap — but only when something remains (3.2-safe).
    if [[ "$(pending_count)" -gt 0 ]]; then
        PENDING_IDS=("${PENDING_IDS[@]}"); PENDING_NARRATED=("${PENDING_NARRATED[@]}")
    fi
}

# ─── Try to finish an approved request: execute(id). Returns 0 to keep it, 1 to drop it,
#     and sets STOPPED=1 on a revoked/locked answer. ───────────────────────────
STOPPED=0
try_execute() {
    local id="$1" t0 t1
    t0="$(now_ms)"
    mcp execute "${id}"
    t1="$(now_ms)"
    if [[ ${MCP_RC} -eq 0 && "$(mcp_field '.status')" == "broadcast" ]]; then
        local tx; tx="$(mcp_field '.tx_hash')"
        say "broadcast ✓ ${tx}"
        [[ ${CAPTURE} -eq 1 ]] && printf 'Atlas[capture]: execute→broadcast %s ms\n' "$((t1 - t0))"
        BROADCASTS=$((BROADCASTS + 1))
        return 1
    fi
    if mcp_is_stop; then STOPPED=1; return 1; fi
    if mcp_says 'a human denied|user_denied'; then
        say "the human denied that request — dropping it."
        return 1
    fi
    # not_approved (or any other transient): keep waiting.
    return 0
}

# ─── One iteration of the watch loop. Sets STOPPED=1 on a STOP signal. ────────
iterate() {
    mcp balance
    if [[ ${MCP_RC} -ne 0 ]]; then
        if mcp_is_stop; then STOPPED=1; return; fi
        return   # transient (empty/garbled) — skip this tick
    fi
    # STOP mid-IDLE: a locked daemon still SUCCEEDS at `balance` (exit 0, public_wei "0"), so the
    # non-zero-exit STOP branch above is dead for an idle loop. Detect the kill in read_status.
    if balance_is_locked; then STOPPED=1; return; fi
    local public_wei; public_wei="$(mcp_field '.public_wei')"
    [[ -z "${public_wei}" ]] && return

    # (a) Ratchet DOWN: a post-shield balance drop must not permanently mask later deposits.
    if [[ "$(wei_le "${public_wei}" "${LAST_SEEN_WEI}")" == "1" ]]; then
        LAST_SEEN_WEI="${public_wei}"
    fi

    # (b) Did fresh, above-floor ETH arrive?
    local delta; delta="$(wei_sub "${public_wei}" "${LAST_SEEN_WEI}")"
    if [[ "$(wei_gt "${delta}" 0)" == "1" && "$(wei_ge "${delta}" "${AUTO_MIN_WEI}")" == "1" ]]; then
        # Leave a little gas headroom if the deposit is ~the whole balance: shield the delta,
        # but never more than (balance - 0.001 ETH) so the wallet can pay for the shield tx.
        local headroom="1000000000000000"   # 0.001 ETH
        local cap_wei shield_wei
        cap_wei="$(wei_sub "${public_wei}" "${headroom}")"
        shield_wei="${delta}"
        [[ "$(wei_gt "${shield_wei}" "${cap_wei}")" == "1" ]] && shield_wei="${cap_wei}"
        if [[ "$(wei_le "${shield_wei}" 0)" == "1" ]]; then
            LAST_SEEN_WEI="${public_wei}"   # all dust/gas — nothing to shield
            return
        fi
        local amount_eth; amount_eth="$(cast from-wei "${shield_wei}")"
        say "noticed +$(cast from-wei "${delta}") ETH · proposing shield of ${amount_eth} ETH…"

        local t0 t1
        t0="$(now_ms)"
        mcp shield --amount-eth "${amount_eth}"
        t1="$(now_ms)"
        if [[ ${MCP_RC} -ne 0 ]]; then
            if mcp_is_stop; then STOPPED=1; return; fi
            say "shield refused (transient) — will retry next poll."
            printf '%s\n' "${MCP_ERR}" >&2
            return
        fi
        local decision req_id
        decision="$(mcp_field '.decision')"
        req_id="$(mcp_field '.request_id')"
        # Idempotency: advance the high-water mark the instant we've proposed, before execute.
        LAST_SEEN_WEI="${public_wei}"

        if [[ "${decision}" == "allow" ]]; then
            [[ ${CAPTURE} -eq 1 ]] && printf 'Atlas[capture]: propose→decision(allow) %s ms\n' "$((t1 - t0))"
            local e0 e1; e0="$(now_ms)"
            mcp execute "${req_id}"
            e1="$(now_ms)"
            if [[ ${MCP_RC} -eq 0 && "$(mcp_field '.status')" == "broadcast" ]]; then
                say "auto-approved within cap · broadcast ✓ $(mcp_field '.tx_hash')"
                [[ ${CAPTURE} -eq 1 ]] && printf 'Atlas[capture]: execute→broadcast %s ms\n' "$((e1 - e0))"
                BROADCASTS=$((BROADCASTS + 1))
            elif mcp_is_stop; then
                STOPPED=1
            else
                say "execute did not broadcast — saving ${req_id} to retry next poll."
                pending_add "${req_id}"
            fi
        elif [[ "${decision}" == "needs_approval" ]]; then
            [[ ${CAPTURE} -eq 1 ]] && printf 'Atlas[capture]: propose→decision(needs_approval) %s ms\n' "$((t1 - t0))"
            say "over cap — waiting for you in the Deckard app — Activity feed, the 'Needs you' band (⌘⇧A). Saved ${req_id}."
            pending_add "${req_id}"
        else
            say "unexpected shield decision '${decision}' — leaving it for the next poll."
        fi
    fi

    # (c) Retry every request a human still owes an answer on. Iterate a SNAPSHOT of the ids
    #     because try_execute → pending_drop repacks the live array mid-loop.
    [[ "$(pending_count)" -eq 0 ]] && return
    local snapshot id idx
    snapshot=("${PENDING_IDS[@]}")
    for id in "${snapshot[@]}"; do
        if try_execute "${id}"; then
            idx="$(pending_index "${id}")" || continue
            if [[ "${PENDING_NARRATED[$idx]}" == "0" ]]; then
                say "still waiting on your approval for ${id}…"
                PENDING_NARRATED[idx]=1
            fi
        else
            pending_drop "${id}"
            [[ ${STOPPED} -eq 1 ]] && return
        fi
    done
}

# ─── Drive the loop ──────────────────────────────────────────────────────────
if [[ ${ONCE} -eq 1 ]]; then
    iterate
    if [[ ${STOPPED} -eq 1 ]]; then
        say "STOP — key zeroized, in-flight denied; unlock in the app to re-arm."
        exit 0
    fi
    if [[ ${BROADCASTS} -eq 0 ]]; then
        echo "demo-agent --once: no shield broadcast this iteration (fund a deposit first: just demo-deposit)" >&2
        exit 1
    fi
    exit 0
fi

while true; do
    iterate
    if [[ ${STOPPED} -eq 1 ]]; then
        say "STOP — key zeroized, in-flight denied; unlock in the app to re-arm."
        exit 0
    fi
    sleep "${INTERVAL}"
done
