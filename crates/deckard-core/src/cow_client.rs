//! CoW Protocol orderbook REST client — the only HTTP in `deckard-core`, and the reason
//! this whole module is gated behind the default-on `cow-client` feature. The signer daemon
//! builds core with `default-features = false`, so it never compiles `reqwest` or this file;
//! it only ever names the pure types in [`crate::cow_types`].
//!
//! Layout: serde request/response structs that mirror the orderbook OpenAPI, a typed
//! [`CowError`], a set of PURE parse helpers (no socket — they take raw bytes/JSON, so
//! Package D's hostile-input tests can drive them without a server), and the thin async
//! `reqwest` wrappers that do the network round-trip and then hand the body to those helpers.
//!
//! Lint note: like the rest of `deckard-core`, no `unwrap`/`expect`/`panic`/indexing in
//! non-test code. Network and parse errors propagate as `Result`.

use std::fmt;

use alloy::primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};

use crate::cow_types::{apply_slippage, APP_DATA_DOC, APP_DATA_HASH};
use deckard_contract::SwapOrder;

/// Default slippage tolerance applied to a quote's `buyAmount` to derive `buy_amount_min`.
/// 50 bps == 0.50%. The order is FILL-OR-KILL at this floor: a worse fill never settles.
pub const DEFAULT_SLIPPAGE_BPS: u16 = 50;

// ---------------------------------------------------------------------------
// Typed error
// ---------------------------------------------------------------------------

/// Everything that can go wrong talking to the orderbook. Constructed by the pure parse
/// helpers (so they are socket-free and unit-testable) and by the async wrappers. Carries
/// the orderbook's own `{ errorType, description }` for an API-level rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CowError {
    /// The orderbook returned a non-success status with a structured error body.
    Api {
        error_type: String,
        description: String,
    },
    /// A non-success status whose body did not parse as the structured error shape.
    Http { status: u16, body: String },
    /// The success body did not deserialize into the expected shape.
    Decode(String),
    /// The transport itself failed (DNS, TLS, connect, timeout). Network-only.
    Transport(String),
}

impl fmt::Display for CowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CowError::Api {
                error_type,
                description,
            } => write!(f, "cow api error {error_type}: {description}"),
            CowError::Http { status, body } => write!(f, "cow http {status}: {body}"),
            CowError::Decode(msg) => write!(f, "cow decode error: {msg}"),
            CowError::Transport(msg) => write!(f, "cow transport error: {msg}"),
        }
    }
}

impl std::error::Error for CowError {}

// ---------------------------------------------------------------------------
// Wire structs — request/response shapes mirroring the orderbook OpenAPI.
// All amounts cross the wire as decimal strings (TokenAmount); addresses/hashes as 0x-hex,
// which alloy's serde encodes/decodes natively.
// ---------------------------------------------------------------------------

/// `POST /api/v1/quote` request body (the subset Deckard sends). This is a SELL order quoted
/// from the gross pre-fee sell amount (`sellAmountBeforeFee`), valid for `valid_for` seconds.
/// `kind` is pinned `"sell"`; `appData`/`appDataHash` reference the canonical `{}` doc.
#[derive(Clone, Debug, Serialize)]
pub struct QuoteRequest {
    #[serde(rename = "sellToken")]
    pub sell_token: Address,
    #[serde(rename = "buyToken")]
    pub buy_token: Address,
    pub from: Address,
    pub receiver: Address,
    /// gross amount available; fee is deducted from this. Decimal-string on the wire.
    #[serde(rename = "sellAmountBeforeFee", with = "u256_dec")]
    pub sell_amount_before_fee: U256,
    pub kind: String,
    #[serde(rename = "validFor")]
    pub valid_for: u32,
    #[serde(rename = "appData")]
    pub app_data: String,
    #[serde(rename = "appDataHash")]
    pub app_data_hash: B256,
    #[serde(rename = "sellTokenBalance")]
    pub sell_token_balance: String,
    #[serde(rename = "buyTokenBalance")]
    pub buy_token_balance: String,
    #[serde(rename = "signingScheme")]
    pub signing_scheme: String,
}

impl QuoteRequest {
    /// A market SELL quote request for `sell_amount` of `sell_token` into `buy_token`, with
    /// the canonical `{}` app-data and erc20/eip712 defaults pinned. `from`/`receiver` are
    /// the caller's wallet (the daemon binds them; the orderbook simulates against `from`).
    pub fn sell(
        sell_token: Address,
        buy_token: Address,
        wallet: Address,
        sell_amount: U256,
        valid_for: u32,
    ) -> Self {
        QuoteRequest {
            sell_token,
            buy_token,
            from: wallet,
            receiver: wallet,
            sell_amount_before_fee: sell_amount,
            kind: "sell".into(),
            valid_for,
            app_data: APP_DATA_DOC.into(),
            app_data_hash: APP_DATA_HASH,
            sell_token_balance: "erc20".into(),
            buy_token_balance: "erc20".into(),
            signing_scheme: "eip712".into(),
        }
    }
}

/// `POST /api/v1/quote` response. We keep only the fields Deckard needs to build a
/// `SwapOrder`; `#[serde(default)]` on the rest of the doc is implicit because we simply do
/// not name them (serde ignores unknown fields by default), so extra/hostile keys are inert.
#[derive(Clone, Debug, Deserialize)]
pub struct QuoteResponse {
    pub quote: QuoteOrderParameters,
    /// Trader address the quote was simulated for.
    #[serde(default)]
    pub from: Option<Address>,
    /// Fee-offer expiry, ISO-8601 UTC string. Kept verbatim (not parsed) for diagnostics.
    #[serde(default)]
    pub expiration: Option<String>,
    /// Quote id, linking a later order back to this quote. Nullable per the OpenAPI.
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub verified: Option<bool>,
}

/// The `quote` object inside a [`QuoteResponse`] (OpenAPI `OrderParameters`): the priced order
/// the backend will accept. We read sell/buy amounts, validity, fee, and token addresses.
#[derive(Clone, Debug, Deserialize)]
pub struct QuoteOrderParameters {
    #[serde(rename = "sellToken")]
    pub sell_token: Address,
    #[serde(rename = "buyToken")]
    pub buy_token: Address,
    #[serde(default)]
    pub receiver: Option<Address>,
    /// The AFTER-fee sell amount (decimal-string). The gross the user parts with is
    /// `sell_amount + fee_amount` (== the requested `sellAmountBeforeFee`); that gross is what
    /// the signed order carries. Verified against the live Sepolia orderbook.
    #[serde(rename = "sellAmount", with = "u256_dec")]
    pub sell_amount: U256,
    #[serde(rename = "buyAmount", with = "u256_dec")]
    pub buy_amount: U256,
    #[serde(rename = "validTo")]
    pub valid_to: u32,
    /// Estimated network fee in sell-token atoms. We send `0` on the actual order (solvers
    /// take the fee from surplus) but keep the quoted value for diagnostics.
    #[serde(rename = "feeAmount", with = "u256_dec")]
    pub fee_amount: U256,
}

/// `POST /api/v1/orders` request body — a fully-formed, signed order ready to submit. The
/// four constant params are pinned to the GPv2 sell-order shape; `feeAmount` is `0`.
#[derive(Clone, Debug, Serialize)]
pub struct OrderCreation {
    #[serde(rename = "sellToken")]
    pub sell_token: Address,
    #[serde(rename = "buyToken")]
    pub buy_token: Address,
    pub receiver: Address,
    #[serde(rename = "sellAmount", with = "u256_dec")]
    pub sell_amount: U256,
    #[serde(rename = "buyAmount", with = "u256_dec")]
    pub buy_amount: U256,
    #[serde(rename = "validTo")]
    pub valid_to: u32,
    #[serde(rename = "feeAmount", with = "u256_dec")]
    pub fee_amount: U256,
    pub kind: String,
    #[serde(rename = "partiallyFillable")]
    pub partially_fillable: bool,
    #[serde(rename = "sellTokenBalance")]
    pub sell_token_balance: String,
    #[serde(rename = "buyTokenBalance")]
    pub buy_token_balance: String,
    #[serde(rename = "signingScheme")]
    pub signing_scheme: String,
    /// 65-byte r||s||v ECDSA signature, 0x-hex. alloy's `Bytes` serde emits the 0x form.
    pub signature: Bytes,
    pub from: Address,
    /// Full app-data doc string (`"{}"`); the contract `appData` is its keccak hash.
    #[serde(rename = "appData")]
    pub app_data: String,
    #[serde(rename = "appDataHash")]
    pub app_data_hash: B256,
    #[serde(rename = "quoteId", skip_serializing_if = "Option::is_none")]
    pub quote_id: Option<i64>,
}

impl OrderCreation {
    /// Build the submit body from a (signed) [`SwapOrder`] plus its 65-byte signature and the
    /// originating quote id. Mirrors [`crate::cow_types::order_digest`]'s pinned constants so
    /// the submitted order is byte-identical to the one that was signed.
    pub fn from_signed_order(order: &SwapOrder, signature: Bytes, quote_id: Option<i64>) -> Self {
        OrderCreation {
            sell_token: order.sell_token,
            buy_token: order.buy_token,
            receiver: order.receiver,
            sell_amount: order.sell_amount,
            buy_amount: order.buy_amount_min,
            valid_to: order.valid_to,
            fee_amount: U256::ZERO,
            kind: "sell".into(),
            partially_fillable: false,
            sell_token_balance: "erc20".into(),
            buy_token_balance: "erc20".into(),
            signing_scheme: "eip712".into(),
            signature,
            from: order.owner,
            app_data: APP_DATA_DOC.into(),
            app_data_hash: APP_DATA_HASH,
            quote_id,
        }
    }
}

/// `PUT /api/v1/app_data` request body. The orderbook stores the full doc keyed by its hash.
#[derive(Clone, Debug, Serialize)]
pub struct AppDataDoc {
    #[serde(rename = "fullAppData")]
    pub full_app_data: String,
}

/// `GET /api/v1/orders/{uid}/status` response (OpenAPI `CompetitionOrderStatus`). We read the
/// lifecycle `type`; the per-solver `value` array is not needed for Deckard's status display.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OrderStatusResponse {
    /// One of: open, scheduled, active, solved, executing, traded, cancelled.
    #[serde(rename = "type")]
    pub status_type: String,
}

/// One element of `GET /api/v1/account/{owner}/orders` (OpenAPI `Order`). Lean view: the
/// fields Deckard renders in the swap inbox. Unknown keys are ignored by serde.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AccountOrder {
    pub uid: String,
    pub owner: Address,
    /// presignaturePending | open | fulfilled | cancelled | expired
    pub status: String,
    #[serde(rename = "sellToken", default)]
    pub sell_token: Option<Address>,
    #[serde(rename = "buyToken", default)]
    pub buy_token: Option<Address>,
    #[serde(rename = "validTo", default)]
    pub valid_to: Option<u32>,
}

// ---------------------------------------------------------------------------
// SwapOrder builder (pure)
// ---------------------------------------------------------------------------

/// Build a [`SwapOrder`] from a fetched quote, applying `slippage_bps` to the quoted
/// `buyAmount` to derive `buy_amount_min`. `owner`/`receiver` are bound by the caller (the
/// daemon overwrites the owner with the unlocked wallet — never trusts a client-supplied
/// owner). Pure: no network, fully unit-testable.
///
/// **Fee model (verified against the live Sepolia orderbook):** the quote response's
/// `quote.sellAmount` is the AFTER-fee amount and `quote.feeAmount` is the fee, with
/// `sellAmountBeforeFee == sellAmount + feeAmount`. The order we sign carries the **gross**
/// `sellAmount` (= `sellAmount + feeAmount`, the full amount the user parts with) and
/// `feeAmount = 0` — CoW's surplus-fee model, where the solver takes its fee out of the margin
/// between the gross sell and the (after-fee) `buy_amount_min`. Signing the after-fee
/// `quote.sellAmount` instead would leave no surplus for the solver and the order would not fill.
pub fn swap_order_from_quote(
    quote: &QuoteResponse,
    chain_id: u64,
    owner: Address,
    receiver: Address,
    slippage_bps: u16,
) -> SwapOrder {
    SwapOrder {
        chain_id,
        owner,
        sell_token: quote.quote.sell_token,
        buy_token: quote.quote.buy_token,
        // Gross pre-fee sell amount: the user parts with `sellAmount + feeAmount`, and the order's
        // own feeAmount is 0 (set in `OrderCreation::from_signed_order`).
        sell_amount: quote
            .quote
            .sell_amount
            .saturating_add(quote.quote.fee_amount),
        buy_amount_min: apply_slippage(quote.quote.buy_amount, slippage_bps),
        receiver,
        valid_to: quote.quote.valid_to,
        app_data: APP_DATA_HASH,
    }
}

// ---------------------------------------------------------------------------
// Pure parse helpers — socket-free, so Package D can feed them hostile JSON.
// Each takes the raw HTTP status + body and returns a typed Result.
// ---------------------------------------------------------------------------

/// The structured error body the orderbook returns on a 4xx/5xx: `{ errorType, description }`.
#[derive(Clone, Debug, Deserialize)]
struct ApiErrorBody {
    #[serde(rename = "errorType")]
    error_type: String,
    description: String,
}

/// Turn a non-2xx (status, body) into a typed [`CowError`]: the structured `{errorType,
/// description}` shape if it parses, else a raw [`CowError::Http`]. Pure.
pub fn parse_error_body(status: u16, body: &str) -> CowError {
    match serde_json::from_str::<ApiErrorBody>(body) {
        Ok(parsed) => CowError::Api {
            error_type: parsed.error_type,
            description: parsed.description,
        },
        Err(_) => CowError::Http {
            status,
            body: body.to_owned(),
        },
    }
}

/// Decode a success `POST /api/v1/quote` body into a [`QuoteResponse`]. Pure.
pub fn parse_quote_response(body: &str) -> Result<QuoteResponse, CowError> {
    serde_json::from_str(body).map_err(|e| CowError::Decode(e.to_string()))
}

/// Decode a `GET /status` body into an [`OrderStatusResponse`]. Pure.
pub fn parse_order_status(body: &str) -> Result<OrderStatusResponse, CowError> {
    serde_json::from_str(body).map_err(|e| CowError::Decode(e.to_string()))
}

/// Decode a `GET /account/{owner}/orders` body into a `Vec<AccountOrder>`. Pure.
pub fn parse_account_orders(body: &str) -> Result<Vec<AccountOrder>, CowError> {
    serde_json::from_str(body).map_err(|e| CowError::Decode(e.to_string()))
}

/// Decode a `POST /api/v1/orders` success body (a 0x-hex UID string, JSON-quoted) into the
/// raw uid string. Pure.
pub fn parse_order_uid(body: &str) -> Result<String, CowError> {
    serde_json::from_str(body).map_err(|e| CowError::Decode(e.to_string()))
}

// ---------------------------------------------------------------------------
// Async network wrappers — the ONLY socket code. Each does the round-trip then hands the
// body to a pure helper above. Errors surface as `anyhow::Error` (the crate idiom), with the
// typed `CowError` preserved as the source.
// ---------------------------------------------------------------------------

/// Read the response status + body, then dispatch: 2xx → `Ok(body)`, else the typed API/HTTP
/// error. One place so every call site treats orderbook errors identically.
async fn into_body(resp: reqwest::Response) -> Result<String, CowError> {
    let status = resp.status();
    let code = status.as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(parse_error_body(code, &body))
    }
}

/// `POST {base}/api/v1/quote` → priced order parameters.
pub async fn post_quote(
    client: &reqwest::Client,
    base: &str,
    req: &QuoteRequest,
) -> anyhow::Result<QuoteResponse> {
    let url = format!("{base}/api/v1/quote");
    let resp = client
        .post(url)
        .json(req)
        .send()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    let body = into_body(resp).await?;
    Ok(parse_quote_response(&body)?)
}

/// `PUT {base}/api/v1/app_data` — register the full app-data doc so the orderbook can link an
/// order's `appDataHash` to its document. Idempotent on the backend (200 if it already
/// exists, 201 if newly stored); we treat any 2xx as success.
pub async fn put_app_data(client: &reqwest::Client, base: &str, doc: &str) -> anyhow::Result<()> {
    let url = format!("{base}/api/v1/app_data");
    let resp = client
        .put(url)
        .json(&AppDataDoc {
            full_app_data: doc.to_owned(),
        })
        .send()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    into_body(resp).await?;
    Ok(())
}

/// `POST {base}/api/v1/orders` → the created order's uid (0x-hex string).
pub async fn post_order(
    client: &reqwest::Client,
    base: &str,
    order: &OrderCreation,
) -> anyhow::Result<String> {
    let url = format!("{base}/api/v1/orders");
    let resp = client
        .post(url)
        .json(order)
        .send()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    let body = into_body(resp).await?;
    Ok(parse_order_uid(&body)?)
}

/// `GET {base}/api/v1/orders/{uid}/status` → the lifecycle status.
pub async fn get_order_status(
    client: &reqwest::Client,
    base: &str,
    uid: &str,
) -> anyhow::Result<OrderStatusResponse> {
    let url = format!("{base}/api/v1/orders/{uid}/status");
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    let body = into_body(resp).await?;
    Ok(parse_order_status(&body)?)
}

/// `GET {base}/api/v1/account/{owner}/orders` → the account's recent orders.
pub async fn get_account_orders(
    client: &reqwest::Client,
    base: &str,
    owner: Address,
) -> anyhow::Result<Vec<AccountOrder>> {
    let url = format!("{base}/api/v1/account/{owner}/orders");
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CowError::Transport(e.to_string()))?;
    let body = into_body(resp).await?;
    Ok(parse_account_orders(&body)?)
}

// ---------------------------------------------------------------------------
// CowOrderbook — a thin handle that OWNS a `reqwest::Client` so the app can drive the
// orderbook WITHOUT naming `reqwest` itself. Each method takes high-level args plus the
// orderbook `base` URL and delegates to the free functions above (which the tests still use
// directly). Additive: the free functions are unchanged.
// ---------------------------------------------------------------------------

/// A reusable CoW orderbook client. Builds one `reqwest::Client` (connection-pool reuse across
/// calls) and exposes the orderbook operations the app needs, taking only high-level arguments
/// and the orderbook `base` URL — so callers never depend on `reqwest` directly.
#[derive(Clone, Debug)]
pub struct CowOrderbook {
    client: reqwest::Client,
}

impl Default for CowOrderbook {
    fn default() -> Self {
        Self::new()
    }
}

impl CowOrderbook {
    /// Build a new orderbook handle with a default `reqwest::Client`.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// `POST {base}/api/v1/quote` → priced order parameters.
    pub async fn quote(&self, base: &str, req: &QuoteRequest) -> anyhow::Result<QuoteResponse> {
        post_quote(&self.client, base, req).await
    }

    /// `PUT {base}/api/v1/app_data` — register the full app-data doc (idempotent on the backend).
    pub async fn put_app_data(&self, base: &str, doc: &str) -> anyhow::Result<()> {
        put_app_data(&self.client, base, doc).await
    }

    /// `POST {base}/api/v1/orders` → the created order's uid (0x-hex string).
    pub async fn submit(&self, base: &str, order: &OrderCreation) -> anyhow::Result<String> {
        post_order(&self.client, base, order).await
    }

    /// `GET {base}/api/v1/orders/{uid}/status` → the lifecycle status.
    pub async fn status(&self, base: &str, uid: &str) -> anyhow::Result<OrderStatusResponse> {
        get_order_status(&self.client, base, uid).await
    }

    /// `GET {base}/api/v1/account/{owner}/orders` → the account's recent orders.
    pub async fn account_orders(
        &self,
        base: &str,
        owner: Address,
    ) -> anyhow::Result<Vec<AccountOrder>> {
        get_account_orders(&self.client, base, owner).await
    }

    /// Blocking [`Self::quote`] for callers without a tokio reactor (the GPUI app runs on its own
    /// executor; reqwest/hickory require a tokio runtime). Bridges through a dedicated current-thread
    /// runtime — see [`block_on_orderbook`].
    pub fn quote_blocking(&self, base: &str, req: &QuoteRequest) -> anyhow::Result<QuoteResponse> {
        block_on_orderbook(self.quote(base, req))
    }

    /// Blocking [`Self::put_app_data`] — see [`Self::quote_blocking`].
    pub fn put_app_data_blocking(&self, base: &str, doc: &str) -> anyhow::Result<()> {
        block_on_orderbook(self.put_app_data(base, doc))
    }

    /// Blocking [`Self::submit`] — see [`Self::quote_blocking`].
    pub fn submit_blocking(&self, base: &str, order: &OrderCreation) -> anyhow::Result<String> {
        block_on_orderbook(self.submit(base, order))
    }
}

/// Drive a CoW orderbook future to completion on a dedicated current-thread tokio runtime. The CoW
/// HTTP client (reqwest/hickory DNS) requires a tokio reactor; callers on a non-tokio executor (the
/// GPUI app, whose worker model keeps the UI off tokio — see `eth.rs`) use the `*_blocking` methods,
/// which bridge through here. Returns an error (never panics) if the runtime can't be built. CoW
/// calls are user-initiated and infrequent, so a fresh current-thread runtime per call is fine.
fn block_on_orderbook<T, F: std::future::Future<Output = anyhow::Result<T>>>(
    fut: F,
) -> anyhow::Result<T> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(fut)
}

// ---------------------------------------------------------------------------
// U256 decimal-string (de)serialization. TokenAmount on the CoW wire is a decimal string
// (NOT 0x-hex), so alloy's default U256 serde (which is 0x-hex) is wrong here.
// ---------------------------------------------------------------------------

mod u256_dec {
    use alloy::primitives::U256;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<U256>().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    /// A canned good quote response; assert it decodes and builds a SwapOrder with the
    /// default-slippage floor applied to buyAmount and the gross sellAmount carried through.
    #[test]
    fn parse_good_quote_and_build_order() {
        let body = r#"{
            "quote": {
                "sellToken": "0xfff9976782d46cc05630d1f6ebab18b2324d6b14",
                "buyToken": "0xbe72e441bf55620febc26715db68d3494213d8cb",
                "receiver": "0x1111111111111111111111111111111111111111",
                "sellAmount": "1000000000000000000",
                "buyAmount": "2000000000000000000",
                "validTo": 1700000000,
                "feeAmount": "5000000000000000",
                "kind": "sell",
                "partiallyFillable": false
            },
            "from": "0x1111111111111111111111111111111111111111",
            "expiration": "2026-01-01T00:00:00Z",
            "id": 42,
            "verified": true
        }"#;
        let quote = parse_quote_response(body).expect("good quote decodes");
        assert_eq!(quote.id, Some(42));
        assert_eq!(
            quote.quote.sell_amount,
            U256::from(1_000_000_000_000_000_000u64)
        );
        assert_eq!(
            quote.quote.buy_amount,
            U256::from(2_000_000_000_000_000_000u64)
        );
        assert_eq!(quote.quote.valid_to, 1_700_000_000);

        let owner = address!("0x2222222222222222222222222222222222222222");
        let order = swap_order_from_quote(&quote, 11155111, owner, owner, DEFAULT_SLIPPAGE_BPS);
        assert_eq!(order.chain_id, 11155111);
        assert_eq!(order.owner, owner);
        assert_eq!(order.receiver, owner);
        assert_eq!(order.sell_token, quote.quote.sell_token);
        assert_eq!(order.buy_token, quote.quote.buy_token);
        // GROSS sell amount = quote.sellAmount (after-fee, 1e18) + quote.feeAmount (5e15).
        assert_eq!(order.sell_amount, U256::from(1_005_000_000_000_000_000u64));
        // 50 bps of 2e18 = 1e16 → min 1.99e18
        assert_eq!(
            order.buy_amount_min,
            U256::from(1_990_000_000_000_000_000u64)
        );
        assert_eq!(order.app_data, APP_DATA_HASH);
    }

    /// Regression guard for the fee arithmetic, pinned to a REAL Sepolia quote captured live
    /// (`sellAmountBeforeFee` of 0.05 WETH → COW). The signed order must carry the gross 0.05,
    /// reconstructed as `quote.sellAmount + quote.feeAmount`. A regression to the after-fee
    /// `quote.sellAmount` would leave no solver surplus and the order would not fill.
    #[test]
    fn fee_arithmetic_matches_live_sepolia_quote() {
        // Verbatim from `POST https://api.cow.fi/sepolia/api/v1/quote` (sellAmountBeforeFee 5e16).
        let after_fee = U256::from(37_989_365_556_267_132u64);
        let fee = U256::from(12_010_634_443_732_868u64);
        let buy = U256::from_str_radix("1953742300219817002", 10).unwrap();
        let gross = U256::from(50_000_000_000_000_000u64); // == sellAmountBeforeFee
        assert_eq!(
            after_fee + fee,
            gross,
            "the live quote's after-fee + fee == gross"
        );

        let quote = QuoteResponse {
            quote: QuoteOrderParameters {
                sell_token: address!("0xfff9976782d46cc05630d1f6ebab18b2324d6b14"),
                buy_token: address!("0x0625afb445c3b6b7b929342a04a22599fd5dbb59"),
                receiver: None,
                sell_amount: after_fee,
                buy_amount: buy,
                valid_to: 1_781_261_340,
                fee_amount: fee,
            },
            from: None,
            expiration: None,
            id: Some(1_506_978),
            verified: Some(true),
        };
        let owner = address!("0x1111111111111111111111111111111111111111");
        let order = swap_order_from_quote(&quote, 11155111, owner, owner, DEFAULT_SLIPPAGE_BPS);
        assert_eq!(
            order.sell_amount, gross,
            "signed order sells the GROSS amount"
        );
        assert_eq!(
            order.buy_amount_min,
            apply_slippage(buy, DEFAULT_SLIPPAGE_BPS)
        );
    }

    /// LIVE smoke test against the real Sepolia orderbook (network — `#[ignore]`d, run with
    /// `cargo test -p deckard-core --features cow-client -- --ignored`). Drives the actual
    /// `post_quote` → `swap_order_from_quote` → `order_digest` path end-to-end and asserts the
    /// fee arithmetic the offline KATs can't reach. Panics with a clear message on a
    /// network/liquidity failure so a flaky run is self-explanatory.
    #[tokio::test]
    #[ignore = "live network: POSTs to api.cow.fi/sepolia"]
    async fn live_sepolia_quote_roundtrip() {
        let base = crate::cow_api_base(11155111).expect("sepolia base url");
        let client = reqwest::Client::new();
        let weth = address!("0xfff9976782d46cc05630d1f6ebab18b2324d6b14");
        let cow = address!("0x0625afb445c3b6b7b929342a04a22599fd5dbb59");
        let from = address!("0x1111111111111111111111111111111111111111");
        let gross = U256::from(50_000_000_000_000_000u64); // 0.05 WETH

        let req = QuoteRequest::sell(weth, cow, from, gross, 1800);
        let quote = match post_quote(&client, base, &req).await {
            Ok(q) => q,
            Err(e) => panic!("live Sepolia quote failed (network or liquidity?): {e}"),
        };

        // The orderbook splits our requested gross into after-fee + fee.
        assert_eq!(
            quote.quote.sell_amount + quote.quote.fee_amount,
            gross,
            "after-fee + fee must reconstruct the requested sellAmountBeforeFee"
        );
        assert!(
            quote.quote.buy_amount > U256::ZERO,
            "a live quote prices buyAmount"
        );

        // Our builder produces a signable order selling the GROSS amount with a slippage floor.
        let order = swap_order_from_quote(&quote, 11155111, from, from, DEFAULT_SLIPPAGE_BPS);
        assert_eq!(
            order.sell_amount, gross,
            "signed order sells the gross amount"
        );
        assert!(
            order.buy_amount_min < quote.quote.buy_amount,
            "slippage floor below quote"
        );
        assert_ne!(
            crate::cow_types::order_digest(&order),
            alloy::primitives::B256::ZERO,
            "the order produces a non-zero EIP-712 digest"
        );
        eprintln!(
            "LIVE sepolia OK: gross={gross} after_fee={} fee={} buy={} -> order.sell={} buy_min={}",
            quote.quote.sell_amount,
            quote.quote.fee_amount,
            quote.quote.buy_amount,
            order.sell_amount,
            order.buy_amount_min
        );
    }

    /// quoteId is nullable in the OpenAPI; a missing/null `id` decodes to `None`, not an error.
    #[test]
    fn quote_id_is_optional() {
        let body = r#"{
            "quote": {
                "sellToken": "0xfff9976782d46cc05630d1f6ebab18b2324d6b14",
                "buyToken": "0xbe72e441bf55620febc26715db68d3494213d8cb",
                "sellAmount": "1000",
                "buyAmount": "2000",
                "validTo": 1700000000,
                "feeAmount": "0"
            }
        }"#;
        let quote = parse_quote_response(body).expect("decodes without id");
        assert_eq!(quote.id, None);
        assert_eq!(quote.from, None);
    }

    /// A structured `{errorType, description}` error body → typed `CowError::Api`.
    #[test]
    fn parse_structured_error_body() {
        let body = r#"{"errorType":"NoLiquidity","description":"no route found"}"#;
        let err = parse_error_body(404, body);
        assert_eq!(
            err,
            CowError::Api {
                error_type: "NoLiquidity".into(),
                description: "no route found".into(),
            }
        );
        // Display includes both fields.
        let shown = err.to_string();
        assert!(shown.contains("NoLiquidity"));
        assert!(shown.contains("no route found"));
    }

    /// A non-structured error body → typed `CowError::Http` carrying status + raw body.
    #[test]
    fn parse_unstructured_error_body() {
        let err = parse_error_body(502, "<html>bad gateway</html>");
        assert_eq!(
            err,
            CowError::Http {
                status: 502,
                body: "<html>bad gateway</html>".into(),
            }
        );
    }

    /// Hostile/garbage success bodies must produce a typed `Decode` error, never a panic.
    #[test]
    fn hostile_quote_bodies_decode_to_error() {
        for hostile in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"quote": null}"#,
            r#"{"quote": {"sellToken": "not-an-address"}}"#,
            // sellAmount as a bare number (wire requires a decimal STRING) → reject
            r#"{"quote": {"sellToken":"0xfff9976782d46cc05630d1f6ebab18b2324d6b14",
                "buyToken":"0xbe72e441bf55620febc26715db68d3494213d8cb",
                "sellAmount": 1000, "buyAmount":"2000","validTo":1,"feeAmount":"0"}}"#,
            r#"{not even json"#,
        ] {
            assert!(
                matches!(parse_quote_response(hostile), Err(CowError::Decode(_))),
                "expected Decode error for hostile body: {hostile}"
            );
        }
    }

    #[test]
    fn parse_order_status_ok_and_hostile() {
        let ok = parse_order_status(r#"{"type":"open","value":[]}"#).expect("status decodes");
        assert_eq!(ok.status_type, "open");
        // missing required `type` → Decode error, not panic.
        assert!(matches!(
            parse_order_status(r#"{"value":[]}"#),
            Err(CowError::Decode(_))
        ));
    }

    #[test]
    fn parse_account_orders_ok_and_hostile() {
        let body = r#"[
            {"uid":"0xabc","owner":"0x1111111111111111111111111111111111111111",
             "status":"open","sellToken":"0xfff9976782d46cc05630d1f6ebab18b2324d6b14"},
            {"uid":"0xdef","owner":"0x1111111111111111111111111111111111111111",
             "status":"fulfilled"}
        ]"#;
        let orders = parse_account_orders(body).expect("orders decode");
        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].uid, "0xabc");
        assert_eq!(orders[0].status, "open");
        assert!(orders[1].sell_token.is_none());
        // a non-array → Decode error.
        assert!(matches!(
            parse_account_orders(r#"{"not":"an array"}"#),
            Err(CowError::Decode(_))
        ));
    }

    #[test]
    fn parse_order_uid_ok_and_hostile() {
        assert_eq!(
            parse_order_uid(r#""0xdeadbeef""#).expect("uid decodes"),
            "0xdeadbeef"
        );
        assert!(matches!(
            parse_order_uid("0xdeadbeef"),
            Err(CowError::Decode(_))
        ));
    }

    /// The owning `CowOrderbook` handle constructs via `new`/`Default` without a network call,
    /// so the app can build one cheaply and hold it. (Round-trips are covered by the live
    /// `#[ignore]`d test and Package D's helper tests.)
    #[test]
    fn cow_orderbook_constructs() {
        let _ob = CowOrderbook::new();
        let _default = CowOrderbook::default();
    }

    /// QuoteRequest serializes amounts as decimal strings (TokenAmount), not 0x-hex.
    #[test]
    fn quote_request_serializes_decimal_amount() {
        let req = QuoteRequest::sell(
            address!("0xfff9976782d46cc05630d1f6ebab18b2324d6b14"),
            address!("0xbe72e441bf55620febc26715db68d3494213d8cb"),
            address!("0x1111111111111111111111111111111111111111"),
            U256::from(1_500_000u64),
            1800,
        );
        let json = serde_json::to_string(&req).expect("serialize quote request");
        assert!(
            json.contains("\"sellAmountBeforeFee\":\"1500000\""),
            "got: {json}"
        );
        assert!(json.contains("\"kind\":\"sell\""));
        assert!(json.contains("\"validFor\":1800"));
        assert!(json.contains("\"appData\":\"{}\""));
    }

    /// OrderCreation from a signed order pins the constant params and feeAmount 0.
    #[test]
    fn order_creation_from_signed_order() {
        let order = SwapOrder {
            chain_id: 11155111,
            owner: address!("0x2222222222222222222222222222222222222222"),
            sell_token: address!("0xfff9976782d46cc05630d1f6ebab18b2324d6b14"),
            buy_token: address!("0xbe72e441bf55620febc26715db68d3494213d8cb"),
            sell_amount: U256::from(1_000_000u64),
            buy_amount_min: U256::from(990_000u64),
            receiver: address!("0x2222222222222222222222222222222222222222"),
            valid_to: 1_700_000_000,
            app_data: APP_DATA_HASH,
        };
        let sig = alloy::primitives::Bytes::from(vec![0xCDu8; 65]);
        let creation = OrderCreation::from_signed_order(&order, sig, Some(7));
        let json = serde_json::to_string(&creation).expect("serialize order creation");
        assert!(json.contains("\"feeAmount\":\"0\""), "got: {json}");
        assert!(json.contains("\"kind\":\"sell\""));
        assert!(json.contains("\"partiallyFillable\":false"));
        assert!(json.contains("\"sellTokenBalance\":\"erc20\""));
        assert!(json.contains("\"quoteId\":7"));
        assert!(json.contains("\"sellAmount\":\"1000000\""));
        assert!(json.contains("\"buyAmount\":\"990000\""));
    }
}
