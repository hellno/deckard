//! Balances — native ETH + curated ERC-20 holdings, read in a single round-trip via
//! **Multicall3** (`0xcA11…CA11`). One `aggregate3` call batches `getEthBalance` plus a
//! `balanceOf` per bundled token, so a full portfolio refresh is one RPC request.
//! Per-token calls tolerate failure (a quirky token can't break the whole refresh).

use alloy::primitives::{address, Address, U256};
use alloy::providers::{DynProvider, Provider};
use alloy::sol;
use alloy::sol_types::SolCall;

use crate::tokens::DEFAULT_TOKENS;

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

/// One token holding: enough to render a row and (later) value it.
#[derive(Clone, Debug)]
pub struct TokenBalance {
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

/// Read the full portfolio for `address` in one Multicall3 round-trip.
pub async fn fetch_portfolio(
    provider: &DynProvider,
    address: Address,
) -> anyhow::Result<Portfolio> {
    let mc = IMulticall3::new(MULTICALL3, provider);

    let mut calls = Vec::with_capacity(DEFAULT_TOKENS.len() + 1);
    // [0] = native ETH balance (read through Multicall3 itself).
    calls.push(IMulticall3::Call3 {
        target: MULTICALL3,
        allowFailure: false,
        callData: IMulticall3::getEthBalanceCall { addr: address }
            .abi_encode()
            .into(),
    });
    // [1..] = balanceOf per listed token, failure-tolerant.
    for t in DEFAULT_TOKENS {
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
    for (t, r) in DEFAULT_TOKENS.iter().zip(results.iter().skip(1)) {
        if !r.success {
            continue;
        }
        if let Ok(raw) = IERC20::balanceOfCall::abi_decode_returns(&r.returnData) {
            if !raw.is_zero() {
                tokens.push(TokenBalance {
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
