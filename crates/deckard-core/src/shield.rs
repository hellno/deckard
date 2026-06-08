//! Key-less Railgun **shield**-calldata builder.
//!
//! A SHIELD (depositing public ETH into a Railgun `0zk` private balance) is **key-less**:
//! it needs only the recipient [`RailgunAddress`], the chain, and the value — never the
//! spending key. And — the de-risked hero finding — a shield does **NO client-side ZK
//! proof**: [`ShieldBuilder::build`] only encrypts the note (`encrypt_shield`) and
//! `abi_encode`s the calldata; the on-chain contract verifies the commitment and deducts
//! the 25-bps fee. So this builder is pure, synchronous, and instant.
//!
//! It builds the calldata and wraps it as an [`Intent`] with [`IntentKind::Shield`]; the
//! daemon (which never sees this heavy ZK crate) just signs + broadcasts the handed
//! `{to, value, calldata}`. That split is deliberate: the heavy `railgun` dep + any sync
//! stays OUT of the key-holding daemon.
//!
//! Gated behind the default-on `shield` Cargo feature so the heavy ZK `railgun` tree is
//! toggleable. When the feature is off, [`build_shield_native_intent`] is replaced by a
//! stub (declared in `lib.rs`) that returns a clear "shield unavailable (feature off)"
//! error — never a fake success.

use alloy_primitives::U256;
use anyhow::{anyhow, ensure};

use deckard_contract::{Intent, IntentKind};

// Re-exported from `lib.rs` (gated) so the daemon's test can name the recipient type
// without taking a direct `railgun` dependency.
pub use railgun::account::address::RailgunAddress;

/// Build the key-less Railgun native-shield calldata and wrap it as an
/// `Intent { kind: Shield, .. }`.
///
/// Key-less: shielding native ETH to `recipient` (a `0zk…` [`RailgunAddress`]) needs only
/// the recipient, the chain config, and the value — never the spending key. The on-chain
/// 25-bps (0.25%) shield fee is deducted by the contract; the calldata carries the full
/// pre-fee `value`, so the synced private balance reads `value - value*25/10000`.
///
/// `value` is wei. For a *native* shield the builder always produces **exactly one**
/// `TxData` (a single RelayAdapt `wrapBase + shield` multicall), so the 1-intent:1-tx model
/// holds; we assert that invariant rather than silently dropping a tx.
pub fn build_shield_native_intent(
    chain_id: u64,
    recipient: RailgunAddress,
    value: U256,
) -> anyhow::Result<Intent> {
    let chain = railgun::chain_config::ChainConfig::from_chain_id(chain_id)
        .ok_or_else(|| anyhow!("shield: unsupported chain_id {chain_id}"))?;

    // The note preimage carries a u128 value; reject anything that can't fit (a shield of
    // > ~3.4e20 ETH is not a real case, but never silently truncate).
    let value_u128: u128 = value
        .try_into()
        .map_err(|_| anyhow!("shield: value exceeds u128"))?;

    // Pure + synchronous: no provider, no sync, no key. `build` only does symmetric note
    // encryption + ABI-encode (no ZK proof). NOTE: railgun pins `rand` 0.9; deckard-core's
    // own `rand` is 0.8, so this rng comes from the 0.9 crate aliased as `rand_09` in
    // Cargo.toml to satisfy the `R: rand::Rng` (0.9) bound on `build`.
    let mut txs = railgun::transact::ShieldBuilder::new(chain)
        .shield_native(recipient, value_u128)
        .build(&mut rand_09::rng())
        .map_err(|e| anyhow!("shield build: {e}"))?;

    ensure!(
        txs.len() == 1,
        "shield_native produced {} txs, expected exactly 1",
        txs.len()
    );
    // `ensure!` above guarantees exactly one tx, so this never errors — propagate rather than
    // panic (deckard-core forbids unwrap/expect/panic in non-test code).
    let tx = txs
        .pop()
        .ok_or_else(|| anyhow!("shield: tx set empty after length check"))?;

    Ok(Intent {
        chain_id,
        to: tx.to,         // RelayAdapt contract
        token: None,       // native shield; the value rides as msg.value
        value: tx.value,   // == the gross native total (wei); contract deducts the fee
        calldata: tx.data, // RelayAdapt.multicall(wrapBase + shield)
        kind: IntentKind::Shield,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh ephemeral 0zk recipient → a well-formed Shield intent (key-less, no network):
    /// exactly one tx, native (token None), non-empty calldata, value preserved (gross).
    #[test]
    fn builds_a_single_native_shield_intent() {
        let chain = railgun::chain_config::ChainConfig::sepolia();
        let acct = railgun::account::signer::PrivateKeySigner::new_evm(
            rand_09::random(),
            rand_09::random(),
            chain.id,
        );
        use railgun::account::signer::RailgunSigner;
        let recipient = acct.address();

        let value = U256::from(1_000_000u64);
        let intent = build_shield_native_intent(chain.id, recipient, value).expect("build");

        assert_eq!(intent.kind, IntentKind::Shield);
        assert_eq!(intent.token, None, "native shield carries no token");
        assert_eq!(
            intent.value, value,
            "calldata carries the GROSS (pre-fee) value"
        );
        assert!(
            !intent.calldata.is_empty(),
            "shield calldata must be present"
        );
        assert_eq!(
            intent.to, chain.relay_adapt_contract,
            "native shield targets the RelayAdapt contract"
        );
    }

    /// An unsupported chain id is a clear error, not a panic.
    #[test]
    fn unsupported_chain_errors() {
        let chain = railgun::chain_config::ChainConfig::sepolia();
        let acct = railgun::account::signer::PrivateKeySigner::new_evm(
            rand_09::random(),
            rand_09::random(),
            chain.id,
        );
        use railgun::account::signer::RailgunSigner;
        let err = build_shield_native_intent(424242, acct.address(), U256::from(1u64))
            .expect_err("unsupported chain must error");
        assert!(err.to_string().contains("unsupported chain_id"));
    }
}
