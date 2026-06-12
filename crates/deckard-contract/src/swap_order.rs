//! The swap-order wire type. The agent proposes a `SwapOrder`; the daemon binds the owner
//! to the unlocked wallet, runs [`evaluate_order`](crate::policy::evaluate_order), and — only
//! after a human approves — signs it as a GPv2 EIP-712 `Order`. This crate carries no chain
//! knowledge: the four constant order params and the EIP-712 machinery live in
//! `deckard-core`'s `cow_types`; here the order is just the data that crosses the socket.

use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

/// A market SELL order the agent proposes and the daemon signs as a GPv2 EIP-712 `Order`.
/// The four constant order params (kind="sell", partiallyFillable=false, sell/buyTokenBalance
/// ="erc20") and feeAmount=0 are pinned in deckard-core's cow_types, NOT carried on the wire.
/// `owner` is bound by the daemon to the unlocked wallet (never inferred from a signature).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwapOrder {
    pub chain_id: u64,
    pub owner: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_amount: U256,    // gross sellAmountBeforeFee
    pub buy_amount_min: U256, // quote.buyAmount minus slippage
    pub receiver: Address,
    pub valid_to: u32,  // unix secs; order expiry
    pub app_data: B256, // keccak256 of the canonical app-data doc (the on-chain appData field)
}
