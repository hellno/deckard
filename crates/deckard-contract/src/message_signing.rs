//! Message-signing wire types for clear-signing v2.
//!
//! These are reviewed payloads, not browser JSON-RPC frames. A future EIP-1193 bridge
//! parses `personal_sign` / `eth_signTypedData_v4` into this bounded model before the
//! daemon proposes it. The daemon signs only a stored, approved payload.

use alloy_primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

/// A human-reviewed off-chain signature request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignMessage {
    /// The active signer chain. For typed data this must match `domain_chain_id` when present.
    pub chain_id: u64,
    /// Unverified requester label/origin. Display-only; never a trust root.
    pub origin: String,
    pub kind: SignMessageKind,
}

/// The message family. `EthSign` and `Authorization7702` are explicit so the policy can refuse
/// them with stable reasons instead of letting them masquerade as harmless bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignMessageKind {
    /// EIP-191 personal message bytes (`personal_sign`).
    PersonalSign { message: Bytes },
    /// EIP-712 typed-data review payload (`eth_signTypedData_v4`) after bounded parsing.
    TypedDataV4(TypedDataReview),
    /// Raw hash signing (`eth_sign`) — always refused.
    EthSign { digest: B256 },
    /// EIP-7702 authorization — always refused until a dedicated allow path exists.
    Authorization7702 { delegate: Address, nonce: u64 },
}

/// Minimal reviewed EIP-712 surface. The full typed-data JSON remains outside the daemon wire;
/// the parser/resolver feeds the digest and the human-facing domain/type facts here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedDataReview {
    pub domain_name: Option<String>,
    pub domain_version: Option<String>,
    pub domain_chain_id: Option<u64>,
    pub verifying_contract: Option<Address>,
    pub primary_type: String,
    /// Final EIP-712 digest (`"\x19\x01" || domainSeparator || hashStruct(message)`).
    pub digest: B256,
    /// Display warnings derived by the bounded parser / descriptor resolver.
    #[serde(default)]
    pub risks: Vec<MessageSigningRisk>,
    /// Structured Permit/EIP-2612-style review facts, when the typed-data shape is recognized.
    #[serde(default)]
    pub permit: Option<Box<PermitReview>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermitReview {
    pub owner: Address,
    pub spender: Address,
    pub value: U256,
    pub deadline: U256,
}

/// Warnings the clear-signing card can render without trusting arbitrary descriptor text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageSigningRisk {
    PermitLike,
    UnlimitedAllowance,
    LongDeadline,
    OwnershipChange,
    SeaportOrder,
    UnknownVerifyingContract,
    DescriptorMissing,
    DescriptorInvalid,
}

impl SignMessage {
    #[must_use]
    pub fn signing_digest(&self) -> Option<B256> {
        match &self.kind {
            SignMessageKind::PersonalSign { .. } => None,
            SignMessageKind::TypedDataV4(review) => Some(review.digest),
            SignMessageKind::EthSign { digest } => Some(*digest),
            SignMessageKind::Authorization7702 { .. } => None,
        }
    }
}
