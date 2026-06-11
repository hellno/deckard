//! Secret-bearing-flag rejection (the Splits `SPLITS_MCP_MODE` discipline, applied
//! unconditionally): no `deckard-mcp` command takes a secret, so ANY secret-shaped flag is
//! refused before clap ever parses — in `--mcp` mode or plain CLI mode alike. The flag's
//! VALUE is never echoed (it would land in the very transcript this rule protects).
//!
//! This is belt-and-suspenders: the sidecar is key-less by architecture, but the rule still
//! keeps a pasted passphrase or RPC bearer token out of tool-call transcripts and shell
//! history bridges.

/// Secret keywords. A flag is refused when ANY of its `-`/`_`-delimited components matches
/// one of these — so compound flags (`--rpc-api-key`, `--private_key`, `--my-seed-phrase`)
/// are caught, not just the bare stems. Kept as whole words to avoid false positives on
/// substrings (`--config-dir` carries none of these; `--token`-style flags do).
const SECRET_KEYWORDS: &[&str] = &[
    "passphrase",
    "password",
    "key",
    "apikey",
    "seed",
    "mnemonic",
    "secret",
    "private",
    "token",
    "bearer",
];

/// Scan raw argv (without the program name). On a secret-shaped flag, returns the refusal
/// message naming ONLY the flag — never its value.
pub fn reject_secret_flags<I: IntoIterator<Item = String>>(args: I) -> Result<(), String> {
    for arg in args {
        let Some(flag) = arg.strip_prefix("--") else {
            continue;
        };
        // `--passphrase=hunter2` → compare the part before '='; value is never touched.
        let name = flag.split('=').next().unwrap_or("");
        let lname = name.to_ascii_lowercase();
        // Delimiter-aware: split on '-'/'_' and reject if any component is a secret keyword.
        // This catches `--rpc-api-key`, `--private_key`, `--railgun-viewing-key`, etc., while
        // `--mcp` / `--amount-eth` / `--demo` / `--config-dir` keep none of the keywords.
        let is_secret = lname
            .split(['-', '_'])
            .any(|part| SECRET_KEYWORDS.contains(&part));
        if is_secret {
            return Err(format!(
                "secrets are not accepted on the deckard-mcp command line: flag --{name} \
                 rejected (its value was not read and will not be echoed). deckard-mcp is \
                 key-less — secrets live only in the Deckard app/daemon."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_secret_flags_without_echoing_values() {
        for argv in [
            vec!["--mcp".to_string(), "--passphrase=hunter2".to_string()],
            vec!["--key=deadbeef".to_string()],
            vec!["balance".to_string(), "--TOKEN=abc".to_string()],
            vec!["--mnemonic=a b c".to_string()],
            // Compound flags: a secret keyword in ANY '-'/'_' component must still reject,
            // so clap never gets a chance to echo the value back in its error output.
            vec!["--rpc-api-key=abc".to_string()],
            vec!["--private_key=deadbeef".to_string()],
            vec!["--railgun-viewing-key=zzz".to_string()],
            vec!["--my-seed-phrase=a b c".to_string()],
            vec!["--AUTH_TOKEN=hunter2".to_string()],
        ] {
            let err =
                reject_secret_flags(argv.clone()).expect_err(&format!("{argv:?} must be rejected"));
            assert!(err.contains("secrets are not accepted"), "{err}");
            for secret in ["hunter2", "deadbeef", "abc", "a b c", "zzz"] {
                assert!(!err.contains(secret), "value echoed: {err}");
            }
        }
    }

    #[test]
    fn passes_normal_flags() {
        for argv in [
            vec!["--mcp".to_string()],
            vec![
                "shield".to_string(),
                "--amount-eth".to_string(),
                "0.02".to_string(),
            ],
            vec!["install".to_string(), "--demo".to_string()],
            // Legitimate compound flags must NOT trip the delimiter-aware match: none of
            // their components is a secret keyword.
            vec!["--config-dir".to_string(), "/tmp/d".to_string()],
            vec!["--chain-id".to_string(), "1".to_string()],
            vec!["--socket-path".to_string(), "/tmp/s".to_string()],
        ] {
            assert!(reject_secret_flags(argv).is_ok());
        }
    }
}
