//! Balances — native ETH + curated ERC-20 holdings, read in a single round-trip via
//! **Multicall3** (`0xcA11…CA11`). One `aggregate3` call batches `getEthBalance` plus a
//! `balanceOf` per bundled token, so a full portfolio refresh is one RPC request.
//! Per-token calls tolerate failure (a quirky token can't break the whole refresh).

use alloy::primitives::{address, Address, U256};
use alloy::providers::{DynProvider, Provider};
use alloy::sol;
use alloy::sol_types::SolCall;

use crate::tokens::tokens_for;

sol! {
    #[sol(rpc)]
    interface IMulticall3 {
        struct Call3 { address target; bool allowFailure; bytes callData; }
        struct Result { bool success; bytes returnData; }
        function aggregate3(Call3[] calldata calls) external payable returns (Result[] memory returnData);
        function getEthBalance(address addr) external view returns (uint256);
    }
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address owner) external view returns (uint256);
    }
}

/// Multicall3 is deployed at the same address on every chain it supports.
const MULTICALL3: Address = address!("0xcA11bde05977b3631167028862bE2a173976CA11");

/// One token holding: enough to render a row, (later) value it, and prefill a swap.
#[derive(Clone, Debug)]
pub struct TokenBalance {
    /// The token's ERC-20 contract address — the GUI needs it to prefill a swap's sell token.
    pub address: Address,
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
    pub raw: U256,
}

/// A point-in-time snapshot of an address's native + listed-ERC-20 balances.
#[derive(Clone, Debug)]
pub struct Portfolio {
    pub address: Address,
    pub native_wei: U256,
    /// Non-zero listed-token holdings only (zero balances are omitted from the UI).
    pub tokens: Vec<TokenBalance>,
}

/// Read the full portfolio for `address` on `chain_id` in one Multicall3 round-trip. The
/// curated ERC-20 set is keyed by chain via [`tokens_for`] (mainnet majors, the Sepolia
/// swap-test set, or empty for an unknown chain — in which case only native ETH is read).
pub async fn fetch_portfolio(
    provider: &DynProvider,
    address: Address,
    chain_id: u64,
) -> anyhow::Result<Portfolio> {
    let listed = tokens_for(chain_id);
    let mc = IMulticall3::new(MULTICALL3, provider);

    let mut calls = Vec::with_capacity(listed.len() + 1);
    // [0] = native ETH balance (read through Multicall3 itself).
    calls.push(IMulticall3::Call3 {
        target: MULTICALL3,
        allowFailure: false,
        callData: IMulticall3::getEthBalanceCall { addr: address }
            .abi_encode()
            .into(),
    });
    // [1..] = balanceOf per listed token, failure-tolerant.
    for t in listed {
        calls.push(IMulticall3::Call3 {
            target: t.address,
            allowFailure: true,
            callData: IERC20::balanceOfCall { owner: address }.abi_encode().into(),
        });
    }

    // If Multicall3 isn't deployed (e.g. a custom RPC pointed at a fork/L2 without it),
    // degrade gracefully to the native balance rather than failing the whole portfolio.
    let results = match mc.aggregate3(calls).call().await {
        Ok(r) if !r.is_empty() => r,
        _ => {
            let native_wei = provider.get_balance(address).await?;
            return Ok(Portfolio {
                address,
                native_wei,
                tokens: Vec::new(),
            });
        }
    };

    // `results` is non-empty here (guarded by the match above); `.first()` avoids raw indexing.
    let native = results
        .first()
        .ok_or_else(|| anyhow::anyhow!("multicall returned no results"))?;
    let native_wei = IMulticall3::getEthBalanceCall::abi_decode_returns(&native.returnData)?;

    let mut tokens = Vec::new();
    for (t, r) in listed.iter().zip(results.iter().skip(1)) {
        if !r.success {
            continue;
        }
        if let Ok(raw) = IERC20::balanceOfCall::abi_decode_returns(&r.returnData) {
            if !raw.is_zero() {
                tokens.push(TokenBalance {
                    address: t.address,
                    symbol: t.symbol,
                    name: t.name,
                    decimals: t.decimals,
                    raw,
                });
            }
        }
    }

    Ok(Portfolio {
        address,
        native_wei,
        tokens,
    })
}

/// Format a raw integer balance into a clean, grouped human string, e.g.
/// `1_934_500_000_000_000_000` @ 18 decimals → `"1.9345"`. Truncates (never rounds
/// up) to `max_frac` fractional digits and strips trailing zeros.
pub fn format_amount(raw: U256, decimals: u8, max_frac: usize) -> String {
    let full =
        alloy::primitives::utils::format_units(raw, decimals).unwrap_or_else(|_| "0".to_string());
    let (int_part, frac_part) = full.split_once('.').unwrap_or((full.as_str(), ""));

    let frac: String = frac_part.chars().take(max_frac).collect();
    let frac = frac.trim_end_matches('0');

    let int_grouped = group_thousands(int_part);
    if frac.is_empty() {
        int_grouped
    } else {
        format!("{int_grouped}.{frac}")
    }
}

/// Insert thousands separators into the integer part of a decimal string.
fn group_thousands(int_part: &str) -> String {
    let digits = int_part.trim_start_matches('-');
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (len - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if int_part.starts_with('-') {
        format!("-{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::SEPOLIA_TOKENS;
    use alloy::sol_types::SolValue;

    /// An ABI-encoded `balanceOf` return decodes back to its `U256`, and a `TokenBalance`
    /// built from a real Sepolia token carries its address + 18 decimals (the test-USDC quirk).
    #[test]
    fn decodes_balance_and_builds_token() {
        // The Sepolia test USDC is an 18-decimals mock (see tokens.rs module note).
        let usdc = SEPOLIA_TOKENS
            .iter()
            .find(|t| t.symbol == "USDC")
            .expect("sepolia USDC present");
        assert_eq!(usdc.decimals, 18, "sepolia test USDC is 18 decimals, not 6");

        // 1.5 USDC at 18 decimals, ABI-encoded the way an ERC-20 balanceOf return is.
        let raw = U256::from(1_500_000_000_000_000_000u64);
        let encoded = raw.abi_encode();
        let decoded =
            IERC20::balanceOfCall::abi_decode_returns(&encoded).expect("decode balanceOf");
        assert_eq!(decoded, raw);

        let tb = TokenBalance {
            address: usdc.address,
            symbol: usdc.symbol,
            name: usdc.name,
            decimals: usdc.decimals,
            raw: decoded,
        };
        assert_eq!(tb.address, usdc.address);
        assert_eq!(format_amount(tb.raw, tb.decimals, 4), "1.5");
    }

    #[test]
    fn formats_and_groups() {
        // 1.9345 ETH
        assert_eq!(
            format_amount(U256::from(1_934_500_000_000_000_000u128), 18, 4),
            "1.9345"
        );
        // 1,200.00 USDC (6 decimals) → trailing zeros stripped
        assert_eq!(format_amount(U256::from(1_200_000_000u64), 6, 2), "1,200");
        // 12,345,678 whole units
        assert_eq!(format_amount(U256::from(12_345_678u64), 0, 2), "12,345,678");
        // dust below display precision truncates to integer part
        assert_eq!(format_amount(U256::from(1u64), 18, 4), "0");
    }
}
