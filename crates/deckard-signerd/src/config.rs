//! Daemon configuration, all environment-driven so CI/tests point at a local anvil and
//! production points at Sepolia/mainnet by config.
//!
//! - `DECKARD_RPC_URL`    — JSON-RPC endpoint to broadcast through (default: the public RPC).
//! - `DECKARD_CHAIN_ID`   — the chain the daemon signs for (default: 1 = mainnet). A
//!   `propose` whose `intent.chain_id` differs is denied `chain_mismatch`.
//! - `DECKARD_CONFIG_DIR` — where `vault.bin` + `policy.json` live (default: the platform
//!   config dir, shared with the GUI app via `deckard_core::config`). Tests set this.
//! - `DECKARD_SOCKET_PATH`— explicit UDS path (default: the per-uid runtime path). Tests +
//!   the app set this so both ends agree.

use std::path::PathBuf;

/// Resolved daemon configuration.
#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,
    pub chain_id: u64,
    pub config_dir: PathBuf,
    pub socket_path: PathBuf,
}

impl Config {
    /// Resolve the config from the environment, applying the documented defaults.
    pub fn from_env() -> anyhow::Result<Self> {
        let rpc_url = std::env::var("DECKARD_RPC_URL")
            .unwrap_or_else(|_| deckard_core::DEFAULT_RPC.to_string());

        let chain_id = match std::env::var("DECKARD_CHAIN_ID") {
            Ok(s) => s
                .trim()
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("DECKARD_CHAIN_ID must be a u64, got {s:?}"))?,
            Err(_) => 1,
        };

        let config_dir = match std::env::var_os("DECKARD_CONFIG_DIR") {
            Some(d) => PathBuf::from(d),
            None => deckard_core::config_dir()
                .ok_or_else(|| anyhow::anyhow!("no platform config directory available"))?,
        };

        let socket_path = match std::env::var_os("DECKARD_SOCKET_PATH") {
            Some(p) => PathBuf::from(p),
            None => crate::socket::default_socket_path(),
        };

        Ok(Self {
            rpc_url,
            chain_id,
            config_dir,
            socket_path,
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
/// project secret in the path/query) never reaches a log line.
fn redact_url(url: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::redact_url;

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
}
