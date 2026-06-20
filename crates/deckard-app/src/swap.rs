//! swap — the CoW Protocol swap path's pure helpers + the off-thread orchestrator that turns a
//! compose-screen snapshot into an ACCEPTED + OPEN order on the CoW orderbook (#25). All
//! snapshot-based: NOT a single function takes `&Shell` (codex must-do #5 — `Shell` isn't `Send`
//! across a spawn). The signing/approval helpers it calls live in [`crate::signer`] (key-less
//! wire-only); this module owns the *sequence* (propose-order → allowance → exact-gross approve →
//! resolve+sign → put-app-data → submit) and the quote↔order mapping.
//!
//! The orchestrator [`confirm_swap_blocking`] is fully synchronous: the orderbook HTTP goes through
//! deckard-core's `CowOrderbook::*_blocking` wrappers (which own a tokio runtime — the GPUI app
//! itself never touches tokio), the allowance read blocks on the `EthProvider` worker, and the
//! signer calls are already `*_blocking`. The caller (the shell) MUST run it inside
//! `cx.background_spawn` so it blocks the spawned background task, never the UI thread.

use alloy_primitives::{Address, U256};
use deckard_contract::{Decision, ExecuteResult, SwapOrder};
use deckard_core::{
    cow_api_base, swap_order_from_quote, tokens_for, CowError, CowOrderbook, EthProvider,
    OrderCreation, QuoteRequest, QuoteResponse, APP_DATA_DOC, DEFAULT_SLIPPAGE_BPS,
    GPV2_VAULT_RELAYER,
};
use deckard_signerd::{ControlChannel, SignerClient};

use crate::signer;

/// How long a quote stays valid, in seconds (30 min). Sits comfortably inside the daemon's 24h
/// `valid_to` horizon (the `valid_to_too_far` policy gate) AND gives the user room to read the
/// review card before the order's `validTo` lapses. `confirm_swap_blocking` re-quotes at confirm time
/// regardless (codex must-do #4), so this is the budget for the WHOLE compose→confirm window.
pub const QUOTE_VALID_FOR: u32 = 1800;

/// Build the orderbook quote request for a sell order: `sell_wei` of `sell` into `buy`, quoted for
/// the caller's `wallet`, valid for [`QUOTE_VALID_FOR`]. A thin, intention-revealing wrapper over
/// [`QuoteRequest::sell`] so the compose screen and the confirm-time re-quote build the SAME shape.
pub fn quote_request(sell: Address, buy: Address, wallet: Address, sell_wei: U256) -> QuoteRequest {
    QuoteRequest::sell(sell, buy, wallet, sell_wei, QUOTE_VALID_FOR)
}

/// Map a fetched quote into a signable [`SwapOrder`], binding BOTH owner and receiver to the
/// wallet (the receiver is always your own wallet in v1 — you never swap funds to a third party)
/// and applying the default 0.5% slippage floor. Delegates to [`swap_order_from_quote`], which
/// carries the GROSS sell amount (`quote.sellAmount + feeAmount`) and a `feeAmount` of 0 (CoW's
/// surplus-fee model). The daemon rebinds owner = wallet before hashing, so deriving the request
/// id requires re-binding via [`signer::bind_swap_order`] first.
pub fn order_from_quote(quote: &QuoteResponse, chain_id: u64, wallet: Address) -> SwapOrder {
    swap_order_from_quote(quote, chain_id, wallet, wallet, DEFAULT_SLIPPAGE_BPS)
}

/// The exact-gross sell amount the vault relayer must be allowed to pull — `order.sell_amount`,
/// which is the GROSS `quote.sellAmount + feeAmount` (already computed by core). This IS the exact
/// approve amount (codex must-do #1): the on-chain allowance must cover the full amount the relayer
/// moves, never the after-fee figure.
pub fn gross_sell_amount(order: &SwapOrder) -> U256 {
    order.sell_amount
}

/// True when the wallet must approve the vault relayer before this order can settle: the current
/// `allowance` is short of the order's GROSS sell amount. An exact-equal allowance is sufficient
/// (`allowance == gross → false`), so the exact-gross approve is never over-issued.
pub fn needs_approval(allowance: U256, gross: U256) -> bool {
    allowance < gross
}

/// The minimum the wallet receives if the order fills — `order.buy_amount_min`, the quoted
/// `buyAmount` with the slippage floor already applied. A worse price never settles; this is the
/// floor the review card shows and the order is signed against. (Kept + unit-tested as the symmetric
/// pair to `gross_sell_amount`; the live review derives the same value from the quote + slippage.)
#[allow(dead_code)]
pub fn min_receive(order: &SwapOrder) -> U256 {
    order.buy_amount_min
}

/// The ERC-20 decimals for a token on `chain_id`, from the curated [`tokens_for`] list. Defaults to
/// 18 when the address isn't listed, so a swap summary never mis-scales an amount (a wrong-decimals
/// display is worse than a generic 18-place one). NOTE: the Sepolia test-USDC is an 18-decimals
/// mock, NOT mainnet's 6 — `tokens_for` already pins that.
pub fn token_decimals(chain_id: u64, token: Address) -> u8 {
    tokens_for(chain_id)
        .iter()
        .find(|t| t.address == token)
        .map(|t| t.decimals)
        .unwrap_or(18)
}

/// The ticker for a token on `chain_id`, from the curated [`tokens_for`] list. Empty string when
/// the address isn't listed (the summary then shows the amount alone rather than a wrong symbol).
pub fn token_symbol(chain_id: u64, token: Address) -> &'static str {
    tokens_for(chain_id)
        .iter()
        .find(|t| t.address == token)
        .map(|t| t.symbol)
        .unwrap_or("")
}

/// The compose-screen snapshot the orchestrator confirms against — every value captured at confirm
/// time, NOT a borrow of `Shell` (which isn't `Send` across a spawn). The shell builds this from
/// its swap fields just before calling [`confirm_swap_blocking`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapInputs {
    pub chain_id: u64,
    pub wallet: Address,
    pub sell_token: Address,
    pub buy_token: Address,
    /// Gross sell amount in the sell token's atoms (wei for an 18-decimals token).
    pub sell_wei: U256,
}

/// The terminal outcome of a confirm: a submitted uid (the order is now on the orderbook), or a
/// human-readable denial. Every error path resolves to one of these or an `Err` — the shell renders
/// `Denied { reason }` inline (never "swap failed").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwapConfirmOutcome {
    /// The order was accepted by the orderbook; `uid` is its 0x-hex order uid.
    Submitted { uid: String },
    /// A daemon deny, an orderbook rejection, or a re-quote failure — already turned into a calm,
    /// swap-specific line.
    Denied { reason: String },
}

/// How long the confirm waits for the exact-gross approve to land on-chain before giving up and
/// asking the user to hold again. ~16 checks × 2s ≈ 32s covers a Sepolia/mainnet block or two; on
/// a local auto-mining fork the very first check already passes (the loop checks BEFORE it sleeps),
/// so fast chains pay no wait. This is what makes a swap a SINGLE hold on a real network: the first
/// hold broadcasts the approve AND waits for it to confirm AND submits the order.
const APPROVE_CONFIRM_POLLS: u32 = 16;
const APPROVE_CONFIRM_POLL_SECS: u64 = 2;

/// Confirm a reviewed swap, off-thread: re-quote, propose the order, approve the vault relayer for
/// the exact gross IF the allowance is short, resolve+sign over the PRIVATE control channel, then
/// submit the signed order to the orderbook. Returns the order uid on success.
///
/// EXACT sequence (codex must-dos #1, #2, #4) — the ordering is load-bearing:
/// 1. **Re-quote** at confirm time: `quote.validTo` and the daemon's approval TTL are independent
///    races, so a quote fetched on the compose screen may have lapsed. A signed order built off a
///    stale quote is rejected `EXPIRED`/`InvalidSignature`; re-quoting here avoids that.
/// 2. Map the fresh quote → a `SwapOrder` (owner = receiver = wallet), bind it the way the daemon
///    will, and derive the request id.
/// 3. **`propose_order` FIRST** — it must answer `NeedsApproval` (swaps never auto-allow in v1); a
///    `Deny` short-circuits to `Denied`. Proposing the order before the approve is mandatory: the
///    daemon admits the shaped approve ONLY when a matching pending order already exists
///    (`approve_no_matching_order` otherwise).
/// 4. Read the relayer **allowance**; if short of the gross, build the **exact-gross** approve,
///    propose it (now admitted, because step 3 left the order pending), then resolve+execute it
///    over the control/public split and wait for the broadcast.
/// 5. **Resolve+sign** the order over the control channel (the completed hold IS the approval) →
///    the 65-byte EIP-712 signature.
/// 6. Register the app-data doc, then **submit** the signed order → the uid.
///
/// Typed orderbook errors (`EXPIRED` / `NoLiquidity` / `InsufficientBalance` / `InvalidSignature`)
/// and daemon denies surface as distinct `Denied { reason }` copy, never a bare "swap failed". A
/// session-ended deny (`locked`/`revoked`) is returned verbatim so the shell caller can detect it
/// via `errors::is_session_ended` and bounce to the unlock gate.
///
/// Mixes async (the orderbook + the allowance read) with the daemon's `*_blocking` signer calls;
/// run it inside `cx.background_spawn` so the blocking calls block the spawned task, not the UI.
pub fn confirm_swap_blocking(
    ob: &CowOrderbook,
    eth: &EthProvider,
    client: &SignerClient,
    control: &ControlChannel,
    base: &'static str,
    inputs: SwapInputs,
) -> anyhow::Result<SwapConfirmOutcome> {
    let SwapInputs {
        chain_id,
        wallet,
        sell_token,
        buy_token,
        sell_wei,
    } = inputs;

    // (1) Re-quote at confirm time — the compose-screen quote may have lapsed.
    let quote = match ob.quote_blocking(
        base,
        &quote_request(sell_token, buy_token, wallet, sell_wei),
    ) {
        Ok(q) => q,
        Err(e) => return Ok(deny_from_cow(&e, "couldn't re-price the swap")),
    };

    // (2) Map → order, bind it the way the daemon will, derive the matching request id.
    let order = order_from_quote(&quote, chain_id, wallet);
    let bound = signer::bind_swap_order(&order, wallet);
    let id = SignerClient::request_id_for_swap_order(&bound);
    let gross = gross_sell_amount(&order);

    // (3) Propose the order FIRST. A valid order is always NeedsApproval; a Deny is terminal. The
    //     pending order is also what admits the exact-gross approve in step 4. App-origin: this is
    //     the user's foreground GUI swap, so the feed labels it "You", not "Atlas".
    match client.propose_order_blocking(&order, deckard_contract::ProposalOrigin::App)? {
        Decision::NeedsApproval { .. } => {}
        Decision::Allow => {
            // v1 swaps never auto-allow; treat an unexpected Allow as a refusal rather than signing
            // an order the daemon didn't gate behind the hold.
            return Ok(SwapConfirmOutcome::Denied {
                reason: "the signer didn't require approval for this swap. Review again.".into(),
            });
        }
        Decision::Deny { reason } => return Ok(SwapConfirmOutcome::Denied { reason }),
    }

    // (4) Allowance check. The vault relayer must be allowed to pull the GROSS sell amount; if it
    //     can't, issue the exact-gross approve (admitted only because the order is now pending).
    let allowance = eth
        .allowance(wallet, GPV2_VAULT_RELAYER, sell_token)
        .recv()
        .map_err(|_| anyhow::anyhow!("network worker stopped"))??;
    if needs_approval(allowance, gross) {
        let approve = signer::build_exact_approve_intent(chain_id, sell_token, gross);
        let approve_id = SignerClient::request_id_for_intent(&approve);
        // The user's foreground swap from the app → App-origin (the wire requires origin).
        match client.propose_blocking(&approve, deckard_contract::ProposalOrigin::App)? {
            // The approve is shaped to be NeedsApproval (the completed hold authorizes it). An
            // Allow is fine too — both reach `approve_and_execute_blocking` below; only a Deny
            // short-circuits.
            Decision::NeedsApproval { .. } | Decision::Allow => {}
            Decision::Deny { reason } => return Ok(SwapConfirmOutcome::Denied { reason }),
        }
        match signer::approve_and_execute_blocking(client, control, approve_id, true)? {
            ExecuteResult::Broadcast { .. } => {}
            ExecuteResult::Denied { reason } => return Ok(SwapConfirmOutcome::Denied { reason }),
        }
        // The approve tx must be ON-CHAIN before we submit the order, or the orderbook rejects it
        // with InsufficientAllowance. Poll the relayer allowance until it covers the gross so the
        // swap submits in the SAME hold — one gesture, not two. The loop checks BEFORE each sleep,
        // so a local auto-mining fork (and an already-sufficient allowance) returns immediately; on
        // a public network the approve confirms in ~one block and we wait a bounded ~32s. A timeout
        // returns the honest "hold again" line rather than a confusing orderbook InsufficientAllowance.
        let mut approved = false;
        for attempt in 0..APPROVE_CONFIRM_POLLS {
            let confirmed = eth
                .allowance(wallet, GPV2_VAULT_RELAYER, sell_token)
                .recv()
                .map_err(|_| anyhow::anyhow!("network worker stopped"))??;
            if !needs_approval(confirmed, gross) {
                approved = true;
                break;
            }
            if attempt + 1 < APPROVE_CONFIRM_POLLS {
                std::thread::sleep(std::time::Duration::from_secs(APPROVE_CONFIRM_POLL_SECS));
            }
        }
        if !approved {
            return Ok(SwapConfirmOutcome::Denied {
                reason:
                    "the token approval is still confirming on-chain. Give it a few seconds, then hold to swap again."
                        .into(),
            });
        }
    }

    // (5) Resolve over the control channel (the hold IS the approval), then sign the order.
    let signature = match signer::sign_and_resolve_blocking(client, control, id) {
        Ok(sig) => sig,
        Err(e) => return Ok(SwapConfirmOutcome::Denied { reason: short(&e) }),
    };

    // (6) Register the app-data doc, then submit the signed order → its uid.
    if let Err(e) = ob.put_app_data_blocking(base, APP_DATA_DOC) {
        return Ok(deny_from_cow(&e, "couldn't register the order's app-data"));
    }
    let creation = OrderCreation::from_signed_order(&bound, signature, quote.id);
    match ob.submit_blocking(base, &creation) {
        Ok(uid) => Ok(SwapConfirmOutcome::Submitted { uid }),
        Err(e) => Ok(deny_from_cow(&e, "the orderbook rejected the order")),
    }
}

/// The orderbook REST base for a chain, or `None` for an unsupported chain. Re-exported convenience
/// so the shell builds the `&'static str` base it passes to [`confirm_swap_blocking`] from one place.
pub fn orderbook_base(chain_id: u64) -> Option<&'static str> {
    cow_api_base(chain_id)
}

/// Turn a compose-time quote failure into a calm, swap-specific line for the compose screen — the
/// same honest CoW `errorType` mapping `confirm_swap_blocking` uses, never a generic "couldn't quote".
/// (Distinct from the confirm path: this is indicative pricing, so "no route" / "try again" reads
/// right here too.)
pub fn humanize_quote_error(e: &anyhow::Error) -> String {
    match deny_from_cow(e, "couldn't price the swap") {
        SwapConfirmOutcome::Denied { reason } => reason,
        // `deny_from_cow` only ever returns `Denied`, but be total rather than panic.
        SwapConfirmOutcome::Submitted { .. } => "couldn't price the swap".into(),
    }
}

/// Turn an `anyhow`-wrapped orderbook error into a distinct, swap-specific `Denied` line. Downcasts
/// to the typed [`CowError`] so the well-known `errorType`s read honestly (codex must-do #4); an
/// un-typed transport/decode error falls back to `context` plus the trimmed message.
fn deny_from_cow(e: &anyhow::Error, context: &str) -> SwapConfirmOutcome {
    let reason = match e.downcast_ref::<CowError>() {
        Some(CowError::Api { error_type, .. }) => humanize_cow_api(error_type),
        Some(CowError::Http { status, .. }) => {
            format!("{context}: the orderbook returned HTTP {status}")
        }
        Some(CowError::Decode(_)) => {
            format!("{context}: the orderbook sent an unexpected response")
        }
        Some(CowError::Transport(_)) => {
            format!("{context}: check your network and try again")
        }
        None => format!("{context}: {}", short(e)),
    };
    SwapConfirmOutcome::Denied { reason }
}

/// Map a CoW orderbook `errorType` to a calm, swap-specific line. The well-known rejection types
/// each get distinct copy (never a generic "swap failed"); an unrecognised type falls through with
/// its raw tag so a new orderbook error isn't silently swallowed.
fn humanize_cow_api(error_type: &str) -> String {
    match error_type {
        // The quote/order lapsed between pricing and submit — the user can simply try again.
        "OrderExpired" | "Expired" | "EXPIRED" => {
            "the price quote expired before the order was placed. Try the swap again.".into()
        }
        // No solver route at any price for this pair/size.
        "NoLiquidity" => {
            "there's no route to swap these tokens right now. Try a different pair or amount."
                .into()
        }
        // The wallet doesn't hold enough of the sell token (or hasn't approved the relayer).
        "InsufficientBalance" => {
            "your wallet doesn't have enough of the sell token for this swap".into()
        }
        // The EIP-712 signature didn't validate against the submitted order (usually a stale quote).
        "InvalidSignature" => {
            "the order signature didn't validate. Re-quote and try the swap again.".into()
        }
        // Allowance shortfall the orderbook caught (we approve the exact gross, so this is rare).
        "InsufficientAllowance" => {
            "the vault relayer isn't approved to move enough of the sell token. Try again.".into()
        }
        // A duplicate of an already-placed order.
        "DuplicatedOrder" => "this exact order is already on the orderbook".into(),
        other => format!("the orderbook rejected the order ({other})"),
    }
}

/// One short line from an error (first line, trimmed, capped). Local copy mirroring
/// `errors::short_err` so the orchestrator doesn't depend on the shell's error module.
fn short(e: &anyhow::Error) -> String {
    let line = e.to_string();
    let line = line.lines().next().unwrap_or("").trim();
    line.chars().take(140).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_core::{apply_slippage, QuoteOrderParameters};

    /// A curated Sepolia token address by symbol, sourced from `tokens_for(11155111)` so the tests
    /// stay in lock-step with the real list (and never depend on the `address!` macro / a hard-coded
    /// checksum that could drift from `tokens.rs`).
    fn sepolia_token(symbol: &str) -> Address {
        tokens_for(11155111)
            .iter()
            .find(|t| t.symbol == symbol)
            .map(|t| t.address)
            .unwrap_or_else(|| panic!("Sepolia token {symbol} missing from tokens_for(11155111)"))
    }

    /// A throwaway wallet address for the binding tests.
    fn wallet_addr() -> Address {
        Address::repeat_byte(0x11)
    }

    /// The live-Sepolia fee vector reused across the swap tests: a 0.05 WETH gross broken into the
    /// orderbook's after-fee + fee split. `after_fee + fee == 0.05e18` (the requested
    /// `sellAmountBeforeFee`). Mirrors the vector in `signer.rs` + `cow_client.rs`.
    fn sepolia_quote() -> QuoteResponse {
        let after_fee = U256::from(37_989_365_556_267_132u64);
        let fee = U256::from(12_010_634_443_732_868u64);
        let buy = U256::from(1_953_742_300_219_817_002u64);
        QuoteResponse {
            quote: QuoteOrderParameters {
                sell_token: sepolia_token("WETH"),
                buy_token: sepolia_token("COW"),
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
        }
    }

    /// `needs_approval` boundary: an exact-equal allowance is sufficient (the exact-gross approve is
    /// never over-issued), short → true, over → false.
    #[test]
    fn needs_approval_treats_exact_allowance_as_sufficient() {
        let gross = U256::from(50_000_000_000_000_000u64);
        // allowance == gross → no approve needed.
        assert!(!needs_approval(gross, gross));
        // allowance < gross → approve needed.
        assert!(needs_approval(gross - U256::from(1u64), gross));
        assert!(needs_approval(U256::ZERO, gross));
        // allowance > gross → no approve needed.
        assert!(!needs_approval(gross + U256::from(1u64), gross));
        // gross == 0 (degenerate) → never needs approval.
        assert!(!needs_approval(U256::ZERO, U256::ZERO));
    }

    /// `order_from_quote` pins owner == receiver == wallet and carries the GROSS sell amount
    /// (`quote.sellAmount + feeAmount`, == the requested `sellAmountBeforeFee`), with the slippage
    /// floor applied to the buy side.
    #[test]
    fn order_from_quote_binds_wallet_and_carries_gross() {
        let wallet = wallet_addr();
        let quote = sepolia_quote();
        let order = order_from_quote(&quote, 11155111, wallet);

        assert_eq!(order.chain_id, 11155111);
        assert_eq!(order.owner, wallet, "owner is bound to the wallet");
        assert_eq!(order.receiver, wallet, "receiver is always your own wallet");
        // GROSS = after-fee 37989365556267132 + fee 12010634443732868 == 0.05e18.
        let gross = U256::from(50_000_000_000_000_000u64);
        assert_eq!(order.sell_amount, gross, "the order sells the GROSS amount");
        assert_eq!(gross_sell_amount(&order), gross);
        // The buy floor is the quoted buyAmount minus the default 0.5% slippage.
        assert_eq!(
            order.buy_amount_min,
            apply_slippage(quote.quote.buy_amount, DEFAULT_SLIPPAGE_BPS)
        );
        assert_eq!(min_receive(&order), order.buy_amount_min);
    }

    /// `quote_request` requests the gross `sellAmountBeforeFee` as a decimal string and pins
    /// `validFor == QUOTE_VALID_FOR` (1800s). Mirrors `cow_client`'s
    /// `quote_request_serializes_decimal_amount`.
    #[test]
    fn quote_request_pins_valid_for_and_serializes_decimal() {
        assert_eq!(QUOTE_VALID_FOR, 1800);
        let req = quote_request(
            sepolia_token("WETH"),
            sepolia_token("COW"),
            wallet_addr(),
            U256::from(50_000_000_000_000_000u64),
        );
        assert_eq!(req.valid_for, QUOTE_VALID_FOR);
        let json = serde_json::to_string(&req).expect("serialize quote request");
        assert!(
            json.contains("\"sellAmountBeforeFee\":\"50000000000000000\""),
            "gross amount serializes as a decimal string: {json}"
        );
        assert!(json.contains("\"validFor\":1800"), "got: {json}");
        assert!(json.contains("\"kind\":\"sell\""));
    }

    /// `token_decimals`/`token_symbol` over the Sepolia (11155111) curated list: the test-USDC is
    /// 18 decimals (NOT mainnet's 6), GNO + WETH are 18, and an unknown address defaults to 18 / ""
    /// rather than panicking.
    #[test]
    fn token_lookups_use_sepolia_curated_list() {
        let usdc = sepolia_token("USDC");
        let gno = sepolia_token("GNO");
        let weth = sepolia_token("WETH");
        // Sepolia test-USDC is an 18-decimals mock — using 6 would misprice by 10^12.
        assert_eq!(token_decimals(11155111, usdc), 18);
        assert_eq!(token_symbol(11155111, usdc), "USDC");
        assert_eq!(token_decimals(11155111, gno), 18);
        assert_eq!(token_symbol(11155111, gno), "GNO");
        assert_eq!(token_decimals(11155111, weth), 18);
        assert_eq!(token_symbol(11155111, weth), "WETH");
        // An unlisted address: safe defaults, never a panic.
        let unknown = Address::repeat_byte(0xAB);
        assert_eq!(token_decimals(11155111, unknown), 18);
        assert_eq!(token_symbol(11155111, unknown), "");
        // An unsupported chain has no list at all → defaults too.
        assert_eq!(token_decimals(31337, weth), 18);
        assert_eq!(token_symbol(31337, weth), "");
    }

    /// `gross_sell_amount` is the order's `sell_amount` and `min_receive` is its `buy_amount_min` —
    /// the exact-approve figure and the displayed floor agree with the signed order byte-for-byte.
    #[test]
    fn gross_and_min_receive_track_the_order() {
        let order = SwapOrder {
            chain_id: 11155111,
            owner: Address::repeat_byte(0x11),
            sell_token: Address::repeat_byte(0x55),
            buy_token: Address::repeat_byte(0x66),
            sell_amount: U256::from(50_000_000_000_000_000u64),
            buy_amount_min: U256::from(1_944_000_000_000_000_000u64),
            receiver: Address::repeat_byte(0x11),
            valid_to: 1_781_261_340,
            app_data: deckard_core::APP_DATA_HASH,
        };
        assert_eq!(gross_sell_amount(&order), order.sell_amount);
        assert_eq!(min_receive(&order), order.buy_amount_min);
    }

    /// `orderbook_base` resolves the supported chains and refuses the rest (so the shell can gate
    /// the swap on a real orderbook base).
    #[test]
    fn orderbook_base_known_chains() {
        assert_eq!(orderbook_base(1), Some("https://api.cow.fi/mainnet"));
        assert_eq!(orderbook_base(11155111), Some("https://api.cow.fi/sepolia"));
        assert_eq!(orderbook_base(31337), None);
    }

    /// `deny_from_cow` maps each well-known orderbook `errorType` to DISTINCT copy and never emits a
    /// bare "swap failed". The typed `CowError` is recovered through the `anyhow` wrapper.
    #[test]
    fn cow_errors_map_to_distinct_inline_copy() {
        let expired = anyhow::Error::new(CowError::Api {
            error_type: "OrderExpired".into(),
            description: "expired".into(),
        });
        let no_liq = anyhow::Error::new(CowError::Api {
            error_type: "NoLiquidity".into(),
            description: "no route".into(),
        });
        let bad_sig = anyhow::Error::new(CowError::Api {
            error_type: "InvalidSignature".into(),
            description: "bad sig".into(),
        });
        let low_bal = anyhow::Error::new(CowError::Api {
            error_type: "InsufficientBalance".into(),
            description: "broke".into(),
        });

        let lines: Vec<String> = [&expired, &no_liq, &bad_sig, &low_bal]
            .iter()
            .map(|e| match deny_from_cow(e, "ctx") {
                SwapConfirmOutcome::Denied { reason } => reason,
                other => panic!("expected Denied, got {other:?}"),
            })
            .collect();

        // Each line is distinct and honest (no generic "swap failed").
        for line in &lines {
            assert!(!line.is_empty());
            assert!(
                !line.to_lowercase().contains("swap failed"),
                "must not be a generic failure: {line}"
            );
        }
        assert!(lines[0].contains("expired"), "expired copy: {}", lines[0]);
        assert!(
            lines[1].contains("route"),
            "no-liquidity copy: {}",
            lines[1]
        );
        assert!(
            lines[2].contains("signature"),
            "invalid-signature copy: {}",
            lines[2]
        );
        assert!(
            lines[3].contains("enough"),
            "insufficient-balance copy: {}",
            lines[3]
        );

        // An unrecognised errorType falls through carrying its raw tag (not swallowed).
        let novel = anyhow::Error::new(CowError::Api {
            error_type: "BrandNewRejection".into(),
            description: "x".into(),
        });
        match deny_from_cow(&novel, "ctx") {
            SwapConfirmOutcome::Denied { reason } => {
                assert!(reason.contains("BrandNewRejection"), "got: {reason}")
            }
            other => panic!("expected Denied, got {other:?}"),
        }

        // A transport error falls back to the context line, never a panic.
        let transport = anyhow::Error::new(CowError::Transport("dns".into()));
        match deny_from_cow(&transport, "couldn't re-price the swap") {
            SwapConfirmOutcome::Denied { reason } => {
                assert!(
                    reason.contains("couldn't re-price the swap"),
                    "got: {reason}"
                )
            }
            other => panic!("expected Denied, got {other:?}"),
        }
    }
}
