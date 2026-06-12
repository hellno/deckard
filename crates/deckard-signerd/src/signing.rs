//! The broadcast path: build an **EIP-1559** transaction with alloy's recommended fillers
//! (nonce from the pending count, gas from fee/gas estimation, chain id), sign it, and
//! broadcast it through the configured RPC.
//!
//! ## The signer version bridge
//! `deckard-core`'s keystore yields a signer from `alloy-signer-local` **2.0.5**, but the
//! provider here is built from the `alloy` meta-crate (bundled signer-local **1.8.3**). The
//! two `PrivateKeySigner` types are incompatible — so we never hand one to the other. The
//! caller extracts the raw 32-byte secp256k1 scalar (the version-stable `B256` is the only
//! thing that crosses), and we reconstruct the signer in *this* alloy stack from those
//! bytes. The scalar is held in a `Zeroizing` buffer by the caller.

use alloy::network::{Ethereum, EthereumWallet, TransactionBuilder};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy_primitives::{Address, Bytes, B256, U256};

/// Sign + broadcast a native-ETH send and return the broadcast tx hash.
///
/// Thin wrapper over [`broadcast_intent`] with empty calldata, so the native-send call
/// sites (and their tests) keep byte-identical behaviour: no `input` field is set, the gas
/// filler produces the same type-2 tx as before.
///
/// `scalar` is the raw 32-byte private key (the caller keeps it in `Zeroizing`).
pub async fn broadcast_native_send(
    scalar: &[u8],
    rpc_url: &str,
    chain_id: u64,
    to: Address,
    value_wei: U256,
) -> anyhow::Result<B256> {
    broadcast_intent(scalar, rpc_url, chain_id, to, value_wei, &Bytes::new()).await
}

/// Sign + broadcast an intent's `(to, value, calldata)` and return the broadcast tx hash.
///
/// Generalizes the native send to carry **calldata**, so a Shield / ContractCall intent
/// broadcasts the RelayAdapt (or other) call the key-less builder handed over. The selection
/// is implicit: empty `input` ⇒ a plain native send (identical to the old path); a non-empty
/// `input` ⇒ a contract call. The daemon stays ZK-free — it only signs+broadcasts the bytes.
///
/// `scalar` is the raw 32-byte private key (the caller keeps it in `Zeroizing`).
pub async fn broadcast_intent(
    scalar: &[u8],
    rpc_url: &str,
    chain_id: u64,
    to: Address,
    value_wei: U256,
    input: &Bytes,
) -> anyhow::Result<B256> {
    let signer = PrivateKeySigner::from_slice(scalar)
        .map_err(|e| anyhow::anyhow!("reconstruct signer: {e}"))?;
    let wallet = EthereumWallet::from(signer);

    let url = rpc_url
        .parse()
        .map_err(|e| anyhow::anyhow!("bad RPC URL {rpc_url:?}: {e}"))?;
    // `new()` installs the recommended fillers (nonce/gas/chain-id); `.wallet()` adds signing.
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

    // `to`/`value` (and, for a shield/contract call, `input`) set ⇒ the gas filler produces an
    // EIP-1559 (type-2) tx and fills the fee fields — now estimating gas against the calldata
    // too; the nonce filler uses the pending count; chain id is pinned explicitly.
    //
    // The `TransactionBuilder` methods are disambiguated to alloy's `Ethereum` network:
    // pulling helios-ethereum into the tree (via deckard-core's `verified-reads`) adds a
    // second `TransactionBuilder<helios_ethereum::spec::Ethereum>` impl for
    // `TransactionRequest`, so the chained builder calls would otherwise be ambiguous.
    // Setting via `&mut` with an `Ethereum`-typed binding anchors every call to alloy's impl.
    let mut tx = TransactionRequest::default();
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_to(&mut tx, to);
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_value(&mut tx, value_wei);
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_chain_id(&mut tx, chain_id);
    if !input.is_empty() {
        <TransactionRequest as TransactionBuilder<Ethereum>>::set_input(&mut tx, input.clone());
    }

    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("broadcast: {e}"))?;
    Ok(*pending.tx_hash())
}

/// Sign a 32-byte EIP-712 digest with the reconstructed local signer → a 65-byte
/// `r(32) || s(32) || v(1)` signature with `v` normalized to legacy **27/28**.
///
/// The CoW orderbook (GPv2) requires the legacy `v` encoding, but alloy 1.8's `Signature`
/// exposes the recovery bit as a `bool` y-parity (`.v()`), so we add 27 to get 27/28 — never
/// the EIP-155 form (that's for whole-tx signing, not an EIP-712 order digest).
///
/// Sync + offline: no network, no nonce, no provider. `scalar` is the raw 32-byte secp256k1
/// key; the caller keeps it in `Zeroizing` and it never leaves this function reconstructed.
/// The byte layout is built with `split_at_mut`/`copy_from_slice` (no index expressions, to
/// match the trust-core lint posture even though this crate isn't under that deny list).
pub fn sign_order_digest(scalar: &[u8], digest: B256) -> anyhow::Result<[u8; 65]> {
    let signer = PrivateKeySigner::from_slice(scalar)
        .map_err(|e| anyhow::anyhow!("reconstruct signer: {e}"))?;
    let sig = signer
        .sign_hash_sync(&digest)
        .map_err(|e| anyhow::anyhow!("sign digest: {e}"))?;
    let mut out = [0u8; 65];
    let (r, rest) = out.split_at_mut(32);
    let (s, v) = rest.split_at_mut(32);
    r.copy_from_slice(&sig.r().to_be_bytes::<32>());
    s.copy_from_slice(&sig.s().to_be_bytes::<32>());
    // alloy 1.8 `Signature::v()` is the y-parity (bool: false→0, true→1); legacy v = 27 + parity.
    if let Some(slot) = v.first_mut() {
        *slot = 27u8 + u8::from(sig.v());
    }
    Ok(out)
}

/// Read an address's public (native) balance through `read_url` — key-less, read-only.
///
/// `read_url` is the endpoint the consumer provider reads through. With verified reads
/// on (the default) the daemon passes Helios's **localhost** URL here, so this read is
/// proof-checked; with the feature off it is the raw RPC (and the caller tags the result
/// `Unsynced`). The `with_default_block(latest)` fix is applied uniformly: alloy defaults
/// `eth_call`/`estimateGas` to the `pending` tag, which a Helios light client cannot
/// serve — `get_balance` itself targets `latest`, but layering the default keeps every
/// read path uniform with the eth_call-backed reads.
pub async fn read_balance(read_url: &str, addr: Address) -> anyhow::Result<U256> {
    let url = read_url
        .parse()
        .map_err(|e| anyhow::anyhow!("bad read URL {read_url:?}: {e}"))?;
    let provider = ProviderBuilder::new()
        .with_default_block(alloy::eips::BlockId::latest())
        .connect_http(url);
    provider
        .get_balance(addr)
        .await
        .map_err(|e| anyhow::anyhow!("get_balance: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Signature;

    /// The 65-byte `sign_order_digest` output must round-trip: recovering the address from
    /// `(digest, r||s||v)` returns the signer's own address. This is what proves the `v`
    /// normalization to 27/28 (a wrong recovery byte recovers a different address).
    #[test]
    fn sign_order_digest_recovers_signer_address() {
        // A fixed non-zero scalar (any valid secp256k1 key works for the round-trip).
        let scalar = [0x11u8; 32];
        let signer = PrivateKeySigner::from_slice(&scalar).expect("valid key");
        let expected = signer.address();

        let digest = B256::repeat_byte(0x42);
        let sig_bytes = sign_order_digest(&scalar, digest).expect("sign");

        // v must be legacy 27/28, never 0/1 (that's the bug the test guards against).
        let v = *sig_bytes.last().expect("65 bytes");
        assert!(v == 27 || v == 28, "v should be legacy 27/28, got {v}");

        // Recover via alloy's Signature (`from_raw` normalizes the Electrum-notation v).
        let sig = Signature::from_raw(&sig_bytes).expect("parse 65-byte sig");
        let recovered = sig
            .recover_address_from_prehash(&digest)
            .expect("recover address");
        assert_eq!(recovered, expected, "recovered address must equal signer");
    }
}
