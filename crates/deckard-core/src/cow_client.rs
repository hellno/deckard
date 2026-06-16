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

use alloy::primitives::{keccak256, Address, Bytes, B256, U256};
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
    /// Captured ONCE at construction. The demo knobs are set at process launch and never change
    /// mid-run, so reading them once gives every method — AND the `simulated` label — a SINGLE
    /// source of truth: `quote`/`put_app_data`/`submit` and `is_simulated()` can never disagree
    /// (each derives from `simulated`), and the on/off flag is cleanly separate from the fill URL.
    simulated: bool,
    /// The fork RPC the stub credits simulated fills on (`DECKARD_RPC_URL`); `None` → fills are
    /// skipped (honest un-credited). Only consulted when `simulated`.
    fill_rpc: Option<String>,
}

impl Default for CowOrderbook {
    fn default() -> Self {
        Self::new()
    }
}

impl CowOrderbook {
    /// Build a new orderbook handle with a default `reqwest::Client`, snapshotting the demo-stub
    /// flag ([`crate::env::demo_swap_stub`]) and fill RPC ([`crate::env::demo_swap_fill_rpc`]).
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            simulated: crate::env::demo_swap_stub(),
            fill_rpc: crate::env::demo_swap_fill_rpc(),
        }
    }

    /// Whether this orderbook handle is running in demo-stub mode (so callers can label a uid as
    /// "simulated — demo fork" honestly). Returns the construction-time snapshot of the
    /// [`crate::env::demo_swap_stub`] flag — the same value the routing below branches on.
    /// **Off in production** — the flag is unset unless `just demo` / `install --demo` set it.
    pub fn is_simulated(&self) -> bool {
        self.simulated
    }

    /// `POST {base}/api/v1/quote` → priced order parameters. In demo-stub mode the live orderbook
    /// is bypassed for a deterministic fixture, because a real CoW order can't be accepted+open
    /// from a local Sepolia fork — see [`demo_stub`].
    pub async fn quote(&self, base: &str, req: &QuoteRequest) -> anyhow::Result<QuoteResponse> {
        if self.simulated {
            return Ok(demo_stub::quote(req));
        }
        post_quote(&self.client, base, req).await
    }

    /// `PUT {base}/api/v1/app_data` — register the full app-data doc (idempotent on the backend).
    /// In demo-stub mode this is a no-op `Ok(())`: there is no live backend to register against.
    pub async fn put_app_data(&self, base: &str, doc: &str) -> anyhow::Result<()> {
        if self.simulated {
            return Ok(());
        }
        put_app_data(&self.client, base, doc).await
    }

    /// `POST {base}/api/v1/orders` → the created order's uid (0x-hex string). In demo-stub mode the
    /// order is "filled" on the fork by crediting the buy token to the receiver (known demo tokens
    /// only, on [`Self::fill_rpc`]) and a synthetic uid is returned; an uncreditable token or a
    /// missing/unreachable fill RPC never fabricates a verified success — see [`demo_stub::submit`].
    pub async fn submit(&self, base: &str, order: &OrderCreation) -> anyhow::Result<String> {
        if self.simulated {
            let (uid, _credited) =
                demo_stub::submit(&self.client, self.fill_rpc.as_deref(), order).await?;
            return Ok(uid);
        }
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
// Demo-fork swap stub — HONEST in-fork simulation. Gated by `DECKARD_DEMO_SWAP_STUB` carrying a
// fork RPC URL (see `crate::env::demo_swap_stub`); UNSET in production, so this never runs there.
// ---------------------------------------------------------------------------

/// HONEST in-fork swap simulation, used ONLY when `DECKARD_DEMO_SWAP_STUB` carries a fork RPC URL
/// (see [`crate::env::demo_swap_stub`]). A real CoW order can't be accepted+open from a local
/// Sepolia fork — the live orderbook validates real-Sepolia balances — so the demo routes
/// quote/put_app_data/submit through here. NEVER fabricates a Verified-looking success: an
/// unknown buy token (or any fill-RPC failure) yields a synthetic accepted uid with `credited =
/// false`, and the caller surfaces an explicit "balance not credited" note.
///
/// This module is feature-gated by living in `cow_client.rs`, which only compiles under the
/// `cow-client` feature; the signer daemon (built `--no-default-features`) never sees it. The
/// anvil cheatcode is just a JSON-RPC POST on the orderbook's own `reqwest::Client`, so no new
/// dependency is introduced and no `reqwest` type leaks to callers.
mod demo_stub {
    use super::*;

    /// Sepolia WETH9 (`0xfFf9…6B14`). Its canonical layout puts the `balanceOf` mapping at storage
    /// slot 3 (`name`=0, `symbol`=1, `decimals`=2, `balanceOf`=3, `allowance`=4) — so the fill can
    /// credit it confidently. Other Sepolia demo tokens use opaque/proxy layouts whose slot we have
    /// not verified, so [`balances_slot`] returns `None` for them (honest fallback, never a guess).
    const SEPOLIA_WETH: Address = Address::new([
        0xff, 0xf9, 0x97, 0x67, 0x82, 0xd4, 0x6c, 0xc0, 0x56, 0x30, 0xd1, 0xf6, 0xeb, 0xab, 0x18,
        0xb2, 0x32, 0x4d, 0x6b, 0x14,
    ]);

    /// The ERC-20 `balanceOf` mapping storage slot for a KNOWN Sepolia demo token, or `None` when
    /// the slot is not verified for that token. Guessing a slot would silently mis-fill, so an
    /// unverified token deliberately gets `None` (the fill then leaves balances untouched and the
    /// caller labels the outcome `credited = false`).
    fn balances_slot(token: Address) -> Option<U256> {
        if token == SEPOLIA_WETH {
            // WETH9: balanceOf is the first mapping declared after name/symbol/decimals → slot 3.
            Some(U256::from(3u8))
        } else {
            None
        }
    }

    /// The storage key for `mapping(address => uint256) balanceOf` at `slot`, holding `holder`'s
    /// balance: `keccak256(left_pad32(holder) ‖ uint256(slot))` — Solidity's mapping-slot layout.
    /// Pure; no indexing (lint): the 64-byte preimage is assembled with `split_at_mut`.
    fn balances_storage_key(holder: Address, slot: U256) -> B256 {
        let mut preimage = [0u8; 64];
        let (key_word, slot_word) = preimage.split_at_mut(32);
        // left-pad the 20-byte address into the low 20 bytes of the first 32-byte word.
        let (_pad, key_tail) = key_word.split_at_mut(12);
        key_tail.copy_from_slice(holder.as_slice());
        slot_word.copy_from_slice(&slot.to_be_bytes::<32>());
        keccak256(preimage)
    }

    /// A deterministic, clearly-synthetic CoW order uid for a stubbed fill. CoW uids are 56 bytes
    /// (`digest(32) ‖ owner(20) ‖ validTo(4 BE)`); we keep that shape so the response looks like a
    /// real accept+open, but derive the 32-byte "digest" by hashing the order's salient fields
    /// (NOT a real EIP-712 digest) so it can never be confused with a verified mainnet uid. Pure;
    /// the 56-byte buffer is assembled with `split_at_mut` (no indexing, lint).
    fn synthetic_uid(order: &OrderCreation) -> String {
        let mut preimage = Vec::with_capacity(20 + 20 + 20 + 32 + 32 + 4);
        preimage.extend_from_slice(b"deckard-demo-swap-stub");
        preimage.extend_from_slice(order.from.as_slice());
        preimage.extend_from_slice(order.buy_token.as_slice());
        preimage.extend_from_slice(&order.sell_amount.to_be_bytes::<32>());
        preimage.extend_from_slice(&order.buy_amount.to_be_bytes::<32>());
        preimage.extend_from_slice(&order.valid_to.to_be_bytes());
        let digest = keccak256(&preimage);

        let mut uid = [0u8; 56];
        let (d, rest) = uid.split_at_mut(32);
        let (owner, valid_to) = rest.split_at_mut(20);
        d.copy_from_slice(digest.as_slice());
        owner.copy_from_slice(order.from.as_slice());
        valid_to.copy_from_slice(&order.valid_to.to_be_bytes());
        format!("0x{}", alloy::hex::encode(uid))
    }

    /// A deterministic fixture quote for `req`, mirroring the live wire shape so
    /// [`swap_order_from_quote`] builds a normal `SwapOrder` (gross sell preserved, non-zero
    /// `buy_amount_min`). The price is a flat 1:1 (buy amount == requested gross), and the fee is a
    /// fixed 1% of the gross carved off the sell side so the `gross == after_fee + fee` invariant
    /// the builder relies on holds exactly. `id` is `None` — there is no real quote id on a fork.
    pub fn quote(req: &QuoteRequest) -> QuoteResponse {
        let gross = req.sell_amount_before_fee;
        // Fee is a fixed 1% of the gross (integer floor), capped strictly below the gross so the
        // after-fee amount stays positive. `after_fee + fee == gross` by construction, which is the
        // invariant `swap_order_from_quote` depends on to reconstruct the requested gross sell.
        let fee = gross / U256::from(100u8);
        let after_fee = gross.saturating_sub(fee);
        // `valid_to` is an ABSOLUTE Unix timestamp (the live orderbook returns now + validFor),
        // so add the request's relative `valid_for` duration to the current time. A clock-read
        // failure floors at 0 and the cast saturates — no panic in the trust core. Copying the
        // raw `valid_for` here would date the simulated order to 1970.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let valid_to = u32::try_from(now_secs)
            .unwrap_or(u32::MAX)
            .saturating_add(req.valid_for);
        QuoteResponse {
            quote: QuoteOrderParameters {
                sell_token: req.sell_token,
                buy_token: req.buy_token,
                receiver: Some(req.receiver),
                sell_amount: after_fee,
                // Flat 1:1 fixture rate (in the buy token's own atoms): a reproducible price with no
                // live solver. `apply_slippage` then floors `buy_amount_min` just below this.
                buy_amount: gross,
                valid_to,
                fee_amount: fee,
            },
            from: Some(req.from),
            expiration: None,
            id: None,
            verified: Some(false),
        }
    }

    /// Simulate a fill on the fork: credit the BUY token to `order.receiver` via `anvil_setStorageAt`
    /// at the token's `balanceOf` slot (KNOWN demo tokens only — see [`balances_slot`]), then return
    /// a synthetic accepted uid. Returns `(synthetic_uid, credited)`:
    /// - `credited == true` only when the buy token's slot is known AND both the read + write
    ///   JSON-RPC calls to the fork succeed (the new balance is the prior balance + `buy_amount`).
    /// - `credited == false` for an unknown buy token, a missing fill RPC (`fork_rpc == None`), OR
    ///   any fill-RPC failure (no anvil reachable, bad URL, non-2xx). The error is SWALLOWED into
    ///   `false` — NEVER propagated as a swap failure and NEVER turned into a fabricated verified
    ///   success. A `0x`-hex uid is always returned so the response looks like a real accept+open
    ///   and the caller can add an honest "balance not credited" note.
    pub async fn submit(
        client: &reqwest::Client,
        fork_rpc: Option<&str>,
        order: &OrderCreation,
    ) -> anyhow::Result<(String, bool)> {
        let uid = synthetic_uid(order);
        let credited = match (fork_rpc, balances_slot(order.buy_token)) {
            (Some(rpc), Some(slot)) => credit_buy_token(client, rpc, order, slot)
                .await
                .unwrap_or(false),
            _ => false,
        };
        Ok((uid, credited))
    }

    /// Best-effort: read the receiver's current buy-token balance on the fork, then set it to
    /// `current + buy_amount` via `anvil_setStorageAt`. Returns `Ok(true)` only on a clean
    /// round-trip; any transport/RPC error becomes `Ok(false)` at the [`submit`] call site (so the
    /// swap is never failed by an unreachable fork). Adds to the existing balance rather than
    /// clobbering, so a pre-seeded balance is preserved.
    async fn credit_buy_token(
        client: &reqwest::Client,
        fork_rpc: &str,
        order: &OrderCreation,
        slot: U256,
    ) -> anyhow::Result<bool> {
        let key = balances_storage_key(order.receiver, slot);
        let current = read_balance(client, fork_rpc, order.buy_token, key).await?;
        let credited = current.saturating_add(order.buy_amount);
        let value = format!("0x{}", alloy::hex::encode(credited.to_be_bytes::<32>()));
        let token = order.buy_token.to_string();
        let key_hex = key.to_string();
        let params = serde_json::json!([token, key_hex, value]);
        // The write's `result` (`null` on anvil) is uninteresting; we only need it to have succeeded.
        rpc_call(client, fork_rpc, "anvil_setStorageAt", params).await?;
        Ok(true)
    }

    /// Read the 32-byte storage word at `key` for `token` via `eth_getStorageAt` (the receiver's
    /// raw balance slot). Returns the value as a `U256`; a missing/short hex word reads as zero.
    async fn read_balance(
        client: &reqwest::Client,
        fork_rpc: &str,
        token: Address,
        key: B256,
    ) -> anyhow::Result<U256> {
        let params = serde_json::json!([token.to_string(), key.to_string(), "latest"]);
        let result = rpc_call(client, fork_rpc, "eth_getStorageAt", params).await?;
        let hex = result.as_str().unwrap_or("0x0");
        Ok(U256::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(U256::ZERO))
    }

    /// One JSON-RPC round-trip to the fork. Returns the `result` value; a transport error, a
    /// non-2xx status, an undecodable body, or a JSON-RPC `error` member all surface as
    /// `anyhow::Error` (which [`submit`] swallows into `credited = false`).
    async fn rpc_call(
        client: &reqwest::Client,
        fork_rpc: &str,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let resp = client
            .post(fork_rpc)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("fork rpc {method} transport error: {e}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("fork rpc {method} http {}", resp.status().as_u16());
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("fork rpc {method} decode error: {e}"))?;
        if let Some(err) = value.get("error") {
            anyhow::bail!("fork rpc {method} error: {err}");
        }
        Ok(value
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use alloy::primitives::address;

        /// The stub quote builds a valid order: gross sell preserved through the
        /// after-fee+fee split, and a non-zero `buy_amount_min`.
        #[test]
        fn stub_quote_builds_a_valid_order() {
            let weth = address!("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14");
            let cow = address!("0x0625aFB445C3B6B7B929342a04A22599fd5dBB59");
            let wallet = address!("0x1111111111111111111111111111111111111111");
            let gross = U256::from(50_000_000_000_000_000u64); // 0.05 WETH
            let req = QuoteRequest::sell(weth, cow, wallet, gross, 1800);

            let quote = quote(&req);
            assert_eq!(quote.id, None);
            // Invariant the builder relies on: after_fee + fee == requested gross.
            assert_eq!(quote.quote.sell_amount + quote.quote.fee_amount, gross);

            let order =
                swap_order_from_quote(&quote, 11155111, wallet, wallet, DEFAULT_SLIPPAGE_BPS);
            assert_eq!(order.sell_amount, gross, "order sells the requested gross");
            assert!(
                order.buy_amount_min > U256::ZERO,
                "a stubbed quote prices a non-zero buy_amount_min"
            );
            assert_eq!(order.sell_token, weth);
            assert_eq!(order.buy_token, cow);
            // `valid_to` is an ABSOLUTE forward timestamp (now + valid_for), never the raw
            // relative `valid_for` (which would date the order to 1970).
            assert!(
                order.valid_to > 1_700_000_000,
                "stub valid_to is a forward absolute Unix timestamp, got {}",
                order.valid_to
            );
        }

        /// `balances_slot` knows WETH (slot 3) and honestly returns `None` for everything else.
        #[test]
        fn balances_slot_known_and_unknown() {
            let weth = address!("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14");
            assert_eq!(balances_slot(weth), Some(U256::from(3u8)));
            // The const must equal the curated Sepolia WETH address.
            assert_eq!(SEPOLIA_WETH, weth);
            // Other Sepolia demo tokens have unverified slots → None (no guessing).
            for unknown in [
                address!("0x0625aFB445C3B6B7B929342a04A22599fd5dBB59"), // COW
                address!("0xbe72E441BF55620febc26715db68d3494213D8Cb"), // test USDC
                address!("0xd3f3d46FeBCD4CdAa2B83799b7A5CdcB69d135De"), // GNO
                Address::ZERO,
            ] {
                assert_eq!(balances_slot(unknown), None);
            }
        }

        /// Storage-key derivation matches the canonical `keccak256(pad32(holder) ‖ uint256(slot))`
        /// against an independently-recomputed vector (no indexing in the impl).
        #[test]
        fn storage_key_matches_keccak_vector() {
            let holder = address!("0x1111111111111111111111111111111111111111");
            let slot = U256::from(3u8);
            // Recompute the 64-byte preimage by hand and hash it.
            let mut preimage = [0u8; 64];
            preimage[12..32].copy_from_slice(holder.as_slice());
            preimage[32..64].copy_from_slice(&slot.to_be_bytes::<32>());
            let expected = keccak256(preimage);
            assert_eq!(balances_storage_key(holder, slot), expected);
            // A different holder yields a different key (the holder is part of the preimage).
            let other = address!("0x2222222222222222222222222222222222222222");
            assert_ne!(balances_storage_key(other, slot), expected);
        }

        /// The synthetic uid is a 0x-hex, 56-byte (112 hex chars) CoW-shaped value and is
        /// deterministic for a given order.
        #[test]
        fn synthetic_uid_is_deterministic_and_cow_shaped() {
            let weth = address!("0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14");
            let cow = address!("0x0625aFB445C3B6B7B929342a04A22599fd5dBB59");
            let wallet = address!("0x1111111111111111111111111111111111111111");
            let order = OrderCreation::from_signed_order(
                &SwapOrder {
                    chain_id: 11155111,
                    owner: wallet,
                    sell_token: weth,
                    buy_token: cow,
                    sell_amount: U256::from(1_000_000u64),
                    buy_amount_min: U256::from(990_000u64),
                    receiver: wallet,
                    valid_to: 1_700_000_000,
                    app_data: APP_DATA_HASH,
                },
                Bytes::from(vec![0xCDu8; 65]),
                None,
            );
            let uid = synthetic_uid(&order);
            assert!(uid.starts_with("0x"));
            assert_eq!(uid.len(), 2 + 56 * 2, "56-byte CoW uid is 112 hex chars");
            assert_eq!(
                synthetic_uid(&order),
                uid,
                "deterministic for the same order"
            );
        }
    }
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

    /// `is_simulated()` is wired to the `DECKARD_DEMO_SWAP_STUB` knob: it agrees with
    /// [`crate::env::demo_swap_stub`] exactly. The test process does not set the knob, so the
    /// production-safe default is OFF — proving the stub never activates without an explicit
    /// opt-in. (The knob's pure parsing is unit-tested in `env.rs`; this pins the routing.)
    #[test]
    fn is_simulated_reflects_the_demo_swap_stub_knob() {
        let ob = CowOrderbook::new();
        assert_eq!(ob.is_simulated(), crate::env::demo_swap_stub());
        // Unset in the test process → off (fail-safe).
        assert!(
            !ob.is_simulated(),
            "the demo swap stub must be OFF unless DECKARD_DEMO_SWAP_STUB is set"
        );
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
