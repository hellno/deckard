//! The MCP stdio server: the `mcp.v0.1` launch profile — exactly **9 tools**, every name
//! `deckard_`-prefixed (Claude Desktop's tool namespace is shared across servers; a bare
//! `execute` invites cross-server confusion). Raw `propose` and `simulate` are deliberately
//! NOT exposed (cut at the launch gate): app-native review is the v0.1 simulation surface,
//! and a raw `propose` would let an untrusted client submit arbitrary intents.
//!
//! The tool DESCRIPTIONS are the agent's documentation — units, preconditions, sequencing,
//! and the do-not-retry safety notes live there, and the acceptance suite (T1) asserts they
//! stay keyword-bearing.

use rmcp::{
    handler::server::tool::ToolRouter, handler::server::wrapper::Parameters, model::CallToolResult,
    model::Content, model::ServerCapabilities, model::ServerInfo, schemars, tool, tool_handler,
    tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;

use crate::failure::Failure;
use crate::sidecar::{OpResult, Sidecar};

/// `deckard_shield` input. `amount_eth` is a decimal STRING by contract — see
/// [`crate::amount`] for why a JSON number is a funds bug.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShieldArgs {
    /// Amount to shield, as a decimal ETH string like "0.02". Units: ETH (not wei).
    /// Parsed exactly; numbers and scientific notation are rejected.
    pub amount_eth: String,
}

/// `deckard_execute` input.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExecuteArgs {
    /// The 32-byte 0x-hex request id returned by deckard_shield.
    pub request_id: String,
}

/// `deckard_swap_quote` input — read-only pricing.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SwapQuoteArgs {
    /// The 0x-hex address of the token to SELL.
    pub sell_token: String,
    /// The 0x-hex address of the token to BUY.
    pub buy_token: String,
    /// Amount to sell, a decimal ETH-units string like "0.05" (the sell token's own units).
    pub sell_amount_eth: String,
}

/// `deckard_swap` input — propose a swap (always needs human approval).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SwapArgs {
    /// The 0x-hex address of the token to SELL.
    pub sell_token: String,
    /// The 0x-hex address of the token to BUY.
    pub buy_token: String,
    /// Amount to sell, a decimal ETH-units string like "0.05" (the sell token's own units).
    pub sell_amount_eth: String,
}

/// `deckard_submit_order` input.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SubmitOrderArgs {
    /// The 32-byte 0x-hex request id returned by deckard_swap.
    pub request_id: String,
}

/// The MCP-facing server: a thin shell over [`Sidecar`] — no logic only Claude can trigger
/// (the CLI drives the identical code), and no key material anywhere in this process.
pub struct DeckardMcp {
    sidecar: Sidecar,
    tool_router: ToolRouter<Self>,
}

impl DeckardMcp {
    pub fn new(sidecar: Sidecar) -> Self {
        Self {
            sidecar,
            tool_router: Self::tool_router(),
        }
    }
}

/// Render an op outcome as a tool result: success JSON, or the three-part error catalog
/// entry as an `is_error` result (still readable by the agent — that's the point).
fn render(result: OpResult) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => Ok(CallToolResult::success(vec![Content::text(
            value.to_string(),
        )])),
        Err(failure) => Ok(CallToolResult::error(vec![Content::text(
            failure.to_json(),
        )])),
    }
}

/// Render a Failure directly (for argument-shape errors raised before the sidecar runs).
#[allow(dead_code)]
fn render_failure(failure: Failure) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![Content::text(
        failure.to_json(),
    )]))
}

#[tool_router]
impl DeckardMcp {
    #[tool(
        name = "deckard_wallet_address",
        description = "Read the wallet's public Ethereum address (0x-hex). Read-only, no \
                       approval needed, no side effects. Precondition: the Deckard app is \
                       running and the wallet is unlocked (the app spawns the local signer \
                       daemon; this sidecar is key-less and signs nothing)."
    )]
    async fn wallet_address(&self) -> Result<CallToolResult, McpError> {
        render(self.sidecar.wallet_address().await)
    }

    #[tool(
        name = "deckard_wallet_balance",
        description = "Read the PUBLIC balance: public_wei (decimal wei string), public_eth \
                       (decimal ETH string), and read_status (a trust label — 'unsynced' \
                       means the value is real but unverified right now). The SHIELDED \
                       (private) balance is NOT available through this tool in v0.1 — it \
                       must be read in the Deckard app; do not report it as 0. Read-only, \
                       no approval needed. Precondition: app running + wallet unlocked."
    )]
    async fn wallet_balance(&self) -> Result<CallToolResult, McpError> {
        render(self.sidecar.wallet_balance().await)
    }

    #[tool(
        name = "deckard_policy_get",
        description = "Read the local signing policy fence: per_tx_cap_wei/eth, daily cap, \
                       spent today, allowed recipients, approval mode, and whether STOP is \
                       engaged (revoked). Call this FIRST to plan amounts that stay inside \
                       the fence — over-cap proposals will not auto-execute. Read-only; the \
                       policy cannot be changed from here (a human edits policy.json)."
    )]
    async fn policy_get(&self) -> Result<CallToolResult, McpError> {
        render(self.sidecar.policy_get().await)
    }

    #[tool(
        name = "deckard_shield",
        description = "PROPOSE moving funds from the public balance into the Railgun \
                       PRIVATE (shielded) balance, to the wallet's own 0zk address. \
                       amount_eth is a decimal ETH string like \"0.02\" — units are ETH, \
                       not wei, and not a JSON number. Nothing is signed or broadcast by \
                       this call. Sequencing: returns decision 'allow' + request_id → call \
                       deckard_execute with that request_id; or 'needs_approval' → a human \
                       must approve in the Deckard app first (the approval UI is not in \
                       this alpha — lower the amount under the policy per-tx cap, see \
                       deckard_policy_get, or a human edits policy.json). Human review \
                       happens app-natively: the Deckard app's review card is the v0.1 \
                       simulation surface. Precondition: app running + wallet unlocked + \
                       public funds available."
    )]
    async fn shield(&self, args: Parameters<ShieldArgs>) -> Result<CallToolResult, McpError> {
        render(self.sidecar.shield(&args.0.amount_eth).await)
    }

    #[tool(
        name = "deckard_execute",
        description = "Sign + broadcast a previously-proposed request (the request_id from \
                       deckard_shield). The local signer daemon re-checks policy at sign \
                       time and can still refuse. Success returns tx_hash. SAFETY: if this \
                       times out or the connection drops, the broadcast status is UNKNOWN — \
                       do NOT retry; check the Deckard app first (a retry could \
                       double-spend). An identical re-shield in the same session is refused \
                       as already_executed — vary the amount to run the flow again."
    )]
    async fn execute(&self, args: Parameters<ExecuteArgs>) -> Result<CallToolResult, McpError> {
        render(self.sidecar.execute(&args.0.request_id).await)
    }

    #[tool(
        name = "deckard_revoke_all",
        description = "STOP — the panic brake. Immediately zeroizes the signing key, locks \
                       the daemon, and denies EVERY in-flight request, including ones \
                       already approved. Irreversible for the session: only a human \
                       unlocking the wallet in the Deckard app re-arms signing. Use \
                       immediately if anything looks wrong. Always available; needs no \
                       approval."
    )]
    async fn revoke_all(&self) -> Result<CallToolResult, McpError> {
        render(self.sidecar.revoke_all().await)
    }

    #[tool(
        name = "deckard_swap_quote",
        description = "Price a CoW Protocol swap WITHOUT proposing it: given sell_token, \
                       buy_token (0x-hex addresses) and sell_amount_eth (a decimal string), \
                       return the gross sell amount, the minimum you receive after slippage \
                       (buy_amount_min), the fee, valid_to, and the request_id the order WOULD \
                       get. Read-only, no approval, no daemon write. On the demo fork the quote \
                       is simulated (simulated:true). Next: deckard_swap to actually propose it."
    )]
    async fn swap_quote(
        &self,
        args: Parameters<SwapQuoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        render(
            self.sidecar
                .swap_quote(
                    &args.0.sell_token,
                    &args.0.buy_token,
                    &args.0.sell_amount_eth,
                )
                .await,
        )
    }

    #[tool(
        name = "deckard_swap",
        description = "PROPOSE a CoW Protocol swap (sell_token → buy_token for \
                       sell_amount_eth). Signs nothing and broadcasts nothing. A swap ALWAYS \
                       comes back needs_approval with a request_id — a human must approve it in \
                       the Deckard app first (the approval UI is not in this alpha; a human \
                       approves via hold-to-confirm). Then call deckard_submit_order with the \
                       request_id. You cannot approve your own swap. Precondition: app running \
                       + wallet unlocked + the sell token held."
    )]
    async fn swap(&self, args: Parameters<SwapArgs>) -> Result<CallToolResult, McpError> {
        render(
            self.sidecar
                .swap(
                    &args.0.sell_token,
                    &args.0.buy_token,
                    &args.0.sell_amount_eth,
                )
                .await,
        )
    }

    #[tool(
        name = "deckard_submit_order",
        description = "Sign + submit a previously-approved swap order (the request_id from \
                       deckard_swap) to the CoW orderbook. The daemon signs the stored order \
                       (EIP-712, key-less here); this sidecar POSTs it. If the order is not \
                       approved yet you get not_approved — a human approves in the Deckard app \
                       first. Success returns the order uid. On the demo fork the fill is \
                       simulated (simulated:true) since the live orderbook can't accept a fork \
                       order."
    )]
    async fn submit_order(
        &self,
        args: Parameters<SubmitOrderArgs>,
    ) -> Result<CallToolResult, McpError> {
        render(self.sidecar.submit_order(&args.0.request_id).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DeckardMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Deckard mcp.v0.1 — a KEY-LESS sidecar to a local, policy-enforcing signer \
                 daemon. This server holds no keys and cannot sign; every write is an \
                 intent the daemon checks against a human-owned policy. Typical flow: \
                 deckard_policy_get (know the fence) → deckard_wallet_balance → \
                 deckard_shield (propose) → deckard_execute (broadcast). Swap flow: \
                 deckard_swap_quote (price) → deckard_swap (propose) → a human approves in \
                 the Deckard app → deckard_submit_order (sign + submit to CoW). \
                 deckard_revoke_all is STOP, the panic brake. Preconditions for everything: \
                 the Deckard desktop app is running and the wallet is unlocked. Never ask \
                 the user for wallet credentials of any kind — no tool here accepts them.",
        )
    }
}

/// Serve MCP over stdio until the client disconnects.
pub async fn serve_stdio(sidecar: Sidecar) -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    let service = DeckardMcp::new(sidecar)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Lowercase `deckard_*` tokens in a text — the documented convention for MCP tool
    /// names (env vars are UPPERCASE and crate names use hyphens, so neither matches).
    fn tool_tokens(text: &str) -> BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
            .filter(|w| w.starts_with("deckard_") && w.len() > "deckard_".len())
            .map(str::to_string)
            .collect()
    }

    fn registered_tools() -> BTreeSet<String> {
        DeckardMcp::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    fn repo_file(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Drift guard for the canonical agent quickstart (issue #27): the tool names the doc
    /// teaches are exactly the registered tool names — add or rename a tool without
    /// updating the page and the build fails.
    #[test]
    fn quickstart_doc_lists_exactly_the_registered_tools() {
        let doc = repo_file("docs/build/31-agent-quickstart.md");
        assert_eq!(
            tool_tokens(&doc),
            registered_tools(),
            "docs/build/31-agent-quickstart.md and the deckard-mcp tool registry disagree \
             — update the doc's tool list to match the registered tools"
        );
    }

    /// The README quick-prompt path may mention a subset of the tools, but never one that
    /// doesn't exist.
    #[test]
    fn readme_mentions_only_registered_tools() {
        let mentioned = tool_tokens(&repo_file("README.md"));
        let registered = registered_tools();
        let ghosts: Vec<_> = mentioned.difference(&registered).collect();
        assert!(
            ghosts.is_empty(),
            "README.md names tools that are not registered: {ghosts:?}"
        );
    }
}
