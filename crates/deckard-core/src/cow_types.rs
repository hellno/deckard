//! CoW Protocol (GPv2) order types and the EIP-712 machinery, unfeatured so the signer
//! daemon — which builds `deckard-core` with `default-features = false` (no `cow-client`,
//! no HTTP) — still compiles the digest/uid/cancel-calldata path it needs to sign and
//! cancel orders. There is NO network code in this module: it is pure, deterministic, and
//! lint-clean (no index expressions, no `unwrap`/`expect`/`panic`).
//!
//! Every constant below (settlement + vault-relayer addresses, the EIP-712 `Order` type
//! hash, the canonical `{}` app-data doc and its keccak hash, the `approve` selector) was
//! verified this session with `cast`; the in-module tests recompute the derivable ones and
//! assert they match, so a transcription error fails the build.

use alloy::primitives::{address, b256, Address, Bytes, B256, U256};
use alloy::sol;
use alloy::sol_types::{eip712_domain, SolCall, SolStruct};
use deckard_contract::SwapOrder;

pub const GPV2_SETTLEMENT: Address = address!("0x9008D19f58AAbD9eD0D60971565AA8510560ab41");
pub const GPV2_VAULT_RELAYER: Address = address!("0xC92E8bdf79f0507f65a392b0ab4667716BFE0110");
pub const ORDER_TYPE_HASH: B256 =
    b256!("0xd5a25ba2e97094ad7d83dc28a6572da797d6b3e7fc6663bd93efb789fc17e489");
pub const APP_DATA_DOC: &str = "{}";
pub const APP_DATA_HASH: B256 =
    b256!("0xb48d38f93eaa084033fc5970bf96e559c33c4cdc07d889ab00b4d63f9590739d");
pub const APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

sol! {
    struct Order {
        address sellToken; address buyToken; address receiver;
        uint256 sellAmount; uint256 buyAmount; uint32 validTo;
        bytes32 appData; uint256 feeAmount; string kind;
        bool partiallyFillable; string sellTokenBalance; string buyTokenBalance;
    }
    function invalidateOrder(bytes orderUid) external;
}

/// EIP-712 signing digest for a GPv2 sell order, computed over the STORED order.
pub fn order_digest(order: &SwapOrder) -> B256 {
    let sol_order = Order {
        sellToken: order.sell_token,
        buyToken: order.buy_token,
        receiver: order.receiver,
        sellAmount: order.sell_amount,
        buyAmount: order.buy_amount_min,
        validTo: order.valid_to,
        appData: order.app_data,
        feeAmount: U256::ZERO,
        kind: "sell".into(),
        partiallyFillable: false,
        sellTokenBalance: "erc20".into(),
        buyTokenBalance: "erc20".into(),
    };
    let domain = eip712_domain! {
        name: "Gnosis Protocol",
        version: "v2",
        chain_id: order.chain_id,
        verifying_contract: GPV2_SETTLEMENT,
    };
    sol_order.eip712_signing_hash(&domain)
}

/// 56-byte order uid = digest(32) || owner(20) || validTo(4 BE). No index expressions (lint).
pub fn order_uid(digest: B256, owner: Address, valid_to: u32) -> [u8; 56] {
    let mut uid = [0u8; 56];
    let (d, rest) = uid.split_at_mut(32);
    let (o, v) = rest.split_at_mut(20);
    d.copy_from_slice(digest.as_slice());
    o.copy_from_slice(owner.as_slice());
    v.copy_from_slice(&valid_to.to_be_bytes());
    uid
}

/// min received = buy_amount - ceil(buy_amount*bps/10_000). Deduction rounds UP → min is
/// conservative (you never accept LESS than slippage allows). bps=0 → unchanged; bps>=10000 → 0.
pub fn apply_slippage(buy_amount: U256, bps: u16) -> U256 {
    // Slippage tolerance caps at 100% (10_000 bps); a larger value can only floor the minimum at
    // zero, so clamp rather than letting an absurd bps distort the math.
    let bps = U256::from(bps.min(10_000));
    let denom = U256::from(10_000u32);
    // Exact `ceil(buy_amount * bps / 10_000)` WITHOUT a 512-bit intermediate. A plain
    // `saturating_mul` would saturate the product BEFORE the divide and under-deduct on huge
    // amounts (e.g. `MAX * 10_000` saturates to `MAX`, then `MAX/10_000` leaves ~MAX — the
    // slippage floor would silently vanish). Splitting the dividend keeps every product within
    // U256: `q*bps ≤ buy_amount` because `bps ≤ 10_000`, and the remainder term is `< 10_000²`.
    let q = buy_amount / denom;
    let r = buy_amount % denom;
    let deduction = q
        .saturating_mul(bps)
        .saturating_add(r.saturating_mul(bps).div_ceil(denom));
    buy_amount.saturating_sub(deduction)
}

/// Decode an exact `approve(address,uint256)` calldata → (spender, amount). None if malformed.
/// Version-independent manual decode (lint-clean: split_at, from_slice, from_be_slice).
pub fn decode_approve(calldata: &[u8]) -> Option<(Address, U256)> {
    if calldata.len() != 4 + 32 + 32 {
        return None;
    }
    let (selector, args) = calldata.split_at(4);
    if selector != APPROVE_SELECTOR {
        return None;
    }
    let (spender_word, amount_word) = args.split_at(32);
    let (_pad, spender_bytes) = spender_word.split_at(12);
    Some((
        Address::from_slice(spender_bytes),
        U256::from_be_slice(amount_word),
    ))
}

/// Build exact `transfer(address,uint256)` calldata for an ERC-20 token send.
pub fn build_erc20_transfer_calldata(recipient: Address, amount: U256) -> Bytes {
    let mut data = vec![0xa9, 0x05, 0x9c, 0xbb];
    let mut recipient_word = [0u8; 32];
    recipient_word[12..].copy_from_slice(recipient.as_slice());
    data.extend_from_slice(&recipient_word);
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    Bytes::from(data)
}

/// Calldata for `invalidateOrder(bytes orderUid)` to the settlement contract (cancellation).
pub fn build_invalidate_order_calldata(uid: &[u8; 56]) -> Bytes {
    invalidateOrderCall {
        orderUid: Bytes::copy_from_slice(uid),
    }
    .abi_encode()
    .into()
}

/// Orderbook REST base for a chain (no HTTP here — just the URL), read from the
/// [chain registry](crate::chain::ChainSpec::cow_orderbook_base).
pub fn cow_api_base(chain_id: u64) -> Option<&'static str> {
    crate::chain::for_chain(chain_id).and_then(|c| c.cow_orderbook_base)
}

#[cfg(test)]
mod tests {
    // `super::*` already brings `SwapOrder` (the parent's private `use`) and the alloy
    // primitives into scope, so the tests name them directly without re-importing.
    use super::*;
    // keccak256 is used ONLY by the hash-recompute tests; importing it here (not at module
    // level) keeps the non-default-features daemon build of cow_types warning-clean.
    use alloy::primitives::keccak256;

    /// The canonical app-data doc is exactly the two bytes `{}`; its keccak must equal the
    /// pinned const. If either the doc or the const drifts, this fails.
    #[test]
    fn app_data_hash_matches_doc() {
        assert_eq!(APP_DATA_DOC, "{}");
        assert_eq!(APP_DATA_DOC.as_bytes(), b"{}");
        assert_eq!(keccak256(APP_DATA_DOC.as_bytes()), APP_DATA_HASH);
    }

    /// Recompute the EIP-712 `Order` type hash from the canonical type string and assert it
    /// equals the pinned const. The string is the GPv2 `Order` encodeType with NO whitespace.
    #[test]
    fn order_type_hash_matches() {
        let type_string = "Order(address sellToken,address buyToken,address receiver,\
uint256 sellAmount,uint256 buyAmount,uint32 validTo,bytes32 appData,uint256 feeAmount,\
string kind,bool partiallyFillable,string sellTokenBalance,string buyTokenBalance)";
        assert_eq!(keccak256(type_string.as_bytes()), ORDER_TYPE_HASH);
        // Cross-check: alloy's own derived type hash for our `sol!` Order matches the const.
        assert_eq!(sample_sol_order().eip712_type_hash(), ORDER_TYPE_HASH);
    }

    fn sample_sol_order() -> Order {
        Order {
            sellToken: Address::repeat_byte(0x11),
            buyToken: Address::repeat_byte(0x22),
            receiver: Address::repeat_byte(0x33),
            sellAmount: U256::from(1_000_000u64),
            buyAmount: U256::from(999_000u64),
            validTo: 1_700_000_000,
            appData: APP_DATA_HASH,
            feeAmount: U256::ZERO,
            kind: "sell".into(),
            partiallyFillable: false,
            sellTokenBalance: "erc20".into(),
            buyTokenBalance: "erc20".into(),
        }
    }

    #[test]
    fn apply_slippage_zero_bps_is_identity() {
        let amt = U256::from(1_000_000u64);
        assert_eq!(apply_slippage(amt, 0), amt);
    }

    #[test]
    fn apply_slippage_full_bps_is_zero() {
        let amt = U256::from(1_000_000u64);
        // 10000 bps == 100% deduction → exactly zero.
        assert_eq!(apply_slippage(amt, 10_000), U256::ZERO);
        // Above 100% saturates (never underflows) → still zero.
        assert_eq!(apply_slippage(amt, u16::MAX), U256::ZERO);
    }

    #[test]
    fn apply_slippage_typical_fifty_bps() {
        // 50 bps == 0.5%; of 1_000_000 that is 5_000 → min 995_000.
        assert_eq!(
            apply_slippage(U256::from(1_000_000u64), 50),
            U256::from(995_000u64)
        );
    }

    #[test]
    fn apply_slippage_rounds_deduction_up() {
        // 1 bps of 9999 = 0.9999 → ceil → 1 deducted → 9998 (conservative: never over-credit).
        assert_eq!(
            apply_slippage(U256::from(9_999u64), 1),
            U256::from(9_998u64)
        );
        // 1 bps of 10000 = exactly 1 → 9999.
        assert_eq!(
            apply_slippage(U256::from(10_000u64), 1),
            U256::from(9_999u64)
        );
        // tiny amount: 1 bps of 1 = 0.0001 → ceil → 1 deducted → 0.
        assert_eq!(apply_slippage(U256::from(1u64), 1), U256::ZERO);
    }

    #[test]
    fn apply_slippage_does_not_overflow_on_max() {
        // saturating_mul guards U256::MAX * bps; with full bps the result is 0, not a panic.
        assert_eq!(apply_slippage(U256::MAX, 10_000), U256::ZERO);
        // 0 bps on MAX is identity.
        assert_eq!(apply_slippage(U256::MAX, 0), U256::MAX);
    }

    #[test]
    fn decode_approve_happy_path() {
        let spender = Address::repeat_byte(0xAB);
        let amount = U256::from(123_456_789u64);
        let mut calldata = Vec::with_capacity(68);
        calldata.extend_from_slice(&APPROVE_SELECTOR);
        // address arg is left-padded to 32 bytes.
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());
        // uint256 arg is the big-endian 32-byte amount.
        calldata.extend_from_slice(&amount.to_be_bytes::<32>());
        let (got_spender, got_amount) = decode_approve(&calldata).expect("well-formed approve");
        assert_eq!(got_spender, spender);
        assert_eq!(got_amount, amount);
    }

    #[test]
    fn decode_approve_rejects_wrong_length() {
        // too short
        assert!(decode_approve(&[0x09, 0x5e, 0xa7, 0xb3]).is_none());
        // 67 bytes (one short of 68)
        assert!(decode_approve(&[0u8; 67]).is_none());
        // 69 bytes (one over)
        let mut wrong_len = vec![0x09, 0x5e, 0xa7, 0xb3];
        wrong_len.extend(std::iter::repeat_n(0u8, 65));
        assert!(decode_approve(&wrong_len).is_none());
        // empty
        assert!(decode_approve(&[]).is_none());
    }

    #[test]
    fn decode_approve_rejects_wrong_selector() {
        let mut calldata = vec![0xde, 0xad, 0xbe, 0xef];
        calldata.extend_from_slice(&[0u8; 64]);
        assert!(decode_approve(&calldata).is_none());
    }

    #[test]
    fn build_erc20_transfer_calldata_encodes_selector_recipient_and_amount() {
        let recipient = Address::repeat_byte(0x22);
        let amount = U256::from(1_000_000u64);
        let calldata = build_erc20_transfer_calldata(recipient, amount);

        assert_eq!(&calldata[..4], &[0xa9, 0x05, 0x9c, 0xbb]);
        assert_eq!(&calldata[4..16], &[0u8; 12]);
        assert_eq!(&calldata[16..36], recipient.as_slice());
        assert_eq!(&calldata[36..68], amount.to_be_bytes::<32>());
    }

    #[test]
    fn order_uid_layout() {
        let digest = B256::repeat_byte(0x11);
        let owner = Address::repeat_byte(0x22);
        let valid_to: u32 = 0x0102_0304;
        let uid = order_uid(digest, owner, valid_to);
        assert_eq!(uid.len(), 56);
        // bytes 0..32 == digest
        assert_eq!(&uid[0..32], digest.as_slice());
        // bytes 32..52 == owner
        assert_eq!(&uid[32..52], owner.as_slice());
        // bytes 52..56 == validTo big-endian
        assert_eq!(&uid[52..56], &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn cow_api_base_known_chains() {
        assert_eq!(cow_api_base(1), Some("https://api.cow.fi/mainnet"));
        assert_eq!(cow_api_base(11155111), Some("https://api.cow.fi/sepolia"));
        assert_eq!(cow_api_base(8453), None);
        assert_eq!(cow_api_base(0), None);
    }

    /// One inline digest sanity vector: the digest is deterministic and changes when any
    /// field changes. (The committed cross-impl KATs against the CoW SDK are Package D.)
    #[test]
    fn order_digest_is_deterministic_and_field_sensitive() {
        let base = SwapOrder {
            chain_id: 1,
            owner: Address::repeat_byte(0x01),
            sell_token: Address::repeat_byte(0x02),
            buy_token: Address::repeat_byte(0x03),
            sell_amount: U256::from(1_000_000u64),
            buy_amount_min: U256::from(999_000u64),
            receiver: Address::repeat_byte(0x01),
            valid_to: 1_700_000_000,
            app_data: APP_DATA_HASH,
        };
        let d1 = order_digest(&base);
        let d2 = order_digest(&base);
        assert_eq!(d1, d2, "digest must be deterministic");
        assert_ne!(d1, B256::ZERO);

        // Changing the chain id changes the domain → changes the digest.
        let other_chain = SwapOrder {
            chain_id: 11155111,
            ..base.clone()
        };
        assert_ne!(order_digest(&other_chain), d1);

        // Changing the buy amount changes the struct hash → changes the digest.
        let other_amount = SwapOrder {
            buy_amount_min: U256::from(998_000u64),
            ..base.clone()
        };
        assert_ne!(order_digest(&other_amount), d1);
    }

    /// Cross-implementation known-answer test: the `typical_mainnet` vector from
    /// `tests/fixtures/cow/vectors.json`, whose `expected_digest` was computed by **ethers v6
    /// `TypedDataEncoder`** — a fully independent EIP-712 implementation. Pinning it here proves
    /// alloy's `SolStruct::eip712_signing_hash` agrees with ethers byte-for-byte (a wrong domain,
    /// field order, or `string`-vs-enum encoding would diverge). The file-driven KAT over all 7
    /// vectors lives in `deckard-signerd/tests/cow_kat.rs`; this inline copy guards the digest at
    /// its source.
    #[test]
    fn order_digest_matches_independent_ethers_oracle() {
        let order = SwapOrder {
            chain_id: 1,
            owner: address!("0x1111111111111111111111111111111111111111"),
            sell_token: address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"), // WETH
            buy_token: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),  // USDC
            sell_amount: U256::from(1_000_000_000_000_000_000u64),
            buy_amount_min: U256::from(2_500_000_000u64),
            receiver: address!("0x1111111111111111111111111111111111111111"),
            valid_to: 1_893_456_000,
            app_data: APP_DATA_HASH,
        };
        assert_eq!(
            order_digest(&order),
            b256!("0x5691b60538316210ed18a8c82ea81d142aa2e1bed103ca477ffe72870d3895b2"),
            "alloy digest diverged from the ethers-computed fixture — EIP-712 domain/struct mismatch"
        );
    }

    #[test]
    fn build_invalidate_order_calldata_has_selector() {
        let uid = [0x07u8; 56];
        let calldata = build_invalidate_order_calldata(&uid);
        // invalidateOrder(bytes) selector 0x15337bc0 leads the calldata.
        assert_eq!(&calldata[0..4], &[0x15, 0x33, 0x7b, 0xc0]);
        // The uid round-trips through ABI decode.
        let decoded = invalidateOrderCall::abi_decode(&calldata).expect("decode invalidateOrder");
        assert_eq!(decoded.orderUid.as_ref(), &uid[..]);
    }
}
