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
use alloy_primitives::{Address, B256, U256};

/// Sign + broadcast a native-ETH send and return the broadcast tx hash.
///
/// `scalar` is the raw 32-byte private key (the caller keeps it in `Zeroizing`). v1 supports
/// native sends only; ERC-20/contract sends are rejected upstream in `propose`.
pub async fn broadcast_native_send(
    scalar: &[u8],
    rpc_url: &str,
    chain_id: u64,
    to: Address,
    value_wei: U256,
) -> anyhow::Result<B256> {
    let signer = PrivateKeySigner::from_slice(scalar)
        .map_err(|e| anyhow::anyhow!("reconstruct signer: {e}"))?;
    let wallet = EthereumWallet::from(signer);

    let url = rpc_url
        .parse()
        .map_err(|e| anyhow::anyhow!("bad RPC URL {rpc_url:?}: {e}"))?;
    // `new()` installs the recommended fillers (nonce/gas/chain-id); `.wallet()` adds signing.
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

    // Only `to`/`value` set ⇒ the gas filler produces an EIP-1559 (type-2) tx and fills the
    // fee fields; the nonce filler uses the pending count; chain id is pinned explicitly.
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

    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("broadcast: {e}"))?;
    Ok(*pending.tx_hash())
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
