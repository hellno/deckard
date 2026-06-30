# Deckard — task runner. Install `just`: brew install just
# (Everything here is plain cargo + macOS built-ins; you can run the commands by hand too.)
# This is a virtual Cargo workspace: `cargo run` launches the app via default-members
# (crates/deckard-app, binary `deckard`); `--workspace` reaches deckard-core + deckard-contract.

# ─── Demo / local-chain dev loop (Work Package D) ────────────────────────────
# Shared demo world constants. These MUST match `deckard-mcp install --demo`
# (crates/deckard-mcp/src/install.rs) and policy.demo.json exactly — see the
# consistency table in CONTRIBUTING. The demo lives entirely under ~/.deckard/demo
# so it never touches your everyday keystore.
demo_dir            := join(env_var('HOME'), ".deckard", "demo")
demo_socket         := join(demo_dir, "signerd.sock")
demo_chain_id       := "11155111"
demo_rpc_url        := "http://127.0.0.1:8545"
demo_anvil_port     := "8545"
demo_fork_block     := env_var_or_default("DECKARD_DEMO_FORK_BLOCK", "10822990")
demo_fund_eth       := "10"

# QA fast-unlock world constants (see the QA section below). Kept as top-level vars so the
# qa recipes and `qa-bridge` resolve identical values — the bridge socket can't drift.
qa_dir              := join("/tmp", "deckard-qa")
qa_socket           := join(qa_dir, "signerd.sock")
qa_chain_id         := "31337"
qa_rpc_url          := "http://127.0.0.1:8545"

# List available recipes.
default:
    @just --list

# Run the app (debug). This is the one you'll use 99% of the time.
# Build the signer daemon first so the app can spawn it as a sibling binary. Dev runs pass
# `--features dev-signerd-bin` so the loose `DECKARD_SIGNERD_BIN`→sibling→PATH resolver is available
# (a RELEASE build / `just bundle` has it compiled out and uses the verified bundled path — C1/#106).
run:
    cargo build -p deckard-signerd
    cargo run --features dev-signerd-bin

# Run optimized. NOTE: no `dev-signerd-bin` here — that feature is dev/test-only and a release
# profile rejects it at compile time (finding C1 / #106). The release resolver finds the daemon as a
# verified sibling under target/release (built first below), so DECKARD_SIGNERD_BIN isn't needed.
run-release:
    cargo build -p deckard-signerd --release
    cargo run --release

# Run as a menu-bar / tray app (no dock icon).
run-tray:
    cargo build -p deckard-signerd
    cargo run -p deckard-app --features tray,dev-signerd-bin

# ─── QA fast-unlock vault (DevEx) ────────────────────────────────────────────
# Skip onboarding (Create -> passphrase -> seed reveal -> backup challenge) on every
# clicky QA run. `qa-vault` seals a THROWAWAY vault (anvil's dev mnemonic — account 0
# is prefunded on any anvil/fork) under a fixed passphrase with FAST KDF into an
# isolated /tmp config dir; `qa` then launches the app there so it boots straight to
# Unlock — type the passphrase it prints (decrypts in ~tens of ms, not ~1 s).
# Throwaway only; never real funds. Production create/import keep KdfParams::PRODUCTION.

# Seal the QA vault into /tmp/deckard-qa (re-run anytime to reset it).
qa-vault:
    DECKARD_CONFIG_DIR="{{qa_dir}}" cargo run -q -p deckard-core --example qa-vault

# Launch the app against the QA vault, pointed at a local anvil (start `anvil` first
# for live balances / send / shield). Run `just qa-vault` once before this.
qa:
    cargo build -p deckard-signerd
    DECKARD_CONFIG_DIR="{{qa_dir}}" \
    DECKARD_SOCKET_PATH="{{qa_socket}}" \
    DECKARD_CHAIN_ID="{{qa_chain_id}}" \
    DECKARD_RPC_URL="{{qa_rpc_url}}" \
    DECKARD_VERIFIED_READS="0" \
        cargo run --features dev-signerd-bin

# Run the key-less browser bridge against the QA fast-unlock app's signer daemon.
# Prereq: `just qa` running in another terminal, unlocked with passphrase `deckard-qa`.
qa-bridge bind="127.0.0.1:8765":
    DECKARD_SOCKET_PATH="{{qa_socket}}" \
    DECKARD_CHAIN_ID="{{qa_chain_id}}" \
        cargo run -p deckard-browser-bridge -- --bind "{{bind}}"

# Full walkthrough (onboard -> demo-fund -> shield): CONTRIBUTING "Demo / local-chain dev loop".
# Wires an isolated ~/.deckard/demo config dir; verified reads are OFF; Ctrl-C tears anvil down.
# Start the demo: a local anvil fork of Sepolia (pinned block) + the Deckard app & daemon.
demo:
    #!/usr/bin/env bash
    set -euo pipefail

    DEMO_DIR="{{demo_dir}}"
    PORT="{{demo_anvil_port}}"
    FORK_BLOCK="{{demo_fork_block}}"

    # 1. Upstream archive RPC is required to fork Sepolia. It is NOT read by any
    #    Deckard binary — it is anvil's --fork-url source. Fail with the exact line to set.
    if [[ -z "${RPC_URL_SEPOLIA:-}" ]]; then
        echo "error: RPC_URL_SEPOLIA is not set." >&2
        echo "  anvil needs a Sepolia *archive* RPC to fork at block ${FORK_BLOCK}." >&2
        echo "  Get a free archive endpoint (e.g. Alchemy/Infura Sepolia) and set it:" >&2
        echo >&2
        echo "    export RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/<your-key>" >&2
        echo >&2
        echo "  Then re-run: just demo   (verify upstream + chain with: just demo-check)" >&2
        exit 1
    fi

    # 2. Double-run / busy-port detection. Port 8545 is FIXED (install --demo hardcodes
    #    it), so a second `just demo` (or any process on 8545) must fail loudly, not
    #    silently attach the demo to a foreign chain.
    if cast block-number --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
        echo "error: something is already listening on ${PORT} (cast reached it)." >&2
        echo "  A demo anvil is probably still running from another 'just demo'." >&2
        echo "  Stop it (Ctrl-C its terminal, or: pkill -f 'anvil .*--port ${PORT}')," >&2
        echo "  then re-run. Run 'just demo-check' to inspect the current chain." >&2
        exit 1
    fi

    # 3. Defensive shielded-sync cache cleanup (HONEST): the app's Railgun sync uses the
    #    default IN-MEMORY database (shielded.rs never calls .with_database()), so NOTHING
    #    is persisted — a relaunch already rebuilds the shielded balance fresh. We still
    #    rm -rf a railgun cache subdir IF a future code change ever starts persisting one,
    #    so run 2 can never render run 1's stale balance. We never fabricate this dir.
    rm -rf "${DEMO_DIR}/railgun" "${DEMO_DIR}/railgun-cache" 2>/dev/null || true

    # 4. Install the demo policy IF ABSENT (re-runs never clobber your edits at the
    #    installed path). The committed root policy.demo.json is the template.
    mkdir -p "${DEMO_DIR}"
    if [[ ! -f "${DEMO_DIR}/policy.json" ]]; then
        cp "{{justfile_directory()}}/policy.demo.json" "${DEMO_DIR}/policy.json"
        echo "→ installed demo policy at ${DEMO_DIR}/policy.json"
    else
        echo "→ demo policy already present at ${DEMO_DIR}/policy.json (left as-is)"
    fi

    # 5. Launch anvil forking Sepolia at the pinned block, on the FIXED port 8545.
    #    Do NOT pass --chain-id: the fork preserves Sepolia's 11155111 automatically.
    echo "→ starting anvil fork of Sepolia @ block ${FORK_BLOCK} on port ${PORT}…"
    anvil \
        --fork-url "${RPC_URL_SEPOLIA}" \
        --fork-block-number "${FORK_BLOCK}" \
        --mnemonic "test test test test test test test test test test test junk" \
        --port "${PORT}" \
        >/dev/null 2>&1 &
    ANVIL_PID=$!

    # 6. Stop anvil on EXIT/INT/TERM. In a non-interactive recipe anvil is a plain child that
    #    shares this script's process group (no job-control group to signal on its own), so we
    #    kill its PID and reap it, with a port-scoped pkill as the belt-and-suspenders fallback.
    cleanup() {
        echo
        echo "→ stopping demo anvil (pid ${ANVIL_PID})…"
        kill "${ANVIL_PID}" 2>/dev/null || true
        wait "${ANVIL_PID}" 2>/dev/null || true
        pkill -f "anvil .*--port ${PORT}" 2>/dev/null || true
    }
    trap cleanup EXIT INT TERM

    # 7. Wait until anvil answers JSON-RPC (poll eth_blockNumber + eth_chainId).
    echo "→ waiting for anvil to be ready…"
    for _ in $(seq 1 100); do
        if cast block-number --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
            break
        fi
        sleep 0.2
    done
    if ! cast block-number --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
        echo "error: anvil never became ready on ${PORT}. Is RPC_URL_SEPOLIA reachable?" >&2
        echo "  Diagnose with: just demo-check" >&2
        exit 1
    fi
    # 7b. ENFORCE the fork's chain identity. `cast chain-id` prints DECIMAL (same as the
    #     demo-check upstream probe), so compare against the decimal {{demo_chain_id}}. If
    #     RPC_URL_SEPOLIA points at a wrong-chain upstream, the fork's chain id won't be
    #     11155111 — but the app/daemon/MCP are still forced to {{demo_chain_id}}, so the demo
    #     would run on a false chain contract. Fail loudly instead (the EXIT trap tears anvil down).
    CHAIN_ID="$(cast chain-id --rpc-url "{{demo_rpc_url}}" 2>/dev/null || echo "?")"
    if [[ "${CHAIN_ID}" != "{{demo_chain_id}}" ]]; then
        echo "error: demo anvil reports chain ${CHAIN_ID}, expected {{demo_chain_id}} (Sepolia)." >&2
        echo "  RPC_URL_SEPOLIA must point at a Sepolia archive node — a wrong-chain upstream" >&2
        echo "  would force the app/daemon/MCP onto a false chain contract." >&2
        echo "  Diagnose with: just demo-check" >&2
        exit 1
    fi
    echo "→ anvil ready (chain id ${CHAIN_ID} = {{demo_chain_id}}, Sepolia fork)."

    # 7c. Seal a throwaway QA vault for anvil account 0 so the app boots straight to Unlock
    #     (no hand-onboarding) and the headless agent (`just demo-agent`) can drive it. Only
    #     seals if absent — a re-run never clobbers a vault you already unlocked.
    if [[ ! -f "${DEMO_DIR}/vault.bin" ]]; then
        just demo-vault
    else
        echo "→ demo vault already present at ${DEMO_DIR}/vault.bin (left as-is)"
    fi

    echo
    echo "  Demo world:  config=${DEMO_DIR}  socket={{demo_socket}}"
    echo "  Next: in the app, Unlock with: deckard-demo   (anvil account 0). Then, in another terminal:"
    echo "        just demo-deposit     # send a real 0.02 ETH inbound transfer (a positive delta)"
    echo "        just demo-agent       # run the headless 'watch & shield' agent loop"
    echo "        just demo-fund        # (optional) top the wallet up with 10 ETH"
    echo "        deckard-mcp install --demo   # (optional) register the sidecar with Claude/CLI"
    echo

    # 8. Build the daemon (sibling binary the app spawns), then run the app with the
    #    full demo env. Mirrors the `run` recipe. Verified reads OFF (Sepolia fork).
    cargo build -p deckard-signerd
    DECKARD_CONFIG_DIR="${DEMO_DIR}" \
    DECKARD_SOCKET_PATH="{{demo_socket}}" \
    DECKARD_CHAIN_ID="{{demo_chain_id}}" \
    DECKARD_RPC_URL="{{demo_rpc_url}}" \
    DECKARD_VERIFIED_READS=0 \
    DECKARD_DEMO_FORK_BLOCK="${FORK_BLOCK}" \
        cargo run --features dev-signerd-bin
    # On `cargo run` exit (app closed), the EXIT trap stops anvil.

# No arg: asks the demo daemon for the onboarded wallet's address (needs `just demo` running +
# the wallet UNLOCKED). With an explicit address, funds that directly.
#   just demo-fund                 # fund the app's onboarded wallet
#   just demo-fund 0xYourAddress   # fund a specific address
# Fund a wallet on the demo anvil fork with 10 ETH.
demo-fund addr="":
    #!/usr/bin/env bash
    set -euo pipefail

    PORT="{{demo_anvil_port}}"
    FUND_ETH="{{demo_fund_eth}}"

    # anvil must be up.
    if ! cast block-number --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
        echo "error: no demo anvil on ${PORT}. Start it first: just demo" >&2
        exit 1
    fi

    ADDR="{{addr}}"
    if [[ -z "${ADDR}" ]]; then
        # Ask the DEMO daemon for the onboarded address. We must export the SAME demo env
        # block the daemon was launched with, or deckard-mcp would talk to the everyday
        # daemon (or none). Requires the app running + wallet unlocked (else "locked").
        echo "→ asking the demo daemon for the onboarded address…"
        ADDR="$(
            DECKARD_CONFIG_DIR="{{demo_dir}}" \
            DECKARD_SOCKET_PATH="{{demo_socket}}" \
            DECKARD_CHAIN_ID="{{demo_chain_id}}" \
            DECKARD_RPC_URL="{{demo_rpc_url}}" \
            cargo run -q -p deckard-mcp -- address 2>/dev/null | jq -r '.address'
        )" || true
        if [[ -z "${ADDR}" || "${ADDR}" == "null" ]]; then
            echo "error: could not read the wallet address from the demo daemon." >&2
            echo "  Make sure 'just demo' is running AND you've created + UNLOCKED a" >&2
            echo "  throwaway wallet in the app. Or pass an address: just demo-fund 0x…" >&2
            exit 1
        fi
    fi

    # 10 ETH in wei (decimal). cast preferred; raw curl JSON-RPC as a fallback.
    WEI="$(cast to-wei "${FUND_ETH}" ether)"
    HEXWEI="$(cast to-hex "${WEI}")"
    echo "→ funding ${ADDR} with ${FUND_ETH} ETH on the demo fork…"
    if ! cast rpc anvil_setBalance "${ADDR}" "${HEXWEI}" --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
        # Fallback: direct JSON-RPC (handles cast-rpc quirks across foundry versions).
        curl -s -X POST "{{demo_rpc_url}}" \
            -H 'content-type: application/json' \
            --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"anvil_setBalance\",\"params\":[\"${ADDR}\",\"${HEXWEI}\"]}" \
            >/dev/null
    fi
    BAL="$(cast balance "${ADDR}" --rpc-url "{{demo_rpc_url}}" 2>/dev/null || echo "?")"
    echo "→ done. ${ADDR} balance now ${BAL} wei (${FUND_ETH} ETH)."
    echo "  In the app the public balance should refresh shortly (reads are raw/Unsynced on the fork)."

# The passphrase is fixed and printed below — this is a PUBLIC dev key on a local fork, never
# a real keystore. `just demo` calls this for you; run it standalone only to (re)create the vault.
# Seal a throwaway QA vault (anvil account 0) so the app boots straight to Unlock (no onboarding).
demo-vault:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{demo_dir}}"
    cargo run -q -p deckard-core --example seal_qa_vault -- "{{demo_dir}}" deckard-demo
    echo "→ sealed demo vault (anvil account 0) at {{demo_dir}}/vault.bin"
    echo "  Unlock passphrase: deckard-demo"

# Unlike `just demo-fund` (anvil_setBalance, which SETS the balance and yields no repeatable
# delta), this is a genuine value transfer from anvil account 1, so you can fire it again and
# again to drive successive shields:
#   just demo-deposit          # 0.02 ETH — within the cap → the auto-shield hero beat
#   just demo-deposit 0.15     # over the 0.1 ETH per-tx cap → the human-approval beat
# Send a REAL inbound transfer to the demo wallet (a positive balance delta the agent notices).
demo-deposit amount='0.02':
    #!/usr/bin/env bash
    set -euo pipefail

    # The demo wallet (anvil account 0) and the funding key (anvil account 1) come from anvil's
    # well-known dev mnemonic "test test test test …junk" — public test keys, safe to hardcode.
    WALLET="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
    FUNDER_KEY="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"

    # anvil must be up.
    if ! cast block-number --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1; then
        echo "error: no demo anvil on {{demo_anvil_port}}. Start it first: just demo" >&2
        exit 1
    fi

    # SAFETY: never broadcast a transfer with a well-known dev key on a real chain. ALLOWLIST the
    # exact demo chain — a chain-1-only denylist would still pass any OTHER real network fronted at
    # :8545 (a forwarded Sepolia/Holesky node, a tunnel) and broadcast a real tx with a public key.
    # (Matches the `demo` recipe's own `CHAIN_ID == demo_chain_id` enforcement.)
    CHAIN_ID="$(cast chain-id --rpc-url "{{demo_rpc_url}}" 2>/dev/null || echo "?")"
    if [[ "${CHAIN_ID}" != "{{demo_chain_id}}" ]]; then
        echo "error: refusing to run — {{demo_rpc_url}} reports chain ${CHAIN_ID}, not the demo chain {{demo_chain_id}}." >&2
        echo "  demo-deposit signs with a PUBLIC anvil dev key; broadcasting it on any real chain is unsafe." >&2
        echo "  Point {{demo_rpc_url}} at the local demo fork (just demo) and retry." >&2
        exit 1
    fi

    # The demo wallet is anvil's PUBLIC account-0 key. On the forked Sepolia it carries an
    # inherited EIP-7702 delegation (its on-chain code is `0xef0100<delegate>`, set by someone on
    # real Sepolia using that public key). A plain ETH transfer to a 7702-delegated account runs
    # the DELEGATE contract instead of crediting the EOA — so the balance never rises and the
    # agent's "did a deposit arrive?" delta-detector never fires (shields, which only SIGN, still
    # work). Clear the delegation back to a plain EOA so deposits land. Local-fork only — guarded
    # by the chain-id≠1 refusal above; `anvil_setCode` is an anvil cheatcode, not a real tx.
    if [[ -n "$(cast code "${WALLET}" --rpc-url "{{demo_rpc_url}}" 2>/dev/null | sed 's/^0x//')" ]]; then
        echo "→ clearing an inherited EIP-7702 delegation on ${WALLET} (so the deposit credits the EOA)…"
        cast rpc anvil_setCode "${WALLET}" 0x --rpc-url "{{demo_rpc_url}}" >/dev/null 2>&1
    fi

    WEI="$(cast to-wei "{{amount}}" ether)"
    echo "→ sending {{amount}} ETH (${WEI} wei) to ${WALLET} on the demo fork (chain ${CHAIN_ID})…"
    cast send "${WALLET}" --value "${WEI}" --private-key "${FUNDER_KEY}" --rpc-url "{{demo_rpc_url}}" >/dev/null
    BAL="$(cast balance "${WALLET}" --rpc-url "{{demo_rpc_url}}" 2>/dev/null || echo "?")"
    echo "→ done. ${WALLET} public balance now ${BAL} wei."
    echo "  Watch the agent pick it up:   just demo-agent"
    echo "  One-shot smoke (asserts a broadcast): just demo-smoke"

# Drives ONLY the key-less deckard-mcp CLI — same authority as the Claude-Desktop prompt, no
# keys here. Needs `just demo` running with the wallet UNLOCKED (passphrase: deckard-demo).
# Pass through extra flags:
#   just demo-agent            # watch forever; shield deposits within the cap, wait on over-cap
#   just demo-agent --once     # one iteration against the LIVE baseline (a no-op unless you also
#                              #   set DECKARD_AGENT_BASELINE_WEI) — use `just demo-smoke` to assert
#   just demo-agent --capture  # print propose→decision→broadcast millis per shield
# Run the headless watch-and-shield agent loop (scripts/demo-agent.sh) against the demo daemon.
demo-agent *args:
    #!/usr/bin/env bash
    set -euo pipefail
    # Build the CLI once so the agent (and its sub-second poll) never pays a cold compile.
    cargo build -q -p deckard-mcp
    # NOTE: the one-shot that ASSERTS a broadcast is `just demo-smoke` — it captures the pre-deposit
    # balance and pins the agent's baseline to it, so `--once` shields EXACTLY the fresh deposit.
    # We deliberately do NOT default `--once`'s baseline to 0 here: the demo wallet is anvil account
    # 0, which is PREFUNDED (≈10 ETH on the fork), so a baseline of 0 would treat the whole balance
    # as surplus and try to shield all of it — busting the per-tx cap and failing the smoke. With no
    # pin the runner reads the live balance as baseline (a bare `--once` is then a safe no-op), and
    # `demo-smoke` supplies the correct pre-deposit baseline via DECKARD_AGENT_BASELINE_WEI.
    DECKARD_SOCKET_PATH="{{demo_socket}}" \
    DECKARD_CONFIG_DIR="{{demo_dir}}" \
    DECKARD_CHAIN_ID="{{demo_chain_id}}" \
    DECKARD_MCP_BIN="${DECKARD_MCP_BIN:-{{justfile_directory()}}/target/debug/deckard-mcp}" \
        bash "{{justfile_directory()}}/scripts/demo-agent.sh" {{args}}

# One-shot CI smoke that ACTUALLY asserts a broadcast: capture the demo wallet's CURRENT balance,
# send a within-cap deposit, then run ONE agent iteration pinned to that pre-deposit baseline so the
# agent shields EXACTLY the fresh deposit and broadcasts (exit 0), or fails (exit 1). This is the
# deposit-aware wrapper a bare `just demo-agent --once` cannot be: a single iteration has no memory
# of the pre-deposit balance, so it can't tell the deposit apart from the (prefunded) baseline.
# Needs `just demo` running with the wallet UNLOCKED. Amount must be within the per-tx cap (0.1 ETH).
demo-smoke amount='0.02':
    #!/usr/bin/env bash
    set -euo pipefail
    WALLET="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"   # demo wallet = anvil account 0
    B0="$(cast balance "${WALLET}" --rpc-url "{{demo_rpc_url}}")"
    echo "→ pre-deposit baseline: $(cast from-wei "${B0}") ETH ($(cast to-wei "{{amount}}" ether) wei deposit incoming)"
    just demo-deposit {{amount}}
    # Pin the agent's baseline to the pre-deposit balance so surplus == exactly the deposit.
    DECKARD_AGENT_BASELINE_WEI="${B0}" just demo-agent --once

# Exit codes: 0 ready · 10 foundry missing · 11 RPC_URL_SEPOLIA · 12 local anvil · 13 signerd
# build · 14 app not running/unlocked · 15 chain-identity mismatch · 16 policy drift.
# Doctor: diagnose the demo environment and print the exact fix for each failure.
demo-check:
    #!/usr/bin/env bash
    set -uo pipefail   # NOTE: no -e — we want to run every check and report all of them.

    RC=0
    PORT="{{demo_anvil_port}}"
    DEMO_DIR="{{demo_dir}}"
    EXPECT_CHAIN_HEX="0xaa36a7"   # 11155111

    say()  { printf '  %s\n' "$1"; }
    ok()   { printf '✓ %s\n' "$1"; }
    bad()  { printf '✗ %s\n' "$1"; }

    # 1. anvil + cast on PATH (platform-conditional fix).
    if command -v anvil >/dev/null 2>&1 && command -v cast >/dev/null 2>&1; then
        ok "foundry present ($(anvil --version 2>/dev/null | head -1))"
    else
        bad "anvil/cast not found on PATH"
        if [[ "$(uname -s)" == "Darwin" ]]; then
            say "fix: brew install foundry   (or: curl -L https://foundry.paradigm.xyz | bash && foundryup)"
        else
            say "fix: curl -L https://foundry.paradigm.xyz | bash && foundryup"
        fi
        RC=10
    fi

    # 2. RPC_URL_SEPOLIA set + reachable + actually Sepolia (eth_chainId == 0xaa36a7).
    if [[ -z "${RPC_URL_SEPOLIA:-}" ]]; then
        bad "RPC_URL_SEPOLIA is not set (anvil's upstream fork source; not read by Deckard)"
        say "fix: export RPC_URL_SEPOLIA=https://eth-sepolia.g.alchemy.com/v2/<your-key>"
        [[ $RC -eq 0 ]] && RC=11
    else
        UP_CHAIN="$(cast chain-id --rpc-url "${RPC_URL_SEPOLIA}" 2>/dev/null || echo "")"
        if [[ "${UP_CHAIN}" == "11155111" ]]; then
            ok "RPC_URL_SEPOLIA reachable and is Sepolia (11155111)"
        elif [[ -z "${UP_CHAIN}" ]]; then
            bad "RPC_URL_SEPOLIA set but not reachable"
            say "fix: check the URL/key; the endpoint must serve a Sepolia *archive* node"
            [[ $RC -eq 0 ]] && RC=11
        else
            bad "RPC_URL_SEPOLIA reachable but reports chain ${UP_CHAIN}, not Sepolia 11155111"
            say "fix: point RPC_URL_SEPOLIA at a Sepolia endpoint"
            [[ $RC -eq 0 ]] && RC=11
        fi
    fi

    # 3. Local demo anvil on 8545 reachable + correct chain id.
    LOCAL_CHAIN_HEX="$(cast rpc eth_chainId --rpc-url "{{demo_rpc_url}}" 2>/dev/null | tr -d '"' || echo "")"
    if [[ -z "${LOCAL_CHAIN_HEX}" ]]; then
        bad "no local anvil on ${PORT}"
        say "fix: start it with 'just demo'"
        [[ $RC -eq 0 ]] && RC=12
    elif [[ "${LOCAL_CHAIN_HEX}" == "${EXPECT_CHAIN_HEX}" ]]; then
        ok "local anvil on ${PORT} is the Sepolia fork (chain ${LOCAL_CHAIN_HEX} = 11155111)"
    else
        bad "local anvil on ${PORT} reports chain ${LOCAL_CHAIN_HEX}, expected ${EXPECT_CHAIN_HEX} (11155111)"
        say "fix: stop the foreign process on ${PORT} and re-run 'just demo'"
        [[ $RC -eq 0 ]] && RC=12
    fi

    # 4. signerd builds (also pre-build deckard-mcp so the probe below has no compile noise).
    if cargo build -p deckard-signerd -p deckard-mcp >/dev/null 2>&1; then
        ok "deckard-signerd builds"
    else
        bad "deckard-signerd failed to build"
        say "fix: run 'cargo build -p deckard-signerd' and read the error"
        [[ $RC -eq 0 ]] && RC=13
    fi

    # 5. App running + UNLOCKED + chain-identity — probe via `address` (NOT `policy`).
    #    `address` requires the vault UNLOCKED (PolicyGet succeeds even when locked, so it can
    #    NOT prove unlock); and the sidecar's connect-time chain probe runs first, so a
    #    wrong-chain daemon surfaces chain_mismatch once unlocked. Capture stdout+stderr.
    ADDR_PROBE="$(
        DECKARD_CONFIG_DIR="${DEMO_DIR}" \
        DECKARD_SOCKET_PATH="{{demo_socket}}" \
        DECKARD_CHAIN_ID="{{demo_chain_id}}" \
        DECKARD_RPC_URL="{{demo_rpc_url}}" \
        cargo run -q -p deckard-mcp -- address 2>&1
    )" ; PROBE_RC=$?
    if [[ ${PROBE_RC} -eq 0 ]] && echo "${ADDR_PROBE}" | jq -e '.address' >/dev/null 2>&1; then
        ok "demo daemon reachable, unlocked, on the right chain ($(echo "${ADDR_PROBE}" | jq -r '.address'))"
        # 6. Policy drift: diff the LIVE policy (PolicyGet) vs the intended demo values. The
        #    per_tx_cap_wei + require_approval below come from the demo's SEND rule; daily_cap_wei
        #    is the global daily wall. PolicyGet surfaces these top-level for convenience and renders
        #    the send rule's approval as the snake_case string "over_cap" (sidecar.rs).
        POLICY_JSON="$(
            DECKARD_CONFIG_DIR="${DEMO_DIR}" \
            DECKARD_SOCKET_PATH="{{demo_socket}}" \
            DECKARD_CHAIN_ID="{{demo_chain_id}}" \
            DECKARD_RPC_URL="{{demo_rpc_url}}" \
            cargo run -q -p deckard-mcp -- policy 2>/dev/null
        )"
        GOT_PER_TX="$(echo "${POLICY_JSON}" | jq -r '.per_tx_cap_wei // empty')"
        GOT_DAILY="$(echo "${POLICY_JSON}"  | jq -r '.daily_cap_wei // empty')"
        GOT_MODE="$(echo "${POLICY_JSON}"   | jq -r '.require_approval // empty')"
        echo
        echo "─── current demo state ───────────────────────────────────"
        echo "  chain:           11155111 (Sepolia fork) — mainnet guardrail INACTIVE"
        echo "  per_tx_cap_wei:  ${GOT_PER_TX}  (send rule's per-tx cap — 0.1 ETH)"
        echo "  daily_cap_wei:   ${GOT_DAILY}  (global daily wall — 0.5 ETH)"
        echo "  require_approval:${GOT_MODE}  (send rule: within-cap auto-allows; a send over 0.1 ETH -> NeedsApproval)"
        echo "  auto-allow:      ON for within-cap writes (Sepolia; no mainnet guardrail)"
        echo "──────────────────────────────────────────────────────────"
        if [[ "${GOT_PER_TX}" != "100000000000000000" || "${GOT_DAILY}" != "500000000000000000" || "${GOT_MODE}" != "over_cap" ]]; then
            bad "live policy differs from the intended demo policy"
            say "intended: per_tx_cap_wei=100000000000000000 daily_cap_wei=500000000000000000 require_approval=over_cap"
            say "fix: the daemon may have hit POLICY FALLBACK (check the app log for '⚠ POLICY FALLBACK')."
            say "     Repair ${DEMO_DIR}/policy.json (copy from repo policy.demo.json) and relaunch 'just demo'."
            [[ $RC -eq 0 ]] && RC=16
        else
            ok "live policy matches the intended demo policy"
        fi
    else
        # Classify: chain mismatch vs locked/no-wallet vs socket-missing.
        if echo "${ADDR_PROBE}" | grep -qi "chain_mismatch\|chain-1 daemon\|install --demo"; then
            bad "connected to a wrong-chain daemon (chain mismatch)"
            say "fix: quit any everyday Deckard, then run 'just demo'; re-run 'deckard-mcp install --demo'"
            [[ $RC -eq 0 ]] && RC=15
        elif echo "${ADDR_PROBE}" | grep -qi "locked\|no wallet\|no vault"; then
            bad "demo daemon reachable but locked / no wallet yet"
            say "fix: in the 'just demo' app, create a throwaway wallet and UNLOCK it"
            [[ $RC -eq 0 ]] && RC=14
        else
            bad "demo daemon not reachable (is 'just demo' running?)"
            say "fix: start the app with 'just demo' (it spawns the signer daemon)"
            [[ $RC -eq 0 ]] && RC=14
        fi
    fi

    echo
    if [[ $RC -eq 0 ]]; then
        echo "demo-check: READY (exit 0)"
    else
        echo "demo-check: NOT READY (exit ${RC}) — fix the ✗ items above."
    fi
    exit $RC

# Run the key-less browser bridge against the DEMO app's signer daemon.
# Prereq: `just demo` running in another terminal with the wallet unlocked.
# Reuses the demo world's socket+chain so the bridge can never dial the wrong daemon.
demo-bridge bind="127.0.0.1:8765":
    DECKARD_SOCKET_PATH="{{demo_socket}}" \
    DECKARD_CHAIN_ID="{{demo_chain_id}}" \
        cargo run -p deckard-browser-bridge -- --bind "{{bind}}"

# Format + lint the whole workspace (both feature configurations of the app).
fmt:
    cargo fmt
check:
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo clippy --locked -p deckard-app --all-targets --features tray -- -D warnings
    # The dev-only daemon-binary resolver (DECKARD_SIGNERD_BIN→sibling→PATH) is feature-gated out of
    # release builds (finding C1 / #106); lint that arm too so `just run`/`qa`/`demo` can't rot.
    cargo clippy --locked -p deckard-signerd --all-targets --features dev-signerd-bin -- -D warnings

# Engine-only inner loop: checks + tests deckard-core WITHOUT building the gpui app. Use it while
# iterating on keystore/eth/balances. (The heavy verified-reads/shield deps compile once, then it's
# fast.) The full Definition of Done still applies before "done": `just check` + `cargo test --workspace`.
core:
    cargo clippy -p deckard-core --all-targets --locked -- -D warnings
    cargo test -p deckard-core --locked

# Bump the git GPUI stack to the latest upstream commits, then rebuild.
# Reproducibility lives in Cargo.lock — commit it (and rust-toolchain.toml if you
# bumped it) after this succeeds. If the build fails on an unstable-feature error,
# match rust-toolchain.toml to Zed's: https://github.com/zed-industries/zed/blob/main/rust-toolchain.toml
# Full procedure + the crates.io fallback channel: docs/UPGRADING.md
bump-gpui:
    cargo update -p gpui -p gpui_platform -p gpui-component -p gpui-component-assets
    cargo build
    @echo "→ Bumped. Run the app to smoke-test, then commit Cargo.lock (+ rust-toolchain.toml if changed)."

# Build a distributable Deckard.app (needs: cargo install cargo-bundle).
# Two parts, because cargo-bundle only ships THIS crate's own `deckard` binary:
#   1. Build the signer daemon in release so it exists as target/release/deckard-signerd.
#   2. `cargo bundle` (run from crates/deckard-app so cargo-bundle resolves the relative icon
#      path — it uses the CWD, not the manifest). Output → workspace target/release/bundle/osx.
#   3. Copy the daemon into Contents/MacOS/ next to `deckard`. The RELEASE resolver
#      (deckard-signerd::supervise::resolve_binary, finding C1) launches the daemon ONLY as a
#      provenance-verified SIBLING of the app binary — no $PATH / env fallback — so without this
#      copy the bundled app can't spawn the signer, never binds the socket, and Unlock fails with
#      "connect …/signerd.sock: No such file or directory" (issue #134). `install -m 0755` keeps
#      it a regular, owner-only-writable file so verify_bundled_binary accepts it, and is re-run-safe.
bundle:
    cargo build -p deckard-signerd --release
    cd crates/deckard-app && cargo bundle --release
    install -m 0755 target/release/deckard-signerd \
        target/release/bundle/osx/Deckard.app/Contents/MacOS/deckard-signerd
    @echo "→ target/release/bundle/osx/Deckard.app (app + bundled signer daemon)"

# Open the bundled app.
open: bundle
    open "target/release/bundle/osx/Deckard.app"

# Regenerate the app icon (crates/deckard-app/assets/icon.png + .icns) from icon.svg.
# Needs cairosvg (pip install cairosvg); falls back to qlmanage if missing.
# Uses only macOS built-ins (sips, iconutil) for the .icns step.
icon:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/deckard-app/assets
    if command -v cairosvg >/dev/null; then
        cairosvg icon.svg -o icon.png -W 1024 -H 1024
    else
        qlmanage -t -s 1024 -o . icon.svg >/dev/null && mv icon.svg.png icon.png
    fi
    rm -rf icon.iconset && mkdir icon.iconset
    for sz in 16 32 64 128 256 512; do
        sips -z $sz $sz       icon.png --out icon.iconset/icon_${sz}x${sz}.png   >/dev/null
        sips -z $((sz*2)) $((sz*2)) icon.png --out icon.iconset/icon_${sz}x${sz}@2x.png >/dev/null
    done
    sips -z 1024 1024 icon.png --out icon.iconset/icon_512x512@2x.png >/dev/null
    iconutil -c icns icon.iconset -o icon.icns
    rm -rf icon.iconset
    echo "→ assets/icon.png + assets/icon.icns regenerated"

# Pre-flight a release tag BEFORE pushing it: validate the tag shape, assert every
# crate is at that version, and print the tag's CHANGELOG section (the release body).
# This is the exact script .github/workflows/release.yml runs in CI, so local and CI
# agree. Fails loudly with the fix if anything is off.  Usage: just release-check v0.0.2-alpha
release-check tag:
    @scripts/release-check.sh "{{tag}}"
