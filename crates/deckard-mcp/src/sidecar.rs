//! The key-less core both surfaces (CLI + MCP tools) share: one [`SignerClient`] to the
//! daemon socket, the expected chain, and the six operations. **No key material ever enters
//! this process** — writes are `Intent`s proposed to `deckard-signerd`, which enforces
//! policy and signs. The one secret this sidecar transiently handles is the Railgun
//! *viewing* key (it rides alongside the wallet's own 0zk address in `RailgunViewGrant`);
//! it is moved into `Zeroizing` on receipt, never logged, and never put in any response.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use alloy_primitives::{Address, Bytes, B256, U256};
use serde_json::json;
use zeroize::Zeroizing;

use deckard_contract::{
    deny_reasons, ApprovalMode, Decision, ExecuteResult, Intent, IntentKind, Policy, ReadStatus,
    SignerRequest, SignerResponse,
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
        })
    }

    /// Test/builder constructor with explicit wiring.
    pub fn new(socket_path: PathBuf, chain_id: u64, config_dir: Option<PathBuf>) -> Self {
        Self {
            client: SignerClient::new(socket_path),
            chain_id,
            config_dir,
            chain_checked: AtomicBool::new(false),
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
