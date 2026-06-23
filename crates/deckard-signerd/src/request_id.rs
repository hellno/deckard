//! How `propose` assigns a request id.
//!
//! The frozen `Decision::Allow` carries **no** id, yet `execute` is keyed by one. We close
//! that gap deterministically: the request id is `keccak256` of a stable, unambiguous
//! encoding of the intent. So a client that received `Allow` can derive the very same id
//! locally to `execute` it, while a `NeedsApproval` id (returned on the wire) is identical.
//! Both the daemon (server) and [`SignerClient`](crate::SignerClient) call this, so they
//! never disagree.
//!
//! v1 caveats (documented, fast-follow): the id is deterministic, hence *guessable* from the
//! intent — fine for a same-uid socket, but production should salt it (which needs the id to
//! ride the `Allow` on the wire — a contract change). Two identical intents map to one id;
//! the daemon coalesces them (it preserves an already-broadcast record, so this can never
//! double-spend).

use alloy_primitives::keccak256;

use deckard_contract::{Intent, IntentKind, RequestId, SignMessage, SignMessageKind, SwapOrder};

/// Deterministic request id for an intent. Fixed-width fields first, variable `calldata`
/// last, so no field boundary is ambiguous.
pub fn request_id_for(intent: &Intent) -> RequestId {
    let mut buf = Vec::with_capacity(8 + 21 + 32 + 1 + intent.calldata.len());
    buf.extend_from_slice(&intent.chain_id.to_be_bytes()); // 8
    buf.extend_from_slice(intent.to.as_slice()); // 20
    match intent.token {
        Some(token) => {
            buf.push(1);
            buf.extend_from_slice(token.as_slice()); // 20
        }
        None => buf.push(0),
    }
    buf.extend_from_slice(&intent.value.to_be_bytes::<32>()); // 32
    buf.push(match intent.kind {
        IntentKind::Send => 0,
        IntentKind::Shield => 1,
        IntentKind::Unshield => 2,
        IntentKind::ContractCall => 3,
    });
    buf.extend_from_slice(&intent.calldata); // variable, last
    keccak256(&buf)
}

/// Deterministic request id for a swap [`SwapOrder`], computed over the order the daemon
/// BOUND (its `owner`/`receiver` already overwritten with the unlocked wallet).
///
/// The encoding is prefixed with a `0x02` discriminator byte that no [`request_id_for`]
/// encoding starts with (an intent id starts with the 8-byte big-endian chain id, whose first
/// byte is `0x00` for every realistic chain id), so an order id can NEVER collide with an
/// intent id even at the same chain/amounts. All fields are fixed-width, so no boundary is
/// ambiguous. Built with `extend_from_slice` (no index expressions).
pub fn request_id_for_order(order: &SwapOrder) -> RequestId {
    let mut buf = Vec::with_capacity(1 + 8 + 20 * 4 + 32 * 2 + 4 + 32);
    buf.push(0x02); // order discriminator — disjoint from any `request_id_for` prefix
    buf.extend_from_slice(&order.chain_id.to_be_bytes()); // 8
    buf.extend_from_slice(order.sell_token.as_slice()); // 20
    buf.extend_from_slice(order.buy_token.as_slice()); // 20
    buf.extend_from_slice(order.receiver.as_slice()); // 20
    buf.extend_from_slice(order.owner.as_slice()); // 20
    buf.extend_from_slice(&order.sell_amount.to_be_bytes::<32>()); // 32
    buf.extend_from_slice(&order.buy_amount_min.to_be_bytes::<32>()); // 32
    buf.extend_from_slice(&order.valid_to.to_be_bytes()); // 4
    buf.extend_from_slice(order.app_data.as_slice()); // 32
    keccak256(&buf)
}

/// Deterministic request id for an off-chain message-signing request. The `0x03`
/// discriminator is disjoint from transaction (`chain_id` prefix) and order (`0x02`) ids.
pub fn request_id_for_message(message: &SignMessage) -> RequestId {
    let mut buf = Vec::with_capacity(128);
    buf.push(0x03);
    buf.extend_from_slice(&message.chain_id.to_be_bytes());
    buf.extend_from_slice(message.origin.as_bytes());
    buf.push(0x00);
    match &message.kind {
        SignMessageKind::PersonalSign { message } => {
            buf.push(0x01);
            buf.extend_from_slice(message);
        }
        SignMessageKind::TypedDataV4(review) => {
            buf.push(0x02);
            buf.extend_from_slice(review.digest.as_slice());
        }
        SignMessageKind::EthSign { digest } => {
            buf.push(0x03);
            buf.extend_from_slice(digest.as_slice());
        }
        SignMessageKind::Authorization7702 { delegate, nonce } => {
            buf.push(0x04);
            buf.extend_from_slice(delegate.as_slice());
            buf.extend_from_slice(&nonce.to_be_bytes());
        }
    }
    keccak256(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U256};

    fn order(sell: u64) -> SwapOrder {
        SwapOrder {
            chain_id: 11155111,
            owner: Address::repeat_byte(0xAA),
            sell_token: Address::repeat_byte(0x11),
            buy_token: Address::repeat_byte(0x22),
            sell_amount: U256::from(sell),
            buy_amount_min: U256::from(7u64),
            receiver: Address::repeat_byte(0xAA),
            valid_to: 1_700_000_000,
            app_data: B256::repeat_byte(0x33),
        }
    }

    fn send(value: u64) -> Intent {
        Intent {
            chain_id: 31337,
            to: Address::repeat_byte(0x22),
            token: None,
            value: U256::from(value),
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        }
    }

    fn personal_message(text: &str) -> SignMessage {
        SignMessage {
            chain_id: 31337,
            origin: "https://example.test".into(),
            kind: SignMessageKind::PersonalSign {
                message: Bytes::from(text.as_bytes().to_vec()),
            },
        }
    }

    #[test]
    fn deterministic_and_nonzero() {
        let id = request_id_for(&send(100));
        assert_eq!(id, request_id_for(&send(100)), "same intent → same id");
        assert_ne!(id, RequestId::ZERO);
    }

    #[test]
    fn distinguishes_fields() {
        assert_ne!(
            request_id_for(&send(100)),
            request_id_for(&send(101)),
            "value"
        );
        let mut other_to = send(100);
        other_to.to = Address::repeat_byte(0x33);
        assert_ne!(request_id_for(&send(100)), request_id_for(&other_to), "to");
        let mut tokened = send(100);
        tokened.token = Some(Address::repeat_byte(0x22));
        assert_ne!(
            request_id_for(&send(100)),
            request_id_for(&tokened),
            "token presence"
        );
    }

    #[test]
    fn order_deterministic_and_nonzero() {
        let id = request_id_for_order(&order(100));
        assert_eq!(
            id,
            request_id_for_order(&order(100)),
            "same order → same id"
        );
        assert_ne!(id, RequestId::ZERO);
    }

    #[test]
    fn order_distinguishes_fields() {
        let base = request_id_for_order(&order(100));
        assert_ne!(base, request_id_for_order(&order(101)), "sell_amount");

        let mut buy = order(100);
        buy.buy_amount_min = U256::from(8u64);
        assert_ne!(base, request_id_for_order(&buy), "buy_amount_min");

        let mut owner = order(100);
        owner.owner = Address::repeat_byte(0xBB);
        assert_ne!(base, request_id_for_order(&owner), "owner");

        let mut recv = order(100);
        recv.receiver = Address::repeat_byte(0xBB);
        assert_ne!(base, request_id_for_order(&recv), "receiver");

        let mut sell_tok = order(100);
        sell_tok.sell_token = Address::repeat_byte(0x99);
        assert_ne!(base, request_id_for_order(&sell_tok), "sell_token");

        let mut buy_tok = order(100);
        buy_tok.buy_token = Address::repeat_byte(0x99);
        assert_ne!(base, request_id_for_order(&buy_tok), "buy_token");

        let mut vt = order(100);
        vt.valid_to = 1_700_000_001;
        assert_ne!(base, request_id_for_order(&vt), "valid_to");

        let mut app = order(100);
        app.app_data = B256::repeat_byte(0x44);
        assert_ne!(base, request_id_for_order(&app), "app_data");

        let mut chain = order(100);
        chain.chain_id = 1;
        assert_ne!(base, request_id_for_order(&chain), "chain_id");
    }

    /// The `0x02` discriminator guarantees an order id can never equal any intent id.
    #[test]
    fn order_id_never_collides_with_intent_id() {
        assert_ne!(
            request_id_for_order(&order(100)),
            request_id_for(&send(100)),
            "order vs intent must be disjoint"
        );
    }

    #[test]
    fn message_id_deterministic_and_disjoint() {
        let id = request_id_for_message(&personal_message("hello"));
        assert_eq!(id, request_id_for_message(&personal_message("hello")));
        assert_ne!(id, RequestId::ZERO);
        assert_ne!(id, request_id_for(&send(100)), "message vs intent");
        assert_ne!(id, request_id_for_order(&order(100)), "message vs order");
        assert_ne!(
            id,
            request_id_for_message(&personal_message("goodbye")),
            "message bytes"
        );
    }
}
