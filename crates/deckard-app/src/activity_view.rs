//! Activity — the **see-and-stop feed** (#60), shaped as a Superhuman-style **inbox + log**: a
//! "NEEDS YOU" triage band on top, then a settled LOG below (DESIGN §Components → Activity row,
//! §Trust & safety affordances). This is the SINGLE agent-oversight surface: the NEEDS YOU band
//! IS the triage queue (the former separate Approvals surface collapsed into it).
//!
//! Activity is the ledger: it reads the daemon's `ActivityFeed`, so it ALSO shows auto-allowed and
//! executed actions that never wait in `PendingList` — the load-bearing point of #60 ("an auto-
//! allowed within-cap shield executes immediately and never enters the queue, so the daemon must
//! record what the agent *did*").
//!
//! **Inbox + log split.** Still-proposed rows (the things waiting on you) live ONLY in the NEEDS
//! YOU band — the inbox you triage — and never duplicate into the log. Everything settled (auto-
//! allowed, approved, denied, executed) falls to the day-grouped LOG. A pending row is selectable
//! and inline-approvable (select + Enter opens the clear-signing review, ⌘Enter approves, `x`
//! denies, all scoped to this surface's `key_context("Activity")`).
//!
//! **Two-signal fidelity (DESIGN §The actor model).** State is a small circular **glyph** that
//! carries the color (green check = approved/executed, red x = denied/revoked); the outcome
//! *label* stays `muted_foreground` so the log never floods green. The one exception is a row a
//! **human acted on** (see `human_acted`: any non-hands-free approval/execution, plus EVERY denial
//! or STOP revoke — a STOP is a human action even on a row that had auto-allowed) — its label tints
//! amber, so "you acted here" reads warm against the cyan-glyph agent rows. A lapsed `Expired` row
//! had no human and renders neutral. An executed shield reads the *result* ("moved … ETH to your
//! private balance"), the demo's payoff.
//!
//! Each row carries the two-actor chain (the cyan `agent_mark` for the agent, neutral for a
//! foreground app action), the lifecycle glyph, the real broadcast `tx_hash`, a relative
//! timestamp, and — for an over-cap/over-scope proposal — the ACTUAL breached fence (per-tx vs
//! daily, never a hardcoded cite). A header STOP control is the always-reachable panic brake.
//!
//! Render is `&self`; mutation flows through the `cx.listener` closures (open review / approve /
//! deny / STOP arm+confirm). The surface reads `self.activity`, `self.activity_selected`,
//! `self.activity_reviewing`, `self.activity_loading`, `self.activity_error`,
//! `self.activity_stop_arming`, `self.activity_stopped`, and `self.mask`.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    div, px, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use deckard_contract::{
    ActivityLifecycle, ActivityRecord, ApprovalRisk, BreachedLimit, Intent, IntentKind,
    MessageSigningRisk, PendingPayloadView, ProposalOrigin, RequestId, SignMessage,
    SignMessageKind,
};

use crate::money::money;
use crate::shell::Shell;
use crate::theme;
use crate::widgets::agent_mark;

/// The displayed subject for an action's origin: the agent's handle when an agent acted, "You"
/// when the foreground app did (E2, #182 — one agent in demo scope, named via `Shell::agent_handle`).
fn origin_subject(origin: ProposalOrigin, agent_handle: &str) -> &str {
    match origin {
        ProposalOrigin::Agent => agent_handle,
        ProposalOrigin::App => "You",
    }
}

/// A short, EIP-55-checksummed address for the row + card — the canonical
/// first-6+last-4 truncation via [`crate::widgets::short_addr`].
fn short_address(addr: &deckard_core::Address) -> String {
    crate::widgets::short_addr(&addr.to_checksum(None))
}

/// A short tx hash for the trailing cluster (`0x6ea1b2…9f3c`) — proof the action broadcast.
fn short_tx(hash: &RequestId) -> String {
    let hex = format!("{hash:#x}");
    if hex.len() >= 14 {
        format!("{}…{}", &hex[..8], &hex[hex.len() - 4..])
    } else {
        hex
    }
}

/// Wall-clock unix millis — read once per render to label rows relative to "now". Display-only.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// A relative-time label for a row ("just now", "4m ago", "2h ago"). The feed is a short-lived,
/// session-scoped ledger, so relative reads more naturally than an absolute clock — and it needs
/// no timezone dependency (no new crate) to be honest. Saturating so a clock that ticked
/// backward never underflows.
fn relative_time(ts_ms: u64, now: u64) -> String {
    let secs = now.saturating_sub(ts_ms) / 1000;
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// The day-band label for a row, by UTC day vs now. The feed is in-memory and session-scoped,
/// so in practice everything lands under "Today"; the grouping structure is kept so durable
/// calendar days slot in unchanged later.
fn day_label(ts_ms: u64, now: u64) -> &'static str {
    let day = ts_ms / 86_400_000;
    let today = now / 86_400_000;
    if day >= today {
        "Today"
    } else if day + 1 == today {
        "Yesterday"
    } else {
        "Earlier"
    }
}

/// A plain-string masked amount for the one-line summaries. Honors the privacy mask.
fn masked_amount(raw: deckard_core::U256, mask: bool) -> String {
    crate::money::mask_money(mask, &deckard_core::format_amount(raw, 18, 6))
}

/// A one-line "verb + object" summary of an action's payload, for the dense feed row.
fn payload_summary(payload: &PendingPayloadView, mask: bool) -> String {
    match payload {
        PendingPayloadView::Tx(intent) => tx_summary(intent, mask),
        PendingPayloadView::Order(order) => {
            format!(
                "swap → buy ≥ {} (min)",
                masked_amount(order.buy_amount_min, mask)
            )
        }
        PendingPayloadView::Approve {
            token,
            spender,
            risks,
            ..
        } => {
            let prefix = if risks.contains(&ApprovalRisk::UnlimitedAllowance) {
                "unlimited approve"
            } else {
                "approve"
            };
            format!(
                "{prefix} {} to spend {}",
                short_address(spender),
                short_address(token)
            )
        }
        PendingPayloadView::Message(message) => message_summary(message),
    }
}

/// One-line summary for off-chain signatures. Plain language first; details live in the review.
fn message_summary(message: &SignMessage) -> String {
    match &message.kind {
        SignMessageKind::PersonalSign { .. } => format!("sign message from {}", message.origin),
        SignMessageKind::TypedDataV4(review) => {
            format!("sign typed data: {}", review.primary_type)
        }
        SignMessageKind::EthSign { .. } => "refuse raw hash signature".to_string(),
        SignMessageKind::Authorization7702 { .. } => "refuse wallet delegation".to_string(),
    }
}

fn message_preview(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if collapsed.chars().count() > 80 {
                format!("{}…", collapsed.chars().take(80).collect::<String>())
            } else {
                collapsed
            }
        }
        Err(_) => format!("{} bytes (not UTF-8)", bytes.len()),
    }
}

fn message_risk_summary(risks: &[MessageSigningRisk]) -> String {
    risks
        .iter()
        .map(|risk| match risk {
            MessageSigningRisk::PermitLike => "permit-style allowance",
            MessageSigningRisk::UnlimitedAllowance => "unlimited allowance",
            MessageSigningRisk::LongDeadline => "long deadline",
            MessageSigningRisk::OwnershipChange => "ownership change",
            MessageSigningRisk::SeaportOrder => "marketplace order",
            MessageSigningRisk::UnknownVerifyingContract => "unknown contract",
            MessageSigningRisk::DescriptorMissing => "missing descriptor",
            MessageSigningRisk::DescriptorInvalid => "invalid descriptor",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The verb+object for a transaction intent. A shield (the demo hero) reads `shield {amount} ETH`;
/// a native send, `send {amount} ETH → {to}`; an ERC-20 send names "tokens".
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

/// The breached-fence cite for a row/card — the ACTUAL cap hit, recomputed daemon-side and
/// carried on the record. `None` for a within-cap auto-allow (no cap breached) and for a
/// mainnet-guardrail hold (held by the guardrail, not a cap).
fn cite_phrase(reason: BreachedLimit) -> Option<&'static str> {
    match reason {
        BreachedLimit::None => None,
        BreachedLimit::PerTxCap => Some("over per-tx cap"),
        BreachedLimit::DailyCap => Some("over daily cap"),
        BreachedLimit::OffAllowlist => Some("recipient not allow-listed"),
    }
}

/// The breached-fence label for the review card's key/value row (title case).
fn cite_label(reason: BreachedLimit) -> &'static str {
    match reason {
        BreachedLimit::None => "Held for your approval",
        BreachedLimit::PerTxCap => "Per-transaction cap",
        BreachedLimit::DailyCap => "Daily cap",
        BreachedLimit::OffAllowlist => "Recipient allow-list",
    }
}

/// Whether a record is still awaiting a human — the approvable subset of the feed. Only
/// `Proposed` rows are selectable / inline-approvable; everything else is a settled outcome.
fn is_proposed(record: &ActivityRecord) -> bool {
    matches!(record.lifecycle, ActivityLifecycle::Proposed)
}

/// **The approvable subset** of the feed (the NEEDS YOU inbox), in feed (newest-first) order.
/// `activity_selected` indexes into this; only these rows are selectable/approvable.
pub(crate) fn activity_pending(records: &[ActivityRecord]) -> Vec<&ActivityRecord> {
    records.iter().filter(|r| is_proposed(r)).collect()
}

/// The record id that **approve** (⌘Enter) is allowed to resolve: ONLY a still-pending,
/// actively-reviewed record. Returns `None` — meaning "open a review first, resolve NOTHING" — when
/// no review is open OR the reviewed record has left the pending set (settled/expired under a
/// background poll, or a click that raced a settle left a stale id). The no-blind-approve invariant
/// lives here, as a pure function, so it is unit-tested and a future refactor can't quietly
/// re-add a "fall back to the highlighted row" path (which would approve an unreviewed spend).
/// Deny does NOT use this — deny is one-key and may fall back to the highlighted row (it only
/// refuses, the fail-safe direction).
pub(crate) fn approve_target(
    reviewing: Option<RequestId>,
    pending: &[&ActivityRecord],
) -> Option<RequestId> {
    reviewing.filter(|id| pending.iter().any(|r| r.request_id == *id))
}

/// **The settled subset** of the feed (the LOG), in feed (newest-first) order — every row that is
/// no longer waiting on a human (auto-allowed, approved, denied, executed). A proposed row lives
/// ONLY in `activity_pending`, so the two subsets partition the feed with no duplication.
fn activity_settled(records: &[ActivityRecord]) -> Vec<&ActivityRecord> {
    records.iter().filter(|r| !is_proposed(r)).collect()
}

/// Group settled (LOG) rows into day bands (newest band first). The records arrive newest-first,
/// so consecutive rows sharing a `day_label` form one band. Pure: borrows the already-filtered
/// settled slice (the NEEDS YOU band is rendered separately and never day-grouped).
fn activity_feed_groups<'a>(
    records: &[&'a ActivityRecord],
    now: u64,
) -> Vec<(&'static str, Vec<&'a ActivityRecord>)> {
    let mut groups: Vec<(&'static str, Vec<&'a ActivityRecord>)> = Vec::new();
    for &record in records {
        let label = day_label(record.timestamp_ms, now);
        match groups.last_mut() {
            Some((existing, rows)) if *existing == label => rows.push(record),
            _ => groups.push((label, vec![record])),
        }
    }
    groups
}

impl Shell {
    /// The Activity surface: dispatch to the inline clear-signing review when one is open (and
    /// its record is still proposed in the latest snapshot), otherwise the feed.
    pub fn render_activity(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(reviewing) = self.activity_reviewing {
            if let Some(record) = self
                .activity
                .iter()
                .find(|r| r.request_id == reviewing && is_proposed(r))
            {
                return self.render_activity_review(record, cx).into_any_element();
            }
        }
        self.render_activity_feed(cx).into_any_element()
    }

    /// The feed: heading + STOP control + (loading skeleton / error / empty / day-grouped rows).
    fn render_activity_feed(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let raise = theme.secondary;
        let danger = theme.danger;
        let is_dark = theme.is_dark();
        let now = now_ms();

        let body = if self.activity_loading && self.activity.is_empty() {
            // First-load only: 3 skeleton rows (DESIGN §Required states — never a spinner).
            v_flex()
                .w_full()
                .gap_2()
                .children((0..3).map(|_| skeleton_row(raise)))
                .into_any_element()
        } else if let Some(err) = self.activity_error.as_ref() {
            crate::widgets::caution_line(
                theme::amber(is_dark),
                muted,
                false,
                format!("Can't reach the signer. {err}"),
            )
        } else {
            let pending = activity_pending(&self.activity);
            let settled = activity_settled(&self.activity);
            if pending.is_empty() && settled.is_empty() {
                // Empty: one calm muted line, no illustration (DESIGN §Required states → Empty).
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("All clear")
                    .into_any_element()
            } else {
                let selected_id = pending.get(self.activity_selected).map(|r| r.request_id);
                let mut list = v_flex().w_full().gap_5();

                // NEEDS YOU — the inbox you triage. Rendered FIRST, only the proposed rows, each
                // selectable/inline-approvable. Its row index IS the pending-subset index, so the
                // selected lift and the keyboard `activity_selected` line up.
                if !pending.is_empty() {
                    list = list.child(self.activity_needs_you(&pending, selected_id, now, cx));
                } else {
                    // Nothing waiting, but there is history: a quiet "you're caught up" line above
                    // the log (no Needs-you band).
                    list = list.child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("All clear. Nothing needs you"),
                    );
                }

                // LOG — settled rows, day-grouped, newest band first.
                for (label, rows) in activity_feed_groups(&settled, now) {
                    list = list.child(self.activity_group(label, &rows, None, now, cx));
                }
                list.into_any_element()
            }
        };

        // The heading (with the STOP control) is PINNED — it lives OUTSIDE the scroll region, so
        // the panic brake is always on screen no matter how far the log scrolls. Only the post-STOP
        // banner + the feed body scroll beneath it. (Use the Copy color locals, not `theme`, so its
        // cx borrow doesn't outlive the cx-mutable `activity_heading`/`activity_group` calls above.)
        let heading = self.activity_heading(fg, muted, cx);
        let mut scroll_body = v_flex().w_full().gap_4();
        if self.activity_stopped {
            scroll_body = scroll_body.child(stopped_banner(danger, raise));
        }
        scroll_body = scroll_body.child(body);

        div().flex_1().flex().flex_col().items_center().p_8().child(
            // Responsive width: up to 760px, but SHRINK in a narrow window so the row's right
            // cluster (the STOP control, tx hashes, times) is never clipped off the edge.
            v_flex()
                .w_full()
                .max_w(crate::tokens::CONTENT_MAX_W)
                .h_full()
                .min_h_0()
                .items_start()
                .gap_4()
                .child(heading)
                .child(
                    // The scroll lives here (one call site — gpui-component keys the
                    // ScrollHandle by call site), so the pinned heading never scrolls away.
                    div()
                        .id("scroll-activity-body")
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(scroll_body),
                ),
        )
    }

    /// The page heading row: H1 + a persistent session-scope sub-line on the left, the STOP
    /// control on the right (amber when idle, escalating to red when armed).
    fn activity_heading(
        &self,
        fg: gpui::Hsla,
        muted: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap_4()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("Activity"),
                    )
                    .child(div().text_sm().text_color(muted).child(format!(
                        "What {} and you did this session. Watch it, and stop it.",
                        self.agent_handle()
                    ))),
            )
            .child(self.activity_stop_control(cx))
    }

    /// The STOP control — the always-reachable panic brake. Amber outline "STOP" when idle (a
    /// human "where you are" action); a red outline "Confirm STOP — revoke & lock signing · Esc to
    /// cancel" once armed. **Click-to-arm** (NOT hold): two deliberate clicks (or ⌘K → "STOP") fire
    /// it — never a single click, since it zeroizes the key. Esc disarms (handled in
    /// `on_activity_key`).
    fn activity_stop_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let amber = theme::amber(theme.is_dark());
        let danger = theme.danger;

        if self.activity_stop_arming {
            // Armed: escalate from the amber idle outline to a red outline + label — a clear
            // "this is the irreversible one" signal, using only the theme's `danger` color.
            div()
                .id("activity-stop")
                .flex_shrink_0()
                .px_3()
                .py_1p5()
                .rounded(crate::tokens::RADIUS_ROW)
                .border_1()
                .border_color(danger)
                .text_color(danger)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child("Confirm STOP: revoke & lock signing · Esc to cancel")
                .on_click(cx.listener(|this, _, _, cx| this.stop_button_clicked(cx)))
        } else {
            div()
                .id("activity-stop")
                .flex_shrink_0()
                .px_3()
                .py_1p5()
                .rounded(crate::tokens::RADIUS_ROW)
                .border_1()
                .border_color(amber)
                .text_color(amber)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child("STOP")
                .on_click(cx.listener(|this, _, _, cx| this.stop_button_clicked(cx)))
        }
    }

    /// The **NEEDS YOU** band: the triage inbox of still-proposed rows over a quiet uppercase
    /// header. The row index IS the pending-subset index, so the selected lift agrees with the
    /// keyboard `activity_selected`; `selected_id` is the highlighted pending row.
    fn activity_needs_you(
        &self,
        rows: &[&ActivityRecord],
        selected_id: Option<RequestId>,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let amber = theme::amber(cx.theme().is_dark());

        let mut band = v_flex().w_full();
        for record in rows.iter() {
            band = band.child(self.activity_row("needs-you-row", record, selected_id, now, cx));
        }

        // The "Needs you" band label reads in AMBER — the human-attention signal (DESIGN §the
        // actor model: amber = your call). `section_label` only takes the muted slate, so render
        // the band label inline here with the same tiny/uppercase/medium treatment in amber.
        v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_size(crate::tokens::TEXT_LABEL)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(amber)
                    .child(gpui::SharedString::from("NEEDS YOU")),
            )
            .child(band)
    }

    /// One day-band of the settled LOG: a quiet uppercase header over its dense (passive) rows.
    /// Log rows are never proposed, so `selected_id` is always `None` here.
    fn activity_group(
        &self,
        label: &str,
        rows: &[&ActivityRecord],
        selected_id: Option<RequestId>,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;

        let mut band = v_flex().w_full();
        for record in rows.iter() {
            band = band.child(self.activity_row("log-row", record, selected_id, now, cx));
        }

        v_flex()
            .w_full()
            .gap_2()
            .child(crate::widgets::section_label(label, muted))
            .child(band)
    }

    /// One dense feed row — the two-actor chain + the outcome cluster.
    ///
    /// - **Agent, auto-allowed within cap** (no human in the loop): `[K] Kyoto · shield 0.05 ETH
    ///   · auto-approved within cap  ✓ 0x6ea…9f3c · 4m ago` (muted label, green glyph).
    /// - **Agent, executed shield** (the demo payoff): `[K] Kyoto · shield 0.05 ETH · moved
    ///   0.05 ETH to your private balance  ✓ 0x6ea…9f3c · 4m ago`.
    /// - **Agent, proposed (over cap / mainnet)** — lives in NEEDS YOU: `[K] Kyoto · … → You
    ///   waiting … over per-tx cap`, plus the `⌘⏎ · x` hint on the SELECTED row and a hover-only
    ///   "Review →" for the mouse.
    /// - **Agent, human-approved/denied** (`!auto_allowed`, settled): the outcome label tints
    ///   amber — "you acted here" — against the muted auto-allowed rows.
    /// - **App** (you acted directly): `You · sent 0.5 ETH → 0x70…9C8  ✓`.
    ///
    /// `id_ns` namespaces the row element id + hover group (NEEDS-YOU vs LOG); the id keys on the
    /// record's unique `request_id`, so rows never collide across log day-groups. A still-proposed
    /// row is selectable (a brightness lift when highlighted) and opens its inline review on click;
    /// settled rows are passive.
    fn activity_row(
        &self,
        id_ns: &'static str,
        record: &ActivityRecord,
        selected_id: Option<RequestId>,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let lift = theme.secondary;
        let hairline = theme.border;
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);
        let amber = theme::amber(is_dark);

        let is_agent = record.origin == ProposalOrigin::Agent;
        let agent_handle = self.agent_handle();
        let proposed = is_proposed(record);
        let selected = proposed && selected_id == Some(record.request_id);
        // A human is in the chain for anything that was NOT auto-allowed hands-free — an over-cap
        // card, a mainnet-guardrail hold (within-cap but still your call), an approval, a denial.
        // Keyed off the SAME `human_acted` predicate as the outcome-label tint so the two two-signal
        // cues never disagree: a STOP-revoked auto-allow (Decided{false} with auto_allowed still
        // true) is a human action, so it must BOTH show the "→ You" link and tint amber — driving
        // this off bare `!auto_allowed` would render the amber label with no human in the chain.
        let needed_human = human_acted(record);
        let summary = payload_summary(&record.payload, self.mask);

        // The lead glyph: the cyan agent mark (handle-seeded) for an agent, a neutral identity
        // square for an app action — both static.
        let lead = if is_agent {
            agent_mark(
                &agent_handle,
                crate::tokens::MARK_MD,
                crate::tokens::RADIUS_ROW,
                agent,
                agent_tint,
            )
        } else {
            div()
                .size(crate::tokens::MARK_MD)
                .rounded(crate::tokens::RADIUS_ROW)
                .bg(theme::identity_square(is_dark))
                .into_any_element()
        };

        let mut chain = h_flex()
            .items_center()
            .gap_2p5()
            .min_w_0()
            .flex_1()
            .child(lead)
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(origin_subject(record.origin, &agent_handle).to_string()),
            )
            .child(div().flex_shrink_0().text_sm().text_color(muted).child("·"))
            // The verb + object is the part that grows and clamps: it gets the flex space and
            // ellipsizes so a long summary can never push the trailing rail off the pane (the
            // historical horizontal-overflow bug).
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .text_color(fg)
                    .child(summary),
            );

        // The human only enters the chain when a human was actually needed (a card was raised).
        // An auto-allowed within-cap action has NO "→ You" — the daemon decided it hands-free.
        if is_agent && needed_human {
            let human_verb = match record.lifecycle {
                ActivityLifecycle::Proposed => "waiting",
                ActivityLifecycle::Decided { approved: true } | ActivityLifecycle::Executed => {
                    "approved"
                }
                ActivityLifecycle::Decided { approved: false } => "declined",
                // A card went to you but the window lapsed before you acted — not a decline.
                ActivityLifecycle::Expired => "expired",
            };
            chain = chain
                .child(div().flex_shrink_0().text_sm().text_color(muted).child("→"))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg)
                        .child("You"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(muted)
                        .child(human_verb),
                );
        }

        // The trailing side: the outcome cluster, plus — for a proposed row — a hover-revealed
        // "Review →" so a MOUSE user discovers the review card (the keyboard path is unchanged).
        let mut trailing = h_flex()
            .items_center()
            .gap_3()
            .flex_shrink_0()
            .child(self.activity_outcome(record, selected, now, amber, cx));
        // Key the row element id + hover group on the request_id (unique), NOT a per-band-local
        // index — a log row's index repeats across day-groups and would collide.
        let group = format!("{id_ns}-{:x}", record.request_id);
        if proposed {
            trailing = trailing.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .opacity(0.0)
                    .group_hover(group.clone(), |s| s.opacity(1.0))
                    .child("Review →"),
            );
        }

        // Editorial row: hairline-separated and dense (DESIGN §Visual language — hierarchy from
        // type + whitespace + hairlines, NOT cards). One bottom hairline per row, minimal
        // horizontal padding; the selected proposed row lifts with a subtle fill.
        let mut row = h_flex()
            .id(gpui::SharedString::from(group.clone()))
            .group(group)
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .px_1()
            .py_2p5()
            .border_b_1()
            .border_color(hairline)
            .child(chain)
            .child(trailing);

        // A still-proposed row is the live, actionable one: a click opens its inline review (the
        // queue's keyboard-first j/k/Enter also reach it). A settled row is inert by design.
        if proposed {
            let id = record.request_id;
            row = row
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.review_activity_row(id, cx)));
            if selected {
                row = row.bg(lift).rounded(crate::tokens::RADIUS_ROW);
            }
        }
        row
    }

    /// The trailing outcome cluster: for a **proposed** row, the amber breached-fence cite (the
    /// real cap, never hardcoded) + an inline "⌘⏎ · x" hint when this row is `selected`. For a
    /// **settled** row, the outcome label, the tx hash + relative time when broadcast, and the
    /// small circular status glyph.
    ///
    /// **Two-signal discipline (DESIGN §The actor model):** only the GLYPH carries color (green
    /// check approved/executed, red x denied/revoked). The label defaults to `muted_foreground` so
    /// the log never floods green — EXCEPT a row a human acted on (`!auto_allowed`, decided/
    /// executed), whose label tints amber to read "you acted here".
    fn activity_outcome(
        &self,
        record: &ActivityRecord,
        selected: bool,
        now: u64,
        amber: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let success = theme.success;
        let danger = theme.danger;

        let cluster = h_flex().items_center().gap_2p5().flex_shrink_0();

        match record.lifecycle {
            // Awaiting a human: cite the actual breached fence (amber caution); the key hint shows
            // only on the SELECTED row (the others stay quiet — the hover "Review →" guides a mouse).
            ActivityLifecycle::Proposed => {
                let cite = cite_phrase(record.reason).unwrap_or("held for approval");
                let mut c = cluster.child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(IconName::TriangleAlert).text_color(amber).small())
                        .child(div().text_xs().text_color(fg).child(cite)),
                );
                if selected {
                    c = c.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("⌘⏎ approve · x deny"),
                    );
                }
                c
            }
            // Lapsed window — NOBODY acted (the approval TTL elapsed with no human decision).
            // Honest two-signal: a muted neutral dash glyph + muted label + the time it lapsed.
            // NEVER amber (amber claims "you acted"), NEVER the red x of a human denial.
            ActivityLifecycle::Expired => cluster
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(settled_outcome_label(record, self.mask)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(relative_time(record.timestamp_ms, now)),
                )
                .child(Icon::new(IconName::Minus).text_color(muted).small()),
            // Decided / executed: the outcome label + (when broadcast) the tx hash + time + glyph.
            ActivityLifecycle::Decided { .. } | ActivityLifecycle::Executed => {
                // Executed and Decided{approved:true} are both "approved"; only an explicit
                // Decided{approved:false} (denied / revoked / lapsed) is a non-approval.
                let approved = !matches!(
                    record.lifecycle,
                    ActivityLifecycle::Decided { approved: false }
                );
                let label = settled_outcome_label(record, self.mask);
                // The glyph carries the color; the label is muted unless a human acted (amber).
                let label_color = if human_acted(record) { amber } else { muted };
                let glyph: gpui::AnyElement = if !approved {
                    Icon::new(IconName::CircleX)
                        .text_color(danger)
                        .small()
                        .into_any_element()
                } else {
                    Icon::new(IconName::CircleCheck)
                        .text_color(success)
                        .small()
                        .into_any_element()
                };
                let mut c = cluster.child(div().text_xs().text_color(label_color).child(label));
                if let Some(hash) = record.tx_hash {
                    c = c.child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .text_color(muted)
                            .child(short_tx(&hash)),
                    );
                }
                c.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(relative_time(record.timestamp_ms, now)),
                )
                .child(glyph)
            }
        }
    }

    /// The clear-signing review for a single proposed feed row — the shared trust card, with the
    /// REAL breached-fence cite from the record (per-tx vs daily), never a hardcoded
    /// "per-transaction cap". Confirm is `⌘Enter`.
    pub fn render_activity_review(
        &self,
        record: &ActivityRecord,
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
        let agent_handle = self.agent_handle();
        let subject = origin_subject(record.origin, &agent_handle);
        let cite = cite_phrase(record.reason).unwrap_or("held for your approval");

        let band = {
            let lead = if is_agent {
                agent_mark(
                    &agent_handle,
                    crate::tokens::MARK_MD,
                    crate::tokens::RADIUS_ROW,
                    agent,
                    agent_tint,
                )
            } else {
                div()
                    .size(crate::tokens::MARK_MD)
                    .rounded(crate::tokens::RADIUS_ROW)
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
                            "{subject} wants to {} · cites: {cite}",
                            payload_summary(&record.payload, self.mask)
                        )),
                )
        };

        activity_shell(
            v_flex()
                .w_full()
                .gap_4()
                .child(activity_heading_block(
                    "Review request",
                    "Confirm exactly what leaves and where, then approve or deny.",
                    fg,
                    muted,
                ))
                .child(band)
                // Danger early — but tell the truth about WHICH fence was breached.
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(Icon::new(IconName::TriangleAlert).text_color(danger).small())
                        .child(
                            div()
                                .text_sm()
                                .text_color(danger)
                                .child(review_danger_line(record.reason)),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .children(self.activity_review_detail_rows(record, cx)),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("activity-approve")
                                .primary()
                                .label("⌘Enter  Approve")
                                .on_click(cx.listener(|this, _, _, cx| this.approve_activity(cx))),
                        )
                        .child(
                            Button::new("activity-deny")
                                .ghost()
                                .label("x  Deny")
                                .on_click(cx.listener(|this, _, _, cx| this.deny_activity(cx))),
                        )
                        .child(
                            Button::new("activity-cancel")
                                .ghost()
                                .label("Esc  Cancel")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_activity_review(cx)),
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

    /// The canonical key/value rows for the review card, payload-specific, ending in the breached
    /// fence (the real cap from the record).
    fn activity_review_detail_rows(
        &self,
        record: &ActivityRecord,
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
                risks,
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
                if risks.contains(&ApprovalRisk::UnlimitedAllowance) {
                    rows.push(
                        kv_mono_row(
                            "Warning",
                            "Unlimited approval — spender can move all tokens",
                            mono.clone(),
                            fg,
                            muted,
                        )
                        .into_any_element(),
                    );
                }
            }
            PendingPayloadView::Message(message) => {
                rows.push(
                    kv_mono_row("Origin", &message.origin, mono.clone(), fg, muted)
                        .into_any_element(),
                );
                match &message.kind {
                    SignMessageKind::PersonalSign { message } => {
                        rows.push(
                            kv_mono_row("Type", "personal_sign", mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                        rows.push(
                            kv_mono_row(
                                "Message",
                                &message_preview(message),
                                mono.clone(),
                                fg,
                                muted,
                            )
                            .into_any_element(),
                        );
                    }
                    SignMessageKind::TypedDataV4(review) => {
                        rows.push(
                            kv_mono_row("Type", "eth_signTypedData_v4", mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                        rows.push(
                            kv_mono_row(
                                "Primary type",
                                &review.primary_type,
                                mono.clone(),
                                fg,
                                muted,
                            )
                            .into_any_element(),
                        );
                        if let Some(name) = review.domain_name.as_ref() {
                            rows.push(
                                kv_mono_row("Domain", name, mono.clone(), fg, muted)
                                    .into_any_element(),
                            );
                        }
                        if let Some(chain_id) = review.domain_chain_id {
                            rows.push(
                                kv_mono_row(
                                    "Domain chain",
                                    &chain_id.to_string(),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                        }
                        if let Some(contract) = review.verifying_contract.as_ref() {
                            rows.push(
                                kv_mono_row(
                                    "Contract",
                                    &short_address(contract),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                        }
                        if let Some(permit) = review.permit.as_ref() {
                            rows.push(
                                kv_mono_row(
                                    "Owner",
                                    &short_address(&permit.owner),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                            rows.push(
                                kv_mono_row(
                                    "Spender",
                                    &short_address(&permit.spender),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                            rows.push(
                                kv_money_row(
                                    "Permit value",
                                    permit.value,
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
                                    "Permit deadline",
                                    &permit.deadline.to_string(),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                        }
                        if !review.risks.is_empty() {
                            rows.push(
                                kv_mono_row(
                                    "Warnings",
                                    &message_risk_summary(&review.risks),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )
                                .into_any_element(),
                            );
                        }
                        rows.push(
                            kv_mono_row(
                                "Digest",
                                &format!("{:#x}", review.digest),
                                mono.clone(),
                                fg,
                                muted,
                            )
                            .into_any_element(),
                        );
                    }
                    SignMessageKind::EthSign { digest } => {
                        rows.push(
                            kv_mono_row("Type", "eth_sign", mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                        rows.push(
                            kv_mono_row("Digest", &format!("{digest:#x}"), mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                    }
                    SignMessageKind::Authorization7702 { delegate, nonce } => {
                        rows.push(
                            kv_mono_row("Type", "EIP-7702 authorization", mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                        rows.push(
                            kv_mono_row(
                                "Delegate",
                                &short_address(delegate),
                                mono.clone(),
                                fg,
                                muted,
                            )
                            .into_any_element(),
                        );
                        rows.push(
                            kv_mono_row("Nonce", &nonce.to_string(), mono.clone(), fg, muted)
                                .into_any_element(),
                        );
                    }
                }
            }
        }

        // The breached boundary — the real one (per-tx vs daily vs allow-list), stated once.
        rows.push(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .py_1p5()
                .child(div().text_sm().text_color(muted).child("Breached limit"))
                .child(
                    div()
                        .text_sm()
                        .text_color(fg)
                        .child(cite_label(record.reason)),
                )
                .into_any_element(),
        );
        rows
    }
}

/// Whether a HUMAN acted on this settled row — the amber "you acted here" tint (DESIGN §the actor
/// model). True for ANY human-driven negative — a denial OR a STOP revoke, both `Decided{approved:
/// false}` — EVEN when the record was auto-allowed before STOP flipped it (a STOP is a human action;
/// keying purely off `!auto_allowed` would wrongly mute a STOP-revoked auto-allow). True for any
/// non-hands-free approval/execution (`!auto_allowed`). A genuine hands-free within-cap auto-allow is
/// the agent's work → muted. (A lapsed `Expired` row renders in its own neutral arm, never here.)
fn human_acted(record: &ActivityRecord) -> bool {
    matches!(
        record.lifecycle,
        ActivityLifecycle::Decided { approved: false }
    ) || !record.auto_allowed
}

/// The settled-row outcome label, by lifecycle + whether it was auto-allowed hands-free + actor.
/// - auto-allowed within cap (hands-free) → "auto-approved within cap"
/// - human-involved (over-cap OR mainnet-guardrail hold), approved → "you approved" (agent)
/// - denied / STOP-revoked (a human acted) → "not approved"
/// - lapsed window (NObody acted) → "expired"
///
/// Keys off `auto_allowed`, NOT the breach `reason`: a mainnet-guardrail hold breaches no cap
/// (`reason == None`) yet still required a human, so it must read "you approved", not
/// "auto-approved within cap".
fn settled_label(record: &ActivityRecord) -> &'static str {
    let is_agent = record.origin == ProposalOrigin::Agent;
    match record.lifecycle {
        ActivityLifecycle::Decided { approved: false } => "not approved",
        // The approval window lapsed — nobody acted. Read it as such (not "not approved", which
        // implies a human declined it).
        ActivityLifecycle::Expired => "expired",
        ActivityLifecycle::Decided { approved: true } | ActivityLifecycle::Executed => {
            if record.auto_allowed {
                if is_agent {
                    "auto-approved within cap"
                } else {
                    "sent"
                }
            } else if is_agent {
                "you approved"
            } else {
                "sent"
            }
        }
        // Proposed never reaches here (handled before the call site).
        ActivityLifecycle::Proposed => "waiting on you",
    }
}

/// The shielded amount for an EXECUTED shield, if this record is exactly that — an executed
/// `Tx` with `IntentKind::Shield`. The demo's payoff lives here: the row should read the RESULT
/// ("moved … to your private balance"), not just "approved".
fn executed_shield_amount(record: &ActivityRecord) -> Option<deckard_core::U256> {
    if !matches!(record.lifecycle, ActivityLifecycle::Executed) {
        return None;
    }
    match &record.payload {
        PendingPayloadView::Tx(intent) if intent.kind == IntentKind::Shield => Some(intent.value),
        _ => None,
    }
}

/// The settled-row outcome label as a display `String`. An executed shield reads its RESULT —
/// "moved {amount} ETH to your private balance" (honest + plain, mask-aware). Every other settled
/// action keeps the static [`settled_label`] wording (auto-approved / you approved / sent / not
/// approved).
fn settled_outcome_label(record: &ActivityRecord, mask: bool) -> String {
    if let Some(value) = executed_shield_amount(record) {
        return format!(
            "moved {} ETH to your private balance",
            masked_amount(value, mask)
        );
    }
    settled_label(record).to_string()
}

/// The danger line at the top of the review card — names the actual fence breached.
fn review_danger_line(reason: BreachedLimit) -> &'static str {
    match reason {
        BreachedLimit::PerTxCap => "This exceeds the per-transaction limit.",
        BreachedLimit::DailyCap => "This exceeds today's daily limit.",
        BreachedLimit::OffAllowlist => "This recipient is not on the allow-list.",
        BreachedLimit::None => "This is held for your approval.",
    }
}

/// The post-STOP banner: the key is zeroized and in-flight work denied; the feed stays visible so
/// the revoke is seen. Unlock to re-arm.
fn stopped_banner(danger: gpui::Hsla, surface: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_3()
        .py_2p5()
        .rounded_lg()
        .bg(surface)
        .child(Icon::new(IconName::CircleX).text_color(danger).small())
        .child(
            div()
                .text_sm()
                .text_color(danger)
                .child("Stopped. Key zeroized and in-flight work denied. Unlock to re-arm."),
        )
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

/// One loading skeleton row (DESIGN §Required states — never a spinner).
fn skeleton_row(raise: gpui::Hsla) -> impl IntoElement {
    div()
        .w_full()
        .h(px(40.0))
        .rounded(crate::tokens::RADIUS_ROW)
        .bg(raise)
}

/// The review card's heading block (H1 + muted subtitle).
fn activity_heading_block(
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

/// The shared column shell for the Activity surface — the same 760px dense-list column the
/// Approvals queue uses, so the two sibling surfaces frame identically.
fn activity_shell(inner: gpui::AnyElement) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .p_8()
        // Responsive: up to 760px, shrinks in a narrow window (no right-edge clipping).
        .child(
            v_flex()
                .w_full()
                .max_w(crate::tokens::CONTENT_MAX_W)
                .items_start()
                .child(inner),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U256};

    fn record(
        id: u8,
        lifecycle: ActivityLifecycle,
        reason: BreachedLimit,
        auto_allowed: bool,
    ) -> ActivityRecord {
        ActivityRecord {
            request_id: B256::repeat_byte(id),
            origin: ProposalOrigin::Agent,
            payload: PendingPayloadView::Tx(Intent {
                chain_id: 1,
                to: Address::repeat_byte(0x22),
                token: None,
                value: U256::from(1_u64),
                calldata: Bytes::new(),
                kind: IntentKind::Shield,
            }),
            timestamp_ms: 1_700_000_000_000,
            tx_hash: None,
            lifecycle,
            reason,
            auto_allowed,
        }
    }

    #[test]
    fn activity_pending_keeps_only_proposed() {
        let records = vec![
            record(
                0x01,
                ActivityLifecycle::Proposed,
                BreachedLimit::PerTxCap,
                false,
            ),
            record(0x02, ActivityLifecycle::Executed, BreachedLimit::None, true),
            record(
                0x03,
                ActivityLifecycle::Decided { approved: true },
                BreachedLimit::None,
                true,
            ),
            record(
                0x04,
                ActivityLifecycle::Proposed,
                BreachedLimit::DailyCap,
                false,
            ),
        ];
        let pending = activity_pending(&records);
        assert_eq!(pending.len(), 2, "only proposed rows are approvable");
        assert!(pending.iter().all(|r| is_proposed(r)));
    }

    #[test]
    fn cite_reflects_the_actual_cap() {
        // The whole point of #60 acceptance 5: never a hardcoded cite.
        assert_eq!(
            cite_phrase(BreachedLimit::PerTxCap),
            Some("over per-tx cap")
        );
        assert_eq!(cite_phrase(BreachedLimit::DailyCap), Some("over daily cap"));
        assert_eq!(cite_phrase(BreachedLimit::None), None);
        assert_eq!(cite_label(BreachedLimit::DailyCap), "Daily cap");
        assert_eq!(cite_label(BreachedLimit::PerTxCap), "Per-transaction cap");
    }

    #[test]
    fn settled_label_distinguishes_auto_from_human_approval() {
        // Hands-free auto-allow → "auto-approved within cap".
        let auto = record(0x01, ActivityLifecycle::Executed, BreachedLimit::None, true);
        assert_eq!(settled_label(&auto), "auto-approved within cap");
        // Over-cap, human-approved → "you approved".
        let human = record(
            0x02,
            ActivityLifecycle::Executed,
            BreachedLimit::PerTxCap,
            false,
        );
        assert_eq!(settled_label(&human), "you approved");
        // The codex #2 case: a MAINNET-GUARDRAIL hold breaches NO cap (reason None) but still
        // required a human, so it must read "you approved", NOT "auto-approved within cap".
        let guardrail = record(
            0x03,
            ActivityLifecycle::Executed,
            BreachedLimit::None,
            false,
        );
        assert_eq!(
            settled_label(&guardrail),
            "you approved",
            "a within-cap mainnet hold that a human approved must never read as hands-free"
        );
        let denied = record(
            0x04,
            ActivityLifecycle::Decided { approved: false },
            BreachedLimit::PerTxCap,
            false,
        );
        assert_eq!(settled_label(&denied), "not approved");
        // Amber-honesty (codex MEDIUM): a LAPSED window had NO human action, so it must read
        // "expired", NOT "not approved" (which would imply a human declined it). It is its own
        // lifecycle so the renderer can tint it neutral instead of the amber "you acted".
        let lapsed = record(
            0x05,
            ActivityLifecycle::Expired,
            BreachedLimit::PerTxCap,
            false,
        );
        assert_eq!(settled_label(&lapsed), "expired");
        assert_ne!(
            settled_label(&lapsed),
            settled_label(&denied),
            "a lapsed window must not read identically to a human denial"
        );
    }

    #[test]
    fn approval_and_message_risk_summaries_are_plain_language() {
        let approve = PendingPayloadView::Approve {
            token: Address::repeat_byte(0xA1),
            spender: Address::repeat_byte(0xC9),
            amount: U256::MAX,
            risks: vec![ApprovalRisk::UnlimitedAllowance],
        };
        assert!(payload_summary(&approve, false).starts_with("unlimited approve"));
        assert_eq!(
            message_risk_summary(&[
                MessageSigningRisk::PermitLike,
                MessageSigningRisk::UnlimitedAllowance,
                MessageSigningRisk::LongDeadline,
            ]),
            "permit-style allowance, unlimited allowance, long deadline"
        );
    }

    #[test]
    fn approve_target_never_falls_back_to_the_highlighted_row() {
        // The no-blind-approve guarantee: approve may resolve ONLY a still-pending reviewed record.
        let a = record(
            0x0A,
            ActivityLifecycle::Proposed,
            BreachedLimit::PerTxCap,
            false,
        );
        let b = record(
            0x0B,
            ActivityLifecycle::Proposed,
            BreachedLimit::PerTxCap,
            false,
        );
        let both: Vec<&ActivityRecord> = vec![&a, &b];

        // Reviewing a still-pending row → resolve exactly it.
        assert_eq!(
            approve_target(Some(a.request_id), &both),
            Some(a.request_id)
        );

        // Reviewing a row that has LEFT the pending set (settled/expired, or a stale click) → None.
        // It must NOT fall back to the other highlighted pending row B — that would blind-approve a
        // spend whose clear-signing card was never shown.
        let only_b: Vec<&ActivityRecord> = vec![&b];
        assert_eq!(
            approve_target(Some(a.request_id), &only_b),
            None,
            "approve must never resolve a row other than the one under review"
        );

        // No review open → None (⌘Enter opens a review instead of resolving anything).
        assert_eq!(approve_target(None, &both), None);
    }

    #[test]
    fn human_acted_tints_amber_for_every_human_action() {
        // Hands-free within-cap auto-allow → the agent's work → NOT human-acted (muted).
        let auto = record(0x01, ActivityLifecycle::Executed, BreachedLimit::None, true);
        assert!(
            !human_acted(&auto),
            "a hands-free auto-allow is the agent's work, not amber"
        );
        // Over-cap human approval (!auto_allowed) → a human acted → amber.
        let approved = record(
            0x02,
            ActivityLifecycle::Executed,
            BreachedLimit::PerTxCap,
            false,
        );
        assert!(human_acted(&approved));
        // A human denial → a human acted → amber.
        let denied = record(
            0x03,
            ActivityLifecycle::Decided { approved: false },
            BreachedLimit::PerTxCap,
            false,
        );
        assert!(human_acted(&denied));
        // The codex #6 case: a STOP revoke of a row that had AUTO-ALLOWED — lock() flips it to
        // Decided{approved:false} but leaves `auto_allowed == true`. A STOP is a human action, so
        // it MUST still be human-acted (amber). Keying purely off `!auto_allowed` would mute it.
        let stop_revoked_auto = record(
            0x04,
            ActivityLifecycle::Decided { approved: false },
            BreachedLimit::None,
            true,
        );
        assert!(
            human_acted(&stop_revoked_auto),
            "a STOP revoke of an auto-allowed row is a human action and must tint amber"
        );
    }

    #[test]
    fn feed_groups_are_newest_first_under_one_session_band() {
        let now = 1_700_000_050_000;
        // The LOG is built from the SETTLED subset only — `activity_settled` excludes the proposed
        // row (which would live in the NEEDS YOU band), so the log groups just the two executed.
        let records = vec![
            record(0x03, ActivityLifecycle::Executed, BreachedLimit::None, true),
            record(0x02, ActivityLifecycle::Executed, BreachedLimit::None, true),
            record(
                0x01,
                ActivityLifecycle::Proposed,
                BreachedLimit::PerTxCap,
                false,
            ),
        ];
        let settled = activity_settled(&records);
        assert_eq!(settled.len(), 2, "the proposed row is not in the log");
        let groups = activity_feed_groups(&settled, now);
        assert_eq!(groups.len(), 1, "one session band (all today)");
        let (label, rows) = groups.first().expect("a group");
        assert_eq!(*label, "Today");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].request_id, B256::repeat_byte(0x03), "newest first");
    }

    #[test]
    fn pending_and_settled_partition_the_feed() {
        // A proposed row appears ONLY in NEEDS YOU (pending); never duplicated into the LOG.
        let records = vec![
            record(
                0x01,
                ActivityLifecycle::Proposed,
                BreachedLimit::PerTxCap,
                false,
            ),
            record(0x02, ActivityLifecycle::Executed, BreachedLimit::None, true),
            record(
                0x03,
                ActivityLifecycle::Decided { approved: false },
                BreachedLimit::PerTxCap,
                false,
            ),
        ];
        let pending = activity_pending(&records);
        let settled = activity_settled(&records);
        assert_eq!(pending.len(), 1, "only the proposed row needs you");
        assert_eq!(settled.len(), 2, "the rest fall to the log");
        // No request_id appears in both subsets.
        for p in &pending {
            assert!(
                !settled.iter().any(|s| s.request_id == p.request_id),
                "a pending row must never duplicate into the log"
            );
        }
    }

    #[test]
    fn relative_time_reads_naturally() {
        let now = 1_700_000_000_000;
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - 30_000, now), "30s ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5m ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3h ago");
    }

    #[test]
    fn empty_feed_has_no_groups() {
        let empty: [&ActivityRecord; 0] = [];
        assert!(activity_feed_groups(&empty, 1_700_000_000_000).is_empty());
    }

    #[test]
    fn human_acted_rows_are_distinguished_from_auto_allowed() {
        // The two-signal fidelity hinge: a row a human acted on (`auto_allowed == false`, settled)
        // is "human-acted" — its outcome label tints amber. An auto-allowed agent row is not.
        let auto = record(0x01, ActivityLifecycle::Executed, BreachedLimit::None, true);
        assert!(
            auto.auto_allowed,
            "an auto-allowed within-cap action is hands-free"
        );

        // Human-APPROVED (auto_allowed == false): a human acted, so it must read as human-acted.
        let human_approved = record(
            0x02,
            ActivityLifecycle::Executed,
            BreachedLimit::PerTxCap,
            false,
        );
        assert!(
            !human_approved.auto_allowed,
            "an over-cap row a human approved is human-acted (label tints amber, not muted)"
        );

        // A denial is also human-acted.
        let denied = record(
            0x03,
            ActivityLifecycle::Decided { approved: false },
            BreachedLimit::PerTxCap,
            false,
        );
        assert!(!denied.auto_allowed, "a denial is a human action");
    }

    #[test]
    fn executed_shield_reads_the_private_balance_payoff() {
        // An EXECUTED shield reads the RESULT — the demo's payoff — not just "approved".
        let mut shield = record(0x01, ActivityLifecycle::Executed, BreachedLimit::None, true);
        // 0.05 ETH, so the rendered amount is meaningful (the helper's 1-wei default is dust).
        shield.payload = PendingPayloadView::Tx(Intent {
            chain_id: 1,
            to: Address::repeat_byte(0x22),
            token: None,
            value: U256::from(50_000_000_000_000_000u64),
            calldata: Bytes::new(),
            kind: IntentKind::Shield,
        });
        assert_eq!(
            settled_outcome_label(&shield, false),
            "moved 0.05 ETH to your private balance",
            "an executed shield surfaces the private-balance payoff"
        );
        // Mask-aware: a masked shield hides the amount with the fixed bullets.
        assert_eq!(
            settled_outcome_label(&shield, true),
            format!(
                "moved {} ETH to your private balance",
                crate::money::MASK_BULLETS
            ),
        );

        // A NON-shield executed action keeps the existing wording (no payoff hijack). A native
        // send the agent ran hands-free reads "auto-approved within cap".
        let mut send = record(0x02, ActivityLifecycle::Executed, BreachedLimit::None, true);
        send.payload = PendingPayloadView::Tx(Intent {
            chain_id: 1,
            to: Address::repeat_byte(0x33),
            token: None,
            value: U256::from(5_u64),
            calldata: Bytes::new(),
            kind: IntentKind::Send,
        });
        assert_eq!(
            settled_outcome_label(&send, false),
            "auto-approved within cap",
            "a non-shield executed action keeps its plain settled label"
        );

        // A PROPOSED shield is not executed, so it never reads the payoff.
        let proposed_shield = record(
            0x03,
            ActivityLifecycle::Proposed,
            BreachedLimit::PerTxCap,
            false,
        );
        assert!(executed_shield_amount(&proposed_shield).is_none());
    }
}
