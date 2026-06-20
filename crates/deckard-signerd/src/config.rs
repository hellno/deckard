//! Daemon configuration, all environment-driven so CI/tests point at a local anvil and
//! production points at Sepolia/mainnet by config.
//!
//! - `DECKARD_RPC_URL`    — JSON-RPC endpoint to broadcast through (default: the chain registry's
//!   per-chain default RPC for `DECKARD_CHAIN_ID` — never silently mainnet for a non-mainnet chain).
//! - `DECKARD_CHAIN_ID`   — the chain the daemon signs for (default: 1 = mainnet). A
//!   `propose` whose `intent.chain_id` differs is denied `chain_mismatch`.
//! - `DECKARD_CONFIG_DIR` — where `vault.bin` + `policy.json` live (default: the platform
//!   config dir, shared with the GUI app via `deckard_core::config`). Tests set this.
//! - `DECKARD_SOCKET_PATH`— explicit UDS path (default: the per-uid runtime path). Tests +
//!   the app set this so both ends agree.
//!
//! One more env var exists (the auto-approval-guardrail override). It is deliberately NOT named
//! in this doc comment, in any reason string, or in any client-visible text — it is documented
//! exactly once, in THREAT-MODEL.md. See [`Config::autonomy_override`].

use std::path::PathBuf;

/// Resolved daemon configuration.
#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,
    pub chain_id: u64,
    pub config_dir: PathBuf,
    pub socket_path: PathBuf,
    /// True when the operator explicitly disarmed the auto-approval guardrail via the override
    /// env var (documented only in THREAT-MODEL.md). Default false. DEFAULT-DENY: on every
    /// real-value chain (any chain NOT a testnet/dev id — see
    /// `deckard_core::chain::is_testnet_or_dev`) every auto-Allow is downgraded to `NeedsApproval`, so no
    /// hands-free agent spend exists THERE — a human must approve each write in the Deckard app's
    /// Approvals queue / activity feed (#60). On an exempt testnet/dev chain (the demo's Sepolia
    /// fork, the local anvil) within-cap auto-allow is deliberately hands-free, so the demo can
    /// run and be watched-and-stopped; the limits are software-enforced, not chain-enforced
    /// (ADR-0002, THREAT-MODEL.md). The override now disarms the guardrail on ANY chain, not just
    /// mainnet — its env-var NAME is unchanged for back-compat but is documented (only in
    /// THREAT-MODEL.md) to mean exactly that. The NAME must never be echoed into a reason string
    /// or tool response — a guardrail with printed disable-instructions is a speed bump, not a control.
    pub autonomy_override: bool,
}

impl Config {
    /// Resolve the config from the environment, applying the documented defaults.
    pub fn from_env() -> anyhow::Result<Self> {
        let chain_id = match std::env::var("DECKARD_CHAIN_ID") {
            Ok(s) => s
                .trim()
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("DECKARD_CHAIN_ID must be a u64, got {s:?}"))?,
            Err(_) => 1,
        };
        // Never run on chain 0: an EIP-155 signature over chain_id 0 is malformed/replayable, and
        // the spec's hard rule is "never sign chain_id == 0". Refuse it at config time so the
        // daemon can't even start mis-wired, rather than relying on a propose-time equality check.
        anyhow::ensure!(
            chain_id != 0,
            "DECKARD_CHAIN_ID must not be 0 (an unsigned-replayable chain id)"
        );

        // Per-chain default RPC (#97), in parity with the app's `settings::effective_rpc`: an
        // unset/empty `DECKARD_RPC_URL` resolves to THIS chain's node from the registry, never
        // silently mainnet. Closes the silent-read/sign-wrong-chain footgun on the signing path for
        // a standalone (non-supervised) daemon launch; the supervised path always passes the app's
        // already-resolved per-chain URL (so its behavior is unchanged). `chain_id` is resolved
        // first because the default depends on it.
        let rpc_url = match std::env::var("DECKARD_RPC_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => deckard_core::chain::default_rpc(chain_id).to_string(),
        };

        // An empty `DECKARD_CONFIG_DIR=` is treated as unset (parity with `deckard_core::config_dir`),
        // so it falls through to the platform dir instead of resolving the vault CWD-relative.
        let config_dir = match std::env::var_os("DECKARD_CONFIG_DIR").filter(|d| !d.is_empty()) {
            Some(d) => PathBuf::from(d),
            None => deckard_core::config_dir()
                .ok_or_else(|| anyhow::anyhow!("no platform config directory available"))?,
        };

        let socket_path = match std::env::var_os("DECKARD_SOCKET_PATH") {
            Some(p) => PathBuf::from(p),
            None => crate::socket::default_socket_path(),
        };

        // The autonomy override (disarms the auto-approval guardrail on ANY real-value chain).
        // Env-var NAME kept for back-compat; its broadened meaning is documented only in
        // THREAT-MODEL.md. Never echo this name into a client-visible string.
        let autonomy_override = std::env::var("DECKARD_I_KNOW_THIS_IS_MAINNET")
            .map(|v| v.trim() == "1")
            .unwrap_or(false);

        Ok(Self {
            rpc_url,
            chain_id,
            config_dir,
            socket_path,
            autonomy_override,
        })
    }

    /// The encrypted keystore path the daemon reads on `Unlock`.
    pub fn vault_path(&self) -> PathBuf {
        self.config_dir.join(deckard_core::config::VAULT_FILE)
    }

    /// The signer policy path (a sane default is used if absent).
    pub fn policy_path(&self) -> PathBuf {
        self.config_dir.join(deckard_core::config::POLICY_FILE)
    }

    /// The RPC endpoint with any embedded credentials/host elided — safe to log.
    pub fn redacted_rpc(&self) -> String {
        redact_url(&self.rpc_url)
    }
}

/// Reduce an RPC URL to `scheme://host[:port]` so an embedded API key (e.g. an Infura
/// project secret in the path/query) never reaches a log line. Public so the GUI app
/// (which also logs its resolved RPC at startup) shares this one redaction implementation.
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some(parts) => parts,
        None => return "<redacted>".to_string(),
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        // strip any userinfo (user:pass@host)
        .rsplit('@')
        .next()
        .unwrap_or("");
    if authority.is_empty() {
        "<redacted>".to_string()
    } else {
        format!("{scheme}://{authority}")
    }
}

/// Scrub every URL embedded in an arbitrary error/`reason` string down to
/// `scheme://host[:port]`. Transport errors (alloy/reqwest) routinely echo the full request
/// URL — which for an RPC endpoint carries the API key in its path/query/userinfo — and
/// `reason` strings cross the trust boundary into agent transcripts. So every reason is
/// scrubbed at the daemon boundary, not just truncated.
///
/// Token-based: any whitespace-delimited token containing `://` is replaced by its
/// [`redact_url`] form (leading punctuation like `(` is preserved so error text stays
/// readable). Tokens without a URL shape pass through untouched.
pub(crate) fn sanitize_reason(s: &str) -> String {
    s.split_whitespace()
        .map(|token| {
            if let Some(at) = token.find("://") {
                // Preserve any leading punctuation (e.g. the `(` in reqwest's
                // `... for url (https://…)`), then redact from the scheme on.
                let scheme_start = token[..at]
                    .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.')
                    // `rfind` returns the byte index of the char's START — advance past the
                    // whole char (it may be multibyte, e.g. a curly quote) or we'd slice
                    // mid-codepoint and panic.
                    .map(|i| i + token[i..].chars().next().map_or(1, char::len_utf8))
                    .unwrap_or(0);
                format!(
                    "{}{}",
                    &token[..scheme_start],
                    redact_url(&token[scheme_start..])
                )
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{redact_url, sanitize_reason};

    #[test]
    fn redaction_drops_paths_and_userinfo() {
        assert_eq!(
            redact_url("https://mainnet.infura.io/v3/SECRETKEY"),
            "https://mainnet.infura.io"
        );
        assert_eq!(redact_url("http://127.0.0.1:8545"), "http://127.0.0.1:8545");
        assert_eq!(
            redact_url("https://user:pass@rpc.example.com/path?token=abc"),
            "https://rpc.example.com"
        );
        assert_eq!(redact_url("not-a-url"), "<redacted>");
    }

    #[test]
    fn sanitize_scrubs_urls_inside_error_text() {
        // The reqwest/alloy shape: full request URL echoed in parentheses.
        assert_eq!(
            sanitize_reason(
                "broadcast: error sending request for url (https://eth.example.com/v3/SECRETKEY123)"
            ),
            "broadcast: error sending request for url (https://eth.example.com"
        );
        // Userinfo credentials are dropped too.
        assert_eq!(
            sanitize_reason("connect https://user:hunter2@rpc.example.com/path failed"),
            "connect https://rpc.example.com failed"
        );
        // Query-string keys never survive.
        let out = sanitize_reason("get http://127.0.0.1:8545/?apikey=TOPSECRET refused");
        assert!(!out.contains("TOPSECRET"), "query key leaked: {out}");
        // Plain text passes through (modulo whitespace normalization).
        assert_eq!(sanitize_reason("no url here"), "no url here");
    }

    #[test]
    fn sanitize_survives_multibyte_punctuation_before_scheme() {
        // A multibyte char (curly quote) directly before the scheme must not cause a
        // mid-codepoint slice panic — `rfind` returns the char's START byte index.
        assert_eq!(
            sanitize_reason("error “https://eth.example.com/v3/SECRETKEY” returned 401"),
            "error “https://eth.example.com returned 401"
        );
        // Same with an ellipsis glued to the front.
        let out = sanitize_reason("…http://user:pw@rpc.example.com/key fail");
        assert!(!out.contains("pw"), "userinfo leaked: {out}");
        assert!(out.contains("…http://rpc.example.com"), "got: {out}");
    }
}
