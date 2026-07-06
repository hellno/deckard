//! Send — the native-ETH transfer flow's [`CommitView`] descriptor. The actual compose → review
//! → done rendering lives in `commit_view.rs` (the generic renderer shared with Shield in Step 2);
//! this file is now just the byte-for-byte table of Send's copy, button ids, the heading glyph,
//! and the handler hooks.
//!
//! A send has no Railgun fee and no private side — so `extra_rows` is empty (no fee row, no net
//! line), the compose hint is a single static line, and the honesty surface has two lines. The
//! heading glyph is the neutral low-chroma "public" identity tone (a public transfer sits off the
//! cyan/agent axis; the human signal lives on the amber hold-to-confirm, not the heading).

use gpui::Context;

use crate::commit_view::{CommitView, HonestyLine};
use crate::shell::{Shell, Surface};
use crate::theme;

/// The Send surface descriptor. Reproduces the shipped Send flow (#54) EXACTLY: same strings,
/// button ids, layout, and the amber hold-to-confirm. Routed from `Shell::render` via
/// `render_commit(&SEND_VIEW, cx)`.
pub static SEND_VIEW: CommitView = CommitView {
    // The send flow's live state + the neutral "public" heading glyph.
    flow: send_flow,
    glyph_tone: theme::identity_square,

    // --- compose ---
    compose_title: "Send ETH",
    compose_subtitle:
        "Transfer native ETH from your wallet. This transaction is public on Ethereum and can't be undone.",
    recipient_label: "Recipient (0x address or ENS name)",
    review_button_id: "send-review",
    review_label: "Review transfer",
    cancel_button_id: "send-cancel",
    compose_hint: Some(
        "An ENS name is resolved when you review. You'll confirm the exact address before sending.",
    ),
    compose_hint_dynamic: None,

    // --- review ---
    // No Railgun fee / private net line for a public send.
    extra_rows: &[],
    // The shared Review renders the ONE canonical danger line ("This can't be undone.") itself, so
    // the descriptor carries only the surface-specific amber caution below it (DESIGN §Clear-signing).
    honesty: &[HonestyLine {
        text: "Double-check the destination address; funds sent to the wrong address are lost.",
        emphasized: true,
        danger: false,
    }],
    hold_id: "send-hold",
    hold_label_idle: "Send",
    hold_label_busy: "Sending…",
    edit_button_id: "send-edit",

    // --- done ---
    done_title: "Transfer sent",
    done_body:
        "Your ETH is on its way. It settles after on-chain confirmation; your balance updates on the next sync.",
    copy_button_id: "send-copy-tx",
    done_button_id: "send-done",

    // --- handlers (the existing `impl Shell` send methods) ---
    on_review: review_send,
    on_edit: open_send,
    on_cancel: open_home,
    on_done: open_home,
    on_hold_start: send_hold_start,
};

/// Re-acquire the send flow's state from the shell (the descriptor's `flow` selector).
fn send_flow(shell: &Shell) -> &crate::commit_flow::CommitFlow {
    &shell.send
}

// Thin free-function adapters so the descriptor's `fn(&mut Shell, &mut Context<Shell>)` slots can
// name the surface's handlers (a `&'static` descriptor can't hold a closure, and the methods take
// `&mut self`). Each is a one-line forward to the existing handler.
fn review_send(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.review_send(cx);
}
fn open_send(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open_send(cx);
}
fn open_home(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open(Surface::Home, cx);
}
fn send_hold_start(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.send_hold_start(cx);
}
