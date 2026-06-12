//! The key-less client the GUI app (and, later, `deckard-mcp`) use to talk to the daemon.
//!
//! One request → one response over a fresh connection (the daemon serializes everything
//! behind its state, so per-call connections are correct and simple at this call frequency).
//! [`SignerClient::request`] is async; [`SignerClient::request_blocking`] wraps it in a
//! short-lived current-thread runtime for callers without one (the app's background thread).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::net::UnixStream;
use zeroize::Zeroize;

use deckard_contract::{
    ApprovalStatus, Decision, ExecuteResult, Intent, PendingRecord, RailgunViewGrant, RequestId,
    SignOrderResult, SignerRequest, SignerResponse, SwapOrder, UnlockOutcome,
};

use crate::frame;
use crate::request_id::{request_id_for, request_id_for_order};

/// How long to keep retrying `connect` before giving up — covers the brief window where the
/// app has spawned the daemon but it hasn't bound the socket yet.
const CONNECT_DEADLINE: Duration = Duration::from_secs(3);

/// A handle to the daemon socket. Cheap to clone; holds only the path.
#[derive(Clone, Debug)]
pub struct SignerClient {
    path: PathBuf,
}

impl SignerClient {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The socket path this client dials.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Send one request and read one response (connecting with a short retry so a just-spawned
    /// daemon is given a moment to bind).
    pub async fn request(&self, req: &SignerRequest) -> anyhow::Result<SignerResponse> {
        let mut stream = self.connect().await?;
        let mut body = frame::encode(req)?;
        frame::write_frame(&mut stream, &body).await?;
        // Scrub the encoded request: an `Unlock` frame carries the passphrase bytes, which must
        // not linger in the client heap after they've gone over the socket.
        body.zeroize();
        let mut resp = frame::read_frame(&mut stream)
            .await?
            .ok_or_else(|| anyhow::anyhow!("daemon closed without responding"))?;
        let decoded = frame::decode(&resp);
        // Scrub the raw response: a `RailgunViewGrant` reply carries viewing-key bytes, which
        // must not linger in the client heap past the decode (mirrors the server's inbound scrub).
        resp.zeroize();
        decoded
    }

    /// Connect, retrying with capped backoff until [`CONNECT_DEADLINE`] — so the first call
    /// right after the app spawns the daemon doesn't lose a startup race.
    async fn connect(&self) -> anyhow::Result<UnixStream> {
        let deadline = Instant::now() + CONNECT_DEADLINE;
        let mut delay = Duration::from_millis(25);
        loop {
            match UnixStream::connect(&self.path).await {
                Ok(stream) => return Ok(stream),
                Err(e) => {
                    if Instant::now() >= deadline {
                        return Err(anyhow::anyhow!("connect {}: {e}", self.path.display()));
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_millis(200));
                }
            }
        }
    }

    /// Blocking convenience for callers without a tokio runtime (e.g. a GUI background
    /// thread). Spins a short-lived current-thread runtime for the round-trip.
    pub fn request_blocking(&self, req: &SignerRequest) -> anyhow::Result<SignerResponse> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| anyhow::anyhow!("build runtime: {e}"))?;
        rt.block_on(self.request(req))
    }

    // --- typed helpers over `request` (used by the app + tests) ---------------------------

    /// Unlock the vault over the socket (the app's lock screen sends this; the key never
    /// enters the app's address space — only the returned address does).
    pub async fn unlock(&self, passphrase: &str) -> anyhow::Result<UnlockOutcome> {
        match self
            .request(&SignerRequest::Unlock {
                passphrase: passphrase.to_string(),
            })
            .await?
        {
            SignerResponse::Unlock(outcome) => Ok(outcome),
            other => Err(unexpected("Unlock", other)),
        }
    }

    /// Blocking [`unlock`](Self::unlock).
    pub fn unlock_blocking(&self, passphrase: &str) -> anyhow::Result<UnlockOutcome> {
        match self.request_blocking(&SignerRequest::Unlock {
            passphrase: passphrase.to_string(),
        })? {
            SignerResponse::Unlock(outcome) => Ok(outcome),
            other => Err(unexpected("Unlock", other)),
        }
    }

    /// Lock the session (STOP-lite): zeroize the key, deny in-flight approvals.
    pub fn lock_blocking(&self) -> anyhow::Result<()> {
        match self.request_blocking(&SignerRequest::Lock)? {
            SignerResponse::Ack => Ok(()),
            other => Err(unexpected("Lock", other)),
        }
    }

    /// Blocking [`propose`](Self::propose) — policy check, no signing, for callers
    /// without a tokio runtime (the app's GUI background thread).
    pub fn propose_blocking(&self, intent: &Intent) -> anyhow::Result<Decision> {
        match self.request_blocking(&SignerRequest::Propose {
            intent: intent.clone(),
        })? {
            SignerResponse::Decision(d) => Ok(d),
            other => Err(unexpected("Propose", other)),
        }
    }

    /// Blocking [`execute`](Self::execute) — sign + broadcast (or denial).
    pub fn execute_blocking(&self, request_id: RequestId) -> anyhow::Result<ExecuteResult> {
        match self.request_blocking(&SignerRequest::Execute { request_id })? {
            SignerResponse::Execute(r) => Ok(r),
            other => Err(unexpected("Execute", other)),
        }
    }

    /// Blocking resolve: close a `NeedsApproval` loop by flipping its `Pending` record
    /// to `Allowed` (`approved: true`) or `Denied` (`approved: false`).
    pub fn resolve_blocking(&self, request_id: RequestId, approved: bool) -> anyhow::Result<()> {
        match self.request_blocking(&SignerRequest::Resolve {
            request_id,
            approved,
        })? {
            SignerResponse::Ack => Ok(()),
            other => Err(unexpected("Resolve", other)),
        }
    }

    /// Blocking: fetch the read-only Railgun view grant (0zk address + viewing key) for
    /// shielded-balance sync. A locked daemon or a failed derivation gate comes back as a
    /// `Decision::Deny`, surfaced here as an error.
    pub fn railgun_view_grant_blocking(
        &self,
        chain_id: u64,
        index: u32,
    ) -> anyhow::Result<RailgunViewGrant> {
        match self.request_blocking(&SignerRequest::RailgunViewGrant { chain_id, index })? {
            SignerResponse::RailgunView(grant) => Ok(grant),
            SignerResponse::Decision(Decision::Deny { reason }) => {
                anyhow::bail!("railgun view grant denied: {reason}")
            }
            other => Err(unexpected("RailgunViewGrant", other)),
        }
    }

    /// Blocking poll of an approval loop → its current [`ApprovalStatus`].
    pub fn status_blocking(&self, request_id: RequestId) -> anyhow::Result<ApprovalStatus> {
        match self.request_blocking(&SignerRequest::Status { request_id })? {
            SignerResponse::Status(s) => Ok(s),
            other => Err(unexpected("Status", other)),
        }
    }

    /// Propose an intent → a `Decision`. Note: the returned `request_id` for an `Allow` is
    /// derivable locally via [`request_id_for_intent`](Self::request_id_for_intent).
    pub async fn propose(&self, intent: &Intent) -> anyhow::Result<Decision> {
        match self
            .request(&SignerRequest::Propose {
                intent: intent.clone(),
            })
            .await?
        {
            SignerResponse::Decision(d) => Ok(d),
            other => Err(unexpected("Propose", other)),
        }
    }

    /// Execute a previously-proposed request id → sign + broadcast (or denial).
    pub async fn execute(&self, request_id: RequestId) -> anyhow::Result<ExecuteResult> {
        match self.request(&SignerRequest::Execute { request_id }).await? {
            SignerResponse::Execute(r) => Ok(r),
            other => Err(unexpected("Execute", other)),
        }
    }

    /// The deterministic request id for an intent — lets a client `execute` an `Allow` it
    /// derived locally (the daemon assigns the very same id).
    pub fn request_id_for_intent(intent: &Intent) -> RequestId {
        request_id_for(intent)
    }

    // --- swap order helpers (the agent proposes/signs/cancels CoW orders) ------------------

    /// Propose a swap order → a `Decision`. A valid order is always `NeedsApproval` (swaps
    /// never auto-allow in v1). The daemon binds the order's owner/receiver to the unlocked
    /// wallet, so the returned id is for the BOUND order — derive it locally via
    /// [`request_id_for_swap_order`](Self::request_id_for_swap_order) only after binding.
    pub async fn propose_order(&self, order: &SwapOrder) -> anyhow::Result<Decision> {
        match self
            .request(&SignerRequest::ProposeOrder {
                order: order.clone(),
            })
            .await?
        {
            SignerResponse::Decision(d) => Ok(d),
            other => Err(unexpected("ProposeOrder", other)),
        }
    }

    /// Blocking [`propose_order`](Self::propose_order).
    pub fn propose_order_blocking(&self, order: &SwapOrder) -> anyhow::Result<Decision> {
        match self.request_blocking(&SignerRequest::ProposeOrder {
            order: order.clone(),
        })? {
            SignerResponse::Decision(d) => Ok(d),
            other => Err(unexpected("ProposeOrder", other)),
        }
    }

    /// Sign a stored, approved order's EIP-712 digest → its 65-byte signature (no HTTP).
    pub async fn sign_order(&self, request_id: RequestId) -> anyhow::Result<SignOrderResult> {
        match self
            .request(&SignerRequest::SignOrder { request_id })
            .await?
        {
            SignerResponse::SignOrder(r) => Ok(r),
            other => Err(unexpected("SignOrder", other)),
        }
    }

    /// Blocking [`sign_order`](Self::sign_order).
    pub fn sign_order_blocking(&self, request_id: RequestId) -> anyhow::Result<SignOrderResult> {
        match self.request_blocking(&SignerRequest::SignOrder { request_id })? {
            SignerResponse::SignOrder(r) => Ok(r),
            other => Err(unexpected("SignOrder", other)),
        }
    }

    /// Broadcast an `invalidateOrder` cancel for a stored order → an [`ExecuteResult`].
    pub async fn cancel_order(&self, request_id: RequestId) -> anyhow::Result<ExecuteResult> {
        match self
            .request(&SignerRequest::CancelOrder { request_id })
            .await?
        {
            SignerResponse::Execute(r) => Ok(r),
            other => Err(unexpected("CancelOrder", other)),
        }
    }

    /// Blocking [`cancel_order`](Self::cancel_order).
    pub fn cancel_order_blocking(&self, request_id: RequestId) -> anyhow::Result<ExecuteResult> {
        match self.request_blocking(&SignerRequest::CancelOrder { request_id })? {
            SignerResponse::Execute(r) => Ok(r),
            other => Err(unexpected("CancelOrder", other)),
        }
    }

    /// List every in-flight pending record WITH its payload — the GUI approval inbox.
    pub async fn pending_list(&self) -> anyhow::Result<Vec<PendingRecord>> {
        match self.request(&SignerRequest::PendingList).await? {
            SignerResponse::Pending(records) => Ok(records),
            other => Err(unexpected("PendingList", other)),
        }
    }

    /// Blocking [`pending_list`](Self::pending_list).
    pub fn pending_list_blocking(&self) -> anyhow::Result<Vec<PendingRecord>> {
        match self.request_blocking(&SignerRequest::PendingList)? {
            SignerResponse::Pending(records) => Ok(records),
            other => Err(unexpected("PendingList", other)),
        }
    }

    /// The deterministic request id for a swap order — lets a client poll/sign an order it
    /// proposed. NOTE: the daemon binds `owner`/`receiver` to the unlocked wallet before
    /// hashing, so the caller must bind the same fields before deriving a matching id.
    pub fn request_id_for_swap_order(order: &SwapOrder) -> RequestId {
        request_id_for_order(order)
    }
}

fn unexpected(req: &str, got: SignerResponse) -> anyhow::Error {
    anyhow::anyhow!("daemon returned an unexpected response to {req}: {got:?}")
}
