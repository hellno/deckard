//! The app's key-less bridge to `deckard-signerd`.
//!
//! This is the whole signing story for the GUI: the app **spawns + supervises** the daemon
//! and talks to it over the socket. It holds NO key material — no `UnlockedVault`, no
//! `PrivateKeySigner`. Unlock happens *in the daemon* (the app sends the passphrase and gets
//! back only an address); the send path sends an `Intent` and gets back a `Decision`/tx hash.
//! The keystore is only ever touched in-process by *onboarding* (to write `vault.bin`), never
//! to sign.

use std::ffi::OsString;
use std::path::PathBuf;

use alloy_primitives::{Address, Bytes, B256, U256};
use deckard_contract::{Decision, ExecuteResult, Intent, IntentKind, RequestId, UnlockOutcome};
use deckard_signerd::{ControlChannel, DaemonSupervisor, SignerClient};

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
    /// are passed to the daemon so it broadcasts on the same chain the app reads from. The
    /// socket path honors `DECKARD_SOCKET_PATH` (see [`resolve_socket_path`]).
    pub fn launch(rpc_url: String, chain_id: u64) -> Self {
        let socket_path = resolve_socket_path();
        // The supervisor exports this exact path to the daemon child (DECKARD_SOCKET_PATH), and
        // the client below binds the same one — so both ends always agree, demo or not.
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

    /// The private capability channel the daemon authenticates approvals on (PRD-01). The app
    /// sends `Resolve` here after a completed hold-to-confirm; the public socket refuses it.
    pub fn control(&self) -> ControlChannel {
        self._supervisor.control()
    }
}

/// Resolve the daemon socket path: the `DECKARD_SOCKET_PATH` override, else the per-uid
/// default. The app MUST honor the override — if it didn't, a demo daemon on its own socket
/// would lose the single-instance flock to an everyday Deckard, and the demo app would
/// silently attach to the **mainnet** daemon. `just demo` sets a dedicated socket under the
/// demo config dir so the two never collide.
fn resolve_socket_path() -> PathBuf {
    socket_path_from(std::env::var_os("DECKARD_SOCKET_PATH"))
}

/// Pure core of [`resolve_socket_path`]: a non-empty override wins; otherwise the per-uid
/// default. Split out so the precedence is unit-testable without mutating process env.
fn socket_path_from(override_path: Option<OsString>) -> PathBuf {
    match override_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => deckard_signerd::socket::default_socket_path(),
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

/// Execute a reviewed proposal, key-less. For a `NeedsApproval` proposal (over-cap, or the
/// daemon's mainnet guardrail downgrading an auto-allow), the completed hold-to-confirm IS
/// the human approval — the app is the wire contract's designated human-facing resolver — so
/// this first sends `Resolve{approved: true}` over the **private capability channel** (the only
/// channel the daemon authenticates approvals on, PRD-01) to flip the `Pending` record to
/// `Allowed`, then `Execute` over the public socket. For an `Allow` proposal it goes straight
/// to `Execute`. Blocking; called from a background thread. `execute` stays on the public
/// socket because it only signs an already-`Allowed` record — the authority is the `Resolve`.
pub fn approve_and_execute_blocking(
    client: &SignerClient,
    control: &ControlChannel,
    request_id: RequestId,
    needs_resolve: bool,
) -> anyhow::Result<ExecuteResult> {
    if needs_resolve {
        control.resolve(request_id, true)?;
    }
    client.execute_blocking(request_id)
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

/// Build a key-less native-ETH **send** intent: a plain transfer of `value_wei` to `to`, on
/// `chain_id`. Native only (`token: None`) with empty calldata — the empty payload IS the
/// native/contract-call discriminator the daemon switches on, and the policy gate requires a
/// `Send` to carry no calldata (`deckard-contract::policy::calldata_ok`). Infallible: the
/// recipient is already a resolved [`Address`] (the caller turns `0x…`/ENS into one), and the
/// amount is pre-parsed wei, so there is nothing left to fail. The daemon decides + signs.
pub fn build_native_send_intent(chain_id: u64, to: Address, value_wei: U256) -> Intent {
    Intent {
        chain_id,
        to,
        token: None,
        value: value_wei,
        calldata: Bytes::new(),
        kind: IntentKind::Send,
    }
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

    /// The mainnet-guardrail regression guard, post-PRD-01: a `NeedsApproval` shield completes
    /// by sending `Resolve{approved: true}` over the **private capability channel** (the
    /// hold-to-confirm is the human approval) and THEN `Execute` over the **public socket**,
    /// signing nothing in-process. Proves the split: approval authority rides the control
    /// channel, execution rides the public socket.
    #[test]
    fn approve_resolves_over_control_then_executes_over_public_socket() {
        let dir =
            std::env::temp_dir().join(format!("deckard-appresolve-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("signerd.sock");

        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = mpsc::channel();

        // Public recording server: ONE connection (Execute only — Resolve never arrives here).
        let seen_pub = Arc::clone(&seen);
        let sock_srv = sock.clone();
        let public_server = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::UnixListener::bind(&sock_srv).unwrap();
                ready_tx.send(()).unwrap();
                let (mut stream, _) = listener.accept().await.unwrap();
                let buf = frame::read_frame(&mut stream).await.unwrap().unwrap();
                match frame::decode::<SignerRequest>(&buf).unwrap() {
                    SignerRequest::Execute { .. } => {
                        seen_pub.lock().unwrap().push("Execute".into());
                        let resp = SignerResponse::Execute(ExecuteResult::Broadcast {
                            tx_hash: B256::repeat_byte(0xCD),
                        });
                        let body = frame::encode(&resp).unwrap();
                        frame::write_frame(&mut stream, &body).await.unwrap();
                    }
                    other => panic!("public socket must only see Execute, got {other:?}"),
                }
            });
        });
        ready_rx.recv().unwrap();

        // Control channel: a real socketpair (minted exactly as the supervisor does). The app
        // keeps `control`; a thread plays the daemon end, expecting a single Resolve → Ack.
        let (app_end, child_fd) = deckard_signerd::supervise::control_pair().unwrap();
        let control = deckard_signerd::ControlChannel::connected(app_end);
        let seen_ctrl = Arc::clone(&seen);
        let control_server = std::thread::spawn(move || {
            let mut daemon_end = std::os::unix::net::UnixStream::from(child_fd);
            let buf = frame::read_frame_blocking(&mut daemon_end)
                .unwrap()
                .unwrap();
            match frame::decode::<SignerRequest>(&buf).unwrap() {
                SignerRequest::Resolve { approved, .. } => {
                    assert!(approved, "hold-to-confirm must approve, not deny");
                    seen_ctrl.lock().unwrap().push("Resolve".into());
                    let body = frame::encode(&SignerResponse::Ack).unwrap();
                    frame::write_frame_blocking(&mut daemon_end, &body).unwrap();
                }
                other => panic!("control channel saw a non-Resolve request: {other:?}"),
            }
        });

        let client = SignerClient::new(sock);
        let result =
            approve_and_execute_blocking(&client, &control, RequestId::repeat_byte(0x77), true)
                .unwrap();

        assert_eq!(
            result,
            ExecuteResult::Broadcast {
                tx_hash: B256::repeat_byte(0xCD)
            }
        );
        // Resolve (control) strictly precedes Execute (public): the Ack gates the execute.
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["Resolve".to_string(), "Execute".to_string()]
        );

        control_server.join().unwrap();
        public_server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Send builder produces a *native* transfer: no token, empty calldata (so the
    /// daemon broadcasts it as a plain ETH send and the policy gate's `calldata_ok` admits
    /// it), `kind == Send`, with `to`/`value` carried verbatim.
    #[test]
    fn build_native_send_intent_is_a_native_send() {
        let to = Address::repeat_byte(0x44);
        let intent = build_native_send_intent(31337, to, U256::from(1_234u64));
        assert_eq!(intent.chain_id, 31337);
        assert_eq!(intent.to, to);
        assert_eq!(intent.token, None);
        assert_eq!(intent.value, U256::from(1_234u64));
        assert!(intent.calldata.is_empty());
        assert_eq!(intent.kind, IntentKind::Send);
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
    fn socket_path_honors_the_override_else_falls_back_to_default() {
        // An explicit override is used verbatim (the demo's dedicated socket).
        let forced = OsString::from("/tmp/deckard-demo/signerd.sock");
        assert_eq!(
            socket_path_from(Some(forced.clone())),
            PathBuf::from(&forced)
        );
        // Empty / unset → the per-uid default (a real path, not the override).
        let default = deckard_signerd::socket::default_socket_path();
        assert_eq!(socket_path_from(None), default);
        assert_eq!(socket_path_from(Some(OsString::new())), default);
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
