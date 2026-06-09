//! The app's key-less bridge to `deckard-signerd`.
//!
//! This is the whole signing story for the GUI: the app **spawns + supervises** the daemon
//! and talks to it over the socket. It holds NO key material — no `UnlockedVault`, no
//! `PrivateKeySigner`. Unlock happens *in the daemon* (the app sends the passphrase and gets
//! back only an address); the send path sends an `Intent` and gets back a `Decision`/tx hash.
//! The keystore is only ever touched in-process by *onboarding* (to write `vault.bin`), never
//! to sign.

use alloy_primitives::{Address, B256, U256};
use deckard_contract::{Decision, ExecuteResult, Intent, RequestId, UnlockOutcome};
use deckard_signerd::{DaemonSupervisor, SignerClient};

/// Result of the app's send path (propose, then execute on `Allow`). The path is implemented
/// and unit-tested here; the GUI send screen that calls it is T-UX (out of scope), so no view
/// invokes it yet.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum SendOutcome {
    /// Signed + broadcast by the daemon.
    Broadcast { tx_hash: B256 },
    /// Over cap / approval-required: a card must be approved, then `execute(request_id)`.
    NeedsApproval { request_id: RequestId },
    /// Refused (locked, off-allowlist, chain mismatch, …).
    Denied { reason: String },
}

/// Owns the supervised daemon child and a client to its socket. Dropping it stops the
/// supervisor and kills the daemon.
pub struct AppSigner {
    _supervisor: DaemonSupervisor,
    client: SignerClient,
}

impl AppSigner {
    /// Launch + supervise the daemon and return a key-less handle to it. `rpc_url`/`chain_id`
    /// are passed to the daemon so it broadcasts on the same chain the app reads from.
    pub fn launch(rpc_url: String, chain_id: u64) -> Self {
        let socket_path = deckard_signerd::socket::default_socket_path();
        let supervisor = DaemonSupervisor::spawn(socket_path.clone(), rpc_url, chain_id);
        let client = SignerClient::new(socket_path);
        Self {
            _supervisor: supervisor,
            client,
        }
    }

    /// A cloneable client for background tasks (the supervisor stays owned by the app). The
    /// shell uses this for unlock/lock/send so the work runs off the UI thread.
    pub fn client(&self) -> SignerClient {
        self.client.clone()
    }
}

/// The send path, key-less: `propose` → on `Allow`, `execute`. Never signs in-process — it
/// only issues `Propose`/`Execute` over the socket. Free function over a [`SignerClient`] so
/// background threads can call it with a cheap clone. (Awaiting the T-UX send screen; proven
/// by the unit test below.)
#[allow(dead_code)]
pub fn send_blocking(client: &SignerClient, intent: &Intent) -> anyhow::Result<SendOutcome> {
    use deckard_contract::SignerRequest;

    let decision = match client.request_blocking(&SignerRequest::Propose {
        intent: intent.clone(),
    })? {
        deckard_contract::SignerResponse::Decision(d) => d,
        other => anyhow::bail!("unexpected propose response: {other:?}"),
    };
    match decision {
        Decision::Deny { reason } => Ok(SendOutcome::Denied { reason }),
        Decision::NeedsApproval { request_id } => Ok(SendOutcome::NeedsApproval { request_id }),
        Decision::Allow => {
            // The daemon assigns a deterministic id; derive it locally to execute the Allow.
            let id = SignerClient::request_id_for_intent(intent);
            match client.request_blocking(&SignerRequest::Execute { request_id: id })? {
                deckard_contract::SignerResponse::Execute(ExecuteResult::Broadcast { tx_hash }) => {
                    Ok(SendOutcome::Broadcast { tx_hash })
                }
                deckard_contract::SignerResponse::Execute(ExecuteResult::Denied { reason }) => {
                    Ok(SendOutcome::Denied { reason })
                }
                other => anyhow::bail!("unexpected execute response: {other:?}"),
            }
        }
    }
}

/// Build a key-less Railgun **shield** intent from a free-text `0zk…` recipient (T5). Parses
/// the recipient into a [`RailgunAddress`](deckard_core::RailgunAddress) — surfacing a clear
/// error on a malformed address rather than building junk calldata — then wraps
/// `deckard_core::build_shield_native_intent`. Pure + synchronous (a native shield does NO
/// client-side ZK proof; it only encrypts the note + ABI-encodes), so the caller runs it on a
/// background thread purely to keep the propose round-trip off the UI thread. Own-address
/// auto-fill (replacing the free-text recipient) is Wave 2.
pub fn build_shield_intent(
    chain_id: u64,
    recipient_0zk: &str,
    value_wei: U256,
) -> anyhow::Result<Intent> {
    let recipient: deckard_core::RailgunAddress = recipient_0zk
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("not a valid 0zk address: {e}"))?;
    deckard_core::build_shield_native_intent(chain_id, recipient, value_wei)
}

/// Parse a decimal ETH amount (`"0.05"`, `"1"`, `"1.234"`) into wei. Pure + total: rejects
/// empties, signs, non-digits, a second dot, and >18 fractional places, so the shield amount
/// field never builds a wrong-magnitude intent. Returns a short, user-facing error string.
pub fn parse_eth_to_wei(input: &str) -> Result<U256, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter an amount".into());
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err("Enter a valid amount like 0.05".into());
    }
    let all_digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(int_part) || !all_digits(frac_part) {
        return Err("Amount must be a number like 0.05".into());
    }
    if frac_part.len() > 18 {
        return Err("Too many decimal places (max 18 for ETH)".into());
    }
    // Concatenate the integer part with the fractional part right-padded to 18 digits → wei.
    let mut digits = String::with_capacity(int_part.len() + 18);
    digits.push_str(if int_part.is_empty() { "0" } else { int_part });
    digits.push_str(frac_part);
    for _ in frac_part.len()..18 {
        digits.push('0');
    }
    U256::from_str_radix(&digits, 10).map_err(|_| "Amount is too large".into())
}

/// Interpret an [`UnlockOutcome`] into either the wallet address or a user-facing error.
pub fn address_or_error(outcome: UnlockOutcome) -> Result<Address, String> {
    match outcome {
        UnlockOutcome::Unlocked { address } => Ok(address),
        UnlockOutcome::BadPassphrase => {
            Err("Wrong passphrase, or the vault was tampered with".to_string())
        }
        UnlockOutcome::NoVault => Err("No wallet found — create or import one first".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, U256};
    use deckard_contract::{ExecuteResult, IntentKind, SignerRequest, SignerResponse};
    use deckard_signerd::frame;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    fn send_intent() -> Intent {
        Intent {
            chain_id: 31337,
            to: Address::repeat_byte(0x22),
            token: None,
            value: U256::from(1_000u64),
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        }
    }

    /// #9: the app's send path issues `Propose` then `Execute` over the socket — proving it
    /// signs nothing in-process (it holds no key; it only speaks the wire). A tiny recording
    /// UDS server stands in for the daemon and replies `Allow` then `Broadcast`.
    #[test]
    fn send_path_issues_propose_then_execute_over_the_socket() {
        let dir =
            std::env::temp_dir().join(format!("deckard-appsigner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("signerd.sock");

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_srv = Arc::clone(&seen);
        let sock_srv = sock.clone();
        let (ready_tx, ready_rx) = mpsc::channel();

        // Recording server on its own current-thread runtime; handles two per-call
        // connections (Propose, then Execute).
        let server = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::UnixListener::bind(&sock_srv).unwrap();
                ready_tx.send(()).unwrap();
                for _ in 0..2 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let buf = frame::read_frame(&mut stream).await.unwrap().unwrap();
                    let req: SignerRequest = frame::decode(&buf).unwrap();
                    let resp = match &req {
                        SignerRequest::Propose { .. } => {
                            seen_srv.lock().unwrap().push("Propose".into());
                            SignerResponse::Decision(Decision::Allow)
                        }
                        SignerRequest::Execute { .. } => {
                            seen_srv.lock().unwrap().push("Execute".into());
                            SignerResponse::Execute(ExecuteResult::Broadcast {
                                tx_hash: B256::repeat_byte(0xAB),
                            })
                        }
                        other => panic!("unexpected request on the wire: {other:?}"),
                    };
                    let body = frame::encode(&resp).unwrap();
                    frame::write_frame(&mut stream, &body).await.unwrap();
                }
            });
        });

        ready_rx.recv().unwrap();

        let client = SignerClient::new(sock);
        let outcome = send_blocking(&client, &send_intent()).unwrap();

        assert_eq!(
            outcome,
            SendOutcome::Broadcast {
                tx_hash: B256::repeat_byte(0xAB)
            }
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["Propose".to_string(), "Execute".to_string()]
        );

        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_eth_to_wei_handles_decimals_and_rejects_junk() {
        // Whole + fractional ETH parse to exact wei.
        assert_eq!(parse_eth_to_wei("1").unwrap(), U256::from(10u128.pow(18)));
        assert_eq!(
            parse_eth_to_wei("0.05").unwrap(),
            U256::from(50_000_000_000_000_000u128)
        );
        assert_eq!(
            parse_eth_to_wei(" 1.234 ").unwrap(),
            U256::from(1_234_000_000_000_000_000u128)
        );
        assert_eq!(parse_eth_to_wei("0").unwrap(), U256::ZERO);
        // Full 18-place precision survives.
        assert_eq!(
            parse_eth_to_wei("0.000000000000000001").unwrap(),
            U256::from(1u64)
        );
        // Junk is rejected, never silently coerced to a wrong magnitude.
        for bad in [
            "",
            " ",
            ".",
            "abc",
            "-1",
            "1.2.3",
            "1,5",
            "0.1234567890123456789",
        ] {
            assert!(parse_eth_to_wei(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn unlock_outcomes_map_to_address_or_message() {
        let addr = Address::repeat_byte(0x11);
        assert_eq!(
            address_or_error(UnlockOutcome::Unlocked { address: addr }),
            Ok(addr)
        );
        assert!(address_or_error(UnlockOutcome::BadPassphrase).is_err());
        assert!(address_or_error(UnlockOutcome::NoVault).is_err());
    }
}
