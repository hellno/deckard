//! Spike (#206) — Question 1 evidence.
//!
//! Build the SAME EIP-3009 `TransferWithAuthorization` authorization two ways and
//! compare the EIP-712 digest AND the ECDSA signature byte-for-byte:
//!
//!   A. HAND-ROLL  — a local `alloy sol!` struct + `eip712_domain!` + `eip712_signing_hash`.
//!                   This is exactly how `deckard-core/src/cow_types.rs` builds CoW orders today.
//!   B. x402-rs    — `x402_chain_eip155::v1_eip155_exact::types::TransferWithAuthorization`.
//!
//! If A == B byte-for-byte, then depending on `x402-types` buys deckard-core nothing over the
//! `sol!` it already uses — and the printed vector doubles as the KAT #34 gates against.

use alloy_primitives::{address, b256, Address, B256, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol, SolStruct};

// A. HAND-ROLL: the deckard-core way. Identical Solidity struct shape to EIP-3009.
sol! {
    #[allow(non_snake_case)]
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

// B. The x402-rs reference type.
use x402_chain_eip155::v1_eip155_exact::types::TransferWithAuthorization as X402TWA;

fn main() {
    // ---- Fixed KAT inputs (fresh throwaway keys; zero secrets) ----
    // NB: do NOT reuse anvil's canonical test accounts (0x7099.., 0x3C44..) — on real
    // Sepolia those carry EIP-7702 delegation designators (0xef0100..), so USDC's
    // SignatureChecker routes them through EIP-1271 and rejects a plain EOA signature.
    let from: Address = address!("06B92bd300C5Cf9A8ECc64D7B9c51163d6b177a1"); // buyer (dec00d01)
    let to: Address = address!("791668cB0CB50D90DFb1Ee215De4CefA3EAb953e"); // payTo (dec00d02)
    let value = U256::from(1_000_000u64); // 1.000000 USDC (6 decimals)
    let valid_after = U256::from(0u64);
    let valid_before = U256::from(4_102_444_800u64); // 2100-01-01, always in-window on the fork
    let nonce: B256 =
        b256!("1111111111111111111111111111111111111111111111111111111111111111");

    // EIP-712 domain of Circle USDC on Ethereum Sepolia (verified against on-chain DOMAIN_SEPARATOR).
    let verifying_contract: Address = address!("1c7D4B196Cb0C7B01d743Fbc6116a902379C7238");
    let chain_id: u64 = 11_155_111;

    // Buyer's throwaway key. Recovers to `from`.
    let buyer: PrivateKeySigner =
        "0x00000000000000000000000000000000000000000000000000000000dec00d01"
            .parse()
            .expect("valid key");
    assert_eq!(buyer.address(), from, "buyer key must recover to `from`");

    let domain = eip712_domain! {
        name: "USDC",
        version: "2",
        chain_id: chain_id,
        verifying_contract: verifying_contract,
    };

    // A. hand-rolled
    let a = TransferWithAuthorization {
        from,
        to,
        value,
        validAfter: valid_after,
        validBefore: valid_before,
        nonce,
    };
    // B. x402-rs
    let b = X402TWA {
        from,
        to,
        value,
        validAfter: valid_after,
        validBefore: valid_before,
        nonce,
    };

    let type_hash_a = TransferWithAuthorization::eip712_type_hash(&a);
    let type_hash_b = X402TWA::eip712_type_hash(&b);
    let digest_a = a.eip712_signing_hash(&domain);
    let digest_b = b.eip712_signing_hash(&domain);

    let sig_a = buyer.sign_hash_sync(&digest_a).expect("sign a");
    let sig_b = buyer.sign_hash_sync(&digest_b).expect("sign b");
    let sig_a_hex = format!("0x{}", hex(&sig_a.as_bytes()));
    let sig_b_hex = format!("0x{}", hex(&sig_b.as_bytes()));

    // ---- Assertions (Question 1) ----
    assert_eq!(type_hash_a, type_hash_b, "type hash diverged");
    assert_eq!(digest_a, digest_b, "EIP-712 digest diverged");
    assert_eq!(sig_a_hex, sig_b_hex, "signature diverged");

    // Independent cross-check: the EIP-3009 struct type string is the canonical one.
    let expected_type =
        "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)";
    assert_eq!(
        <TransferWithAuthorization as SolStruct>::eip712_encode_type(),
        expected_type,
        "canonical EIP-712 type string diverged"
    );

    println!("== EIP-3009 TransferWithAuthorization: hand-roll vs x402-rs ==");
    println!("typeHash(handroll)  = {type_hash_a}");
    println!("typeHash(x402-rs)   = {type_hash_b}");
    println!("digest(handroll)    = {digest_a}");
    println!("digest(x402-rs)     = {digest_b}");
    println!("sig(handroll)       = {sig_a_hex}");
    println!("sig(x402-rs)        = {sig_b_hex}");
    println!(
        "MATCH: typeHash={} digest={} signature={}",
        type_hash_a == type_hash_b,
        digest_a == digest_b,
        sig_a_hex == sig_b_hex
    );

    // ---- Emit the reproducible KAT vector for #34 ----
    let kat = serde_json::json!({
        "note": "EIP-3009 TransferWithAuthorization KAT for Circle USDC on Ethereum Sepolia (fork). deckard #206.",
        "domain": {
            "name": "USDC",
            "version": "2",
            "chainId": chain_id,
            "verifyingContract": format!("{verifying_contract:#x}"),
        },
        "message": {
            "from": format!("{from:#x}"),
            "to": format!("{to:#x}"),
            "value": value.to_string(),
            "validAfter": valid_after.to_string(),
            "validBefore": valid_before.to_string(),
            "nonce": format!("{nonce:#x}"),
        },
        "eip712_type": expected_type,
        "type_hash": format!("{type_hash_a:#x}"),
        "digest": format!("{digest_a:#x}"),
        "signer": format!("{from:#x}"),
        "signature": sig_a_hex,
    });
    std::fs::write("kat.json", serde_json::to_string_pretty(&kat).unwrap()).unwrap();
    // Also emit just the signature for the facilitator HTTP loop shell step.
    std::fs::write("sig.hex", &sig_a_hex).unwrap();
    println!("wrote ./kat.json and ./sig.hex");
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
