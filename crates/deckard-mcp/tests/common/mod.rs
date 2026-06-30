//! Acceptance-test harness: an in-process **mock signerd** speaking the real wire (frames +
//! `deckard_contract::evaluate`, so the verdict can't drift from the daemon), a spawned
//! `deckard-mcp --mcp` child driven over raw line-delimited JSON-RPC (every byte of both
//! directions captured as THE transcript), and the T9 structural allowlist scanner.
//!
//! Every read carries a HARD timeout — a wedged stdio server fails the suite in seconds,
//! never hangs CI.

#![allow(dead_code)] // each test binary uses a subset

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy_primitives::{Address, B256, U256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::{Child, ChildStdin, Command};

use deckard_contract::{
    evaluate, ActivityLifecycle, Allowlist, ApprovalMode, ApprovalStatus, Decision, Effect,
    ExecuteResult, Intent, Policy, RailgunViewGrant, ReadStatus, RequestId, Rule,
    SignMessageResult, SignOrderResult, SignerRequest, SignerResponse, StatusView, UnlockOutcome,
    POLICY_VERSION,
};
use deckard_signerd::{frame, request_id_for};

/// Hard ceiling for any single read from the child (CI requirement: no hangs).
pub const IO_TIMEOUT: Duration = Duration::from_secs(20);

/// The demo chain (Sepolia) — what the mock daemon signs for and the child is pointed at.
pub const MOCK_CHAIN: u64 = 11_155_111;
/// The mock wallet's public address.
pub fn mock_address() -> Address {
    Address::repeat_byte(0x42)
}
/// The deterministic broadcast hash the mock returns.
pub fn mock_tx_hash() -> B256 {
    B256::repeat_byte(0xab)
}

/// 0.05 ETH per-tx cap (the default-policy number the acceptance scenario uses).
pub const PER_TX_CAP_WEI: u128 = 50_000_000_000_000_000;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniquely-named temp dir, removed on drop.
pub struct TempDir {
    dir: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("deckard-mcp-it-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self { dir }
    }
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// --- the mock signerd ------------------------------------------------------------------

/// Mock daemon state. Verdicts come from the ONE shared `evaluate` (the parity contract);
/// the process-level pre-checks (locked / chain) mirror the daemon's order.
pub struct MockState {
    pub policy: Policy,
    pub locked: bool,
    requests: HashMap<RequestId, MockReq>,
    /// When set, `Execute` fails with exactly this reason — UNREDACTED. Used to prove the
    /// T9 scanner catches a daemon that leaks (the real daemon's redaction is pinned in
    /// deckard-signerd's own tests).
    pub force_broadcast_error: Option<String>,
    /// The view grant the mock serves: a REAL 0zk address (the sidecar parses it) + a fake
    /// viewing key that doubles as a leak canary for the transcript scan.
    pub grant: (String, String),
    pub balance_wei: U256,
}

struct MockReq {
    intent: Intent,
    status: ApprovalStatus,
    approved: bool,
    broadcast: bool,
}

pub fn demo_policy() -> Policy {
    // Policy v2 (ADR 0005): a versioned default-deny rule list. The acceptance suite derives
    // `per_tx_cap_eth == "0.05"` / `require_approval == "over_cap"` from the Send rule, so its
    // cap is `PER_TX_CAP_WEI` (0.05 ETH) and its mode is `OverCap`.
    //
    // The acceptance scenarios drive `deckard_shield`, and a v2 `Shield` rule carries NO per-tx
    // cap (only `Send`/`Unshield` do) — so the global daily cap is the one fence a shield is
    // gated against. It is `PER_TX_CAP_WEI` (0.05 ETH) so the suite's invariants hold under the
    // OverCap mode exactly as they did under v1's flat per-tx cap: a 0.02/0.01 ETH shield is
    // within the fence (Allow), a 0.2 ETH shield is over it (NeedsApproval).
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(PER_TX_CAP_WEI), // 0.05 ETH — the shield fence (see above)
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128), // 0.01 ETH
        spent_today_wei: U256::ZERO,
        rules: vec![
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(PER_TX_CAP_WEI)),
                recipients: Allowlist::Any,
            },
            Rule::Shield {
                approval: ApprovalMode::OverCap,
            },
            Rule::Swap {
                tokens: Allowlist::Any, // any token; swap tools land in the MCP child (#26)
            },
        ],
    }
}

/// A real, parseable 0zk address for the mock wallet (fixed entropy → deterministic) plus
/// the matching viewing key — the latter is the in-transcript leak canary.
pub fn mock_grant() -> (String, String) {
    deckard_core::railgun_keys::railgun_view_grant_from_entropy(&[42u8; 16], MOCK_CHAIN, 0)
        .expect("derive mock grant")
}

impl MockState {
    pub fn new() -> Self {
        Self {
            policy: demo_policy(),
            locked: false, // the mock plays an already-unlocked daemon
            requests: HashMap::new(),
            force_broadcast_error: None,
            grant: mock_grant(),
            balance_wei: U256::from(1_000_000_000_000_000_000u128), // 1 ETH funded
        }
    }

    fn handle(&mut self, req: SignerRequest) -> SignerResponse {
        match req {
            SignerRequest::Unlock { .. } => {
                self.locked = false;
                self.policy.revoked = false;
                self.requests.clear();
                SignerResponse::Unlock(UnlockOutcome::Unlocked {
                    address: mock_address(),
                })
            }
            SignerRequest::Lock | SignerRequest::RevokeAll => {
                self.locked = true;
                self.policy.revoked = true;
                for r in self.requests.values_mut() {
                    if !r.broadcast
                        && matches!(r.status, ApprovalStatus::Pending | ApprovalStatus::Allowed)
                    {
                        r.status = ApprovalStatus::Denied {
                            reason: "revoked".into(),
                        };
                    }
                }
                SignerResponse::Ack
            }
            SignerRequest::Resolve {
                request_id,
                approved,
            } => {
                if let Some(r) = self.requests.get_mut(&request_id) {
                    if r.status == ApprovalStatus::Pending {
                        if approved {
                            r.status = ApprovalStatus::Allowed;
                            r.approved = true;
                        } else {
                            r.status = ApprovalStatus::Denied {
                                reason: "user_denied".into(),
                            };
                        }
                    }
                }
                SignerResponse::Ack
            }
            SignerRequest::Propose { intent, .. } => {
                SignerResponse::Decision(self.propose(&intent))
            }
            SignerRequest::Execute { request_id } => {
                SignerResponse::Execute(self.execute(request_id))
            }
            SignerRequest::Status { request_id } => {
                SignerResponse::Status(match self.requests.get(&request_id) {
                    Some(r) => r.status.clone(),
                    None => ApprovalStatus::Denied {
                        reason: "unknown_request".into(),
                    },
                })
            }
            SignerRequest::StatusView { request_id } => {
                SignerResponse::StatusView(match self.requests.get(&request_id) {
                    Some(r) => StatusView {
                        request_id,
                        status: r.status.clone(),
                        // A snapshot: a live Pending/Allowed card has time left; terminal
                        // states report 0 (mirrors the daemon's contract).
                        remaining_ms: match r.status {
                            ApprovalStatus::Pending | ApprovalStatus::Allowed => 60_000,
                            _ => 0,
                        },
                        tx_hash: if r.broadcast {
                            Some(mock_tx_hash())
                        } else {
                            None
                        },
                        lifecycle: lifecycle_for(r),
                    },
                    // An unknown request id is Expired in the lifecycle and carries the
                    // unknown_request deny tag (matches the daemon's `StatusView` contract).
                    None => StatusView {
                        request_id,
                        status: ApprovalStatus::Denied {
                            reason: "unknown_request".into(),
                        },
                        remaining_ms: 0,
                        tx_hash: None,
                        lifecycle: ActivityLifecycle::Expired,
                    },
                })
            }
            SignerRequest::PolicyGet => SignerResponse::Policy(self.policy.clone()),
            SignerRequest::Address => {
                if self.locked {
                    SignerResponse::Decision(Decision::Deny {
                        reason: "locked".into(),
                    })
                } else {
                    SignerResponse::Address(mock_address())
                }
            }
            SignerRequest::Balance { .. } => {
                SignerResponse::Balance(deckard_contract::BalanceReport {
                    public_wei: self.balance_wei,
                    shielded_wei: U256::ZERO,
                    read_status: ReadStatus::unsynced("verification disabled"),
                })
            }
            SignerRequest::RailgunViewGrant { .. } => {
                if self.locked {
                    SignerResponse::Decision(Decision::Deny {
                        reason: "locked".into(),
                    })
                } else {
                    SignerResponse::RailgunView(RailgunViewGrant {
                        address: self.grant.0.clone(),
                        viewing_key: self.grant.1.clone(),
                    })
                }
            }
            // Swap (CoW) requests are exercised by the dedicated swap-trust-path + MCP-swap
            // children (#24/#26); this MCP acceptance mock predates them and only needs to stay
            // exhaustive. It answers honestly: it does not implement the swap path.
            SignerRequest::ProposeOrder { .. } => SignerResponse::Decision(Decision::Deny {
                reason: "swap_unsupported_in_mock".into(),
            }),
            SignerRequest::SignOrder { .. } => SignerResponse::SignOrder(SignOrderResult::Denied {
                reason: "swap_unsupported_in_mock".into(),
            }),
            // Message-signing is intentionally outside the current MCP acceptance mock: the
            // browser bridge will own dapp-origin message requests. Stay exhaustive and fail closed.
            SignerRequest::ProposeMessage { .. } => SignerResponse::Decision(Decision::Deny {
                reason: "message_signing_unsupported_in_mock".into(),
            }),
            SignerRequest::SignMessage { .. } => {
                SignerResponse::SignMessage(SignMessageResult::Denied {
                    reason: "message_signing_unsupported_in_mock".into(),
                })
            }
            SignerRequest::CancelOrder { .. } => SignerResponse::Execute(ExecuteResult::Denied {
                reason: "swap_unsupported_in_mock".into(),
            }),
            SignerRequest::PendingList => SignerResponse::Pending(Vec::new()),
            // The MCP surface never reads the activity feed (it's a GUI-only surface); the mock
            // just answers an empty ledger so the request shape stays covered.
            SignerRequest::ActivityFeed => SignerResponse::Activity(Vec::new()),
        }
    }

    fn propose(&mut self, intent: &Intent) -> Decision {
        // The daemon's pre-check order: chain (needs no key), then locked, then the shared
        // evaluate. Chain-first is what makes a `locked` reply conclusive for the sidecar's
        // connect-time chain probe — keep this in lockstep with deckard-signerd's `propose`.
        if intent.chain_id != MOCK_CHAIN {
            return Decision::Deny {
                reason: "chain_mismatch".into(),
            };
        }
        if self.locked {
            return Decision::Deny {
                reason: "locked".into(),
            };
        }
        let id = request_id_for(intent);
        if let Some(existing) = self.requests.get(&id) {
            return match &existing.status {
                _ if existing.broadcast => Decision::Deny {
                    reason: "already_executed".into(),
                },
                ApprovalStatus::Pending => Decision::NeedsApproval { request_id: id },
                ApprovalStatus::Allowed => Decision::Allow,
                ApprovalStatus::Denied { reason } => Decision::Deny {
                    reason: reason.clone(),
                },
                ApprovalStatus::Expired => Decision::Deny {
                    reason: "expired".into(),
                },
            };
        }
        let status = match evaluate(intent, &self.policy) {
            deny @ Decision::Deny { .. } => return deny,
            Decision::Allow => ApprovalStatus::Allowed,
            Decision::NeedsApproval { .. } => ApprovalStatus::Pending,
        };
        self.requests.insert(
            id,
            MockReq {
                intent: intent.clone(),
                status: status.clone(),
                approved: false,
                broadcast: false,
            },
        );
        match status {
            ApprovalStatus::Allowed => Decision::Allow,
            _ => Decision::NeedsApproval { request_id: id },
        }
    }

    fn execute(&mut self, request_id: RequestId) -> ExecuteResult {
        if self.locked {
            return ExecuteResult::Denied {
                reason: "revoked".into(),
            };
        }
        let forced = self.force_broadcast_error.clone();
        let policy = self.policy.clone();
        let Some(req) = self.requests.get_mut(&request_id) else {
            return ExecuteResult::Denied {
                reason: "unknown_request".into(),
            };
        };
        if req.broadcast {
            return ExecuteResult::Denied {
                reason: "already_executed".into(),
            };
        }
        match &req.status {
            ApprovalStatus::Allowed => {}
            ApprovalStatus::Pending => {
                return ExecuteResult::Denied {
                    reason: "not_approved".into(),
                }
            }
            ApprovalStatus::Denied { reason } => {
                return ExecuteResult::Denied {
                    reason: reason.clone(),
                }
            }
            ApprovalStatus::Expired => {
                return ExecuteResult::Denied {
                    reason: "expired".into(),
                }
            }
        }
        if !req.approved && evaluate(&req.intent, &policy) != Decision::Allow {
            return ExecuteResult::Denied {
                reason: "cap_exceeded".into(),
            };
        }
        if let Some(reason) = forced {
            return ExecuteResult::Denied { reason };
        }
        req.broadcast = true;
        ExecuteResult::Broadcast {
            tx_hash: mock_tx_hash(),
        }
    }

    /// Test-only: flip a pending record's status the way a human approval/denial does in the
    /// app (the MCP sidecar can't self-approve, so the acceptance loop drives this directly
    /// on the shared state). Mirrors the `Resolve` arm but reusable from tests.
    pub fn resolve(&mut self, request_id: RequestId, approved: bool) {
        if let Some(r) = self.requests.get_mut(&request_id) {
            if r.status == ApprovalStatus::Pending {
                if approved {
                    r.status = ApprovalStatus::Allowed;
                    r.approved = true;
                } else {
                    r.status = ApprovalStatus::Denied {
                        reason: "user_denied".into(),
                    };
                }
            }
        }
    }

    /// Force a pending card to lapse its approval window — the mock stand-in for the daemon's
    /// TTL expiry, so a test can drive the Expired branch deterministically (no human approves
    /// in time).
    pub fn expire(&mut self, request_id: RequestId) {
        if let Some(r) = self.requests.get_mut(&request_id) {
            if r.status == ApprovalStatus::Pending {
                r.status = ApprovalStatus::Expired;
            }
        }
    }
}

/// Map a mock request record to its lifecycle position — the same split the daemon's
/// `StatusView` uses (Executed once broadcast, then by approval status).
fn lifecycle_for(req: &MockReq) -> ActivityLifecycle {
    if req.broadcast {
        return ActivityLifecycle::Executed;
    }
    match &req.status {
        ApprovalStatus::Pending => ActivityLifecycle::Proposed,
        ApprovalStatus::Allowed => ActivityLifecycle::Decided { approved: true },
        ApprovalStatus::Denied { .. } => ActivityLifecycle::Decided { approved: false },
        ApprovalStatus::Expired => ActivityLifecycle::Expired,
    }
}

/// Spawn the mock daemon on `socket`. Returns a handle to its shared state (tests mutate it
/// to force errors). Serves until the listener is dropped with the runtime.
pub fn spawn_mock_daemon(socket: &Path) -> Arc<Mutex<MockState>> {
    let state = Arc::new(Mutex::new(MockState::new()));
    let listener = UnixListener::bind(socket).expect("bind mock daemon socket");
    let state_srv = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let state = Arc::clone(&state_srv);
            tokio::spawn(async move {
                loop {
                    let Ok(Some(buf)) = frame::read_frame(&mut stream).await else {
                        return;
                    };
                    let Ok(req) = frame::decode::<SignerRequest>(&buf) else {
                        return;
                    };
                    let resp = state.lock().expect("mock state lock").handle(req);
                    let Ok(body) = frame::encode(&resp) else {
                        return;
                    };
                    if frame::write_frame(&mut stream, &body).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    state
}

// --- the MCP child + raw JSON-RPC client ------------------------------------------------

/// A spawned `deckard-mcp --mcp` child plus the FULL transcript: every JSON-RPC line sent
/// and received (the thing T9 scans), and the child's captured stderr.
pub struct McpChild {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    pub transcript: Vec<String>,
    pub stderr: Arc<Mutex<Vec<String>>>,
    next_id: u64,
}

impl McpChild {
    /// Spawn against `socket`, with a scrubbed env (only the wiring we set), plus
    /// `extra_env` (e.g. the poisoned `DECKARD_RPC_URL` for the env-leak case).
    pub async fn spawn(socket: &Path, config_dir: &Path, extra_env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_deckard-mcp"));
        cmd.arg("--mcp")
            .env_clear()
            .env("DECKARD_SOCKET_PATH", socket)
            .env("DECKARD_CONFIG_DIR", config_dir)
            .env("DECKARD_CHAIN_ID", MOCK_CHAIN.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn deckard-mcp --mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout")).lines();

        // Drain stderr concurrently (a full pipe would deadlock the child).
        let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_sink = Arc::clone(&stderr_lines);
        let mut err_reader = BufReader::new(child.stderr.take().expect("child stderr")).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = err_reader.next_line().await {
                stderr_sink.lock().expect("stderr lock").push(line);
            }
        });

        let mut this = Self {
            child,
            stdin,
            stdout,
            transcript: Vec::new(),
            stderr: stderr_lines,
            next_id: 1,
        };
        this.handshake().await;
        this
    }

    async fn handshake(&mut self) {
        let init = self
            .rpc(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": { "name": "deckard-acceptance", "version": "0.0.0" }
                }),
            )
            .await;
        assert!(
            init.get("serverInfo").is_some(),
            "initialize must return serverInfo: {init}"
        );
        self.notify("notifications/initialized", serde_json::json!({}))
            .await;
    }

    async fn send_line(&mut self, line: String) {
        self.transcript.push(line.clone());
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write to child");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush");
    }

    /// One JSON-RPC request → its result value. HARD timeout on the read.
    pub async fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let req =
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send_line(req.to_string()).await;

        loop {
            let line = tokio::time::timeout(IO_TIMEOUT, self.stdout.next_line())
                .await
                .expect("HARD TIMEOUT: no response from deckard-mcp within 20s")
                .expect("read child stdout")
                .expect("child closed stdout");
            self.transcript.push(line.clone());
            let value: serde_json::Value =
                serde_json::from_str(&line).expect("child emitted non-JSON line");
            // Skip server-initiated notifications; match our request id.
            if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(err) = value.get("error") {
                    panic!("JSON-RPC error from {method}: {err}");
                }
                return value["result"].clone();
            }
        }
    }

    pub async fn notify(&mut self, method: &str, params: serde_json::Value) {
        let req = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send_line(req.to_string()).await;
    }

    /// `tools/list` → the tools array.
    pub async fn list_tools(&mut self) -> Vec<serde_json::Value> {
        let result = self.rpc("tools/list", serde_json::json!({})).await;
        result["tools"].as_array().expect("tools array").clone()
    }

    /// `tools/call` → (is_error, first text content, full result).
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> (bool, String, serde_json::Value) {
        let result = self
            .rpc(
                "tools/call",
                serde_json::json!({ "name": name, "arguments": arguments }),
            )
            .await;
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        (is_error, text, result)
    }

    /// The full transcript (both directions) as one string, for contains-style asserts.
    pub fn transcript_text(&self) -> String {
        self.transcript.join("\n")
    }

    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
    }
}

// --- the T9 structural allowlist scanner -------------------------------------------------

/// Fields whose values are ALLOWED to be 32-byte hex (the known schema fields, permitted on
/// both the request and response side).
const HEX_ALLOWED_FIELDS: &[&str] = &["tx_hash", "request_id"];

/// Scan one free-text string for secret-shaped material. Returns findings (empty = clean).
pub fn scan_text(field: &str, text: &str, findings: &mut Vec<String>) {
    let lower = text.to_ascii_lowercase();
    // 1. Secret vocabulary — ANY occurrence. The tool descriptions are deliberately worded
    //    without these words, so the transcript-wide assert can stay absolute.
    for needle in ["passphrase", "bearer ", "authorization:"] {
        if lower.contains(needle) {
            findings.push(format!("{field}: contains {needle:?}"));
        }
    }
    // 2. 64-hex runs (raw key / hash shapes) outside the allowlisted fields.
    if !HEX_ALLOWED_FIELDS.contains(&field) {
        let mut run = 0usize;
        for c in lower.chars() {
            if c.is_ascii_hexdigit() {
                run += 1;
                if run >= 64 {
                    findings.push(format!("{field}: 64-hex run in {text:?}"));
                    break;
                }
            } else {
                run = 0;
            }
        }
    }
    // 3. Key-in-URL: any URL whose userinfo/path/query carries a long alnum token.
    for token in text.split_whitespace() {
        if let Some(idx) = token.find("://") {
            let rest = &token[idx + 3..];
            if rest.contains('@') {
                findings.push(format!(
                    "{field}: URL with userinfo credentials in {token:?}"
                ));
                continue;
            }
            let after_host = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
            let mut run = 0usize;
            for c in after_host.chars() {
                if c.is_ascii_alphanumeric() {
                    run += 1;
                    if run >= 20 {
                        findings.push(format!("{field}: long token in URL path/query {token:?}"));
                        break;
                    }
                } else {
                    run = 0;
                }
            }
        }
    }
    // 4. High-entropy-ish runs: ≥32 alnum chars mixing cases and digits (API-key shape).
    for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() >= 32
            && token.chars().any(|c| c.is_ascii_digit())
            && token.chars().any(|c| c.is_ascii_lowercase())
            && token.chars().any(|c| c.is_ascii_uppercase())
            && !HEX_ALLOWED_FIELDS.contains(&field)
        {
            findings.push(format!("{field}: high-entropy token {token:?}"));
        }
    }
}

/// Walk a JSON value structurally, scanning every string with its field name as context.
pub fn walk_json(field: &str, value: &serde_json::Value, findings: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                walk_json(k, v, findings);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_json(field, item, findings);
            }
        }
        serde_json::Value::String(s) => {
            // Tool results embed their payload as JSON-in-a-string (content[0].text); walk
            // it structurally too, so request_id/tx_hash inside stay field-allowlisted.
            match serde_json::from_str::<serde_json::Value>(s) {
                Ok(nested) if nested.is_object() || nested.is_array() => {
                    walk_json(field, &nested, findings)
                }
                _ => scan_text(field, s, findings),
            }
        }
        _ => {}
    }
}

/// T9: structurally scan an entire transcript (every line, both directions) plus stderr.
/// Returns all findings; the suite asserts empty (or, for the seeded canary, NON-empty).
pub fn scan_transcript(lines: &[String], stderr: &[String]) -> Vec<String> {
    let mut findings = Vec::new();
    for line in lines {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => walk_json("<root>", &value, &mut findings),
            Err(_) => scan_text("<non-json-line>", line, &mut findings),
        }
    }
    for line in stderr {
        scan_text("<stderr>", line, &mut findings);
    }
    findings
}
