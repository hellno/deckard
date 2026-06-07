//! The app's key-less bridge to `deckard-signerd`.
//!
//! This is the whole signing story for the GUI: the app **spawns + supervises** the daemon
//! and talks to it over the socket. It holds NO key material — no `UnlockedVault`, no
//! `PrivateKeySigner`. Unlock happens *in the daemon* (the app sends the passphrase and gets
//! back only an address); the send path sends an `Intent` and gets back a `Decision`/tx hash.
//! The keystore is only ever touched in-process by *onboarding* (to write `vault.bin`), never
//! to sign.

use alloy_primitives::{Address, B256};
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
