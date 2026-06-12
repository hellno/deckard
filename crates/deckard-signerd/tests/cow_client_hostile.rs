//! #3 — hostile-input coverage for the CoW orderbook PURE parse helpers.
//!
//! The orderbook is an untrusted remote: every byte it returns is attacker-influenceable.
//! These tests feed the SOCKET-FREE parse helpers (`parse_error_body`, `parse_quote_response`,
//! `parse_order_status`, `parse_account_orders`, `parse_order_uid`) malformed / truncated /
//! garbage JSON and assert they map to a TYPED `CowError` (never a panic), plus the one valid
//! body that must decode cleanly. No live HTTP and no socket — only the pure functions.
//!
//! Gating note: these helpers live behind deckard-core's default-on `cow-client` feature (the
//! daemon itself builds core WITHOUT it — see `feature_gate.rs`). This whole file is therefore
//! `#![cfg(feature = "cow-client")]`: it is compiled + run only when an orchestrator build
//! enables the feature for this test crate; otherwise it is inert (deckard-core's own in-module
//! tests already cover these helpers, so dropping the file is never a coverage hole).
#![cfg(feature = "cow-client")]

use deckard_core::{
    parse_account_orders, parse_error_body, parse_order_status, parse_order_uid,
    parse_quote_response, CowError,
};

/// A structured `{errorType, description}` body maps to the typed `CowError::Api`.
#[test]
fn structured_error_body_maps_to_typed_api_error() {
    let body = r#"{"errorType":"InsufficientAllowance","description":"erc20 allowance too low"}"#;
    let err = parse_error_body(400, body);
    assert_eq!(
        err,
        CowError::Api {
            error_type: "InsufficientAllowance".into(),
            description: "erc20 allowance too low".into(),
        },
        "the orderbook's own error shape must surface as CowError::Api"
    );
    // Display carries both fields (for the daemon's redacted log line / the GUI cue).
    let shown = err.to_string();
    assert!(shown.contains("InsufficientAllowance"), "got: {shown}");
    assert!(shown.contains("erc20 allowance too low"), "got: {shown}");
}

/// A non-structured error body falls back to `CowError::Http` (status + raw body preserved),
/// never a panic.
#[test]
fn unstructured_error_body_maps_to_http() {
    let err = parse_error_body(502, "<html>502 Bad Gateway</html>");
    assert_eq!(
        err,
        CowError::Http {
            status: 502,
            body: "<html>502 Bad Gateway</html>".into(),
        }
    );
}

/// Truncated / garbage / wrong-shape quote bodies must all decode to a typed `CowError::Decode`
/// (never an unwrap panic or an index-out-of-bounds).
#[test]
fn hostile_quote_bodies_decode_to_typed_error_not_panic() {
    let hostile = [
        "",                                           // empty
        "null",                                       // JSON null
        "[]",                                         // wrong top-level type
        "{}",                                         // missing `quote`
        r#"{"quote": null}"#,                         // null nested
        r#"{"quote": {"sellToken": "not-an-addr"}}"#, // bad address
        // sellAmount as a bare number — the wire requires a decimal STRING.
        r#"{"quote":{"sellToken":"0xfff9976782d46cc05630d1f6ebab18b2324d6b14",
            "buyToken":"0xbe72e441bf55620febc26715db68d3494213d8cb",
            "sellAmount":1000,"buyAmount":"2000","validTo":1,"feeAmount":"0"}}"#,
        r#"{not even json"#,                  // syntactically invalid
        "{\"quote\":{\"sellToken\":\"0xfff9", // truncated mid-token
    ];
    for body in hostile {
        let got = parse_quote_response(body);
        assert!(
            matches!(got, Err(CowError::Decode(_))),
            "expected Decode error for hostile quote body, got {got:?} (body: {body})"
        );
    }
}

/// A VALID quote body decodes cleanly and surfaces the fields the daemon needs.
#[test]
fn valid_quote_body_parses_fields() {
    let body = r#"{
        "quote": {
            "sellToken": "0xfff9976782d46cc05630d1f6ebab18b2324d6b14",
            "buyToken": "0xbe72e441bf55620febc26715db68d3494213d8cb",
            "receiver": "0x1111111111111111111111111111111111111111",
            "sellAmount": "1000000000000000000",
            "buyAmount": "2000000000000000000",
            "validTo": 1893456000,
            "feeAmount": "5000000000000000",
            "kind": "sell",
            "partiallyFillable": false
        },
        "from": "0x1111111111111111111111111111111111111111",
        "expiration": "2030-01-01T00:00:00Z",
        "id": 99,
        "verified": true,
        "hostileUnknownKey": {"ignored": true}
    }"#;
    let quote = parse_quote_response(body).expect("a valid quote body must decode");
    assert_eq!(quote.id, Some(99));
    assert_eq!(quote.quote.valid_to, 1_893_456_000);
    assert_eq!(
        quote.quote.sell_amount.to_string(),
        "1000000000000000000",
        "gross sell amount parsed from the decimal-string wire form"
    );
    assert_eq!(quote.quote.buy_amount.to_string(), "2000000000000000000");
}

/// Order-status: a good `{"type": ...}` body decodes; a missing-`type` body is a typed Decode.
#[test]
fn order_status_hostile_and_valid() {
    let ok = parse_order_status(r#"{"type":"open","value":[]}"#).expect("status decodes");
    assert_eq!(ok.status_type, "open");
    for hostile in ["", "null", "{}", r#"{"value":[]}"#, "not json"] {
        assert!(
            matches!(parse_order_status(hostile), Err(CowError::Decode(_))),
            "expected Decode error for hostile status body: {hostile}"
        );
    }
}

/// Account-orders: a good array decodes; a non-array / garbage body is a typed Decode.
#[test]
fn account_orders_hostile_and_valid() {
    let body = r#"[
        {"uid":"0xabc","owner":"0x1111111111111111111111111111111111111111","status":"open"},
        {"uid":"0xdef","owner":"0x1111111111111111111111111111111111111111","status":"fulfilled"}
    ]"#;
    let orders = parse_account_orders(body).expect("orders decode");
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].status, "open");
    for hostile in ["", "{}", r#"{"not":"an array"}"#, "garbage"] {
        assert!(
            matches!(parse_account_orders(hostile), Err(CowError::Decode(_))),
            "expected Decode error for hostile account-orders body: {hostile}"
        );
    }
}

/// Order-uid: a JSON-quoted string decodes; a bare (unquoted) hex string is a typed Decode.
#[test]
fn order_uid_hostile_and_valid() {
    assert_eq!(
        parse_order_uid(r#""0xdeadbeef""#).expect("uid decodes"),
        "0xdeadbeef"
    );
    for hostile in ["0xdeadbeef", "", "null", "{}"] {
        assert!(
            matches!(parse_order_uid(hostile), Err(CowError::Decode(_))),
            "expected Decode error for hostile uid body: {hostile}"
        );
    }
}
