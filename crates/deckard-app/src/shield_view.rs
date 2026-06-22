//! Shield — the privacy hero's trigger flow (T5)'s [`CommitView`] descriptor. The actual
//! compose → review → done rendering lives in `commit_view.rs` (the generic renderer shared with
//! Send); this file is now just the byte-for-byte table of Shield's copy, button ids, the heading
//! glyph, the fee/net money rows, the 3-way conditional compose hint, and the handler hooks.
//!
//! A shield carries a Railgun fee and a private side — so `extra_rows` holds the 0.25% fee row +
//! the net "you'll receive (private)" line, the compose hint is the 3-way conditional line (own
//! 0zk address vs double-check vs enter), and the honesty surface has three lines. The heading
//! glyph is the neutral low-chroma "shield / private" tone (privacy sits off the cyan/agent +
//! amber/human actor axis; the human signal lives on the amber hold-to-confirm, not the heading).

use gpui::Context;

use deckard_core::U256;

use crate::commit_view::{CommitView, HonestyLine, MoneyRow};
use crate::shell::{Shell, Surface};
use crate::theme;

/// The Railgun shield fee, 25 bps (0.25%) — matches `deckard_core::shield`'s on-chain
/// deduction (`value - value*25/10000`). Shown so the review card never hides the haircut.
fn shield_fee(value: U256) -> U256 {
    value * U256::from(25u64) / U256::from(10_000u64)
}

/// The net the recipient receives after the Railgun fee (gross − fee). Mirrors the old
/// `render_shield_review`'s `gross.saturating_sub(fee)`.
fn shield_net(value: U256) -> U256 {
    value.saturating_sub(shield_fee(value))
}

/// The Shield surface descriptor. Reproduces the shipped Shield flow (T5) EXACTLY: same strings,
/// button ids, layout, the fee + net rows, the three honesty lines, the 3-way compose hint, and
/// the amber hold-to-confirm. Routed from `Shell::render` via `render_commit(&SHIELD_VIEW, cx)`.
pub static SHIELD_VIEW: CommitView = CommitView {
    // The shield flow's live state + the neutral "shield / private" heading glyph.
    flow: shield_flow,
    glyph_tone: theme::shield,

    // --- compose ---
    compose_title: "Shield to private",
    compose_subtitle:
        "Move public ETH into a Railgun private balance. The deposit itself is visible on Ethereum; the balance after is not.",
    recipient_label: "Recipient (your 0zk address)",
    review_button_id: "shield-review",
    review_label: "Review deposit",
    cancel_button_id: "shield-cancel",
    // The hint is conditional (3-way), driven by `shield_compose_hint`; no static line.
    compose_hint: None,
    compose_hint_dynamic: Some(shield_compose_hint),

    // --- review ---
    review_title: "Review deposit",
    review_subtitle: "Confirm what leaves, where it goes, and the fee, then shield with ⌘↵.",
    // The Railgun fee + the net private receipt, computed from the proposal's gross value.
    extra_rows: &[
        MoneyRow {
            label: "Railgun fee · 0.25%",
            compute: shield_fee,
        },
        MoneyRow {
            label: "You'll receive (private)",
            compute: shield_net,
        },
    ],
    honesty: &[
        HonestyLine {
            text: "This deposit is public on Ethereum.",
            emphasized: true,
            danger: false,
        },
        HonestyLine {
            text: "Avoid round or unusual amounts.",
            emphasized: true,
            danger: false,
        },
        HonestyLine {
            text: "A 0.25% Railgun fee is deducted; your private balance will read slightly less.",
            emphasized: false,
            danger: false,
        },
    ],
    hold_id: "shield-hold",
    hold_label_idle: "Shield to private",
    hold_label_busy: "Shielding…",
    edit_button_id: "shield-edit",

    // --- done ---
    done_title: "Deposit sent",
    done_body:
        "Your deposit is on its way to a private balance. It becomes spendable after on-chain confirmation and a private sync.",
    copy_button_id: "shield-copy-tx",
    done_button_id: "shield-done",

    // --- handlers (the existing `impl Shell` shield methods) ---
    on_review: review_shield,
    on_edit: open_shield,
    on_cancel: open_home,
    on_done: open_home,
    on_hold_start: shield_hold_start,
};

/// Re-acquire the shield flow's state from the shell (the descriptor's `flow` selector).
fn shield_flow(shell: &Shell) -> &crate::commit_flow::CommitFlow {
    &shell.shield
}

/// The 3-way conditional compose hint: only call the recipient "your own 0zk address" when it
/// actually matches the wallet's auto-filled address — a user-typed/edited recipient gets neutral
/// copy so the line never misrepresents where the deposit is going. Mirrors the old
/// `render_shield_compose`'s inline block exactly. `recipient_raw` is the recipient text the
/// renderer already read from the input.
fn shield_compose_hint(shell: &Shell, recipient_raw: &str) -> &'static str {
    let recipient = recipient_raw.trim();
    let is_own_address = shell.railgun_address.as_deref().map(str::trim) == Some(recipient);
    if recipient.is_empty() {
        "Enter the 0zk address that will receive the private balance."
    } else if is_own_address {
        "Pre-filled with your own 0zk address. Edit it to shield to a different recipient."
    } else {
        "Shielding to the 0zk address above. Double-check it before you continue."
    }
}

// Thin free-function adapters so the descriptor's `fn(&mut Shell, &mut Context<Shell>)` slots can
// name the surface's handlers (a `&'static` descriptor can't hold a closure, and the methods take
// `&mut self`). Each is a one-line forward to the existing handler.
fn review_shield(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.review_shield(cx);
}
fn open_shield(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open_shield(cx);
}
fn open_home(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open(Surface::Home, cx);
}
fn shield_hold_start(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.shield_hold_start(cx);
}
