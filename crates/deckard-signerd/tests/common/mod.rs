//! Shared helpers for the deckard-signerd integration tests: unique temp dirs, vault
//! sealing, spawning the daemon binary, and (optionally) a local anvil node + chain reads.

#![allow(dead_code)] // each test binary uses a different subset of these helpers

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionReceipt;
use alloy_primitives::{Address, B256, U256};
use deckard_contract::{Allowlist, ApprovalMode, Effect, Policy, Rule, POLICY_VERSION};
use deckard_core::{KdfParams, Vault};

/// Anvil's default dev mnemonic — account 0 is prefunded with 10000 ETH at the same BIP-44
/// path the keystore derives, so a vault sealed from this phrase controls a funded account.
pub const MNEMONIC: &str = "test test test test test test test test test test test junk";
/// The keystore passphrase the tests seal with and unlock over the socket.
pub const PASS: &str = "integration-test-pass";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Fast Argon2 params for tests (8 MiB / t=1) — the keystore's production params are too slow
/// to run on every unlock here. (`KdfParams` fields are public, so we construct directly; this
/// is the in-bounds minimum `validate()` accepts.)
pub fn fast_kdf() -> KdfParams {
    KdfParams {
        m_kib: 8 * 1024,
        t: 1,
        p: 1,
    }
}

/// A uniquely-named temp dir, removed on drop.
pub struct TempDir {
    dir: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("deckard-it-{tag}-{}-{n}", std::process::id()));
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

/// Default per-tx cap the cap-semantics tests assert against: 0.05 ETH.
pub const TEST_PER_TX_CAP_WEI: u128 = 50_000_000_000_000_000;
/// Default daily cap: 0.2 ETH.
pub const TEST_DAILY_CAP_WEI: u128 = 200_000_000_000_000_000;

/// The v1 policy the integration tests run against by default: a `Send` rule with the OverCap
/// approval mode + a 0.05 ETH per-tx cap + any recipient, a `Shield` rule (same mode), and a
/// `Swap` rule allowing any token. This preserves the pre-v2 default-policy semantics the e2e
/// tests assert (within cap → Allow, over cap → NeedsApproval, any swap token admitted) — the
/// daemon's *built-in* default is now the friendlier Always-card policy, so the tests pin the
/// old cap/swap behavior with an explicit on-disk policy instead.
pub fn test_policy() -> Policy {
    Policy {
        version: POLICY_VERSION,
        default_effect: Effect::Deny,
        revoked: false,
        daily_cap_wei: U256::from(TEST_DAILY_CAP_WEI),
        auto_shield_min_wei: U256::from(10_000_000_000_000_000u128),
        spent_today_wei: U256::ZERO,
        rules: vec![
            Rule::Send {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: Some(U256::from(TEST_PER_TX_CAP_WEI)),
                recipients: Allowlist::Any,
            },
            Rule::Shield {
                approval: ApprovalMode::OverCap,
                per_tx_cap_wei: None,
            },
            Rule::Swap {
                tokens: Allowlist::Any,
            },
        ],
    }
}

/// Write `policy` to `<dir>/policy.json` (the path the daemon loads). Used by tests that need a
/// policy other than [`test_policy`] (a tight cap, a restricted allowlist, a specific mode).
pub fn write_policy(dir: &Path, policy: &Policy) {
    std::fs::write(
        dir.join("policy.json"),
        serde_json::to_vec(policy).expect("serialize policy"),
    )
    .expect("write policy.json");
}

/// Seal a vault for anvil's account 0 into `<dir>/vault.bin`, write the default cap-semantics
/// [`test_policy`] alongside it, and return `(account0_address, account1_address)` (the wallet +
/// a recipient), derived locally without any key leaving this process. Tests that need a
/// different policy simply [`write_policy`] over the file afterward.
pub fn seal_account0(dir: &Path) -> (Address, Address) {
    let vault = Vault::import_mnemonic(MNEMONIC, PASS, fast_kdf()).expect("import mnemonic");
    let unlocked = vault.unlock(PASS).expect("unlock");
    let wallet = unlocked.primary_address().expect("account 0 address");
    let recipient = unlocked.account_address(1).expect("account 1 address");
    vault
        .write_atomic(&dir.join("vault.bin"))
        .expect("write vault");
    write_policy(dir, &test_policy());
    (wallet, recipient)
}

/// A spawned daemon process; killed on drop. Carries the private capability channel the test
/// harness mints exactly as the app's supervisor does (PRD-01), so a test can approve via
/// [`DaemonProc::resolve`] — a `Resolve` on the public socket is now refused.
pub struct DaemonProc {
    child: Child,
    pub socket_path: PathBuf,
    control: deckard_signerd::ControlChannel,
}

impl DaemonProc {
    /// Approve/deny a pending request over the authenticated control channel (the daemon
    /// refuses `Resolve` on the public socket). Panics on a transport error — a test that
    /// can't reach the resolver channel should fail loudly.
    pub fn resolve(&self, request_id: deckard_contract::RequestId, approved: bool) {
        self.control
            .resolve(request_id, approved)
            .expect("resolve over control channel");
    }
}

impl Drop for DaemonProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn the real `deckard-signerd` binary against `dir` (config + socket) and `rpc_url`,
/// waiting until it binds the socket. Attaches the resolver capability channel by fd
/// inheritance, mirroring `deckard_signerd::supervise` — the daemon serves `Resolve` only on
/// that inherited end.
pub fn spawn_daemon(
    dir: &Path,
    rpc_url: &str,
    chain_id: u64,
    extra_env: &[(&str, &str)],
) -> DaemonProc {
    use std::os::fd::AsRawFd;

    let socket = dir.join("signerd.sock");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_deckard-signerd"));
    cmd.env("DECKARD_CONFIG_DIR", dir)
        .env("DECKARD_SOCKET_PATH", &socket)
        .env("DECKARD_RPC_URL", rpc_url)
        .env("DECKARD_CHAIN_ID", chain_id.to_string());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }

    // Mint + pass the capability channel exactly as the app supervisor does: the child
    // inherits `child_fd` (named in DECKARD_RESOLVE_FD); the harness keeps the app end.
    let (app_end, child_fd) = deckard_signerd::supervise::control_pair().expect("control pair");
    cmd.env("DECKARD_RESOLVE_FD", child_fd.as_raw_fd().to_string());

    let child = cmd.spawn().expect("spawn deckard-signerd binary");
    drop(child_fd); // the child inherited its own copy
    let control = deckard_signerd::ControlChannel::connected(app_end);

    assert!(
        wait_for(|| socket.exists(), Duration::from_secs(5)),
        "daemon never bound its socket"
    );
    DaemonProc {
        child,
        socket_path: socket,
        control,
    }
}

/// Poll `cond` until true or `timeout` elapses.
pub fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

// --- anvil lane ----------------------------------------------------------------------------

/// Whether `anvil` is on PATH (tests that broadcast skip gracefully when it isn't).
pub fn anvil_available() -> bool {
    Command::new("anvil")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A spawned anvil node; killed on drop.
pub struct Anvil {
    child: Child,
    port: u16,
    chain_id: u64,
}

impl Anvil {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start a local anvil (chain 31337, prefunded dev accounts, automine).
pub fn start_anvil() -> Anvil {
    let port = free_port();
    let child = Command::new("anvil")
        .args([
            "--mnemonic",
            MNEMONIC,
            "--chain-id",
            "31337",
            "--accounts",
            "10",
            "--balance",
            "10000",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anvil");
    Anvil {
        child,
        port,
        chain_id: 31337,
    }
}

impl Anvil {
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

/// Start a FRESH anvil forking a chain at a pinned block, prefunding account-0 of [`MNEMONIC`]
/// (so a vault sealed from that phrase controls a funded EOA). A fresh fork each run is
/// deterministic — re-using a non-reset fork would accumulate the EOA's balance and drift the
/// asserts. Killed on drop. The fork preserves the upstream chain id (e.g. Sepolia 11155111).
pub fn start_anvil_fork(fork_url: &str, fork_block: u64, chain_id: u64) -> Anvil {
    let port = free_port();
    let child = Command::new("anvil")
        .args([
            "--fork-url",
            fork_url,
            "--fork-block-number",
            &fork_block.to_string(),
            "--mnemonic",
            MNEMONIC,
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anvil fork");
    Anvil {
        child,
        port,
        chain_id,
    }
}

/// Wait until anvil answers JSON-RPC.
pub async fn wait_anvil_ready(url: &str) {
    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if provider.get_block_number().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("anvil never became ready at {url}");
}

/// Public balance of `addr` via `url`.
pub async fn balance(url: &str, addr: Address) -> U256 {
    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    provider.get_balance(addr).await.expect("get_balance")
}

/// Poll for a mined receipt of `hash` via `url`.
pub async fn wait_receipt(url: &str, hash: B256) -> Option<TransactionReceipt> {
    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(Some(receipt)) = provider.get_transaction_receipt(hash).await {
            return Some(receipt);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}
