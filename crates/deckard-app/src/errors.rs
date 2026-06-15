//! errors — UI-facing error shaping shared across the funds-touching surfaces. Trims noisy
//! provider errors to one line (`short_err`), maps terse daemon deny `reason` tags to calm
//! user-facing copy (`humanize_deny`), and flags the deny reasons that mean the unlock session
//! ended (`is_session_ended`). Moved verbatim out of `shell.rs` so Shield/Send/Swap share one copy.

/// Trim a noisy provider error down to one short line for the UI.
pub fn short_err(e: impl std::fmt::Display) -> String {
    let line = e.to_string();
    let line = line.lines().next().unwrap_or("").trim();
    line.chars().take(140).collect()
}

/// Map a daemon deny/`reason` tag to a calm, user-facing line (the wire tags are terse +
/// machine-readable; the UI shouldn't show `chain_mismatch` raw).
pub fn humanize_deny(reason: &str) -> String {
    // The broadcast error carries a variable RPC suffix, so match it by prefix.
    if reason.starts_with("broadcast_failed") {
        return "the deposit couldn't be broadcast — check your network, then review again".into();
    }
    match reason {
        "locked" => "unlock your wallet first".into(),
        "revoked" => "the signer is paused (STOP is active)".into(),
        "chain_mismatch" => {
            "the signer is on a different chain than this deposit — reconcile the chain first"
                .into()
        }
        "over_cap" | "cap_exceeded" => "it exceeds the agent's spending cap".into(),
        "off_allowlist" => "the recipient isn't on the allowlist".into(),
        "undecodable" => "the deposit calldata didn't validate".into(),
        "shield_to_mismatch" => {
            "the deposit doesn't target the Railgun contract for this chain".into()
        }
        "not_approved" => "this deposit hasn't been approved yet — review it again".into(),
        "unknown_request" => {
            "the signer session was reset — review the deposit again".into()
        }
        "erc20_unsupported_v1" => "only native-ETH shields are supported in v1".into(),
        "unsupported_v1" => "that action isn't supported in v1".into(),
        "broadcast_timeout" => {
            "the network didn't confirm in time — your deposit may already be in flight, so check your activity before retrying"
                .into()
        }
        "already_executed" => "this deposit was already submitted".into(),
        other => other.to_string(),
    }
}

/// True for a daemon `reason` that means the unlock **session ended** — the key was zeroized
/// by a STOP (an external `RevokeAll` from an MCP client, or the daemon is otherwise `Locked`).
/// The app must return to the unlock gate, not just show an inline error: a propose against a
/// locked daemon answers `locked`; an execute of a prior request answers `revoked`.
pub fn is_session_ended(reason: &str) -> bool {
    matches!(reason, "locked" | "revoked")
}

#[cfg(test)]
mod tests {
    use super::{humanize_deny, is_session_ended};

    #[test]
    fn session_ended_matches_only_stop_states() {
        // A locked daemon answers `locked` to a propose; an execute after STOP answers
        // `revoked`. Both must bounce the app back to the unlock gate.
        assert!(is_session_ended("locked"));
        assert!(is_session_ended("revoked"));
        // Ordinary policy denials stay inline (the app stays Ready, shows the reason).
        for inline in [
            "over_cap",
            "off_allowlist",
            "chain_mismatch",
            "shield_to_mismatch",
            "not_approved",
            "already_executed",
            "broadcast_timeout",
        ] {
            assert!(!is_session_ended(inline), "{inline} must stay inline");
        }
    }

    #[test]
    fn humanize_deny_maps_known_tags_to_their_lines() {
        // A representative arm from each match clause — the exact copy the UI must show.
        assert_eq!(humanize_deny("locked"), "unlock your wallet first");
        assert_eq!(
            humanize_deny("revoked"),
            "the signer is paused (STOP is active)"
        );
        assert_eq!(
            humanize_deny("chain_mismatch"),
            "the signer is on a different chain than this deposit — reconcile the chain first"
        );
        // The two-tag arm collapses to one line.
        assert_eq!(
            humanize_deny("over_cap"),
            "it exceeds the agent's spending cap"
        );
        assert_eq!(
            humanize_deny("cap_exceeded"),
            "it exceeds the agent's spending cap"
        );
        assert_eq!(
            humanize_deny("off_allowlist"),
            "the recipient isn't on the allowlist"
        );
        assert_eq!(
            humanize_deny("undecodable"),
            "the deposit calldata didn't validate"
        );
        assert_eq!(
            humanize_deny("shield_to_mismatch"),
            "the deposit doesn't target the Railgun contract for this chain"
        );
        assert_eq!(
            humanize_deny("not_approved"),
            "this deposit hasn't been approved yet — review it again"
        );
        assert_eq!(
            humanize_deny("unknown_request"),
            "the signer session was reset — review the deposit again"
        );
        assert_eq!(
            humanize_deny("erc20_unsupported_v1"),
            "only native-ETH shields are supported in v1"
        );
        assert_eq!(
            humanize_deny("unsupported_v1"),
            "that action isn't supported in v1"
        );
        assert_eq!(
            humanize_deny("broadcast_timeout"),
            "the network didn't confirm in time — your deposit may already be in flight, so check your activity before retrying"
        );
        assert_eq!(
            humanize_deny("already_executed"),
            "this deposit was already submitted"
        );
    }

    #[test]
    fn humanize_deny_matches_broadcast_failed_by_prefix() {
        // The broadcast error carries a variable RPC suffix, so any `broadcast_failed*` maps to
        // the same calm line.
        let line = "the deposit couldn't be broadcast — check your network, then review again";
        assert_eq!(humanize_deny("broadcast_failed"), line);
        assert_eq!(
            humanize_deny("broadcast_failed: connection refused (http://localhost:8545)"),
            line
        );
    }

    #[test]
    fn humanize_deny_passes_unknown_tags_through() {
        // An unrecognised tag falls through unchanged (the `other => other.to_string()` arm) —
        // the UI shows it raw rather than swallowing a new, un-mapped reason.
        assert_eq!(humanize_deny("some_new_reason"), "some_new_reason");
        assert_eq!(humanize_deny(""), "");
    }
}
