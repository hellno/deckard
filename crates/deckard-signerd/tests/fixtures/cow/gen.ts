// Known-answer-test generator for the GPv2 (CoW Protocol) `Order` EIP-712 digest.
//
// DEV TOOLING ONLY — NOT run by `cargo test` / CI (no npm in the build). It produces the
// committed `vectors.json` that `tests/cow_kat.rs` consumes. Re-run after changing the order
// schema:  cd crates/deckard-signerd/tests/fixtures/cow && npm i && npx tsx gen.ts
//
// Why this is a valid oracle: the digest is computed by **ethers v6 `TypedDataEncoder`** — a
// completely independent EIP-712 implementation (ethers, not our alloy `sol!`). When the
// `@cowprotocol/contracts` reference package is installed it is ALSO used and the two are
// asserted byte-equal, so a fixture is only emitted when two independent encoders agree. The
// Rust side (`deckard_core::order_digest`) must then reproduce these bytes — three independent
// implementations agreeing is the known-answer guarantee.

import { ethers } from "ethers";
import { writeFileSync } from "node:fs";

const SETTLEMENT = "0x9008D19f58AAbD9eD0D60971565AA8510560ab41"; // mainnet == sepolia
const APP_DATA = "0xb48d38f93eaa084033fc5970bf96e559c33c4cdc07d889ab00b4d63f9590739d"; // keccak256("{}")
const U256_MAX = (1n << 256n) - 1n;
const U32_MAX = (1n << 32n) - 1n;

// The GPv2 `Order` EIP-712 struct. kind / sellTokenBalance / buyTokenBalance are `string`
// (hashed as keccak256(utf8) by EIP-712), NOT enums — this is the load-bearing detail.
const ORDER_TYPE = {
  Order: [
    { name: "sellToken", type: "address" },
    { name: "buyToken", type: "address" },
    { name: "receiver", type: "address" },
    { name: "sellAmount", type: "uint256" },
    { name: "buyAmount", type: "uint256" },
    { name: "validTo", type: "uint32" },
    { name: "appData", type: "bytes32" },
    { name: "feeAmount", type: "uint256" },
    { name: "kind", type: "string" },
    { name: "partiallyFillable", type: "bool" },
    { name: "sellTokenBalance", type: "string" },
    { name: "buyTokenBalance", type: "string" },
  ],
};

// Tokens (mainnet majors + Sepolia test set) — only used as plausible addresses in the
// vectors; the digest does not care whether they are "real", only that bytes round-trip.
const WETH_MAINNET = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
const USDC_MAINNET = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const WETH_SEPOLIA = "0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14";
const COW_SEPOLIA = "0x0625aFB445C3B6B7B929342a04A22599fd5dBB59";
const OWNER = "0x1111111111111111111111111111111111111111";

type Order = {
  sellToken: string;
  buyToken: string;
  receiver: string;
  sellAmount: bigint;
  buyAmount: bigint;
  validTo: number;
  appData: string;
  feeAmount: bigint;
  kind: string;
  partiallyFillable: boolean;
  sellTokenBalance: string;
  buyTokenBalance: string;
};

function order(partial: Partial<Order>): Order {
  return {
    sellToken: WETH_MAINNET,
    buyToken: USDC_MAINNET,
    receiver: OWNER,
    sellAmount: 1_000_000_000_000_000_000n, // 1e18
    buyAmount: 2_500_000_000n,
    validTo: 1_893_456_000, // fixed (2030-01-01) for determinism
    appData: APP_DATA,
    feeAmount: 0n,
    kind: "sell",
    partiallyFillable: false,
    sellTokenBalance: "erc20",
    buyTokenBalance: "erc20",
    ...partial,
  };
}

function domainFor(chainId: number) {
  return { name: "Gnosis Protocol", version: "v2", chainId, verifyingContract: SETTLEMENT };
}

// uid = digest(32) || owner(20) || validTo(4 BE)
function orderUid(digest: string, owner: string, validTo: number): string {
  return ethers.concat([digest, owner, ethers.toBeHex(BigInt(validTo), 4)]);
}

// Optional cross-check against CoW's own reference implementation, if installed.
let cowHashOrder:
  | ((domain: any, o: any) => string)
  | undefined;
let cowDomain: ((chainId: number, settlement: string) => any) | undefined;
try {
  // @cowprotocol/contracts exposes `domain(chainId, verifyingContract)` and `hashOrder`.
  const cc = await import("@cowprotocol/contracts");
  cowHashOrder = cc.hashOrder as any;
  cowDomain = cc.domain as any;
  console.log("cross-check: @cowprotocol/contracts present — asserting agreement");
} catch {
  console.log("cross-check: @cowprotocol/contracts not installed — ethers-only oracle");
}

const cases: { name: string; chain_id: number; order: Order }[] = [
  { name: "typical_mainnet", chain_id: 1, order: order({}) },
  { name: "typical_sepolia", chain_id: 11155111, order: order({ sellToken: WETH_SEPOLIA, buyToken: COW_SEPOLIA }) },
  { name: "max_u256_mainnet", chain_id: 1, order: order({ sellAmount: U256_MAX, buyAmount: U256_MAX }) },
  { name: "max_u256_sepolia", chain_id: 11155111, order: order({ sellToken: WETH_SEPOLIA, buyToken: COW_SEPOLIA, sellAmount: U256_MAX, buyAmount: U256_MAX }) },
  { name: "validto_zero_mainnet", chain_id: 1, order: order({ validTo: 0 }) },
  { name: "validto_max_mainnet", chain_id: 1, order: order({ validTo: Number(U32_MAX) }) },
  { name: "validto_max_sepolia", chain_id: 11155111, order: order({ sellToken: WETH_SEPOLIA, buyToken: COW_SEPOLIA, validTo: Number(U32_MAX) }) },
];

const vectors = cases.map(({ name, chain_id, order }) => {
  const dom = domainFor(chain_id);
  const digest = ethers.TypedDataEncoder.hash(dom, ORDER_TYPE, order);

  if (cowHashOrder && cowDomain) {
    const ref = cowHashOrder(cowDomain(chain_id, SETTLEMENT), {
      ...order,
      kind: order.kind,
      sellTokenBalance: order.sellTokenBalance,
      buyTokenBalance: order.buyTokenBalance,
    });
    if (ref.toLowerCase() !== digest.toLowerCase()) {
      throw new Error(`${name}: cow-sdk digest ${ref} != ethers digest ${digest}`);
    }
  }

  return {
    name,
    chain_id,
    settlement: SETTLEMENT,
    owner: OWNER,
    order: {
      sell_token: order.sellToken,
      buy_token: order.buyToken,
      receiver: order.receiver,
      sell_amount: order.sellAmount.toString(),
      buy_amount_min: order.buyAmount.toString(),
      valid_to: order.validTo,
      app_data: order.appData,
    },
    expected_digest: digest,
    expected_uid: orderUid(digest, OWNER, order.validTo),
  };
});

writeFileSync(
  new URL("vectors.json", import.meta.url),
  JSON.stringify({ note: "GENERATED by gen.ts (ethers v6 + optional @cowprotocol/contracts). Do not edit by hand.", vectors }, null, 2) + "\n",
);
console.log(`wrote ${vectors.length} vectors to vectors.json`);
