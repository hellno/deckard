//! The key-less core both surfaces (CLI + MCP tools) share: one [`SignerClient`] to the
//! daemon socket, the expected chain, and the nine operations. **No key material ever enters
//! this process** — writes are `Intent`s (or shaped CoW `SwapOrder`s) proposed to
//! `deckard-signerd`, which enforces policy and signs. The one secret this sidecar transiently
//! handles is the Railgun
//! *viewing* key (it rides alongside the wallet's own 0zk address in `RailgunViewGrant`);
//! it is moved into `Zeroizing` on receipt, never logged, and never put in any response.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use alloy_primitives::{Address, Bytes, B256, U256};
use serde_json::json;
use zeroize::Zeroizing;

use deckard_contract::{
    deny_reasons, ApprovalMode, Decision, ExecuteResult, Intent, IntentKind, PendingPayloadView,
    Policy, ReadStatus, SignOrderResult, SignerRequest, SignerResponse, SwapOrder,
};
use deckard_core::{
    cow_api_base, swap_order_from_quote, CowError, CowOrderbook, OrderCreation, QuoteRequest,
    APP_DATA_DOC, DEFAULT_SLIPPAGE_BPS,
};
use deckard_signerd::SignerClient;

use crate::amount::{format_wei_as_eth, parse_eth_to_wei};
use crate::failure::{self, Failure};

/// A successful tool/CLI outcome, rendered as JSON for the agent and lines for a human.
pub type OpResult = Result<serde_json::Value, Failure>;

/// The shared sidecar state.
pub struct Sidecar {
    client: SignerClient,
    /// The chain this sidecar builds intents for (`DECKARD_CHAIN_ID`, default 1 — matching
    /// the daemon's own default; `install --demo` pins 11155111 for both processes).
    chain_id: u64,
    /// `DECKARD_CONFIG_DIR` when set — used only to sharpen the `locked` error into the
    /// no-vault case. Never read for secrets.
    config_dir: Option<PathBuf>,
    /// Set once the connect-time chain probe has conclusively confirmed the daemon signs
    /// for [`Self::chain_id`] (so the probe runs at most once per process).
    chain_checked: AtomicBool,
    /// The CoW orderbook client the swap tools quote/submit through. It owns its own
    /// `reqwest::Client` (no key material) and routes through the demo-fork stub when
    /// `DECKARD_DEMO_SWAP_STUB` is set — see [`CowOrderbook::is_simulated`]. Built once
    /// (no network at construction).
    orderbook: CowOrderbook,
}

impl Sidecar {
    /// Resolve from the environment: socket path (`DECKARD_SOCKET_PATH` or the per-uid
    /// default), chain id (`DECKARD_CHAIN_ID`, default 1), optional config dir.
    pub fn from_env() -> anyhow::Result<Self> {
        let socket_path = match std::env::var_os("DECKARD_SOCKET_PATH") {
            Some(p) => PathBuf::from(p),
            None => deckard_signerd::socket::default_socket_path(),
        };
        let chain_id = match std::env::var("DECKARD_CHAIN_ID") {
            Ok(s) => s
                .trim()
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("DECKARD_CHAIN_ID must be a u64, got {s:?}"))?,
            Err(_) => 1,
        };
        let config_dir = std::env::var_os("DECKARD_CONFIG_DIR").map(PathBuf::from);
        Ok(Self {
            client: SignerClient::new(socket_path),
            chain_id,
            config_dir,
            chain_checked: AtomicBool::new(false),
            orderbook: CowOrderbook::new(),
        })
    }

    /// Test/builder constructor with explicit wiring.
    pub fn new(socket_path: PathBuf, chain_id: u64, config_dir: Option<PathBuf>) -> Self {
        Self {
            client: SignerClient::new(socket_path),
            chain_id,
            config_dir,
            chain_checked: AtomicBool::new(false),
            orderbook: CowOrderbook::new(),
        }
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn config_dir(&self) -> Option<&std::path::Path> {
        self.config_dir.as_deref()
    }

    /// One request → one response, with connect failures mapped to the catalog.
    async fn request(&self, req: &SignerRequest) -> Result<SignerResponse, Failure> {
        self.client
            .request(req)
            .await
            .map_err(|_| failure::socket_missing(self.client.path()))
    }

    /// Connect-time chain probe: confirm the daemon signs for our chain BEFORE building
    /// real intents, so a demo sidecar attached to the mainnet daemon (or vice versa)
    /// fails with an actionable error instead of a confusing deny later.
    ///
    /// The probe is a deliberately-undecodable `Send` (non-empty calldata): the daemon's
    /// `chain_mismatch` pre-check runs before the policy gate, and the policy gate's
    /// `undecodable` deny stores no pending record — so the probe is side-effect-free and
    /// can never be executed. A `locked` answer is now CONCLUSIVE for chain identity: the
    /// daemon checks `chain_mismatch` before `locked` (the chain check needs no key), so a
    /// wrong chain would have returned `chain_mismatch` first — a `locked` reply therefore
    /// implies the chain matched. We cache the probe success and let the real call surface
    /// its own locked error.
    async fn ensure_chain(&self) -> Result<(), Failure> {
        if self.chain_checked.load(Ordering::Relaxed) {
            return Ok(());
        }
        let probe = Intent {
            chain_id: self.chain_id,
            to: Address::ZERO,
            token: None,
            value: U256::ZERO,
            calldata: Bytes::from_static(&[0x00]), // undecodable for Send → never stored
            kind: IntentKind::Send,
        };
        match self
            .request(&SignerRequest::Propose { intent: probe })
            .await?
        {
            SignerResponse::Decision(Decision::Deny { reason })
                if reason == deny_reasons::CHAIN_MISMATCH =>
            {
                Err(failure::from_deny_reason(
                    deny_reasons::CHAIN_MISMATCH,
                    self.config_dir(),
                ))
            }
            SignerResponse::Decision(Decision::Deny { reason })
                if reason == deny_reasons::LOCKED =>
            {
                // Conclusive: the daemon checks chain BEFORE locked, so a `locked` reply
                // means the chain matched. Cache the success; the real call surfaces `locked`.
                self.chain_checked.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => {
                self.chain_checked.store(true, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    /// `deckard_wallet_address` / `deckard-mcp address`.
    pub async fn wallet_address(&self) -> OpResult {
        self.ensure_chain().await?;
        match self.request(&SignerRequest::Address).await? {
            SignerResponse::Address(addr) => Ok(json!({ "address": format!("{addr:#x}") })),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Address", &other)),
        }
    }

    /// `deckard_wallet_balance` / `deckard-mcp balance`. Public only in v0.1 — the
    /// shielded field is an honest "unavailable" string, never a fake 0.
    pub async fn wallet_balance(&self) -> OpResult {
        self.ensure_chain().await?;
        match self
            .request(&SignerRequest::Balance { shielded: false })
            .await?
        {
            SignerResponse::Balance(report) => Ok(json!({
                "public_wei": report.public_wei.to_string(),
                "public_eth": format_wei_as_eth(report.public_wei),
                "read_status": read_status_label(&report.read_status),
                "shielded": "unavailable — read it in the Deckard app (v1 limitation)",
            })),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Balance", &other)),
        }
    }

    /// `deckard_policy_get` / part of the CLI surface.
    pub async fn policy_get(&self) -> OpResult {
        self.ensure_chain().await?;
        match self.request(&SignerRequest::PolicyGet).await? {
            SignerResponse::Policy(p) => Ok(policy_json(&p)),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("PolicyGet", &other)),
        }
    }

    /// `deckard_shield` / `deckard-mcp shield --amount-eth`. Builds the key-less Railgun
    /// shield intent for the wallet's OWN 0zk address and proposes it — nothing is signed
    /// or broadcast here; `execute` does that with the returned request_id.
    pub async fn shield(&self, amount_eth: &str) -> OpResult {
        let wei = parse_eth_to_wei(amount_eth).map_err(|msg| {
            Failure::new(
                format!("could not parse amount_eth {amount_eth:?}"),
                msg,
                "pass a decimal ETH string like \"0.02\" (units: ETH, not wei)",
            )
        })?;
        if wei == U256::ZERO {
            return Err(Failure::new(
                "amount_eth is zero",
                "a zero-value shield creates no private note",
                "pass an amount greater than zero, e.g. \"0.02\"",
            ));
        }
        self.ensure_chain().await?;

        // Funded-balance pre-check (best effort): block only on a positive on-chain read
        // that's still below the ask — an unsynced 0 must not false-block a funded wallet.
        if let SignerResponse::Balance(report) = self
            .request(&SignerRequest::Balance { shielded: false })
            .await?
        {
            if report.public_wei > U256::ZERO && wei > report.public_wei {
                return Err(Failure::new(
                    format!(
                        "amount {amount_eth} ETH exceeds the public balance ({} ETH)",
                        format_wei_as_eth(report.public_wei)
                    ),
                    "the wallet's public balance can't cover the shield (plus gas)",
                    "lower the amount, or fund the wallet first (`just demo-fund` in demo \
                     mode)",
                ));
            }
        }

        // The wallet's own 0zk address comes from the view grant. The grant ALSO carries
        // the viewing key — a secret. Move it into Zeroizing immediately and drop it; only
        // the 0zk address is used, and neither ever reaches a response or a log line.
        let recipient_0zk = {
            let grant = match self
                .request(&SignerRequest::RailgunViewGrant {
                    chain_id: self.chain_id,
                    index: 0,
                })
                .await?
            {
                SignerResponse::RailgunView(grant) => grant,
                SignerResponse::Decision(Decision::Deny { reason }) => {
                    return Err(failure::from_deny_reason(&reason, self.config_dir()))
                }
                other => return Err(unexpected("RailgunViewGrant", &other)),
            };
            let deckard_contract::RailgunViewGrant {
                address,
                viewing_key,
            } = grant;
            let viewing_key = Zeroizing::new(viewing_key);
            drop(viewing_key); // recipient derivation needs only the address
            address
        };

        let recipient: deckard_core::RailgunAddress = recipient_0zk.parse().map_err(|_| {
            Failure::new(
                "the daemon returned an unparseable 0zk address",
                "the Railgun view grant did not contain a valid bech32m 0zk address",
                "this is a daemon-side bug — check the Deckard app and report it",
            )
        })?;
        let intent = deckard_core::build_shield_native_intent(self.chain_id, recipient, wei)
            .map_err(|e| {
                Failure::new(
                    "could not build the shield calldata",
                    format!("{e}"),
                    "shielding may be unsupported on this chain — check DECKARD_CHAIN_ID",
                )
            })?;

        match self
            .request(&SignerRequest::Propose {
                intent: intent.clone(),
            })
            .await?
        {
            SignerResponse::Decision(Decision::Allow) => {
                let id = SignerClient::request_id_for_intent(&intent);
                Ok(json!({
                    "decision": "allow",
                    "request_id": format!("{id:#x}"),
                    "amount_eth": amount_eth.trim(),
                    "next": "call deckard_execute with this request_id to sign + broadcast",
                }))
            }
            SignerResponse::Decision(Decision::NeedsApproval { request_id }) => Ok(json!({
                "decision": "needs_approval",
                "request_id": format!("{request_id:#x}"),
                "next": "a human must approve in the Deckard app before deckard_execute \
                         can run; the approval UI is not in this alpha — lower the amount \
                         under the policy per-tx cap (deckard_policy_get) or edit \
                         policy.json",
            })),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Propose", &other)),
        }
    }

    /// `deckard_execute` / `deckard-mcp execute <request_id>`.
    pub async fn execute(&self, request_id_hex: &str) -> OpResult {
        let request_id: B256 = request_id_hex.trim().parse().map_err(|_| {
            Failure::new(
                format!("could not parse request_id {request_id_hex:?}"),
                "a request_id is the 32-byte 0x-hex string returned by deckard_shield",
                "pass the request_id exactly as returned",
            )
        })?;
        self.ensure_chain().await?;
        // A transport error HERE is ambiguous (the broadcast may have happened) — map it
        // to the do-NOT-retry catalog entry, not the generic socket error.
        let resp = self
            .client
            .request(&SignerRequest::Execute { request_id })
            .await
            .map_err(|_| failure::execute_transport_unknown())?;
        match resp {
            SignerResponse::Execute(ExecuteResult::Broadcast { tx_hash }) => Ok(json!({
                "status": "broadcast",
                "tx_hash": format!("{tx_hash:#x}"),
                "note": "broadcast, not yet confirmed — the Deckard app shows the deposit \
                         settle into the shielded balance",
            })),
            SignerResponse::Execute(ExecuteResult::Denied { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Execute", &other)),
        }
    }

    /// `deckard_revoke_all` / `deckard-mcp stop` — STOP, the panic brake.
    pub async fn revoke_all(&self) -> OpResult {
        match self.request(&SignerRequest::RevokeAll).await? {
            SignerResponse::Ack => Ok(json!({
                "status": "stopped",
                "effect": "signing key zeroized; daemon locked; every in-flight request \
                           denied (including already-approved ones)",
                "re_arm": "irreversible for this session — a human must unlock the wallet \
                           in the Deckard app to re-arm",
            })),
            other => Err(unexpected("RevokeAll", &other)),
        }
    }

    /// The wallet's public address as a raw `Address` (the swap binding needs it before the
    /// order is hashed). Same `Address` round-trip as [`Self::wallet_address`], minus the JSON.
    async fn wallet_addr(&self) -> Result<Address, Failure> {
        match self.request(&SignerRequest::Address).await? {
            SignerResponse::Address(addr) => Ok(addr),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Address", &other)),
        }
    }

    /// The orderbook `base` URL for this chain, or an actionable Failure on an unsupported chain.
    fn cow_base(&self) -> Result<&'static str, Failure> {
        cow_api_base(self.chain_id).ok_or_else(|| {
            Failure::new(
                "this chain has no CoW orderbook",
                "swaps are supported on mainnet + Sepolia only",
                "use a supported DECKARD_CHAIN_ID (1 or 11155111)",
            )
        })
    }

    /// `deckard_swap_quote` / `deckard-mcp swap-quote`. Read-only: prices a CoW sell order and
    /// returns the request_id the BOUND order WOULD get — no daemon write, no approval. The id
    /// is advisory (re-derived from the bound order); `deckard_swap` returns the authoritative one.
    pub async fn swap_quote(
        &self,
        sell_token: &str,
        buy_token: &str,
        sell_amount_eth: &str,
    ) -> OpResult {
        self.ensure_chain().await?;
        let base = self.cow_base()?;
        let (sell, buy, sell_wei) = parse_swap_args(sell_token, buy_token, sell_amount_eth)?;
        let wallet = self.wallet_addr().await?;
        let req = QuoteRequest::sell(sell, buy, wallet, sell_wei, 1800);
        let quote = self
            .orderbook
            .quote(base, &req)
            .await
            .map_err(|e| deny_from_cow_failure(&e))?;
        // Bind owner == receiver == wallet BEFORE deriving the id (the daemon binds the same
        // fields before hashing), so the advisory id matches the order the daemon would store.
        let order =
            swap_order_from_quote(&quote, self.chain_id, wallet, wallet, DEFAULT_SLIPPAGE_BPS);
        let id = SignerClient::request_id_for_swap_order(&order);
        let simulated = self.orderbook.is_simulated();
        Ok(json!({
            "sell_token": format!("{sell:#x}"),
            "buy_token": format!("{buy:#x}"),
            "sell_amount": order.sell_amount.to_string(),
            "buy_amount_min": order.buy_amount_min.to_string(),
            "fee_amount": quote.quote.fee_amount.to_string(),
            "valid_to": order.valid_to,
            "request_id": format!("{id:#x}"),
            "simulated": simulated,
            "note": if simulated {
                "simulated quote — demo fork (no live solver); the authoritative request_id \
                 comes from deckard_swap"
            } else {
                "indicative price; re-quoted at deckard_swap time, which returns the \
                 authoritative request_id"
            },
            "next": "call deckard_swap with the same tokens + amount to propose it (a human \
                     approves in the Deckard app)",
        }))
    }

    /// `deckard_swap` / `deckard-mcp swap`. Shaped propose: re-quotes, builds the BOUND
    /// `SwapOrder`, and proposes it. A v1 swap ALWAYS comes back `needs_approval` + request_id —
    /// it signs nothing and broadcasts nothing.
    pub async fn swap(&self, sell_token: &str, buy_token: &str, sell_amount_eth: &str) -> OpResult {
        self.ensure_chain().await?;
        let base = self.cow_base()?;
        let (sell, buy, sell_wei) = parse_swap_args(sell_token, buy_token, sell_amount_eth)?;
        let wallet = self.wallet_addr().await?;
        let req = QuoteRequest::sell(sell, buy, wallet, sell_wei, 1800);
        let quote = self
            .orderbook
            .quote(base, &req)
            .await
            .map_err(|e| deny_from_cow_failure(&e))?;
        let order =
            swap_order_from_quote(&quote, self.chain_id, wallet, wallet, DEFAULT_SLIPPAGE_BPS);
        match self
            .client
            .propose_order(&order)
            .await
            .map_err(|_| failure::socket_missing(self.client.path()))?
        {
            Decision::NeedsApproval { request_id } => Ok(json!({
                "decision": "needs_approval",
                "request_id": format!("{request_id:#x}"),
                "sell_amount": order.sell_amount.to_string(),
                "buy_amount_min": order.buy_amount_min.to_string(),
                "next": "a human must approve this swap in the Deckard app, then call \
                         deckard_submit_order with this request_id; the approval UI is not in \
                         this alpha (a human approves via hold-to-confirm)",
                "simulated": self.orderbook.is_simulated(),
            })),
            Decision::Allow => Err(Failure::new(
                "the signer did not gate this swap behind approval",
                "v1 swaps must always require a human approval",
                "re-run deckard_swap",
            )),
            Decision::Deny { reason } => Err(failure::from_deny_reason(&reason, self.config_dir())),
        }
    }

    /// `deckard_submit_order` / `deckard-mcp submit-order`. Signs a human-approved order (the
    /// daemon refuses with `not_approved` until a human approves it in the app), then submits it
    /// to the CoW orderbook (or simulates the fill on the demo fork).
    pub async fn submit_order(&self, request_id_hex: &str) -> OpResult {
        let request_id: B256 = request_id_hex.trim().parse().map_err(|_| {
            Failure::new(
                format!("could not parse request_id {request_id_hex:?}"),
                "a request_id is the 32-byte 0x-hex string returned by deckard_swap",
                "pass the request_id exactly as returned",
            )
        })?;
        self.ensure_chain().await?;
        let base = self.cow_base()?;
        // (A2) Fetch the bound order via PendingList BEFORE signing — a clean pre-sign error if
        // the id is unknown, and no dependence on post-sign record retention.
        let order = self.pending_order(request_id).await?;
        // The control-channel approval is NOT ours: if no human has approved this order yet, the
        // daemon returns `not_approved` here (the honest no-self-approve refusal).
        let signature = match self
            .client
            .sign_order(request_id)
            .await
            .map_err(|_| failure::socket_missing(self.client.path()))?
        {
            SignOrderResult::Signed { signature } => signature,
            SignOrderResult::Denied { reason } => {
                return Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
        };
        // (A2) quote_id = None: the daemon never stores the quote id, and the submit path does
        // not require it (it is a diagnostic link, not a validation key).
        let creation = OrderCreation::from_signed_order(&order, signature, None);
        if let Err(e) = self.orderbook.put_app_data(base, APP_DATA_DOC).await {
            return Err(deny_from_cow_failure(&e));
        }
        match self.orderbook.submit(base, &creation).await {
            Ok(uid) => {
                let simulated = self.orderbook.is_simulated();
                Ok(json!({
                    "status": "submitted",
                    "uid": uid,
                    "simulated": simulated,
                    "note": if simulated {
                        "simulated fill on the demo fork — the live CoW orderbook can't accept a \
                         fork order; balances are credited where the demo token set is known"
                    } else {
                        "order accepted by the CoW orderbook; track it in the Deckard app"
                    },
                }))
            }
            Err(e) => Err(deny_from_cow_failure(&e)),
        }
    }

    /// Find a stored swap order by request_id via PendingList. (`sign_order` leaves the record
    /// in place, so fetching before OR after sign both work; we fetch before — see
    /// [`Self::submit_order`].)
    async fn pending_order(&self, request_id: B256) -> Result<SwapOrder, Failure> {
        let records = self
            .client
            .pending_list()
            .await
            .map_err(|_| failure::socket_missing(self.client.path()))?;
        records
            .into_iter()
            .find(|r| r.request_id == request_id)
            .and_then(|r| match r.payload {
                PendingPayloadView::Order(o) => Some(o),
                _ => None,
            })
            .ok_or_else(|| {
                Failure::new(
                    "no pending swap order for this request_id",
                    "the daemon has no swap order under this id (it may have expired, been \
                     signed already in a closed session, or the id is from a different flow)",
                    "re-run deckard_swap to get a fresh request_id",
                )
            })
    }
}

/// Render the daemon's trust label for a read.
fn read_status_label(status: &ReadStatus) -> String {
    match status {
        ReadStatus::Verified => "verified (Helios light-client checked)".to_string(),
        ReadStatus::Degraded { reason } => format!("degraded: {reason}"),
        ReadStatus::Unsynced { reason } => format!("unsynced (unverified read): {reason}"),
    }
}

/// The agent-readable policy snapshot, with both wei strings and ETH renderings.
fn policy_json(p: &Policy) -> serde_json::Value {
    json!({
        "per_tx_cap_wei": p.per_tx_cap_wei.to_string(),
        "per_tx_cap_eth": format_wei_as_eth(p.per_tx_cap_wei),
        "daily_cap_wei": p.daily_cap_wei.to_string(),
        "daily_cap_eth": format_wei_as_eth(p.daily_cap_wei),
        "spent_today_wei": p.spent_today_wei.to_string(),
        "spent_today_eth": format_wei_as_eth(p.spent_today_wei),
        "allow_to": p.allow_to.iter().map(|a| format!("{a:#x}")).collect::<Vec<_>>(),
        "allow_to_note": if p.allow_to.is_empty() { "empty = any recipient allowed" } else { "only these recipients" },
        "auto_shield_min_wei": p.auto_shield_min_wei.to_string(),
        "require_approval": match p.require_approval {
            ApprovalMode::Never => "never",
            ApprovalMode::OverCap => "over_cap",
            ApprovalMode::Always => "always",
        },
        "revoked": p.revoked,
        "note": "read-only here — a human edits policy.json in the Deckard config dir",
    })
}

/// A wire response that doesn't match the request shape — a daemon/sidecar version skew.
fn unexpected(what: &str, _resp: &SignerResponse) -> Failure {
    // Deliberately does NOT echo the response payload: an unexpected frame is exactly the
    // case where we can't vouch for its contents being transcript-safe.
    Failure::new(
        format!("the daemon returned an unexpected response to {what}"),
        "the daemon and this sidecar disagree on the wire contract (version skew)",
        "rebuild both from the same checkout (`cargo build -p deckard-signerd -p \
         deckard-mcp`) and restart the app",
    )
}

/// Parse the swap tools' arguments: two 0x-hex token addresses + a decimal ETH-string amount
/// (in the sell token's own units, parsed exactly through [`parse_eth_to_wei`]).
fn parse_swap_args(
    sell_token: &str,
    buy_token: &str,
    sell_amount_eth: &str,
) -> Result<(Address, Address, U256), Failure> {
    let sell: Address = sell_token.trim().parse().map_err(|_| {
        Failure::new(
            format!("could not parse sell_token {sell_token:?}"),
            "sell_token must be a 0x-hex Ethereum address",
            "pass the token's 0x-hex contract address (e.g. WETH on this chain)",
        )
    })?;
    let buy: Address = buy_token.trim().parse().map_err(|_| {
        Failure::new(
            format!("could not parse buy_token {buy_token:?}"),
            "buy_token must be a 0x-hex Ethereum address",
            "pass the token's 0x-hex contract address",
        )
    })?;
    let sell_wei = parse_eth_to_wei(sell_amount_eth).map_err(|msg| {
        Failure::new(
            format!("could not parse sell_amount_eth {sell_amount_eth:?}"),
            msg,
            "pass a decimal amount string like \"0.05\" (the sell token's own units)",
        )
    })?;
    if sell_wei == U256::ZERO {
        return Err(Failure::new(
            "sell_amount_eth is zero",
            "a zero-amount swap has nothing to sell",
            "pass an amount greater than zero, e.g. \"0.05\"",
        ));
    }
    Ok((sell, buy, sell_wei))
}

/// Turn an `anyhow`-wrapped orderbook error into a three-part [`Failure`]. Downcasts to the
/// typed [`CowError`] so the well-known `errorType`s read honestly (mirrors the app's
/// `humanize_cow_api`); an un-typed transport/decode error falls back to calm generic copy.
fn deny_from_cow_failure(e: &anyhow::Error) -> Failure {
    match e.downcast_ref::<CowError>() {
        Some(CowError::Api {
            error_type,
            description,
        }) => humanize_cow_api(error_type, description),
        Some(CowError::Http { status, .. }) => Failure::new(
            format!("the CoW orderbook returned HTTP {status}"),
            "the orderbook rejected the request with a non-structured error",
            "re-run the swap flow from deckard_swap_quote; if it persists, check the Deckard app",
        ),
        Some(CowError::Decode(_)) => Failure::new(
            "the CoW orderbook sent an unexpected response",
            "the success body did not decode into the expected shape (a backend/version skew)",
            "re-run the swap flow from deckard_swap_quote; if it recurs, report it",
        ),
        Some(CowError::Transport(_)) | None => Failure::new(
            "could not reach the CoW orderbook",
            "the request to the orderbook failed at the network layer (DNS/TLS/connect/timeout)",
            "check the network and re-run the swap flow from deckard_swap_quote",
        ),
    }
}

/// Map a CoW orderbook `errorType` to a three-part [`Failure`] (mirrors the app's
/// `humanize_cow_api`). The well-known rejection types each get distinct, actionable copy;
/// an unrecognised type falls through with its raw tag so a new orderbook error isn't swallowed.
fn humanize_cow_api(error_type: &str, description: &str) -> Failure {
    match error_type {
        "OrderExpired" | "Expired" | "EXPIRED" => Failure::new(
            "the price quote expired before the order was placed",
            "the quote/order lapsed between pricing and submit",
            "re-run the swap flow from deckard_swap_quote to get a fresh quote",
        ),
        "NoLiquidity" => Failure::new(
            "there's no route to swap these tokens right now",
            "no solver could price this pair/size at any rate",
            "try a different pair or amount",
        ),
        "InsufficientBalance" => Failure::new(
            "the wallet doesn't hold enough of the sell token for this swap",
            "the orderbook's balance check failed against the wallet's holdings",
            "lower the amount, or fund the wallet with the sell token first",
        ),
        "InvalidSignature" => Failure::new(
            "the order signature didn't validate",
            "usually a stale quote — the signed order no longer matches a live price",
            "re-run the swap flow from deckard_swap_quote and submit again promptly",
        ),
        "InsufficientAllowance" => Failure::new(
            "the CoW vault relayer isn't approved to move enough of the sell token",
            "the exact-gross relayer approval is a separate human-approved step that the \
             key-less sidecar cannot perform",
            "approve the sell token in the Deckard app first, then retry deckard_submit_order",
        ),
        "DuplicatedOrder" => Failure::new(
            "this exact order is already on the orderbook",
            "an identical order was already submitted",
            "do not resubmit; track the existing order in the Deckard app",
        ),
        other => Failure::new(
            format!("the CoW orderbook rejected the order ({other})"),
            format!("the orderbook returned: {description}"),
            "re-run the swap flow from deckard_swap_quote; if the reason is unclear, check the \
             Deckard app",
        ),
    }
}
