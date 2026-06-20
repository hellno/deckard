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

/// Map a raw provider/transport error string to ONE calm, plain line for the UI. Unlike
/// [`humanize_deny`] (which maps the daemon's terse, known `reason` tags), this takes the
/// free-form error text a read/RPC call can return and matches it by substring, so a noisy
/// `reqwest`/transport failure reads as a calm, actionable line instead of raw provider text.
pub fn humanize_read_error(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("insufficient funds") {
        "Not enough ETH to cover the amount plus gas.".into()
    } else if lower.contains("nonce") {
        "Transaction ordering error. Try again.".into()
    } else if lower.contains("connection")
        || lower.contains("timeout")
        || lower.contains("error sending request")
        || lower.contains("reqwest")
        || lower.contains("transport")
    {
        "Couldn't reach the network. Retrying.".into()
    } else {
        "The network rejected the request.".into()
    }
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

/// Map a daemon deny `reason` to calm, **swap-specific** copy (#25). [`humanize_deny`] is
/// deposit/shield-worded ("the deposit…"), which reads wrong on a swap, so the swap path routes
/// its denies here. The swap-only policy/admission tags get distinct copy; anything else falls
/// through to [`humanize_deny`] so the shared session/process tags (`locked`, `chain_mismatch`,
/// `broadcast_*`, …) keep their single source of truth.
pub fn humanize_swap_deny(reason: &str) -> String {
    match reason {
        // --- order admission (deckard-contract::evaluate_order) ---
        "receiver_not_wallet" | "receiver_zero" => {
            "a swap can only send the bought token back to your own wallet".into()
        }
        "off_swap_list" => "one of these tokens isn't on the agent's swap allow-list".into(),
        "valid_to_too_far" => {
            "the order's expiry is too far out — re-quote and try the swap again".into()
        }
        "zero_amount" => "enter an amount greater than zero to swap".into(),
        // --- shaped-approve admission (the exact-gross relayer approve) ---
        "approve_with_value" => "the token approval must not move any ETH".into(),
        "approve_wrong_spender" => {
            "the token approval targets the wrong contract — review the swap again".into()
        }
        "approve_no_matching_order" => {
            "the approval didn't match a pending order — review the swap again".into()
        }
        // --- order sign / id guards ---
        "already_signed" => {
            "this order was already signed — it's on its way to the orderbook".into()
        }
        "already_executed" => "this swap was already submitted".into(),
        "not_an_order" => "the signer session was reset — review the swap again".into(),
        "swap_unsupported_in_mock" => "swaps aren't available in this test build".into(),
        // Everything else (session/process tags) keeps the shared humanizer's copy.
        other => humanize_deny(other),
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
    fn humanize_swap_deny_uses_swap_worded_copy_and_falls_through() {
        use super::humanize_swap_deny;
        // Swap-only tags get swap-worded copy (never "deposit").
        for tag in [
            "receiver_not_wallet",
            "off_swap_list",
            "valid_to_too_far",
            "approve_with_value",
            "approve_wrong_spender",
            "approve_no_matching_order",
            "already_signed",
        ] {
            let line = humanize_swap_deny(tag);
            assert!(!line.is_empty(), "{tag} must map to copy");
            assert!(
                !line.to_lowercase().contains("deposit"),
                "{tag} must not be deposit-worded: {line}"
            );
            assert_ne!(line, tag, "{tag} must be humanized, not shown raw");
        }
        // A shared session/process tag falls through to the shared humanizer (one source of truth).
        assert_eq!(humanize_swap_deny("locked"), humanize_deny("locked"));
        assert_eq!(
            humanize_swap_deny("chain_mismatch"),
            humanize_deny("chain_mismatch")
        );
        // An unknown tag still falls through unchanged (never swallowed).
        assert_eq!(
            humanize_swap_deny("some_new_swap_reason"),
            "some_new_swap_reason"
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
