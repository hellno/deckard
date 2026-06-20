//! Agent surface — the first-class view for an agent (DESIGN.md v2 §The agent
//! interaction model). Selected from the sidebar Agents group. Rendered entirely
//! from policy data + the agent's activity slice (the expandability contract: an
//! agent IS its policy + its activity; new capabilities are new fields, not a
//! redesign). The wallet home now shows only a compact agent presence that links
//! here; the editable policy + controls live on this surface, not in the home.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Disableable,
};

use crate::shell::Shell;
use crate::theme;
use crate::welcome::{agent_policy_rows, fraction};
use crate::widgets::{budget_gauge, section_label};

impl Shell {
    /// The agent surface for the selected agent (currently the one agent, Atlas):
    /// identity + a plain-language autonomy statement + the limits/scope + a budget
    /// gauge + controls (Pause / Rotate / Adjust / Revoke and STOP) + what this
    /// agent did. Built from `self.agent_policy` (the live daemon fence).
    pub fn render_agent_surface(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let secondary_text = theme.secondary_foreground;
        let border = theme.border;
        let track = theme.secondary; // bg.raise — the calm gauge track tone
        let danger = theme.danger;
        let mono = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);
        let amber = theme::amber(is_dark);

        // Derive the per-tx cap (ETH) for the plain-language autonomy line — never
        // invented; the same number the fence shows. `None` until the first fetch.
        let per_tx_eth = self
            .agent_policy
            .as_ref()
            .map(|p| deckard_core::format_amount(p.per_tx_cap_wei, 18, 6));

        // The STOP brake reuses the feed's deliberate two-step arming so the
        // irreversible key-zeroize is never a single click: a first click arms it
        // (`Confirm STOP`), the second fires `stop_revoke_all`. Esc disarms.
        let stop_armed = self.activity_stop_arming;
        let stop_label = if stop_armed {
            "Confirm STOP: revoke & lock"
        } else {
            "Revoke & STOP"
        };

        v_flex()
            .size_full()
            .p_8()
            .gap_4()
            // 1. HEADER — agent squircle + name + acting status, STOP on the right.
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(crate::shell_chrome::agent_squircle(
                        px(34.0),
                        px(9.0),
                        agent,
                        agent_tint,
                    ))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("Atlas"),
                    )
                    // A small "acting" status — cyan, the agent actor signal.
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(div().size(px(6.0)).rounded_full().bg(agent))
                            .child(div().text_xs().text_color(agent).child("acting")),
                    )
                    // Revoke & STOP — the danger brake, pushed right. Mirrors the
                    // feed's `activity_stop_control` div idiom (theme `danger`, no
                    // gpui Button danger variant is used elsewhere in the crate).
                    .child(
                        div()
                            .id("agent-stop")
                            .ml_auto()
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
                            .child(stop_label)
                            .on_click(cx.listener(|this, _, _, cx| this.stop_button_clicked(cx))),
                    ),
            )
            // 2. SUBTITLE — what the agent is for, in plain words.
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("Keeps your incoming funds private. Signs from your wallet, key-less."),
            )
            // The rest of the surface is built entirely from the live policy. Until
            // the first `PolicyGet` lands the surface says so (never invented caps).
            .map(
                |col| match (self.agent_policy.as_ref(), per_tx_eth.as_ref()) {
                    (Some(p), Some(cap)) => {
                        // 3. AUTONOMY STATEMENT — one plain-language paragraph deriving the
                        // per-tx cap from the live policy. A single text `div` so it
                        // word-wraps natively inside the 560px measure; the cap is inlined
                        // as plain text (an `h_flex` of fragments would not re-flow per word).
                        let frac = fraction(p.spent_today_wei, p.daily_cap_wei);
                        let pct = (frac.clamp(0.0, 1.0) * 100.0).round() as u32;
                        let daily_eth = deckard_core::format_amount(p.daily_cap_wei, 18, 6);
                        let spent_eth = deckard_core::format_amount(p.spent_today_wei, 18, 6);
                        let mono_rows = mono.clone();
                        let autonomy = format!(
                            "Atlas acts on its own under {cap} ETH per move and asks you above \
                         that. It can shield ETH only. It never holds your key, and it \
                         cannot send to a new address."
                        );

                        col.child(
                            div()
                                .max_w(px(560.0))
                                .text_size(px(15.0))
                                .text_color(secondary_text)
                                .child(autonomy),
                        )
                        // 4. LIMITS — section label, the live policy rows, then the gauge.
                        .child(
                            v_flex()
                                .w_full()
                                .max_w(px(600.0))
                                .gap_4()
                                .mt_4()
                                .pt_5()
                                .border_t_1()
                                .border_color(border)
                                .child(section_label("Limits", muted))
                                // Policy rows: label left (muted) / mono value right.
                                // Mirrors welcome.rs `render_agent_fence`'s `policy_row`.
                                .child(v_flex().w_full().gap_0().children(
                                    agent_policy_rows(p, self.mask).into_iter().map(
                                        |(label, value)| {
                                            h_flex()
                                                .w_full()
                                                .justify_between()
                                                .items_center()
                                                .py_1p5()
                                                .child(
                                                    div().text_sm().text_color(muted).child(label),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(mono_rows.clone())
                                                        .text_sm()
                                                        .text_color(fg)
                                                        .child(value),
                                                )
                                        },
                                    ),
                                ))
                                // The Spent-today / Daily-budget gauge — color escalates
                                // with pressure (cyan → amber ≥90% → red ≥100%).
                                .child(budget_gauge(
                                    frac,
                                    agent,
                                    amber,
                                    danger,
                                    track,
                                    muted,
                                    format!("Spent today {spent_eth} ETH"),
                                    format!("{pct}% of {daily_eth}"),
                                )),
                        )
                        // 5. CONTROLS — ghost actions. Only STOP (the header brake) has an
                        // obvious existing handler; Pause / Rotate key / Adjust limits are
                        // presented but not yet wired (see uncertainties).
                        .child(
                            h_flex()
                                .max_w(px(600.0))
                                .gap_2()
                                .mt_2()
                                // Not yet wired to the daemon — shown disabled so the control
                                // set reads true without a misleading no-op. Wiring (in-app
                                // limit edit / key rotation / pause) + their ⌘K Commands is a
                                // follow-up (DESIGN.md §The agent interaction model).
                                .child(
                                    Button::new("agent-pause")
                                        .ghost()
                                        .label("Pause")
                                        .disabled(true),
                                )
                                .child(
                                    Button::new("agent-rotate")
                                        .ghost()
                                        .label("Rotate key")
                                        .disabled(true),
                                )
                                .child(
                                    Button::new("agent-adjust")
                                        .ghost()
                                        .label("Adjust limits")
                                        .disabled(true),
                                )
                                .child(
                                    Button::new("agent-revoke")
                                        .danger()
                                        .label("Revoke")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.stop_button_clicked(cx)
                                        })),
                                ),
                        )
                    }
                    // No policy yet — a single quiet line, exactly as the old fence did.
                    _ => col.child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Reading the signer's policy…"),
                    ),
                },
            )
    }
}
