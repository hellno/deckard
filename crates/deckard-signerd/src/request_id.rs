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

use deckard_contract::{Intent, IntentKind, RequestId};

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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, U256};

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
}
