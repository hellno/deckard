//! A curated, bundled token list — a subset of the Uniswap default list (mainnet
//! majors). No third-party portfolio API: balances are read on-chain via Multicall3
//! against exactly these addresses. The honest tradeoff (per spec) is that long-tail
//! / airdropped tokens outside this list won't show — surfaced in the UI as a caveat.
//!
//! `address!` validates each EIP-55 checksum at compile time, so a transcription typo
//! fails the build rather than silently reading the wrong contract.

use alloy::primitives::{address, Address};

/// One bundled token: its mainnet contract, ticker, name, and ERC-20 decimals.
#[derive(Clone, Copy)]
pub struct TokenInfo {
    pub address: Address,
    pub symbol: &'static str,
    pub name: &'static str,
    pub decimals: u8,
}

/// The curated mainnet set. Ordered roughly by how commonly an operator holds them.
pub const DEFAULT_TOKENS: &[TokenInfo] = &[
    TokenInfo {
        address: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        symbol: "USDC",
        name: "USD Coin",
        decimals: 6,
    },
    TokenInfo {
        address: address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
        symbol: "USDT",
        name: "Tether USD",
        decimals: 6,
    },
    TokenInfo {
        address: address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
        symbol: "DAI",
        name: "Dai Stablecoin",
        decimals: 18,
    },
    TokenInfo {
        address: address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"),
        symbol: "WETH",
        name: "Wrapped Ether",
        decimals: 18,
    },
    TokenInfo {
        address: address!("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"),
        symbol: "WBTC",
        name: "Wrapped BTC",
        decimals: 8,
    },
    TokenInfo {
        address: address!("0x514910771AF9Ca656af840dff83E8264EcF986CA"),
        symbol: "LINK",
        name: "Chainlink",
        decimals: 18,
    },
    TokenInfo {
        address: address!("0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984"),
        symbol: "UNI",
        name: "Uniswap",
        decimals: 18,
    },
    TokenInfo {
        address: address!("0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9"),
        symbol: "AAVE",
        name: "Aave",
        decimals: 18,
    },
];
