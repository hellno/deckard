//! `deckard-mcp install [--demo] [--write] [--client <CLIENT>]` — emit (or, with
//! confirmation, write) the MCP registration for this binary.
//!
//! Two clients share ONE source of truth ([`server_entry`], the command/args/env entry):
//! - `claude-desktop` (default): prints the `claude_desktop_config.json` snippet; `--write`
//!   asks for an explicit `y` on stdin before MERGING into that file (never clobbers).
//! - `claude-code`: prints a ready-to-paste `claude mcp add deckard …` command AND the
//!   `.mcp.json` snippet — and writes nothing, ever.
//!
//! The embedded command is the **absolute path of the running binary** — MCP clients launch
//! servers with no shell PATH worth relying on.
//!
//! `--demo` adds the env block that points BOTH the sidecar and (via `just demo`) the
//! daemon at the isolated demo world: dedicated config dir + socket under `~/.deckard/demo`,
//! Sepolia chain id, local anvil RPC. Key-less by construction — no secret ever enters a
//! client config. The demo env MUST ride along on `claude-code` too, or the registration
//! points at the wrong (everyday) daemon.

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

/// Claude Code registration: print a ready-to-paste `claude mcp add` command AND the
/// `.mcp.json` snippet, both built from the SAME [`server_entry`] (no duplicated JSON).
/// Writes nothing — Claude Code owns its own config; the operator pastes one or the other.
pub fn run_claude_code(demo: bool) -> anyhow::Result<()> {
    let binary = std::env::current_exe()?;
    let entry = server_entry(&binary, demo)?;
    let add_cmd = claude_mcp_add_command(&entry)?;

    println!(
        "Claude Code registration for deckard-mcp{}:",
        if demo { " (DEMO mode)" } else { "" }
    );
    println!("\nOption A — run this once (registers the server for Claude Code):\n");
    println!("{add_cmd}");

    // Option B: the .mcp.json snippet, reusing the very same entry (project-scoped config).
    let snippet = json!({ "mcpServers": { "deckard": entry } });
    println!(
        "\nOption B — or paste this into .mcp.json at your project root:\n\n{}",
        serde_json::to_string_pretty(&snippet)?
    );

    if demo {
        println!(
            "\nDemo preconditions: `just demo` running (anvil fork + app on the demo \
             config dir), wallet created + unlocked in that app, `just demo-fund` done."
        );
    }
    println!(
        "\nNothing was written. Verify after registering: ask Claude to run deckard_policy_get."
    );
    Ok(())
}

/// Build the `claude mcp add deckard [-e KEY=VALUE …] -- <abs binary> --mcp` command from a
/// [`server_entry`]. The env block (when present, i.e. `--demo`) is forwarded verbatim — it
/// is what points the registration at the right daemon, so it MUST be included.
fn claude_mcp_add_command(entry: &Value) -> anyhow::Result<String> {
    let command = entry["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server entry has no command path"))?;

    let mut parts = vec![
        "claude".to_string(),
        "mcp".to_string(),
        "add".to_string(),
        "deckard".to_string(),
    ];
    if let Some(env) = entry.get("env").and_then(|e| e.as_object()) {
        // Deterministic order so the printed command is stable across runs.
        let mut pairs: Vec<(&String, &Value)> = env.iter().collect();
        pairs.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in pairs {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("env value for {key} is not a string"))?;
            parts.push("-e".to_string());
            // Quote the VALUE so a path with a space/metachar survives paste (`KEY='a b'`).
            parts.push(format!("{key}={}", shell_quote(value)));
        }
    }
    parts.push("--".to_string());
    // The absolute binary path may contain a space (e.g. a build dir under "Application Support").
    parts.push(shell_quote(command));
    // The MCP server flag (mirrors server_entry's args = ["--mcp"]).
    parts.push("--mcp".to_string());
    Ok(parts.join(" "))
}

/// POSIX single-quote `s` when it contains anything outside a path/url-safe set, so the printed
/// `claude mcp add` line survives a paste even if `$HOME` or the binary path has a space. Simple
/// tokens (the common case) stay unquoted for readability.
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | '+' | ',')
        });
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
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
        // Key-less contract: exactly these four env keys, nothing secret-bearing.
        assert_eq!(env.len(), 4);
    }

    #[test]
    fn plain_entry_has_no_env_block() {
        let entry = server_entry(Path::new("/abs/path/deckard-mcp"), false).unwrap();
        assert!(
            entry.get("env").is_none(),
            "plain install targets the real daemon defaults"
        );
    }

    #[test]
    fn claude_code_demo_command_carries_env_and_the_binary() {
        let entry = server_entry(Path::new("/abs/path/deckard-mcp"), true).unwrap();
        let cmd = claude_mcp_add_command(&entry).unwrap();
        assert!(cmd.starts_with("claude mcp add deckard "), "{cmd}");
        // The demo env MUST be forwarded — otherwise it registers the wrong daemon.
        assert!(cmd.contains("-e DECKARD_CHAIN_ID=11155111"), "{cmd}");
        assert!(
            cmd.contains("-e DECKARD_RPC_URL=http://127.0.0.1:8545"),
            "{cmd}"
        );
        assert!(cmd.contains("-e DECKARD_SOCKET_PATH="), "{cmd}");
        assert!(cmd.contains("-e DECKARD_CONFIG_DIR="), "{cmd}");
        // The transport flag and the absolute binary path follow the `--` separator.
        assert!(cmd.ends_with("-- /abs/path/deckard-mcp --mcp"), "{cmd}");
    }

    #[test]
    fn claude_code_plain_command_has_no_env_flags() {
        let entry = server_entry(Path::new("/abs/path/deckard-mcp"), false).unwrap();
        let cmd = claude_mcp_add_command(&entry).unwrap();
        assert_eq!(cmd, "claude mcp add deckard -- /abs/path/deckard-mcp --mcp");
    }
}
