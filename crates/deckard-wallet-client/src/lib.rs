//! Shared key-less wallet client/session primitives for local Deckard interfaces.
//!
//! This crate owns the signer-daemon client access, chain-id configuration, and common
//! failure mapping used by sibling surfaces such as `deckard-mcp` (agent/MCP) and
//! `deckard-browser-bridge` (dapp/browser). It never holds signing keys.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use alloy_primitives::{Address, Bytes, U256};
use deckard_contract::{
    deny_reasons, Decision, Intent, IntentKind, ProposalOrigin, SignerRequest, SignerResponse,
};
pub use deckard_signerd::SignerClient;

pub mod failure;
pub use failure::Failure;

/// Shared signer-daemon client state for key-less local Deckard interfaces.
pub struct WalletClient {
    client: SignerClient,
    /// The chain this client expects (`DECKARD_CHAIN_ID`, default 1 — matching the daemon's default).
    chain_id: u64,
    /// `DECKARD_CONFIG_DIR` when set — used only to sharpen the `locked` error into the no-vault case.
    config_dir: Option<PathBuf>,
    /// Set once the connect-time chain probe has conclusively confirmed the daemon signs for `chain_id`.
    chain_checked: AtomicBool,
}

impl WalletClient {
    /// Resolve from the environment: socket path (`DECKARD_SOCKET_PATH` or the per-uid default),
    /// chain id (`DECKARD_CHAIN_ID`, default 1), optional config dir.
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
        Ok(Self::new(socket_path, chain_id, config_dir))
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

    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    pub fn signer_client(&self) -> &SignerClient {
        &self.client
    }

    /// One request → one response, with connect failures mapped to the shared catalog.
    pub async fn request(&self, req: &SignerRequest) -> Result<SignerResponse, Failure> {
        self.client
            .request(req)
            .await
            .map_err(|_| failure::socket_missing(self.client.path()))
    }

    /// Connect-time chain probe: confirm the daemon signs for our chain before building real intents.
    pub async fn ensure_chain(&self) -> Result<(), Failure> {
        if self.chain_checked.load(Ordering::Relaxed) {
            return Ok(());
        }
        let probe = Intent {
            chain_id: self.chain_id,
            to: Address::ZERO,
            token: None,
            value: U256::ZERO,
            calldata: Bytes::from_static(&[0x00]),
            kind: IntentKind::Send,
        };
        match self
            .request(&SignerRequest::Propose {
                intent: probe,
                // The agent sidecar's connect-time probe — tag it Agent (the daemon's
                // chain/locked pre-checks run before the policy gate, so this is side-effect-free).
                origin: ProposalOrigin::Agent,
            })
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
                self.chain_checked.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => {
                self.chain_checked.store(true, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    /// Read the wallet's public address through the signer daemon.
    pub async fn wallet_address(&self) -> Result<String, Failure> {
        self.ensure_chain().await?;
        match self.request(&SignerRequest::Address).await? {
            SignerResponse::Address(addr) => Ok(format!("{addr:#x}")),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                Err(failure::from_deny_reason(&reason, self.config_dir()))
            }
            other => Err(unexpected("Address", &other)),
        }
    }
}

/// A wire response that doesn't match the request shape — a daemon/client version skew.
pub fn unexpected(what: &str, _resp: &SignerResponse) -> Failure {
    // Deliberately does NOT echo the response payload: an unexpected frame is exactly the
    // case where we can't vouch for its contents being transcript-safe.
    Failure::new(
        format!("the daemon returned an unexpected response to {what}"),
        "the daemon and this client disagree on the wire contract (version skew)",
        "rebuild local Deckard binaries from the same checkout and restart the app",
    )
}
