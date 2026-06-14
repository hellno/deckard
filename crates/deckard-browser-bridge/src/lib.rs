//! Minimal experimental EIP-1193 browser bridge.
//!
//! This is intentionally a narrow localhost-only vertical slice for the unpacked browser
//! connector. It is key-less: account discovery reads the already-unlocked Deckard address
//! through shared wallet-client primitives, or a dev-only mock address when explicitly
//! enabled for browser-bridge testing.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use deckard_wallet_client::WalletClient;

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
    WalletClient(Arc<WalletClient>),
    DevMock { account: String },
}

impl BridgeBackend {
    pub fn from_env(wallet: WalletClient) -> Self {
        match std::env::var(DEV_ACCOUNT_ENV) {
            Ok(account) => Self::DevMock { account },
            Err(_) => Self::WalletClient(Arc::new(wallet)),
        }
    }

    async fn account(&self) -> Result<String, BridgeError> {
        match self {
            Self::DevMock { account } => Ok(account.clone()),
            Self::WalletClient(wallet) => {
                wallet
                    .wallet_address()
                    .await
                    .map_err(|failure| BridgeError {
                        code: 4900,
                        message: failure.to_human(),
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

/// Serve the bridge JSON-RPC endpoint on a loopback TCP address.
pub async fn serve(
    bind: &str,
    wallet: WalletClient,
    dev_mock_account: Option<String>,
) -> anyhow::Result<()> {
    if !bind.starts_with("127.0.0.1:") && !bind.starts_with("localhost:") {
        anyhow::bail!("browser bridge must bind to loopback (example: 127.0.0.1:8765)");
    }
    let chain_id = wallet.chain_id();
    let backend = match dev_mock_account {
        Some(account) => BridgeBackend::DevMock { account },
        None => BridgeBackend::from_env(wallet),
    };
    let bridge = Arc::new(BrowserBridge::new(chain_id, backend));
    let listener = TcpListener::bind(bind).await?;
    eprintln!(
        "Deckard browser bridge listening on http://{bind}/rpc (dev mock via {})",
        dev_account_env_name()
    );
    loop {
        let (stream, _) = listener.accept().await?;
        let bridge = Arc::clone(&bridge);
        tokio::spawn(async move {
            if let Err(e) = handle_http_connection(stream, bridge).await {
                eprintln!("browser bridge request failed: {e}");
            }
        });
    }
}

async fn handle_http_connection(
    mut stream: TcpStream,
    bridge: Arc<BrowserBridge>,
) -> anyhow::Result<()> {
    let mut buf = vec![0_u8; 64 * 1024];
    let mut read = 0_usize;
    let header_end = loop {
        let n = stream.read(&mut buf[read..]).await?;
        if n == 0 {
            return Ok(());
        }
        read += n;
        if let Some(pos) = find_header_end(&buf[..read]) {
            break pos;
        }
        if read == buf.len() {
            return write_http(&mut stream, 413, "text/plain", "request too large").await;
        }
    };

    let headers = std::str::from_utf8(&buf[..header_end])?.to_string();
    let (method, path) = request_line(&headers)?;
    let origin = header_value(&headers, "x-deckard-origin")
        .or_else(|| header_value(&headers, "origin"))
        .unwrap_or("unknown-origin")
        .to_string();

    if method == "OPTIONS" {
        return write_http(&mut stream, 204, "text/plain", "").await;
    }
    if method != "POST" || path != "/rpc" {
        return write_http(&mut stream, 404, "text/plain", "not found").await;
    }
    if !host_is_loopback(&headers) {
        return write_http(&mut stream, 403, "text/plain", "host must be localhost").await;
    }

    let content_length = header_value(&headers, "content-length")
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| anyhow::anyhow!("missing content-length"))?;
    if content_length > 32 * 1024 {
        return write_http(&mut stream, 413, "text/plain", "body too large").await;
    }

    let body_start = header_end + 4;
    while read < body_start + content_length {
        let n = stream.read(&mut buf[read..]).await?;
        if n == 0 {
            anyhow::bail!("connection closed before request body completed");
        }
        read += n;
    }

    let request: BridgeRequest =
        serde_json::from_slice(&buf[body_start..body_start + content_length])?;
    let response = bridge.handle_request(&origin, request).await;
    let response_body = serde_json::to_string(&response)?;
    write_http(&mut stream, 200, "application/json", &response_body).await
}

async fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\naccess-control-allow-headers: content-type,x-deckard-origin\r\naccess-control-allow-methods: POST,OPTIONS\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_line(headers: &str) -> anyhow::Result<(&str, &str)> {
    let line = headers
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request line"))?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request method"))?;
    let path = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing request path"))?;
    Ok((method, path))
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            Some(value.trim())
        } else {
            None
        }
    })
}

fn host_is_loopback(headers: &str) -> bool {
    header_value(headers, "host")
        .map(|host| host.starts_with("127.0.0.1:") || host.starts_with("localhost:"))
        .unwrap_or(false)
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
