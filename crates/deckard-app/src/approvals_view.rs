//! Approvals — the agent-approval queue + the shared clear-signing review surface
//! (DESIGN §Trust & safety affordances, §Components → Activity row + Clear-signing review).
//!
//! This is the human-in-the-loop seam of the "sovereign autopilot": an agent (Atlas)
//! proposes a write that breaches its budget/scope, the daemon parks it as a `Pending`
//! record, and the human approves or denies it here. Two surfaces over one Shell field
//! (`approvals_reviewing`):
//!
//! - **Queue** — a dense, single-line-per-row list (the DESIGN activity-row schema, NOT a
//!   card mosaic): who proposed it (the cyan agent squircle for an agent, neutral for the
//!   app), the verb+object pulled from the payload, the breached-limit cite, a mm:ss TTL
//!   hint, and a status glyph. The selected row is a **brightness lift** (`theme.secondary`),
//!   NEVER a colored keyline (DESIGN §Components default). Loading is 3 skeleton rows at
//!   `bg.raise` with no spinner; an error fails loud in `danger`; empty is one muted line.
//! - **Review** — the same clear-signing card `send_view` renders for a self-send: a
//!   plain-language headline, an agent header band naming Atlas + the limit it cites, the
//!   canonical what-leaves / where / breached-limit key/value rows in mono-for-money, danger
//!   early, then two affordances. Unlike Send there is NO hold-to-confirm here: this surface
//!   confirms via `⌘Enter` (Lane D wires the key) — the buttons just mirror it on click.
//!
//! Render is `&self`; the only mutation path is `cx.listener` closures calling the Lane-D
//! methods `approve_selected` / `cancel_review`. The queue reads `self.pending`,
//! `self.approvals_selected`, `self.approvals_loading`, `self.approvals_error`,
//! `self.approvals_reviewing`, and `self.mask`; it never mutates Shell directly.

use gpui::{
    div, px, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use deckard_contract::{
    ApprovalStatus, Intent, IntentKind, PendingPayloadView, PendingRecord, ProposalOrigin,
};

use crate::money::money;
use crate::shell::Shell;
use crate::shell_chrome::agent_squircle;
use crate::theme;

/// The displayed subject for a proposal's origin: the agent's name when an agent proposed
/// it, "You" when the foreground app did. There is one agent in the demo scope (Atlas);
/// DESIGN's two-actor model only needs to answer "me or the machine?" in under a second.
fn origin_subject(origin: ProposalOrigin) -> &'static str {
    match origin {
        ProposalOrigin::Agent => "Atlas",
        ProposalOrigin::App => "You",
    }
}

/// Middle-truncate a long `0x…` string for a tight row (mirrors `send_view::short_mid`):
/// enough of each end (first 10 + last 6) that two values stay distinguishable.
fn short_mid(s: &str) -> String {
    if s.len() >= 16 {
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

/// A short, EIP-55-checksummed, middle-truncated address for the row + card. Checksummed
/// (not lowercase `:#x`) because this is a destination the human verifies before approving —
/// the same `to_checksum(None)` the Send flow signs against (DESIGN §Trust: addresses always
/// mono, middle-truncated, distinguishable).
fn short_address(addr: &deckard_core::Address) -> String {
    short_mid(&addr.to_checksum(None))
}

/// A short, stable handle for a request id (`0xA1b2…9F3c`) — used only as a quiet metadata
/// hint, never as the primary identity (the subject + object carry that).
fn short_request_id(id: &deckard_contract::RequestId) -> String {
    let hex = format!("{id:#x}");
    if hex.len() >= 12 {
        format!("{}…{}", &hex[..6], &hex[hex.len() - 4..])
    } else {
        hex
    }
}

/// Format the snapshot `remaining_ms` as a `mm:ss` TTL hint. A pure in-view transform of the
/// daemon's snapshot — it does not tick; the poller refreshes it. `0` (terminal/elapsed) and
/// any sub-second remainder both read as a small floor so an about-to-expire row never shows
/// a misleading `00:00` while still `Pending`.
fn remaining_mmss(remaining_ms: u64) -> String {
    let secs = remaining_ms / 1000;
    let mins = secs / 60;
    let rem = secs % 60;
    format!("{mins:02}:{rem:02}")
}

/// A one-line "verb + object" summary of a pending payload, for the dense queue row. `Tx` is
/// the main case (a native send shows `send {amount} ETH → {to}`); an ERC-20 send and a
/// non-send intent kind degrade to a sensible phrase. `Order`/`Approve` get a one-line summary
/// so the row never crashes and never leaves a gap.
fn payload_summary(payload: &PendingPayloadView, mask: bool) -> String {
    match payload {
        PendingPayloadView::Tx(intent) => tx_summary(intent, mask),
        PendingPayloadView::Order(order) => {
            format!(
                "swap → buy ≥ {} (min)",
                masked_amount(order.buy_amount_min, mask)
            )
        }
        PendingPayloadView::Approve { token, spender, .. } => {
            format!(
                "approve {} to spend {}",
                short_address(spender),
                short_address(token)
            )
        }
    }
}

/// The verb+object for a transaction intent. A native send (`token: None`, kind `Send`) reads
/// `send {amount} ETH → {to}`; shield/unshield/contract-call name their action; an ERC-20
/// send names the token contract so the row stays honest about what's moving.
fn tx_summary(intent: &Intent, mask: bool) -> String {
    let to = short_address(&intent.to);
    match intent.kind {
        IntentKind::Send => {
            let amount = masked_amount(intent.value, mask);
            if intent.token.is_none() {
                format!("send {amount} ETH → {to}")
            } else {
                format!("send {amount} tokens → {to}")
            }
        }
        IntentKind::Shield => format!("shield {} ETH", masked_amount(intent.value, mask)),
        IntentKind::Unshield => format!("unshield {} ETH", masked_amount(intent.value, mask)),
        IntentKind::ContractCall => format!("call → {to}"),
    }
}

/// A plain-string masked amount for the one-line summaries (the spans-based `money()` is for
/// the review card). Honors the privacy mask with the same fixed-bullet glyph.
fn masked_amount(raw: deckard_core::U256, mask: bool) -> String {
    crate::money::mask_money(mask, &deckard_core::format_amount(raw, 18, 6))
}

/// Whether a record is still awaiting the human (only `Pending` rows are actionable). Drives
/// the over-cap cite + the mm:ss hint; terminal rows render their outcome instead.
fn is_pending(status: &ApprovalStatus) -> bool {
    matches!(status, ApprovalStatus::Pending)
}

/// **The queue order.** Pending-status records only (terminal rows belong in Activity), sorted
/// ascending by `remaining_ms` so the soonest-to-expire approval is at the top, ties broken by
/// `request_id` for a stable, deterministic order across refreshes. Pure: borrows the input
/// slice, allocates only the `Vec` of references.
pub(crate) fn approvals_queue(records: &[PendingRecord]) -> Vec<&PendingRecord> {
    let mut queue: Vec<&PendingRecord> = records.iter().filter(|r| is_pending(&r.status)).collect();
    queue.sort_by(|a, b| {
        a.remaining_ms
            .cmp(&b.remaining_ms)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });
    queue
}

impl Shell {
    /// Dispatch: the clear-signing review when one is open (and its record is still pending in
    /// the latest snapshot), otherwise the queue. If the reviewed record vanished from the
    /// snapshot (resolved or expired out from under us) we fall back to the queue rather than
    /// render a stale card.
    pub fn render_approvals(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(reviewing) = self.approvals_reviewing {
            if let Some(record) = self
                .pending
                .iter()
                .find(|r| r.request_id == reviewing && is_pending(&r.status))
            {
                return self.render_approvals_review(record, cx).into_any_element();
            }
        }
        self.render_approvals_queue(cx).into_any_element()
    }

    /// The queue surface: heading + one of the four data states (loading / error / empty /
    /// rows). Centered in the same column shell the rest of the app uses, but full-width so the
    /// dense rows breathe.
    fn render_approvals_queue(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let raise = theme.secondary; // bg.raise — the skeleton tone
        let danger = theme.danger;

        let body = if self.approvals_loading && self.pending.is_empty() {
            // DESIGN §Required states: loading = 3 skeleton rows at `bg.raise`, no spinner.
            v_flex()
                .w_full()
                .gap_2()
                .children((0..3).map(|_| skeleton_row(raise)))
                .into_any_element()
        } else if let Some(err) = self.approvals_error.as_ref() {
            // Fail loud, never an empty queue when errored (mirror `send_view::error_line`).
            error_line(&format!("Can't reach the signer — {err}"), danger).into_any_element()
        } else {
            let queue = approvals_queue(&self.pending);
            if queue.is_empty() {
                // One muted reassurance line — the calm "nothing needs you" state.
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("All clear — Atlas is acting within its limits")
                    .into_any_element()
            } else {
                // A `for` loop (mirrors `palette.rs`) over the snapshot's pending rows, so the
                // `cx.listener` on each row attaches cleanly without a map-closure capturing
                // `&mut cx`.
                let mut list = v_flex().w_full().gap_1();
                for (i, record) in queue.into_iter().enumerate() {
                    list = list.child(self.queue_row(i, record, cx));
                }
                list.into_any_element()
            }
        };

        approvals_shell(
            v_flex()
                .w_full()
                .gap_4()
                .child(approvals_heading(
                    "Approvals",
                    "Agent actions waiting on you. Approve to let Atlas proceed, or deny to stop it.",
                    fg,
                    muted,
                ))
                .child(body)
                .into_any_element(),
        )
    }

    /// One dense activity row: `[agent squircle?] [subject] · [verb+object] · over-cap …… [mm:ss] [status]`.
    /// The selected row lifts to `theme.secondary` (brightness lift, NEVER a keyline). An
    /// `Expired` row carries an inline muted note instead of the cite + clock.
    fn queue_row(
        &self,
        index: usize,
        record: &PendingRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let amber = theme::amber(theme.is_dark());
        let lift = theme.secondary; // selected = brightness lift, not a colored keyline
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        let selected = index == self.approvals_selected;
        let is_agent = record.origin == ProposalOrigin::Agent;
        let pending = is_pending(&record.status);
        let expired = matches!(record.status, ApprovalStatus::Expired);

        // The lead glyph: the ONE cyan surface (the agent squircle) for an agent proposal; a
        // neutral identity dot for an app-origin proposal. Static here (never the "acting"
        // pulse) — a parked approval is not mid-action.
        let lead = if is_agent {
            agent_squircle(
                px(20.0),
                px(6.0),
                false,
                agent,
                agent_tint,
                "approvals-agent",
            )
        } else {
            div()
                .size(px(20.0))
                .rounded(px(6.0))
                .bg(theme::identity_square(is_dark))
                .into_any_element()
        };

        let subject = origin_subject(record.origin);
        let summary = payload_summary(&record.payload, self.mask);

        // The trailing cluster: the over-cap cite + a mm:ss TTL hint for a live row; for an
        // expired row, an inline muted note (no clock, no cite).
        let trailing = if expired {
            div()
                .text_xs()
                .text_color(muted)
                .child("approval window expired")
                .into_any_element()
        } else if pending {
            h_flex()
                .items_center()
                .gap_3()
                .flex_shrink_0()
                // Over-cap is loud-but-quiet: an amber caution icon carries the signal, the
                // word carries the emphasis (DESIGN §Color rule 7 — no keyline, no banner).
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(IconName::TriangleAlert).text_color(amber).small())
                        .child(div().text_xs().text_color(fg).child("over per-tx cap")),
                )
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .text_color(muted)
                        .child(remaining_mmss(record.remaining_ms)),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let mut row = h_flex()
            .id(("approvals-row", index))
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(6.0))
            .child(
                h_flex()
                    .items_center()
                    .gap_2p5()
                    .min_w_0()
                    .child(lead)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child(subject.to_string()),
                    )
                    .child(div().text_sm().text_color(muted).child("·"))
                    .child(div().min_w_0().text_sm().text_color(fg).child(summary))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(short_request_id(&record.request_id)),
                    ),
            )
            .child(trailing);

        // Click-to-open: only the already-selected row is the actionable one, so a click on it
        // opens its clear-signing review via the frozen `open_selected_review` (the queue is
        // keyboard-first — j/k move the highlight, Enter/click opens). A click on a non-selected
        // row is inert by design: there is no per-index select on the frozen Lane-D surface, so
        // we never invent one (see report — flagged for Lane D to add `approvals_select(index)`
        // if click-to-select-any-row is wanted).
        if selected {
            row = row
                .bg(lift)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.open_selected_review(cx)));
        }
        row
    }

    /// The clear-signing review card for a single pending record — the shared trust engine
    /// (DESIGN §Components → Clear-signing review), mirroring `send_view::render_send_review`:
    /// a plain headline, an agent header band naming Atlas + the limit it cites, ONE canonical
    /// key/value list (what leaves / where / breached limit) with no interior grid lines,
    /// danger early, then two affordances. Confirm here is `⌘Enter`, never a hold.
    pub fn render_approvals_review(
        &self,
        record: &PendingRecord,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let danger = theme.danger;
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        let is_agent = record.origin == ProposalOrigin::Agent;
        let subject = origin_subject(record.origin);

        // The agent header band — the ONE cyan surface on this card: the squircle + a
        // plain-language statement of what the agent wants + the boundary it cites. Neutral
        // for an app-origin proposal (no agent acted).
        let band = {
            let lead = if is_agent {
                agent_squircle(
                    px(24.0),
                    px(7.0),
                    false,
                    agent,
                    agent_tint,
                    "approvals-review-agent",
                )
            } else {
                div()
                    .size(px(24.0))
                    .rounded(px(7.0))
                    .bg(theme::identity_square(is_dark))
                    .into_any_element()
            };
            let band_bg = if is_agent { agent_tint } else { surface };
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .px_3()
                .py_2p5()
                .rounded_lg()
                .bg(band_bg)
                .child(lead)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .child(format!(
                            "{subject} wants to {} · cites: over per-tx cap",
                            payload_summary(&record.payload, self.mask)
                        )),
                )
        };

        // The canonical key/value rows, payload-specific. A native send shows amount + dest;
        // every record shows the breached limit. Mono-for-money via `money()`, mask-aware.
        let detail_rows = self.review_detail_rows(record, cx);

        approvals_shell(
            v_flex()
                .w_full()
                .gap_4()
                .child(approvals_heading(
                    "Review request",
                    "Confirm exactly what leaves and where, then approve or cancel.",
                    fg,
                    muted,
                ))
                .child(band)
                // Danger early: an over-cap action is the loud-red moment (DESIGN §Color rule 6).
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(Icon::new(IconName::TriangleAlert).text_color(danger).small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(danger)
                                .child("This exceeds the per-transaction limit."),
                        ),
                )
                // The clear-signing card: one frame, no interior grid lines.
                .child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .children(detail_rows),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("approvals-approve")
                                .primary()
                                .label("⌘Enter  Approve")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.approve_selected(cx)),
                                ),
                        )
                        .child(
                            Button::new("approvals-cancel")
                                .ghost()
                                .label("Esc  Cancel")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_review(cx)),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("Approving authorizes this spend. It will be signed and broadcast, and you can't undo it."),
                )
                .into_any_element(),
        )
    }

    /// The canonical key/value rows for the review card, payload-specific. `Tx` is the main
    /// case: what leaves (amount, mask-aware) + where (the destination address) + the breached
    /// limit. `Order`/`Approve` render a sensible row set so the card never blanks.
    fn review_detail_rows(
        &self,
        record: &PendingRecord,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let mono = theme.mono_font_family.clone();
        let masked = self.mask;

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        match &record.payload {
            PendingPayloadView::Tx(intent) => {
                let unit = if intent.token.is_none() {
                    Some("ETH")
                } else {
                    Some("tokens")
                };
                rows.push(
                    kv_money_row(
                        "Amount",
                        intent.value,
                        unit,
                        masked,
                        mono.clone(),
                        fg,
                        muted,
                    )
                    .into_any_element(),
                );
                rows.push(
                    kv_mono_row("To", &short_address(&intent.to), mono.clone(), fg, muted)
                        .into_any_element(),
                );
            }
            PendingPayloadView::Order(order) => {
                rows.push(
                    kv_money_row(
                        "Sell",
                        order.sell_amount,
                        None,
                        masked,
                        mono.clone(),
                        fg,
                        muted,
                    )
                    .into_any_element(),
                );
                rows.push(
                    kv_money_row(
                        "Buy (min)",
                        order.buy_amount_min,
                        None,
                        masked,
                        mono.clone(),
                        fg,
                        muted,
                    )
                    .into_any_element(),
                );
                rows.push(
                    kv_mono_row(
                        "Receiver",
                        &short_address(&order.receiver),
                        mono.clone(),
                        fg,
                        muted,
                    )
                    .into_any_element(),
                );
            }
            PendingPayloadView::Approve {
                token,
                spender,
                amount,
            } => {
                rows.push(
                    kv_money_row("Approve", *amount, None, masked, mono.clone(), fg, muted)
                        .into_any_element(),
                );
                rows.push(
                    kv_mono_row("Token", &short_address(token), mono.clone(), fg, muted)
                        .into_any_element(),
                );
                rows.push(
                    kv_mono_row("Spender", &short_address(spender), mono.clone(), fg, muted)
                        .into_any_element(),
                );
            }
        }

        // Every record cites the same breached boundary — the canonical "how it was stopped"
        // fact, stated once (not a third restatement of the amount).
        rows.push(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .py_1p5()
                .child(div().text_sm().text_color(muted).child("Breached limit"))
                .child(div().text_sm().text_color(fg).child("Per-transaction cap"))
                .into_any_element(),
        );
        rows
    }
}

/// A key/value row whose value is a mono-for-money amount (DESIGN §Typography), mask-aware.
#[allow(clippy::too_many_arguments)]
fn kv_money_row(
    label: &'static str,
    raw: deckard_core::U256,
    unit: Option<&str>,
    masked: bool,
    mono: gpui::SharedString,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_1p5()
        .child(div().text_sm().text_color(muted).child(label))
        .child(
            div()
                .text_sm()
                .child(money(raw, 18, 6, unit, masked, mono, fg, muted)),
        )
}

/// A key/value row whose value is a mono address/handle (no money formatting).
fn kv_mono_row(
    label: &'static str,
    value: &str,
    mono: gpui::SharedString,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_1p5()
        .child(div().text_sm().text_color(muted).child(label))
        .child(
            div()
                .font_family(mono)
                .text_sm()
                .text_color(fg)
                .child(value.to_string()),
        )
}

/// One loading skeleton row: a subtle `bg.raise` bar (DESIGN §Required states — never a
/// spinner). Three of these stand in for the queue while a `PendingList` round-trip runs.
fn skeleton_row(raise: gpui::Hsla) -> impl IntoElement {
    div().w_full().h(px(40.0)).rounded(px(6.0)).bg(raise)
}

/// A one-line fail-loud error in `danger` (mirrors `send_view::error_line`).
fn error_line(msg: &str, danger: gpui::Hsla) -> impl IntoElement {
    div().text_sm().text_color(danger).child(format!("⚠ {msg}"))
}

/// The shared page heading: H1 (`text.primary`, 600) + a muted one-line subtitle. One anatomy
/// for every surface (DESIGN §Components → Page header).
fn approvals_heading(
    title: &'static str,
    subtitle: &'static str,
    fg: gpui::Hsla,
    muted: gpui::Hsla,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(title),
        )
        .child(div().text_sm().text_color(muted).child(subtitle))
}

/// The shared column shell for both Approvals surfaces. Wider than `send_view`'s centered
/// card (760px) because the queue is a dense full-width list, not a single review card;
/// left-aligned so rows read top-down.
fn approvals_shell(inner: gpui::AnyElement) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .p_8()
        .child(v_flex().w(px(760.0)).items_start().child(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U256};

    fn record(id: u8, status: ApprovalStatus, remaining_ms: u64) -> PendingRecord {
        PendingRecord {
            request_id: B256::repeat_byte(id),
            status,
            payload: PendingPayloadView::Tx(Intent {
                chain_id: 1,
                to: Address::repeat_byte(0x22),
                token: None,
                value: U256::from(1_u64),
                calldata: Bytes::new(),
                kind: IntentKind::Send,
            }),
            remaining_ms,
            origin: ProposalOrigin::Agent,
        }
    }

    #[test]
    fn queue_keeps_only_pending() {
        let records = vec![
            record(0x01, ApprovalStatus::Pending, 5_000),
            record(0x02, ApprovalStatus::Allowed, 0),
            record(0x03, ApprovalStatus::Expired, 0),
            record(
                0x04,
                ApprovalStatus::Denied {
                    reason: "revoked".into(),
                },
                0,
            ),
            record(0x05, ApprovalStatus::Pending, 1_000),
        ];
        let queue = approvals_queue(&records);
        assert_eq!(queue.len(), 2, "only Pending rows survive");
        assert!(queue
            .iter()
            .all(|r| matches!(r.status, ApprovalStatus::Pending)));
    }

    #[test]
    fn queue_sorts_ascending_by_remaining_then_request_id() {
        // Two rows share a remaining_ms; the tie must break by request_id ascending.
        let records = vec![
            record(0x0A, ApprovalStatus::Pending, 9_000),
            record(0x02, ApprovalStatus::Pending, 3_000),
            record(0x01, ApprovalStatus::Pending, 3_000),
            record(0x05, ApprovalStatus::Pending, 1_000),
        ];
        let queue = approvals_queue(&records);
        let order: Vec<_> = queue.iter().map(|r| r.remaining_ms).collect();
        assert_eq!(
            order,
            vec![1_000, 3_000, 3_000, 9_000],
            "ascending remaining_ms"
        );
        // The two 3_000 rows: 0x01 before 0x02 (request_id tie-break, stable).
        let ids: Vec<_> = queue.iter().map(|r| r.request_id).collect();
        assert_eq!(ids.get(1), Some(&B256::repeat_byte(0x01)));
        assert_eq!(ids.get(2), Some(&B256::repeat_byte(0x02)));
    }

    #[test]
    fn queue_is_empty_for_no_pending() {
        let records = vec![record(0x01, ApprovalStatus::Allowed, 0)];
        assert!(approvals_queue(&records).is_empty());
    }

    #[test]
    fn remaining_mmss_formats() {
        assert_eq!(remaining_mmss(0), "00:00");
        assert_eq!(remaining_mmss(1_000), "00:01");
        assert_eq!(remaining_mmss(65_000), "01:05");
        assert_eq!(remaining_mmss(120_000), "02:00");
    }

    #[test]
    fn origin_subject_maps_actor() {
        assert_eq!(origin_subject(ProposalOrigin::Agent), "Atlas");
        assert_eq!(origin_subject(ProposalOrigin::App), "You");
    }
}
