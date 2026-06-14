//! Minimal experimental EIP-1193 browser bridge.
//!
//! This is intentionally a narrow localhost-only vertical slice for the unpacked browser
//! connector. It is key-less: account discovery reads the already-unlocked Deckard address
//! through the existing [`Sidecar`] / signer-daemon path, or a dev-only mock address when
//! explicitly enabled for browser-bridge testing.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Sidecar;

const PERMISSION_ETH_ACCOUNTS: &str = "eth_accounts";
const DEV_ACCOUNT_ENV: &str = "DECKARD_BRIDGE_DEV_ACCOUNT";
const DEFAULT_DEV_ACCOUNT: &str = "0xdeC0ded0000000000000000000000000000001193";

/// Per-origin dapp session remembered by the bridge process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DappSession {
    pub origin: String,
    pub chain_id: u64,
    pub account: String,
    pub permissions: Vec<String>,
    pub created_at: u64,
    pub last_seen: u64,
    pub revoked: bool,
}

#[derive(Default)]
pub struct DappSessionStore {
    sessions: BTreeMap<String, DappSession>,
}

impl DappSessionStore {
    pub fn get(&self, origin: &str) -> Option<&DappSession> {
        self.sessions.get(origin).filter(|session| !session.revoked)
    }

    pub fn grant_accounts(
        &mut self,
        origin: String,
        chain_id: u64,
        account: String,
    ) -> DappSession {
        let now = unix_now();
        let session = self.sessions.entry(origin.clone()).or_insert(DappSession {
            origin,
            chain_id,
            account: account.clone(),
            permissions: vec![PERMISSION_ETH_ACCOUNTS.to_string()],
            created_at: now,
            last_seen: now,
            revoked: false,
        });
        session.chain_id = chain_id;
        session.account = account;
        session.last_seen = now;
        session.revoked = false;
        if !session
            .permissions
            .iter()
            .any(|permission| permission == PERMISSION_ETH_ACCOUNTS)
        {
            session
                .permissions
                .push(PERMISSION_ETH_ACCOUNTS.to_string());
        }
        session.clone()
    }

    #[allow(dead_code)]
    pub fn revoke(&mut self, origin: &str) -> bool {
        match self.sessions.get_mut(origin) {
            Some(session) => {
                session.revoked = true;
                session.last_seen = unix_now();
                true
            }
            None => false,
        }
    }
}

#[derive(Clone)]
pub enum BridgeBackend {
    Sidecar(Arc<Sidecar>),
    DevMock { account: String },
}

impl BridgeBackend {
    pub fn from_env(sidecar: Sidecar) -> Self {
        match std::env::var(DEV_ACCOUNT_ENV) {
            Ok(account) => Self::DevMock { account },
            Err(_) => Self::Sidecar(Arc::new(sidecar)),
        }
    }

    async fn account(&self) -> Result<String, BridgeError> {
        match self {
            Self::DevMock { account } => Ok(account.clone()),
            Self::Sidecar(sidecar) => {
                let value = sidecar
                    .wallet_address()
                    .await
                    .map_err(|failure| BridgeError {
                        code: 4900,
                        message: failure.to_human(),
                    })?;
                value
                    .get("address")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| BridgeError {
                        code: 4900,
                        message: "Deckard returned an address response without an address".into(),
                    })
            }
        }
    }
}

pub struct BrowserBridge {
    chain_id: u64,
    backend: BridgeBackend,
    sessions: Mutex<DappSessionStore>,
}

impl BrowserBridge {
    pub fn new(chain_id: u64, backend: BridgeBackend) -> Self {
        Self {
            chain_id,
            backend,
            sessions: Mutex::new(DappSessionStore::default()),
        }
    }

    pub async fn handle_request(&self, origin: &str, request: BridgeRequest) -> BridgeResponse {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            "eth_chainId" => Ok(json!(format!("0x{:x}", self.chain_id))),
            "eth_accounts" => Ok(json!(self.accounts_for_origin(origin))),
            "eth_requestAccounts" => self
                .request_accounts(origin)
                .await
                .map(|accounts| json!(accounts)),
            method => Err(BridgeError {
                code: 4200,
                message: format!("Deckard browser bridge does not support {method}"),
            }),
        };
        BridgeResponse::from_result(id, result)
    }

    fn accounts_for_origin(&self, origin: &str) -> Vec<String> {
        let Ok(store) = self.sessions.lock() else {
            return Vec::new();
        };
        store
            .get(origin)
            .map(|session| vec![session.account.clone()])
            .unwrap_or_default()
    }

    async fn request_accounts(&self, origin: &str) -> Result<Vec<String>, BridgeError> {
        let account = self.backend.account().await?;
        let session = {
            let mut store = self.sessions.lock().map_err(|_| BridgeError {
                code: 4900,
                message: "Deckard browser bridge session store is unavailable".into(),
            })?;
            store.grant_accounts(origin.to_string(), self.chain_id, account)
        };
        Ok(vec![session.account])
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct BridgeRequest {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct BridgeResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
}

impl BridgeResponse {
    fn from_result(id: Value, result: Result<Value, BridgeError>) -> Self {
        match result {
            Ok(result) => Self {
                jsonrpc: "2.0",
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(error),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BridgeError {
    pub code: i64,
    pub message: String,
}

pub fn dev_account_from_env() -> String {
    std::env::var(DEV_ACCOUNT_ENV).unwrap_or_else(|_| DEFAULT_DEV_ACCOUNT.to_string())
}

pub fn dev_account_env_name() -> &'static str {
    DEV_ACCOUNT_ENV
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://127.0.0.1:8765";

    fn bridge() -> BrowserBridge {
        BrowserBridge::new(
            11155111,
            BridgeBackend::DevMock {
                account: DEFAULT_DEV_ACCOUNT.to_string(),
            },
        )
    }

    #[test]
    fn session_creation_and_lookup() {
        let mut store = DappSessionStore::default();
        let session = store.grant_accounts(
            ORIGIN.to_string(),
            11155111,
            DEFAULT_DEV_ACCOUNT.to_string(),
        );
        assert_eq!(session.origin, ORIGIN);
        assert_eq!(session.chain_id, 11155111);
        assert_eq!(session.account, DEFAULT_DEV_ACCOUNT);
        assert_eq!(session.permissions, vec![PERMISSION_ETH_ACCOUNTS]);
        assert!(!session.revoked);
        assert_eq!(
            store.get(ORIGIN).map(|stored| stored.account.as_str()),
            Some(DEFAULT_DEV_ACCOUNT)
        );
        assert!(store.revoke(ORIGIN));
        assert!(store.get(ORIGIN).is_none());
    }

    #[tokio::test]
    async fn unsupported_method_returns_eip1193_style_error() {
        let response = bridge()
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(7),
                    method: "eth_sendTransaction".into(),
                    params: Value::Null,
                },
            )
            .await;
        let error = response.error.expect("unsupported method error");
        assert_eq!(error.code, 4200);
        assert!(error.message.contains("eth_sendTransaction"));
    }

    #[tokio::test]
    async fn account_request_returns_expected_address_in_dev_mode() {
        let bridge = bridge();
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        assert_eq!(response.result, Some(json!([DEFAULT_DEV_ACCOUNT])));

        let accounts = bridge.accounts_for_origin(ORIGIN);
        assert_eq!(accounts, vec![DEFAULT_DEV_ACCOUNT]);
    }
}
