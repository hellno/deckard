//! QA-only signerd supervisor for browser/WalletBeat local-chain tests.
//!
//! This starts the real `deckard-signerd` binary with the same private resolver capability
//! channel that the app supervisor uses, unlocks the throwaway QA vault, then auto-approves
//! pending requests. It is intentionally an example binary, not production code.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use deckard_contract::{ApprovalStatus, UnlockOutcome};
use deckard_signerd::{ControlChannel, SignerClient};

const QA_PASS: &str = "deckard-qa";

fn main() -> anyhow::Result<()> {
    let config_dir = std::env::var_os("DECKARD_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("DECKARD_CONFIG_DIR must point at a throwaway QA dir"))?;
    let socket_path = std::env::var_os("DECKARD_SOCKET_PATH")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("DECKARD_SOCKET_PATH must point inside the QA dir"))?;
    let rpc_url = std::env::var("DECKARD_RPC_URL")?;
    let chain_id = std::env::var("DECKARD_CHAIN_ID")?;

    let stop = Arc::new(AtomicBool::new(false));
    watch_stdin(Arc::clone(&stop));

    let (app_end, child_fd) = deckard_signerd::supervise::control_pair()?;
    let mut child = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "--locked",
            "-p",
            "deckard-signerd",
            "--no-default-features",
            "--bin",
            "deckard-signerd",
        ])
        .env("DECKARD_CONFIG_DIR", &config_dir)
        .env("DECKARD_SOCKET_PATH", &socket_path)
        .env("DECKARD_RPC_URL", &rpc_url)
        .env("DECKARD_CHAIN_ID", &chain_id)
        .env("DECKARD_RESOLVE_FD", child_fd.as_raw_fd().to_string())
        .spawn()?;
    drop(child_fd);

    let socket_path = std::path::PathBuf::from(socket_path);
    wait_for_socket(&socket_path, &mut child)?;

    let client = SignerClient::new(socket_path);
    match client.unlock_blocking(QA_PASS)? {
        UnlockOutcome::Unlocked { address } => {
            println!("qa-supervisor: unlocked throwaway QA wallet {address:#x}");
        }
        UnlockOutcome::BadPassphrase => anyhow::bail!("qa-supervisor: QA passphrase rejected"),
        UnlockOutcome::NoVault => anyhow::bail!("qa-supervisor: no QA vault in config dir"),
    }

    let control = ControlChannel::connected(app_end);
    println!("qa-supervisor: ready; auto-approving pending QA requests");
    let result = approve_loop(&client, &control, &stop, &mut child);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn watch_stdin(stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 1];
        let _ = std::io::stdin().read(&mut buf);
        stop.store(true, Ordering::SeqCst);
    });
}

fn wait_for_socket(socket_path: &std::path::Path, child: &mut Child) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("deckard-signerd exited before binding socket: {status}");
        }
        if socket_path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "deckard-signerd did not bind socket at {}",
        socket_path.display()
    )
}

fn approve_loop(
    client: &SignerClient,
    control: &ControlChannel,
    stop: &AtomicBool,
    child: &mut Child,
) -> anyhow::Result<()> {
    while !stop.load(Ordering::SeqCst) {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("deckard-signerd exited during QA: {status}");
        }
        for pending in client.pending_list_blocking()? {
            if pending.status == ApprovalStatus::Pending {
                control.resolve(pending.request_id, true)?;
                println!("qa-supervisor: approved request {:#x}", pending.request_id);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
