//! What the agent wants to do — the ONLY thing that crosses `mcp → daemon` for a write.
//! The agent never sends raw signed bytes, only intent; the daemon decides and signs.

use alloy_primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

/// A proposed write. Carries `chain_id` (multi-chain ready); the daemon owns the nonce
/// and assigns it at sign time — there is deliberately no nonce field here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    /// EVM chain the daemon must sign for. The agent picks the chain; the daemon picks
    /// the nonce.
    pub chain_id: u64,
    /// Target: recipient, token contract, or Railgun adapter, depending on `kind`.
    pub to: Address,
    /// `None` = native ETH; `Some` = an ERC-20 contract.
    pub token: Option<Address>,
    /// Wei (native) or token base units.
    pub value: U256,
    /// Empty for a plain send; the encoded call otherwise.
    pub calldata: Bytes,
    /// The discriminator the policy gate switches on.
    pub kind: IntentKind,
}

/// The class of write. The Railgun deposit/withdraw calldata for `Shield`/`Unshield`
/// rides in [`Intent::calldata`] (owned by `docs/build/10-kohaku-shield.md`); this enum
/// is purely the discriminator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum IntentKind {
    /// Plain transfer (native or ERC-20). Calldata is empty for native sends.
    Send,
    /// Railgun deposit — the demo hero. Calldata carries the adapter call.
    Shield,
    /// Railgun withdraw back to a public balance.
    Unshield,
    /// Generic contract write (forward-compat for plugins). Calldata is the call.
    ContractCall,
}
