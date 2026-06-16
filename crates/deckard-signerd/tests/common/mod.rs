//! Shared helpers for the deckard-signerd integration tests: unique temp dirs, vault
//! sealing, spawning the daemon binary, and (optionally) a local anvil node + chain reads.

#![allow(dead_code)] // each test binary uses a different subset of these helpers

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::TransactionReceipt;
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
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

/// Seal a vault for anvil's account 0 into `<dir>/vault.bin` and return
/// `(account0_address, account1_address)` (the wallet + a recipient), derived locally without
/// any key leaving this process.
pub fn seal_account0(dir: &Path) -> (Address, Address) {
    let vault = Vault::import_mnemonic(MNEMONIC, PASS, fast_kdf()).expect("import mnemonic");
    let unlocked = vault.unlock(PASS).expect("unlock");
    let wallet = unlocked.primary_address().expect("account 0 address");
    let recipient = unlocked.account_address(1).expect("account 1 address");
    vault
        .write_atomic(&dir.join("vault.bin"))
        .expect("write vault");
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

// --- ERC-20 fork seeding (swap_e2e) --------------------------------------------------------
//
// `swap_e2e` needs the wallet to hold an ERC-20 (WETH) so the real exact-gross approve broadcast
// succeeds, and it credits a buy token to demonstrate the simulated fill. Both go through
// `anvil_setStorageAt`, the standard cheatcode for editing an arbitrary contract slot on a fork —
// it's just a JSON-RPC POST, so it reuses the alloy provider the rest of this file already uses
// (`raw_request`, no new dependency).

/// The storage key of `balanceOf[holder]` for a Solidity `mapping(address => uint256)` at slot
/// `slot`: `keccak256(left_pad32(holder) ‖ left_pad32(slot))`. (The canonical WETH9 / OZ ERC-20
/// layout; the exact slot index differs per token and is the helper's `slot` argument.)
pub fn erc20_balance_slot_key(holder: Address, slot: U256) -> B256 {
    let mut preimage = [0u8; 64];
    let (key, slot_word) = preimage.split_at_mut(32);
    // `mapping` key word: the 20-byte address left-padded into the low bytes of a 32-byte word.
    key[12..].copy_from_slice(holder.as_slice());
    slot_word.copy_from_slice(&slot.to_be_bytes::<32>());
    keccak256(preimage)
}

/// Credit `amount` of an ERC-20 `token` to `holder` on a fork by writing the token's
/// `balanceOf[holder]` slot directly via `anvil_setStorageAt`. Used by `swap_e2e` to give the
/// wallet the sell token (WETH) so the real exact-gross approve broadcast + (stubbed) order can
/// proceed on a fork, and to seed a buy token for the simulated fill. SETS the balance (overwrites,
/// not adds) — the seed runs on a freshly-forked deterministic anvil. Panics on a transport error,
/// like the other helpers: a seed that can't reach anvil should fail the test loudly.
pub async fn set_erc20_balance(
    url: &str,
    token: Address,
    holder: Address,
    slot: U256,
    amount: U256,
) {
    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    let key = erc20_balance_slot_key(holder, slot);
    let value = B256::from(amount.to_be_bytes::<32>());
    let _: bool = provider
        .raw_request("anvil_setStorageAt".into(), (token, key, value))
        .await
        .expect("anvil_setStorageAt");
}

/// ERC-20 `balanceOf(holder)` of `token` via `url` — `eth_call` of the 0x70a08231 selector with
/// the holder left-padded to a 32-byte word, decoded as a big-endian U256. Lets `swap_e2e` assert
/// the seeded sell token and the simulated-fill buy token landed.
pub async fn erc20_balance(url: &str, token: Address, holder: Address) -> U256 {
    use alloy::network::{Ethereum, TransactionBuilder};
    use alloy::rpc::types::TransactionRequest;

    let provider = ProviderBuilder::new().connect_http(url.parse().unwrap());
    // balanceOf(address) selector ‖ holder left-padded to 32 bytes.
    let mut calldata = Vec::with_capacity(4 + 32);
    calldata.extend_from_slice(&[0x70, 0xa0, 0x82, 0x31]);
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(holder.as_slice());

    // Disambiguate the builder to alloy's `Ethereum` network: verified-reads pulls in
    // helios-ethereum, which adds a second `TransactionBuilder` impl for `TransactionRequest`
    // (mirrors the note in signerd's `signing.rs`).
    let mut tx = TransactionRequest::default();
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_to(&mut tx, token);
    <TransactionRequest as TransactionBuilder<Ethereum>>::set_input(&mut tx, Bytes::from(calldata));
    let raw = provider.call(tx).await.expect("balanceOf eth_call");
    U256::from_be_slice(raw.as_ref())
}
