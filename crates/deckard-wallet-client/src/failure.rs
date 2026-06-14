//! The typed error catalog shared by key-less Deckard local interfaces: every failure maps to a
//! three-part `problem + cause + fix` — actionable, deterministic, and secret-free. An agent's
//! default instinct on error is to retry; these messages say explicitly when retrying is
//! wrong (broadcast-timeout, already_executed) and what to do instead.

use std::path::Path;

use deckard_contract::deny_reasons;
use serde::Serialize;

/// One catalog entry. Rendered as JSON for tool responses and as three lines for the CLI.
/// Never carries a secret: daemon `reason` strings are already URL-redacted at the daemon
/// boundary, and this layer adds only static copy.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub problem: String,
    pub cause: String,
    pub fix: String,
}

impl Failure {
    pub fn new(
        problem: impl Into<String>,
        cause: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            problem: problem.into(),
            cause: cause.into(),
            fix: fix.into(),
        }
    }

    /// JSON object for a tool response (`{"error": {...}}`).
    pub fn to_json(&self) -> String {
        serde_json::json!({ "error": self }).to_string()
    }

    /// Three labelled lines for the CLI.
    pub fn to_human(&self) -> String {
        format!(
            "problem: {}\ncause:   {}\nfix:     {}",
            self.problem, self.cause, self.fix
        )
    }
}

/// The daemon socket couldn't be reached. Distinguishes "no vault yet" from "app not
/// running" when the config dir is known (demo mode sets `DECKARD_CONFIG_DIR`).
pub fn socket_missing(socket_path: &Path) -> Failure {
    Failure::new(
        format!(
            "could not connect to the Deckard signer daemon at {}",
            socket_path.display()
        ),
        "the daemon is not running — it is spawned by the Deckard app, not by this sidecar",
        "start the Deckard app first (it spawns and supervises the signer), then retry; \
         in demo mode run `just demo` and keep the app it opens running",
    )
}

/// Map a daemon `Deny`/`Denied` reason tag to its catalog entry. `config_dir` (when known)
/// lets `locked` distinguish the no-vault case from the merely-locked case.
pub fn from_deny_reason(reason: &str, config_dir: Option<&Path>) -> Failure {
    if reason.starts_with(deny_reasons::BROADCAST_FAILED) {
        return Failure::new(
            format!("the transaction broadcast failed ({reason})"),
            "the daemon signed nothing or the RPC refused the transaction",
            "check the chain/RPC is up (`just demo-check` in demo mode), then re-run the \
             flow from deckard_shield — the request was not consumed by a failed broadcast",
        );
    }
    if reason.starts_with(deny_reasons::SIGNER_ERROR) {
        return Failure::new(
            format!("the daemon could not produce a signer for the wallet ({reason})"),
            "the unlocked key could not be turned into a transaction signer — an internal \
             daemon error, not a policy refusal; nothing was signed",
            "re-unlock the wallet in the Deckard app, then re-run the flow from \
             deckard_shield; if it recurs, restart the app",
        );
    }
    if reason.starts_with(deny_reasons::SIGN_FAILED) {
        return Failure::new(
            format!("signing the order digest failed ({reason})"),
            "the offline EIP-712 signing step errored before anything was submitted",
            "re-run the swap flow from the start; a recurring failure is a client/daemon bug",
        );
    }
    if reason.starts_with(deny_reasons::RAILGUN_KEYS) {
        return Failure::new(
            format!("a Railgun key operation failed ({reason})"),
            "the shielded-key derivation or view grant errored",
            "restart the Deckard app; if it recurs, the chain may be unsupported for shielding",
        );
    }
    match reason {
        deny_reasons::MALFORMED_REQUEST => Failure::new(
            "the daemon could not decode the request frame",
            "the bytes the sidecar sent did not parse as a valid signer request — a \
             client/version mismatch, not a policy refusal",
            "re-run the flow from deckard_shield; if it recurs, make sure the sidecar and \
             the Deckard app are the same version",
        ),
        deny_reasons::LOCKED => {
            let no_vault = config_dir
                .map(|d| !d.join(deckard_core::config::VAULT_FILE).exists())
                .unwrap_or(false);
            if no_vault {
                Failure::new(
                    "no wallet exists yet (the daemon has no keystore to unlock)",
                    "onboarding has not been completed in this config dir — this is NOT a \
                     locked wallet, there is nothing to unlock",
                    "create (or import) a throwaway wallet in the Deckard app — in demo \
                     mode, the app `just demo` opened — then retry",
                )
            } else {
                Failure::new(
                    "the wallet is locked (the daemon holds no key)",
                    "the daemon starts locked, and STOP/lock zeroizes the key",
                    "unlock the wallet in the Deckard app, then retry",
                )
            }
        }
        deny_reasons::REVOKED => Failure::new(
            "the signer is stopped (STOP / revoke_all is engaged)",
            "the panic brake zeroized the key and denied every in-flight request — \
             including ones approved before the STOP",
            "this is irreversible for the session; a human must re-unlock the wallet in \
             the Deckard app to re-arm, then re-run the flow from deckard_shield",
        ),
        deny_reasons::EXPIRED => Failure::new(
            "this request expired before it was executed",
            "approvals have a TTL; a stale request_id can never be executed later",
            "re-run the flow from deckard_shield to get a fresh request_id",
        ),
        deny_reasons::UNKNOWN_REQUEST => Failure::new(
            "the daemon does not know this request_id",
            "the app re-unlocked (or the daemon restarted), which starts a clean session \
             and clears all pending requests",
            "re-run the flow from deckard_shield — do not reuse old request_ids",
        ),
        deny_reasons::ALREADY_EXECUTED => Failure::new(
            "this exact request was already broadcast",
            "request ids are deterministic per intent, so an identical re-shield in the \
             same session maps to the already-broadcast request",
            "do NOT retry this request_id; to demo again, vary the amount (a different \
             amount is a new request) or re-unlock in the app for a fresh session",
        ),
        deny_reasons::BROADCAST_TIMEOUT => Failure::new(
            "the broadcast timed out — transaction status UNKNOWN",
            "the RPC did not answer within the daemon's broadcast window; the transaction \
             MAY already be on-chain",
            "do NOT retry (a retry could double-spend); check the transaction in the \
             Deckard app or with `just demo-check`, and only act once the status is known",
        ),
        deny_reasons::NOT_APPROVED => Failure::new(
            "this request needs a human approval before it can execute",
            "the policy (or the mainnet guardrail) classified it NeedsApproval and no \
             human has approved it yet",
            "a human must approve in the Deckard app; the approval UI is not in this \
             alpha — lower the amount under the policy per-tx cap or edit policy.json",
        ),
        deny_reasons::USER_DENIED => Failure::new(
            "a human denied this request",
            "the approval card was answered with Deny",
            "respect the denial; propose a different action only if the human asks for it",
        ),
        deny_reasons::CHAIN_MISMATCH => Failure::new(
            "the daemon signs for a different chain than this request targets",
            "this sidecar and the daemon disagree on the chain id (e.g. a demo sidecar \
             talking to the real daemon, or vice versa)",
            "re-run `deckard-mcp install --demo` so Claude's config carries the demo \
             socket + chain, and make sure `just demo` (not the everyday app) is running",
        ),
        deny_reasons::OVER_CAP => Failure::new(
            "the amount is over the policy cap and the policy raises no approval card",
            "require_approval is Never, so an over-cap write has nothing to authorize it",
            "lower the amount under policy.per_tx_cap_wei (call deckard_policy_get to read \
             it) or edit policy.json",
        ),
        deny_reasons::CAP_EXCEEDED => Failure::new(
            "executing this request would exceed the spending caps",
            "caps are re-checked at sign time against what was already spent today",
            "lower the amount or wait for the daily window to roll over (UTC midnight); \
             call deckard_policy_get for the current numbers",
        ),
        deny_reasons::OFF_ALLOWLIST => Failure::new(
            "the recipient is not on the policy allowlist",
            "the policy restricts recipients and this target is not listed",
            "use an allowed recipient, or a human must edit policy.json",
        ),
        deny_reasons::UNDECODABLE => Failure::new(
            "the intent's calldata does not match its kind",
            "shape validation failed (e.g. a Shield without RelayAdapt calldata)",
            "this is a bug in the proposing client if it recurs — re-run the flow from \
             deckard_shield",
        ),
        deny_reasons::SHIELD_TO_MISMATCH => Failure::new(
            "the shield does not target the Railgun RelayAdapt contract for this chain",
            "the daemon refuses Shield intents aimed anywhere else (or on chains it has \
             no adapter table for)",
            "re-run the flow from deckard_shield (it builds the correct target); if it \
             recurs, the chain is unsupported for shielding",
        ),
        deny_reasons::ERC20_UNSUPPORTED_V1 | deny_reasons::UNSUPPORTED_V1 => Failure::new(
            "that action is not supported in v0.1",
            "v0.1 supports native-ETH send and shield only",
            "stay with native-ETH deckard_shield / deckard_execute",
        ),
        other => Failure::new(
            format!("the daemon refused: {other}"),
            "see the daemon reason above (it is redacted and safe to read)",
            "call deckard_policy_get to inspect the fence; if the reason is unclear, \
             check the Deckard app",
        ),
    }
}

/// A transport error DURING `execute` is special: the broadcast may or may not have
/// happened. Everything rides on the agent not retrying blind.
pub fn execute_transport_unknown() -> Failure {
    Failure::new(
        "lost the daemon connection while executing — transaction status UNKNOWN",
        "the request may have been signed and broadcast before the connection dropped",
        "do NOT retry; check the transaction in the Deckard app (or `just demo-check`) \
         and only act once the status is known",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every catalog entry carries all three parts, and none of the static copy contains
    /// anything secret-shaped. (The dynamic parts — daemon reasons — are redacted at the
    /// daemon boundary; T9's transcript scan covers the live path.)
    #[test]
    fn catalog_is_three_part_and_static_copy_is_clean() {
        let reasons = [
            "locked",
            "revoked",
            "expired",
            "unknown_request",
            "already_executed",
            "broadcast_timeout",
            "not_approved",
            "user_denied",
            "chain_mismatch",
            "over_cap",
            "cap_exceeded",
            "off_allowlist",
            "undecodable",
            "shield_to_mismatch",
            "erc20_unsupported_v1",
            "unsupported_v1",
            "broadcast_failed: connection refused",
            "something_novel",
        ];
        let mut all = vec![
            socket_missing(Path::new("/tmp/x.sock")),
            execute_transport_unknown(),
        ];
        all.extend(reasons.iter().map(|r| from_deny_reason(r, None)));
        for f in &all {
            assert!(!f.problem.is_empty(), "problem missing: {f:?}");
            assert!(!f.cause.is_empty(), "cause missing: {f:?}");
            assert!(!f.fix.is_empty(), "fix missing: {f:?}");
            let text = f.to_json();
            assert!(
                !text.to_lowercase().contains("passphrase"),
                "static copy must not name secrets: {text}"
            );
        }
    }

    /// The do-NOT-retry instruction is present on the two retry-trap entries.
    #[test]
    fn retry_traps_say_do_not_retry() {
        for reason in ["broadcast_timeout", "already_executed"] {
            let f = from_deny_reason(reason, None);
            assert!(
                f.fix.contains("do NOT retry") || f.fix.contains("vary the amount"),
                "{reason} must warn against retrying: {}",
                f.fix
            );
        }
        assert!(execute_transport_unknown().fix.contains("do NOT retry"));
    }

    /// `locked` distinguishes a missing vault (onboarding) from a locked one.
    #[test]
    fn locked_distinguishes_no_vault() {
        let dir = std::env::temp_dir().join(format!("deckard-mcp-novault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let f = from_deny_reason("locked", Some(&dir));
        assert!(f.problem.contains("no wallet exists"), "{f:?}");

        std::fs::write(dir.join(deckard_core::config::VAULT_FILE), b"sealed").unwrap();
        let f = from_deny_reason("locked", Some(&dir));
        assert!(f.problem.contains("locked"), "{f:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
