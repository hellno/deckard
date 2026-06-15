//! Activity — the **see-and-stop feed** (#60): what the agent + you *did*, not only what is
//! pending (DESIGN §Components → Activity row, §Trust & safety affordances).
//!
//! Approvals is the focused triage queue (pending-only). Activity is the ledger: it reads the
//! daemon's `ActivityFeed`, so it ALSO shows auto-allowed and executed actions that never wait
//! in `PendingList` — the load-bearing point of #60 ("an auto-allowed within-cap shield executes
//! immediately and never enters the queue, so the daemon must record what the agent *did*").
//!
//! Each row carries the two-actor chain (the cyan agent squircle for Atlas, neutral for a
//! foreground app action), a lifecycle-driven outcome glyph, the real broadcast `tx_hash`, a
//! relative timestamp, and — for an over-cap/over-scope proposal — the ACTUAL breached fence
//! (per-tx vs daily, never a hardcoded cite). Proposed rows are inline-approvable: select +
//! ⌘Enter opens the clear-signing review, ⌘Enter approves, `x` denies (the same gestures as the
//! Approvals queue, scoped to this surface's `key_context`). A header STOP control is the
//! always-reachable panic brake.
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
    h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use deckard_contract::{
    ActivityLifecycle, ActivityRecord, BreachedLimit, Intent, IntentKind, PendingPayloadView,
    ProposalOrigin, RequestId,
};

use crate::money::money;
use crate::shell::Shell;
use crate::shell_chrome::agent_squircle;
use crate::theme;

/// The displayed subject for an action's origin: the agent's name when an agent acted, "You"
/// when the foreground app did. One agent in the demo scope (Atlas). Mirrors `approvals_view`'s
/// private copy (the two surfaces stay decoupled by design); keep them in lockstep.
fn origin_subject(origin: ProposalOrigin) -> &'static str {
    match origin {
        ProposalOrigin::Agent => "Atlas",
        ProposalOrigin::App => "You",
    }
}

/// Middle-truncate a long `0x…` string for a tight row (first 10 + last 6).
fn short_mid(s: &str) -> String {
    if s.len() >= 16 {
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

/// A short, EIP-55-checksummed, middle-truncated address for the row + card.
fn short_address(addr: &deckard_core::Address) -> String {
    short_mid(&addr.to_checksum(None))
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

/// A one-line "verb + object" summary of an action's payload, for the dense row. Mirrors
/// `approvals_view::payload_summary` so the two surfaces read as siblings.
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

/// **The approvable subset** of the feed, in feed (newest-first) order. `activity_selected`
/// indexes into this; the feed renders every row but only these are selectable/approvable.
pub(crate) fn activity_pending(records: &[ActivityRecord]) -> Vec<&ActivityRecord> {
    records.iter().filter(|r| is_proposed(r)).collect()
}

/// Group the feed's rows into day bands (newest band first). The records arrive newest-first, so
/// consecutive rows sharing a `day_label` form one band. Pure: borrows the slice.
fn activity_feed_groups<'a>(
    records: &'a [ActivityRecord],
    now: u64,
) -> Vec<(&'static str, Vec<&'a ActivityRecord>)> {
    let mut groups: Vec<(&'static str, Vec<&ActivityRecord>)> = Vec::new();
    for record in records {
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
        let now = now_ms();

        let body = if self.activity_loading && self.activity.is_empty() {
            // First-load only: 3 skeleton rows (DESIGN §Required states — never a spinner).
            v_flex()
                .w_full()
                .gap_2()
                .children((0..3).map(|_| skeleton_row(raise)))
                .into_any_element()
        } else if let Some(err) = self.activity_error.as_ref() {
            div()
                .text_sm()
                .text_color(danger)
                .child(format!("⚠ Can't reach the signer — {err}"))
                .into_any_element()
        } else {
            let groups = activity_feed_groups(&self.activity, now);
            if groups.is_empty() {
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("No activity yet")
                    .into_any_element()
            } else {
                let selected_id = activity_pending(&self.activity)
                    .get(self.activity_selected)
                    .map(|r| r.request_id);
                let mut list = v_flex().w_full().gap_4();
                for (label, rows) in groups {
                    list = list.child(self.activity_group(label, &rows, selected_id, now, cx));
                }
                list.into_any_element()
            }
        };

        let mut header = v_flex()
            .w_full()
            .gap_4()
            .child(self.activity_heading(fg, muted, cx));
        // The post-STOP banner: the key is zeroized; the feed stays visible so the revoke is seen.
        if self.activity_stopped {
            header = header.child(stopped_banner(theme.danger, theme.secondary));
        }
        header = header.child(body);

        activity_shell(header.into_any_element())
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
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("What Atlas and you did this session — watch it, and stop it."),
                    ),
            )
            .child(self.activity_stop_control(cx))
    }

    /// The STOP control — the always-reachable panic brake. Amber outline "STOP" when idle (a
    /// human "where you are" action); a red filled "Confirm STOP" once armed. Two deliberate
    /// clicks (or ⌘K → "STOP") fire it — never a single click, since it zeroizes the key.
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
                .rounded(px(6.0))
                .border_1()
                .border_color(danger)
                .text_color(danger)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child("Confirm STOP — locks the wallet")
                .on_click(cx.listener(|this, _, _, cx| this.stop_button_clicked(cx)))
        } else {
            div()
                .id("activity-stop")
                .flex_shrink_0()
                .px_3()
                .py_1p5()
                .rounded(px(6.0))
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

    /// One day-band: a quiet uppercase header over its dense rows.
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

        let mut band = v_flex().w_full().gap_1();
        for (i, record) in rows.iter().enumerate() {
            band = band.child(self.activity_row(i, record, selected_id, now, cx));
        }

        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted)
                    .child(label.to_uppercase()),
            )
            .child(band)
    }

    /// One dense feed row — the two-actor chain + the outcome cluster.
    ///
    /// - **Agent, auto-allowed within cap** (no human in the loop): `[A] Atlas · shield 0.05 ETH
    ///   · auto-approved within cap  ✓ 0x6ea…9f3c · 4m ago`.
    /// - **Agent, needed a card** (over cap / mainnet): `[A] Atlas · … → You {approved|waiting}`,
    ///   with the real breached-fence cite for a still-proposed row.
    /// - **App** (you acted directly): `You · sent 0.5 ETH → 0x70…9C8  ✓`.
    ///
    /// A still-proposed row is selectable (a brightness lift when highlighted) and opens its
    /// inline review on click; settled rows are passive.
    fn activity_row(
        &self,
        index: usize,
        record: &ActivityRecord,
        selected_id: Option<RequestId>,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let lift = theme.secondary;
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);
        let amber = theme::amber(is_dark);

        let is_agent = record.origin == ProposalOrigin::Agent;
        let proposed = is_proposed(record);
        let selected = proposed && selected_id == Some(record.request_id);
        let needed_human = record.reason != BreachedLimit::None;
        let summary = payload_summary(&record.payload, self.mask);

        // The lead glyph: the cyan agent squircle for an agent, a neutral identity square for an
        // app action. Static (never the acting pulse) — a logged row is not mid-action.
        let lead = if is_agent {
            agent_squircle(
                px(20.0),
                px(6.0),
                false,
                agent,
                agent_tint,
                "activity-agent",
            )
        } else {
            div()
                .size(px(20.0))
                .rounded(px(6.0))
                .bg(theme::identity_square(is_dark))
                .into_any_element()
        };

        let mut chain = h_flex()
            .items_center()
            .gap_2p5()
            .min_w_0()
            .child(lead)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg)
                    .child(origin_subject(record.origin).to_string()),
            )
            .child(div().text_sm().text_color(muted).child("·"))
            .child(div().min_w_0().text_sm().text_color(fg).child(summary));

        // The human only enters the chain when a human was actually needed (a card was raised).
        // An auto-allowed within-cap action has NO "→ You" — the daemon decided it hands-free.
        if is_agent && needed_human {
            let human_verb = match record.lifecycle {
                ActivityLifecycle::Proposed => "waiting",
                ActivityLifecycle::Decided { approved: true } | ActivityLifecycle::Executed => {
                    "approved"
                }
                ActivityLifecycle::Decided { approved: false } => "stopped it",
            };
            chain = chain
                .child(div().text_sm().text_color(muted).child("→"))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg)
                        .child("You"),
                )
                .child(div().text_sm().text_color(muted).child(human_verb));
        }

        let mut row = h_flex()
            .id(("activity-row", index))
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(6.0))
            .child(chain)
            .child(self.activity_outcome(record, now, amber, cx));

        // A still-proposed row is the live, actionable one: a click opens its inline review (the
        // queue's keyboard-first j/k/Enter also reach it). A settled row is inert by design.
        if proposed {
            let id = record.request_id;
            row = row
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.review_activity_row(id, cx)));
            if selected {
                row = row.bg(lift);
            }
        }
        row
    }

    /// The trailing outcome cluster: for a **proposed** row, the amber breached-fence cite (the
    /// real cap, never hardcoded) + an inline "⌘⏎ · x" hint when selected. For a **settled** row,
    /// the auto-approved/approved/sent/not-approved label, the tx hash + relative time when
    /// broadcast, and the status glyph.
    fn activity_outcome(
        &self,
        record: &ActivityRecord,
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
            // Awaiting a human: cite the actual breached fence (amber caution), plus the key hint.
            ActivityLifecycle::Proposed => {
                let cite = cite_phrase(record.reason).unwrap_or("held for approval");
                cluster
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(IconName::TriangleAlert).text_color(amber).small())
                            .child(div().text_xs().text_color(fg).child(cite)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("⌘⏎ approve · x deny"),
                    )
            }
            // Decided / executed: the outcome label + (when broadcast) the tx hash + time + glyph.
            ActivityLifecycle::Decided { approved } | ActivityLifecycle::Executed => {
                let executed = matches!(record.lifecycle, ActivityLifecycle::Executed);
                let label = settled_label(record);
                let (glyph, color): (gpui::AnyElement, gpui::Hsla) = if !approved {
                    (
                        Icon::new(IconName::CircleX)
                            .text_color(danger)
                            .small()
                            .into_any_element(),
                        danger,
                    )
                } else {
                    (
                        Icon::new(IconName::CircleCheck)
                            .text_color(success)
                            .small()
                            .into_any_element(),
                        success,
                    )
                };
                let mut c = cluster.child(div().text_xs().text_color(color).child(label));
                if executed {
                    if let Some(hash) = record.tx_hash {
                        c = c.child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .text_color(muted)
                                .child(short_tx(&hash)),
                        );
                    }
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

    /// The clear-signing review for a single proposed feed row — the shared trust card, mirroring
    /// `approvals_view::render_approvals_review`, but with the REAL breached-fence cite from the
    /// record (per-tx vs daily), never a hardcoded "per-transaction cap". Confirm is `⌘Enter`.
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
        let subject = origin_subject(record.origin);
        let cite = cite_phrase(record.reason).unwrap_or("held for your approval");

        let band = {
            let lead = if is_agent {
                agent_squircle(
                    px(24.0),
                    px(7.0),
                    false,
                    agent,
                    agent_tint,
                    "activity-review-agent",
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
                        .children(self.review_detail_rows(record, cx)),
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
    /// fence (the real cap from the record). Mirrors `approvals_view::review_detail_rows`.
    fn review_detail_rows(
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

/// The settled-row outcome label, by lifecycle + whether a human was needed + actor.
/// - auto-allowed within cap (no breach) → "auto-approved within cap"
/// - over-cap, human-approved → "you approved" (agent) / "sent" (app)
/// - denied / revoked / lapsed → "not approved"
fn settled_label(record: &ActivityRecord) -> &'static str {
    let is_agent = record.origin == ProposalOrigin::Agent;
    match record.lifecycle {
        ActivityLifecycle::Decided { approved: false } => "not approved",
        ActivityLifecycle::Decided { approved: true } | ActivityLifecycle::Executed => {
            if record.reason == BreachedLimit::None {
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
                .child("Stopped — key zeroized and in-flight work denied. Unlock to re-arm."),
        )
}

/// A key/value row whose value is a mono-for-money amount (DESIGN §Typography), mask-aware.
/// Duplicated from `approvals_view` per the two surfaces' deliberate decoupling.
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
    div().w_full().h(px(40.0)).rounded(px(6.0)).bg(raise)
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
        .child(v_flex().w(px(760.0)).items_start().child(inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, Bytes, B256, U256};

    fn record(id: u8, lifecycle: ActivityLifecycle, reason: BreachedLimit) -> ActivityRecord {
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
        }
    }

    #[test]
    fn activity_pending_keeps_only_proposed() {
        let records = vec![
            record(0x01, ActivityLifecycle::Proposed, BreachedLimit::PerTxCap),
            record(0x02, ActivityLifecycle::Executed, BreachedLimit::None),
            record(
                0x03,
                ActivityLifecycle::Decided { approved: true },
                BreachedLimit::None,
            ),
            record(0x04, ActivityLifecycle::Proposed, BreachedLimit::DailyCap),
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
        // Within cap (no breach) → auto-approved; over cap (breach) → you approved.
        let auto = record(0x01, ActivityLifecycle::Executed, BreachedLimit::None);
        assert_eq!(settled_label(&auto), "auto-approved within cap");
        let human = record(0x02, ActivityLifecycle::Executed, BreachedLimit::PerTxCap);
        assert_eq!(settled_label(&human), "you approved");
        let denied = record(
            0x03,
            ActivityLifecycle::Decided { approved: false },
            BreachedLimit::PerTxCap,
        );
        assert_eq!(settled_label(&denied), "not approved");
    }

    #[test]
    fn feed_groups_are_newest_first_under_one_session_band() {
        let now = 1_700_000_050_000;
        let records = vec![
            record(0x02, ActivityLifecycle::Executed, BreachedLimit::None),
            record(0x01, ActivityLifecycle::Proposed, BreachedLimit::PerTxCap),
        ];
        let groups = activity_feed_groups(&records, now);
        assert_eq!(groups.len(), 1, "one session band (all today)");
        let (label, rows) = groups.first().expect("a group");
        assert_eq!(*label, "Today");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].request_id, B256::repeat_byte(0x02), "newest first");
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
        assert!(activity_feed_groups(&[], 1_700_000_000_000).is_empty());
    }
}
