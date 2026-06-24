//! Minimal experimental EIP-1193 browser bridge.
//!
//! This is intentionally a narrow localhost-only vertical slice for the unpacked browser
//! connector. It is key-less: account discovery reads the already-unlocked Deckard address
//! through shared wallet-client primitives, or a dev-only mock address when explicitly
//! enabled for browser-bridge testing.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use alloy_dyn_abi::eip712::TypedData;
use alloy_primitives::{Address, Bytes, B256, U256};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use deckard_contract::{
    ApprovalStatus, Decision, ExecuteResult, Intent, IntentKind, MessageSigningRisk,
    ProposalOrigin, RequestId, SignMessage, SignMessageKind, SignMessageResult, SignerRequest,
    SignerResponse, TypedDataReview,
};
use deckard_wallet_client::WalletClient;

const PERMISSION_ETH_ACCOUNTS: &str = "eth_accounts";
const DEV_ACCOUNT_ENV: &str = "DECKARD_BRIDGE_DEV_ACCOUNT";
const DEFAULT_DEV_ACCOUNT: &str = "0xdec0ded000000000000000000000000000001193";
const MESSAGE_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const MESSAGE_APPROVAL_POLL: Duration = Duration::from_millis(250);
const ERC20_TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
const ERC20_APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

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

    async fn sign_message(&self, message: SignMessage) -> Result<Bytes, BridgeError> {
        match self {
            Self::DevMock { .. } => Ok(dev_signature_for_message(&message)),
            Self::WalletClient(wallet) => sign_message_with_daemon(wallet, message).await,
        }
    }

    async fn send_transaction(&self, intent: Intent) -> Result<B256, BridgeError> {
        match self {
            Self::DevMock { .. } => Ok(dev_tx_hash_for_intent(&intent)),
            Self::WalletClient(wallet) => execute_intent_with_daemon(wallet, intent).await,
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
            "personal_sign" => self
                .personal_sign(origin, request.params)
                .await
                .map(|signature| json!(hex_prefixed(signature.as_ref()))),
            "eth_signTypedData_v4" => self
                .sign_typed_data_v4(origin, request.params)
                .await
                .map(|signature| json!(hex_prefixed(signature.as_ref()))),
            "eth_sendTransaction" => self
                .send_transaction(origin, request.params)
                .await
                .map(|tx_hash| json!(format!("{tx_hash:#x}"))),
            "eth_sign" => Err(BridgeError {
                code: 4200,
                message:
                    "Deckard refuses raw eth_sign because raw hash signing is not clear-signable"
                        .into(),
            }),
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

    async fn personal_sign(&self, origin: &str, params: Value) -> Result<Bytes, BridgeError> {
        let session = self.require_session(origin)?;
        let (account, message) = parse_personal_sign_params(params)?;
        ensure_same_account(&session.account, &account)?;
        self.backend
            .sign_message(SignMessage {
                chain_id: self.chain_id,
                origin: origin.to_string(),
                kind: SignMessageKind::PersonalSign { message },
            })
            .await
    }

    async fn sign_typed_data_v4(&self, origin: &str, params: Value) -> Result<Bytes, BridgeError> {
        let session = self.require_session(origin)?;
        let (account, review) = parse_typed_data_v4_params(params, self.chain_id)?;
        ensure_same_account(&session.account, &account)?;
        self.backend
            .sign_message(SignMessage {
                chain_id: self.chain_id,
                origin: origin.to_string(),
                kind: SignMessageKind::TypedDataV4(review),
            })
            .await
    }

    async fn send_transaction(&self, origin: &str, params: Value) -> Result<B256, BridgeError> {
        let session = self.require_session(origin)?;
        let (from, intent) = parse_send_transaction_params(params, self.chain_id)?;
        ensure_same_account(&session.account, &from)?;
        self.backend.send_transaction(intent).await
    }

    fn require_session(&self, origin: &str) -> Result<DappSession, BridgeError> {
        let store = self.sessions.lock().map_err(|_| BridgeError {
            code: 4900,
            message: "Deckard browser bridge session store is unavailable".into(),
        })?;
        store.get(origin).cloned().ok_or_else(|| BridgeError {
            code: 4100,
            message: "Deckard requires eth_requestAccounts before signing".into(),
        })
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

async fn sign_message_with_daemon(
    wallet: &WalletClient,
    message: SignMessage,
) -> Result<Bytes, BridgeError> {
    let request_id = deckard_wallet_client::SignerClient::request_id_for_message(&message);
    let decision = wallet
        .signer_client()
        .propose_message(&message, ProposalOrigin::App)
        .await
        .map_err(bridge_daemon_error)?;
    match decision {
        Decision::Deny { reason } => return Err(bridge_denied("message signing", reason)),
        Decision::Allow => {}
        Decision::NeedsApproval { .. } => {
            wait_for_approval(wallet, request_id, "message signing").await?
        }
    }
    match wallet
        .signer_client()
        .sign_message(request_id)
        .await
        .map_err(bridge_daemon_error)?
    {
        SignMessageResult::Signed { signature } => Ok(signature),
        SignMessageResult::Denied { reason } => Err(bridge_denied("message signing", reason)),
    }
}

async fn execute_intent_with_daemon(
    wallet: &WalletClient,
    intent: Intent,
) -> Result<B256, BridgeError> {
    let request_id = deckard_wallet_client::SignerClient::request_id_for_intent(&intent);
    let decision = wallet
        .signer_client()
        .propose(&intent, ProposalOrigin::App)
        .await
        .map_err(bridge_daemon_error)?;
    match decision {
        Decision::Deny { reason } => return Err(bridge_denied("transaction", reason)),
        Decision::Allow => {}
        Decision::NeedsApproval { .. } => {
            wait_for_approval(wallet, request_id, "transaction").await?
        }
    }
    match wallet
        .signer_client()
        .execute(request_id)
        .await
        .map_err(bridge_daemon_error)?
    {
        ExecuteResult::Broadcast { tx_hash } => Ok(tx_hash),
        ExecuteResult::Denied { reason } => Err(bridge_denied("transaction", reason)),
    }
}

async fn wait_for_approval(
    wallet: &WalletClient,
    request_id: RequestId,
    label: &str,
) -> Result<(), BridgeError> {
    let deadline = Instant::now() + MESSAGE_APPROVAL_TIMEOUT;
    loop {
        let status = match wallet
            .signer_client()
            .request(&SignerRequest::Status { request_id })
            .await
            .map_err(bridge_daemon_error)?
        {
            SignerResponse::Status(status) => status,
            other => {
                return Err(BridgeError {
                    code: 4900,
                    message: format!("daemon returned unexpected response to Status: {other:?}"),
                })
            }
        };
        match status {
            ApprovalStatus::Allowed => return Ok(()),
            ApprovalStatus::Denied { reason } => return Err(bridge_denied(label, reason)),
            ApprovalStatus::Expired => return Err(bridge_denied(label, "expired".into())),
            ApprovalStatus::Pending => {
                if Instant::now() >= deadline {
                    return Err(BridgeError {
                        code: 4001,
                        message: format!("{label} approval timed out"),
                    });
                }
                sleep(MESSAGE_APPROVAL_POLL).await;
            }
        }
    }
}

fn parse_personal_sign_params(params: Value) -> Result<(Address, Bytes), BridgeError> {
    let values = params_array(params, "personal_sign")?;
    if values.len() != 2 {
        return Err(invalid_params("personal_sign expects [message, account]"));
    }
    let first = param_string(&values[0], "personal_sign first parameter")?;
    let second = param_string(&values[1], "personal_sign second parameter")?;
    match (parse_address(first), parse_address(second)) {
        (Ok(account), Err(_)) => Ok((account, message_bytes(second)?)),
        (Err(_), Ok(account)) => Ok((account, message_bytes(first)?)),
        (Ok(_), Ok(_)) => Err(invalid_params(
            "personal_sign needs one account and one message",
        )),
        (Err(_), Err(_)) => Err(invalid_params("personal_sign missing account parameter")),
    }
}

fn parse_typed_data_v4_params(
    params: Value,
    chain_id: u64,
) -> Result<(Address, TypedDataReview), BridgeError> {
    let values = params_array(params, "eth_signTypedData_v4")?;
    if values.len() != 2 {
        return Err(invalid_params(
            "eth_signTypedData_v4 expects [account, typedData]",
        ));
    }
    let account = parse_address(param_string(
        &values[0],
        "eth_signTypedData_v4 account parameter",
    )?)?;
    let typed_data: TypedData = serde_json::from_value(values[1].clone()).map_err(|error| {
        invalid_params(format!("invalid eth_signTypedData_v4 payload: {error}"))
    })?;
    let digest = typed_data.eip712_signing_hash().map_err(|error| {
        invalid_params(format!("invalid eth_signTypedData_v4 encoding: {error}"))
    })?;
    let domain_chain_id = typed_data
        .domain
        .chain_id
        .and_then(|value| u256_to_u64(value).ok());
    if let Some(domain_chain_id) = domain_chain_id {
        if domain_chain_id != chain_id {
            return Err(BridgeError {
                code: 4901,
                message: format!(
                    "typed-data domain chain id {domain_chain_id} does not match active chain {chain_id}"
                ),
            });
        }
    }
    let risks = if typed_data.domain.verifying_contract.is_none() {
        vec![MessageSigningRisk::UnknownVerifyingContract]
    } else {
        Vec::new()
    };
    Ok((
        account,
        TypedDataReview {
            domain_name: typed_data.domain.name.map(|value| value.into_owned()),
            domain_version: typed_data.domain.version.map(|value| value.into_owned()),
            domain_chain_id,
            verifying_contract: typed_data.domain.verifying_contract,
            primary_type: typed_data.primary_type,
            digest,
            risks,
        },
    ))
}

fn parse_send_transaction_params(
    params: Value,
    chain_id: u64,
) -> Result<(Address, Intent), BridgeError> {
    let values = params_array(params, "eth_sendTransaction")?;
    if values.len() != 1 {
        return Err(invalid_params(
            "eth_sendTransaction expects one transaction object",
        ));
    }
    let tx = values[0]
        .as_object()
        .ok_or_else(|| invalid_params("eth_sendTransaction first param must be an object"))?;
    let from = parse_address(param_string(
        tx.get("from")
            .ok_or_else(|| invalid_params("eth_sendTransaction missing from"))?,
        "eth_sendTransaction from",
    )?)?;
    let to = parse_address(param_string(
        tx.get("to")
            .ok_or_else(|| invalid_params("eth_sendTransaction missing to"))?,
        "eth_sendTransaction to",
    )?)?;
    let value = match tx.get("value") {
        Some(value) => parse_quantity(param_string(value, "eth_sendTransaction value")?)?,
        None => U256::ZERO,
    };
    let data = tx
        .get("data")
        .or_else(|| tx.get("input"))
        .map(|value| param_string(value, "eth_sendTransaction data"))
        .transpose()?
        .unwrap_or("0x");
    if data != "0x" && !data.is_empty() {
        return parse_classified_calldata(chain_id, to, value, data).map(|intent| (from, intent));
    }
    Ok((
        from,
        Intent {
            chain_id,
            to,
            token: None,
            value,
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        },
    ))
}

fn parse_classified_calldata(
    chain_id: u64,
    token: Address,
    native_value: U256,
    data: &str,
) -> Result<Intent, BridgeError> {
    if native_value != U256::ZERO {
        return Err(BridgeError {
            code: 4200,
            message: "Deckard refuses ERC-20 eth_sendTransaction calldata with native value".into(),
        });
    }
    let calldata = message_bytes(data)?;
    let bytes = calldata.as_ref();
    if bytes.len() < 4 {
        return Err(invalid_params("ERC-20 calldata is too short"));
    }
    if bytes.len() != 4 + 32 + 32 {
        return Err(invalid_params("ERC-20 calldata must be exactly 68 bytes"));
    }
    let selector = [bytes[0], bytes[1], bytes[2], bytes[3]];
    match selector {
        ERC20_TRANSFER_SELECTOR => {
            let recipient = abi_address_word(&bytes[4..36])?;
            let amount = U256::from_be_slice(&bytes[36..68]);
            Ok(Intent {
                chain_id,
                to: recipient,
                token: Some(token),
                value: amount,
                calldata: Bytes::new(),
                kind: IntentKind::Send,
            })
        }
        ERC20_APPROVE_SELECTOR => Ok(Intent {
            chain_id,
            to: token,
            token: None,
            value: U256::ZERO,
            calldata,
            kind: IntentKind::ContractCall,
        }),
        _ => Err(BridgeError {
            code: 4200,
            message: "Deckard refuses unsupported transaction calldata selector".into(),
        }),
    }
}

fn abi_address_word(word: &[u8]) -> Result<Address, BridgeError> {
    if word.len() != 32 {
        return Err(invalid_params("ABI address word must be 32 bytes"));
    }
    if word[..12].iter().any(|byte| *byte != 0) {
        return Err(invalid_params("ERC-20 calldata address is not ABI encoded"));
    }
    Ok(Address::from_slice(&word[12..32]))
}

fn params_array(params: Value, method: &str) -> Result<Vec<Value>, BridgeError> {
    match params {
        Value::Array(values) => Ok(values),
        _ => Err(invalid_params(format!("{method} params must be an array"))),
    }
}

fn param_string<'a>(value: &'a Value, label: &str) -> Result<&'a str, BridgeError> {
    value
        .as_str()
        .ok_or_else(|| invalid_params(format!("{label} must be a string")))
}

fn parse_quantity(value: &str) -> Result<U256, BridgeError> {
    if let Some(hex) = value.strip_prefix("0x") {
        if hex.is_empty() {
            return Err(invalid_params("quantity hex string is empty"));
        }
        U256::from_str_radix(hex, 16).map_err(|_| invalid_params("invalid hex quantity"))
    } else {
        U256::from_str_radix(value, 10).map_err(|_| invalid_params("invalid decimal quantity"))
    }
}

fn parse_address(value: &str) -> Result<Address, BridgeError> {
    Address::from_str(value).map_err(|_| invalid_params("invalid Ethereum address"))
}

fn message_bytes(value: &str) -> Result<Bytes, BridgeError> {
    if let Some(hex) = value.strip_prefix("0x") {
        Ok(Bytes::from(hex_to_bytes(hex)?))
    } else {
        Ok(Bytes::copy_from_slice(value.as_bytes()))
    }
}

fn ensure_same_account(session_account: &str, requested: &Address) -> Result<(), BridgeError> {
    let connected = parse_address(session_account)?;
    if connected == *requested {
        Ok(())
    } else {
        Err(BridgeError {
            code: 4100,
            message: "signing account is not connected to this origin".into(),
        })
    }
}

fn u256_to_u64(value: U256) -> Result<u64, ()> {
    if value > U256::from(u64::MAX) {
        Err(())
    } else {
        Ok(value.to::<u64>())
    }
}

fn bridge_daemon_error(error: anyhow::Error) -> BridgeError {
    BridgeError {
        code: 4900,
        message: error.to_string(),
    }
}

fn bridge_denied(action: &str, reason: String) -> BridgeError {
    BridgeError {
        code: 4001,
        message: format!("{action} denied: {reason}"),
    }
}

fn invalid_params(message: impl Into<String>) -> BridgeError {
    BridgeError {
        code: -32602,
        message: message.into(),
    }
}

fn dev_signature_for_message(message: &SignMessage) -> Bytes {
    let digest = match message.signing_digest() {
        Some(digest) => digest,
        None => alloy_primitives::keccak256(frame_dev_personal_message(message)),
    };
    let mut signature = [0_u8; 65];
    signature[..32].copy_from_slice(digest.as_slice());
    signature[32..64].copy_from_slice(digest.as_slice());
    signature[64] = 27;
    Bytes::copy_from_slice(&signature)
}

fn dev_tx_hash_for_intent(intent: &Intent) -> B256 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&intent.chain_id.to_be_bytes());
    bytes.extend_from_slice(intent.to.as_slice());
    bytes.extend_from_slice(&intent.value.to_be_bytes::<32>());
    bytes.extend_from_slice(intent.calldata.as_ref());
    alloy_primitives::keccak256(bytes)
}

fn frame_dev_personal_message(message: &SignMessage) -> Vec<u8> {
    match &message.kind {
        SignMessageKind::PersonalSign { message } => {
            let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
            let mut framed = prefix.into_bytes();
            framed.extend_from_slice(message.as_ref());
            framed
        }
        _ => Vec::new(),
    }
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, BridgeError> {
    if !hex.len().is_multiple_of(2) {
        return Err(invalid_params("hex string has odd length"));
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let high = hex_nibble(bytes[i])?;
        let low = hex_nibble(bytes[i + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8, BridgeError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid_params("hex string contains a non-hex character")),
    }
}

fn hex_prefixed(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
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
    let socket_display = wallet.socket_path().display().to_string();
    let mock_account = dev_mock_account.clone();
    let backend = match dev_mock_account {
        Some(account) => BridgeBackend::DevMock { account },
        None => BridgeBackend::from_env(wallet),
    };
    let bridge = Arc::new(BrowserBridge::new(chain_id, backend));
    let listener = TcpListener::bind(bind).await?;
    match mock_account {
        Some(account) => eprintln!(
            "Deckard browser bridge listening on http://{bind}/rpc — DEV MOCK account {account} (holds no keys, no daemon)."
        ),
        None => eprintln!(
            "Deckard browser bridge listening on http://{bind}/rpc — dialing signer daemon at {socket_display} (chain {chain_id}).\n\
             If requests fail with a connect error, start `just demo` or `just qa`, unlock the wallet, then retry. The bridge holds no keys."
        ),
    }
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
                    method: "wallet_sendCalls".into(),
                    params: Value::Null,
                },
            )
            .await;
        let error = response.error.expect("unsupported method error");
        assert_eq!(error.code, 4200);
        assert!(error.message.contains("wallet_sendCalls"));
    }

    #[tokio::test]
    async fn send_transaction_requires_connected_account() {
        let response = bridge()
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(20),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0x0000000000000000000000000000000000000001", "value": "0x1" }]),
                },
            )
            .await;
        assert_eq!(response.error.expect("unauthorized").code, 4100);
    }

    #[tokio::test]
    async fn send_transaction_native_send_returns_dev_hash() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(21),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0x0000000000000000000000000000000000000001", "value": "0x1" }]),
                },
            )
            .await;
        assert!(response.error.is_none(), "{response:?}");
        let tx_hash = response.result.unwrap().as_str().unwrap().to_string();
        assert!(tx_hash.starts_with("0x"));
        assert_eq!(tx_hash.len(), 66);
    }

    #[tokio::test]
    async fn send_transaction_rejects_wrong_from_account() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(22),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": "0x0000000000000000000000000000000000000002", "to": "0x0000000000000000000000000000000000000001", "value": "0x1" }]),
                },
            )
            .await;
        assert_eq!(response.error.expect("wrong-account error").code, 4100);
    }

    #[tokio::test]
    async fn send_transaction_rejects_unknown_contract_selector() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(23),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0x0000000000000000000000000000000000000001", "data": "0xdeadbeef00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001" }]),
                },
            )
            .await;
        let error = response.error.expect("unknown selector refusal");
        assert_eq!(error.code, 4200);
        assert!(error.message.contains("unsupported transaction calldata"));
    }

    #[tokio::test]
    async fn send_transaction_erc20_transfer_returns_dev_hash() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(24),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "data": "0xa9059cbb00000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240" }]),
                },
            )
            .await;
        assert!(response.error.is_none(), "{response:?}");
        let tx_hash = response.result.unwrap().as_str().unwrap().to_string();
        assert!(tx_hash.starts_with("0x"));
        assert_eq!(tx_hash.len(), 66);
    }

    #[tokio::test]
    async fn send_transaction_erc20_approve_returns_dev_hash() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(25),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "data": "0x095ea7b300000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240" }]),
                },
            )
            .await;
        assert!(response.error.is_none(), "{response:?}");
        let tx_hash = response.result.unwrap().as_str().unwrap().to_string();
        assert!(tx_hash.starts_with("0x"));
        assert_eq!(tx_hash.len(), 66);
    }

    #[tokio::test]
    async fn send_transaction_erc20_calldata_rejects_native_value() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(26),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "value": "0x1", "data": "0x095ea7b300000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240" }]),
                },
            )
            .await;
        let error = response.error.expect("native value refusal");
        assert_eq!(error.code, 4200);
        assert!(error.message.contains("native value"));
    }

    #[tokio::test]
    async fn send_transaction_erc20_calldata_rejects_malformed_length() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(27),
                    method: "eth_sendTransaction".into(),
                    params: json!([{ "from": DEFAULT_DEV_ACCOUNT, "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", "data": "0xa9059cbb00000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e2" }]),
                },
            )
            .await;
        let error = response.error.expect("malformed calldata refusal");
        assert_eq!(error.code, -32602);
        assert!(error.message.contains("ERC-20 calldata"));
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

    #[tokio::test]
    async fn personal_sign_requires_connected_account_and_returns_dev_signature() {
        let bridge = bridge();
        let before_connect = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(2),
                    method: "personal_sign".into(),
                    params: json!(["0x68656c6c6f", DEFAULT_DEV_ACCOUNT]),
                },
            )
            .await;
        assert_eq!(before_connect.error.expect("unauthorized").code, 4100);

        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;

        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(3),
                    method: "personal_sign".into(),
                    params: json!(["0x68656c6c6f", DEFAULT_DEV_ACCOUNT]),
                },
            )
            .await;
        let signature = response
            .result
            .expect("signature result")
            .as_str()
            .expect("hex signature")
            .to_string();
        assert!(signature.starts_with("0x"));
        assert_eq!(signature.len(), 132);
    }

    #[tokio::test]
    async fn personal_sign_accepts_legacy_account_first_order() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(3),
                    method: "personal_sign".into(),
                    params: json!([DEFAULT_DEV_ACCOUNT, "hello"]),
                },
            )
            .await;
        assert!(response.error.is_none(), "{response:?}");
        assert_eq!(response.result.unwrap().as_str().unwrap().len(), 132);
    }

    #[tokio::test]
    async fn eth_sign_is_refused_explicitly() {
        let response = bridge()
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(4),
                    method: "eth_sign".into(),
                    params: json!([DEFAULT_DEV_ACCOUNT, "0x1234"]),
                },
            )
            .await;
        let error = response.error.expect("eth_sign refusal");
        assert_eq!(error.code, 4200);
        assert!(error.message.contains("raw eth_sign"));
    }

    #[tokio::test]
    async fn typed_data_v4_returns_dev_signature_for_walletbeat_shape() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;

        let typed_data = json!({
            "domain": {
                "name": "Test Signature App",
                "version": "1",
                "chainId": 11155111,
                "verifyingContract": "0x0000000000000000000000000000000000000000"
            },
            "types": {
                "EIP712Domain": [
                    {"name": "name", "type": "string"},
                    {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"},
                    {"name": "verifyingContract", "type": "address"}
                ],
                "TestMessage": [
                    {"name": "purpose", "type": "string"},
                    {"name": "message", "type": "string"}
                ]
            },
            "primaryType": "TestMessage",
            "message": {
                "purpose": "Educational Testing Only",
                "message": "This signature is for testing purposes only."
            }
        });
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(5),
                    method: "eth_signTypedData_v4".into(),
                    params: json!([DEFAULT_DEV_ACCOUNT, typed_data]),
                },
            )
            .await;
        assert!(response.error.is_none(), "{response:?}");
        assert_eq!(response.result.unwrap().as_str().unwrap().len(), 132);
    }

    #[tokio::test]
    async fn signing_rejects_wrong_connected_account() {
        let bridge = bridge();
        let _ = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(1),
                    method: "eth_requestAccounts".into(),
                    params: Value::Null,
                },
            )
            .await;
        let response = bridge
            .handle_request(
                ORIGIN,
                BridgeRequest {
                    id: json!(6),
                    method: "personal_sign".into(),
                    params: json!(["0x68656c6c6f", "0x0000000000000000000000000000000000000001"]),
                },
            )
            .await;
        assert_eq!(response.error.expect("wrong-account error").code, 4100);
    }
}
