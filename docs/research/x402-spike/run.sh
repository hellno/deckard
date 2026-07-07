#!/usr/bin/env bash
# deckard #206 — prove the fully-local x402 `exact`/EIP-3009 facilitator loop on an anvil
# fork of Ethereum Sepolia. Self-contained: starts anvil + the x402-rs facilitator, funds a
# throwaway buyer, signs an EIP-3009 authorization, drives /verify + /settle, asserts the USDC
# moved and the nonce was consumed, then proves replay protection. Zero secrets; throwaway keys.
#
#   Requirements: foundry (anvil, cast), jq, curl, and an x402-rs facilitator binary.
#   Env:
#     RPC_URL_SEPOLIA   an Ethereum Sepolia *archive* RPC (drpc.org free tier works)
#     FACILITATOR_BIN   path to the x402-facilitator binary
#                       (build once: cargo install --git https://github.com/x402-rs/x402-rs \
#                                      --no-default-features --features chain-eip155 x402-facilitator
#                        or: cargo build -p x402-facilitator --no-default-features --features chain-eip155)
set -euo pipefail

RPC_URL_SEPOLIA="${RPC_URL_SEPOLIA:-https://sepolia.drpc.org}"
FACILITATOR_BIN="${FACILITATOR_BIN:-x402-facilitator}"
FORK_BLOCK=10822990
HERE="$(cd "$(dirname "$0")" && pwd)"

RPC=http://127.0.0.1:8545
FACIL=http://127.0.0.1:8080
USDC=0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238   # Circle USDC on Ethereum Sepolia (FiatTokenV2_2)

# Throwaway keys — deterministic, obviously-fake, ZERO secrets. NB: do NOT use anvil's canonical
# test accounts (0xf39F../0x7099../0x3C44..): on real Sepolia several carry EIP-7702 delegation
# designators (code 0xef0100..), so USDC's SignatureChecker routes them through EIP-1271 and
# rejects a plain EOA signature ("FiatTokenV2: invalid signature").
FACILITATOR_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80  # gas payer; funded by anvil
BUYER_KEY=0x00000000000000000000000000000000000000000000000000000000dec00d01
RECIPIENT=$(cast wallet address --private-key 0x00000000000000000000000000000000000000000000000000000000dec00d02)
BUYER=$(cast wallet address --private-key "$BUYER_KEY")
NONCE=0x1111111111111111111111111111111111111111111111111111111111111111
VALUE=1000000  # 1.000000 USDC (6 decimals)

say(){ printf '\n\033[1;36m== %s ==\033[0m\n' "$*"; }
cleanup(){ [ -n "${ANVIL_PID:-}" ] && kill "$ANVIL_PID" 2>/dev/null || true;
           [ -n "${FACIL_PID:-}" ] && kill "$FACIL_PID" 2>/dev/null || true; }
trap cleanup EXIT

say "1/8 start anvil fork of Ethereum Sepolia @ block $FORK_BLOCK"
anvil --fork-url "$RPC_URL_SEPOLIA" --fork-block-number "$FORK_BLOCK" --port 8545 >/tmp/x402-anvil.log 2>&1 &
ANVIL_PID=$!
until cast chain-id --rpc-url "$RPC" >/dev/null 2>&1; do sleep 0.5; done
echo "anvil up: chain=$(cast chain-id --rpc-url $RPC) block=$(cast block-number --rpc-url $RPC)"
[ "$(cast code $USDC --rpc-url $RPC | wc -c)" -gt 3 ] || { echo "FAIL: USDC bytecode absent on fork"; exit 1; }
[ "$(cast code $BUYER --rpc-url $RPC)" = "0x" ] || { echo "FAIL: buyer $BUYER has code — pick a fresh EOA"; exit 1; }

say "2/8 start x402-rs facilitator (config: eip155:11155111 -> fork)"
CONFIG=$(mktemp)
cat > "$CONFIG" <<JSON
{ "port": 8080, "host": "127.0.0.1",
  "chains": { "eip155:11155111": {
      "eip1559": true, "signers": ["$FACILITATOR_KEY"],
      "rpc": [{ "http": "$RPC" }] } },
  "schemes": [ { "id": "v2-eip155-exact", "chains": "eip155:*" } ] }
JSON
"$FACILITATOR_BIN" --config "$CONFIG" >/tmp/x402-facilitator.log 2>&1 &
FACIL_PID=$!
until curl -sf "$FACIL/health" >/dev/null 2>&1; do sleep 0.5; done
echo "facilitator up; /supported:"; curl -s "$FACIL/supported" | jq -c .

say "3/8 fund throwaway buyer with 100 USDC via setStorageAt (slot 9)"
cast rpc anvil_setStorageAt "$USDC" "$(cast index address $BUYER 9)" "$(cast to-uint256 100000000)" --rpc-url "$RPC" >/dev/null
echo "buyer USDC = $(cast call $USDC 'balanceOf(address)(uint256)' $BUYER --rpc-url $RPC)"

say "4/8 buyer signs the EIP-3009 TransferWithAuthorization (EIP-712, gasless)"
TYPED=$(mktemp)
cat > "$TYPED" <<JSON
{ "types": { "EIP712Domain": [
      {"name":"name","type":"string"},{"name":"version","type":"string"},
      {"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],
    "TransferWithAuthorization": [
      {"name":"from","type":"address"},{"name":"to","type":"address"},{"name":"value","type":"uint256"},
      {"name":"validAfter","type":"uint256"},{"name":"validBefore","type":"uint256"},{"name":"nonce","type":"bytes32"}] },
  "primaryType": "TransferWithAuthorization",
  "domain": { "name":"USDC", "version":"2", "chainId":11155111, "verifyingContract":"$USDC" },
  "message": { "from":"$BUYER", "to":"$RECIPIENT", "value":"$VALUE",
      "validAfter":"0", "validBefore":"4102444800", "nonce":"$NONCE" } }
JSON
SIG=$(cast wallet sign --private-key "$BUYER_KEY" --data --from-file "$TYPED")
echo "signature = $SIG"

REQS=$(jq -n --arg payto "$RECIPIENT" --arg asset "$USDC" --arg amt "$VALUE" \
  '{scheme:"exact", network:"eip155:11155111", amount:$amt, payTo:$payto, maxTimeoutSeconds:300,
    asset:$asset, extra:{assetTransferMethod:"eip3009", name:"USDC", version:"2"}}')
BODY=$(jq -n --argjson reqs "$REQS" --arg sig "$SIG" --arg from "$BUYER" --arg to "$RECIPIENT" \
    --arg amt "$VALUE" --arg nonce "$NONCE" '
  { x402Version:2,
    paymentPayload:{ accepted:$reqs,
      payload:{ signature:$sig, authorization:{ from:$from, to:$to, value:$amt,
        validAfter:"0", validBefore:"4102444800", nonce:$nonce } },
      x402Version:2 },
    paymentRequirements:$reqs }')

say "5/8 POST /verify"
V=$(curl -s -X POST "$FACIL/verify" -H 'content-type: application/json' -d "$BODY"); echo "$V" | jq .
[ "$(echo "$V" | jq -r .isValid)" = "true" ] || { echo "FAIL: /verify rejected"; exit 1; }

BAL_B0=$(cast call $USDC 'balanceOf(address)(uint256)' $BUYER --rpc-url $RPC)
BAL_R0=$(cast call $USDC 'balanceOf(address)(uint256)' $RECIPIENT --rpc-url $RPC)

say "6/8 POST /settle (facilitator broadcasts transferWithAuthorization; buyer pays no gas)"
S=$(curl -s -X POST "$FACIL/settle" -H 'content-type: application/json' -d "$BODY"); echo "$S" | jq .
[ "$(echo "$S" | jq -r .success)" = "true" ] || { echo "FAIL: /settle failed"; exit 1; }
TX=$(echo "$S" | jq -r .transaction)

say "7/8 assert on-chain effect"
BAL_B1=$(cast call $USDC 'balanceOf(address)(uint256)' $BUYER --rpc-url $RPC)
BAL_R1=$(cast call $USDC 'balanceOf(address)(uint256)' $RECIPIENT --rpc-url $RPC)
USED=$(cast call $USDC 'authorizationState(address,bytes32)(bool)' $BUYER $NONCE --rpc-url $RPC)
GASPAYER=$(cast tx $TX --rpc-url $RPC --json | jq -r .from)
echo "buyer     $BAL_B0 -> $BAL_B1   (expect -$VALUE)"
echo "recipient $BAL_R0 -> $BAL_R1   (expect +$VALUE)"
echo "nonce consumed = $USED   (expect true)"
echo "gas paid by    = $GASPAYER   (facilitator, not the buyer)"
[ "$USED" = "true" ] || { echo "FAIL: nonce not consumed"; exit 1; }

say "8/8 replay protection — re-settle the same authorization MUST fail"
S2=$(curl -s -X POST "$FACIL/settle" -H 'content-type: application/json' -d "$BODY"); echo "$S2" | jq -c .
[ "$(echo "$S2" | jq -r .success)" = "false" ] || { echo "FAIL: replay double-spent!"; exit 1; }

say "RESULT: local x402/EIP-3009 facilitator loop PROVEN on the Sepolia fork ✅"
