//! Secret-bearing-flag rejection (the Splits `SPLITS_MCP_MODE` discipline, applied
//! unconditionally): no `deckard-mcp` command takes a secret, so ANY secret-shaped flag is
//! refused before clap ever parses — in `--mcp` mode or plain CLI mode alike. The flag's
//! VALUE is never echoed (it would land in the very transcript this rule protects).
//!
//! This is belt-and-suspenders: the sidecar is key-less by architecture, but the rule still
//! keeps a pasted passphrase or RPC bearer token out of tool-call transcripts and shell
//! history bridges.

/// Flag stems that may carry a secret. Matched against `--<stem>`, `--<stem>=…`.
const SECRET_FLAG_STEMS: &[&str] = &[
    "passphrase",
    "password",
    "key",
    "private-key",
    "api-key",
    "apikey",
    "seed",
    "mnemonic",
    "secret",
    "token",
    "rpc-token",
    "auth-token",
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
        if SECRET_FLAG_STEMS.iter().any(|stem| lname == *stem) {
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
        ] {
            let err =
                reject_secret_flags(argv.clone()).expect_err(&format!("{argv:?} must be rejected"));
            assert!(err.contains("secrets are not accepted"), "{err}");
            for secret in ["hunter2", "deadbeef", "abc", "a b c"] {
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
        ] {
            assert!(reject_secret_flags(argv).is_ok());
        }
    }
}
