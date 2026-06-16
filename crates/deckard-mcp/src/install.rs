//! `deckard-mcp install [--demo] [--write]` — emit (or, with confirmation, write) the
//! Claude Desktop registration for this binary.
//!
//! Prints by default; `--write` asks for an explicit `y` on stdin before touching
//! `claude_desktop_config.json` (merge, never clobber). The embedded command is the
//! **absolute path of the running binary** — Claude Desktop launches MCP servers with no
//! shell PATH worth relying on.
//!
//! `--demo` adds the env block that points BOTH the sidecar and (via `just demo`) the
//! daemon at the isolated demo world: dedicated config dir + socket under `~/.deckard/demo`,
//! Sepolia chain id, local anvil RPC, and the demo swap stub (so the sidecar's CoW orderbook
//! returns a fixture quote + simulated fill instead of hitting the live orderbook, which
//! can't accept a fork order). Key-less by construction — no secret ever enters Claude's
//! config file.

use std::io::BufRead;
use std::path::PathBuf;

use serde_json::{json, Value};

/// The stable demo config dir (shared contract with `just demo`): `~/.deckard/demo`.
pub fn demo_config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".deckard").join("demo"))
}

/// Build the `mcpServers.deckard` JSON entry.
pub fn server_entry(binary: &std::path::Path, demo: bool) -> anyhow::Result<Value> {
    let mut entry = json!({
        "command": binary.to_str().ok_or_else(|| anyhow::anyhow!("binary path is not UTF-8"))?,
        "args": ["--mcp"],
    });
    if demo {
        let dir = demo_config_dir().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        let dir_str = dir
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("demo dir path is not UTF-8"))?;
        entry["env"] = json!({
            "DECKARD_SOCKET_PATH": format!("{dir_str}/signerd.sock"),
            "DECKARD_CONFIG_DIR": dir_str,
            "DECKARD_CHAIN_ID": "11155111",
            "DECKARD_RPC_URL": "http://127.0.0.1:8545",
            // Demo-only swap stub (ON/OFF flag): flips the CoW orderbook to the fixture quote +
            // simulated fill, because a real CoW order can't be accepted from a local fork. The
            // fill credits balances on DECKARD_RPC_URL (the local anvil above) — separate knob.
            "DECKARD_DEMO_SWAP_STUB": "1",
        });
    }
    Ok(entry)
}

/// The platform path of Claude Desktop's config file.
pub fn claude_desktop_config_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else {
        // Claude Desktop doesn't ship on Linux; the printed snippet still documents the
        // shape for claude-code / other MCP clients.
        Some(home.join(".config/Claude/claude_desktop_config.json"))
    }
}

/// Run the install command. `confirm` reads the y/N answer (stdin in production; injected
/// in tests).
pub fn run(demo: bool, write: bool, confirm: &mut dyn BufRead) -> anyhow::Result<()> {
    let binary = std::env::current_exe()?;
    let entry = server_entry(&binary, demo)?;
    let snippet = json!({ "mcpServers": { "deckard": entry } });
    let pretty = serde_json::to_string_pretty(&snippet)?;

    println!(
        "Claude Desktop registration for deckard-mcp{}:",
        if demo { " (DEMO mode)" } else { "" }
    );
    println!("{pretty}");
    if demo {
        println!(
            "\nDemo preconditions: `just demo` running (anvil fork + app on the demo \
             config dir), wallet created + unlocked in that app, `just demo-fund` done."
        );
    }

    let Some(config_path) = claude_desktop_config_path() else {
        anyhow::bail!("could not resolve the Claude Desktop config path (HOME unset)");
    };

    if !write {
        println!(
            "\nNot written. Merge it into {} yourself, or re-run with --write.",
            config_path.display()
        );
        println!("Verify after restarting Claude Desktop: ask Claude to run deckard_policy_get,");
        println!("or run `{} balance` in a terminal.", binary.display());
        return Ok(());
    }

    // --write: explicit confirmation, then MERGE into the existing config (other servers
    // and unrelated keys survive untouched).
    println!(
        "\nWrite this entry into {}? Existing servers are kept; only mcpServers.deckard is \
         replaced. [y/N]",
        config_path.display()
    );
    let mut answer = String::new();
    confirm.read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        println!("Aborted — nothing written.");
        return Ok(());
    }

    let mut root: Value = match std::fs::read(&config_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            anyhow::anyhow!(
                "{} is not valid JSON ({e}) — fix or remove it first, nothing was written",
                config_path.display()
            )
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    if !root.is_object() {
        anyhow::bail!(
            "{} does not contain a JSON object — fix it first, nothing was written",
            config_path.display()
        );
    }
    let servers = root
        .as_object_mut()
        .and_then(|o| {
            if !o.contains_key("mcpServers") {
                o.insert("mcpServers".into(), json!({}));
            }
            o.get_mut("mcpServers")
        })
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("mcpServers is not an object — fix the config first"))?;
    servers.insert("deckard".into(), server_entry(&binary, demo)?);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, serde_json::to_vec_pretty(&root)?)?;
    println!(
        "Written. Restart Claude Desktop, then verify by asking Claude to run deckard_policy_get."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn demo_entry_carries_the_full_env_block_and_no_secret() {
        let entry = server_entry(Path::new("/abs/path/deckard-mcp"), true).unwrap();
        assert_eq!(entry["command"], "/abs/path/deckard-mcp");
        assert_eq!(entry["args"][0], "--mcp");
        let env = entry["env"].as_object().unwrap();
        assert_eq!(env["DECKARD_CHAIN_ID"], "11155111");
        assert_eq!(env["DECKARD_RPC_URL"], "http://127.0.0.1:8545");
        assert!(env["DECKARD_SOCKET_PATH"]
            .as_str()
            .unwrap()
            .ends_with(".deckard/demo/signerd.sock"));
        assert!(env["DECKARD_CONFIG_DIR"]
            .as_str()
            .unwrap()
            .ends_with(".deckard/demo"));
        // The demo swap stub is an on/off flag (fill RPC = DECKARD_RPC_URL above).
        assert_eq!(env["DECKARD_DEMO_SWAP_STUB"], "1");
        // Key-less contract: exactly these five env keys, nothing secret-bearing.
        assert_eq!(env.len(), 5);
    }

    #[test]
    fn plain_entry_has_no_env_block() {
        let entry = server_entry(Path::new("/abs/path/deckard-mcp"), false).unwrap();
        assert!(
            entry.get("env").is_none(),
            "plain install targets the real daemon defaults"
        );
    }
}
