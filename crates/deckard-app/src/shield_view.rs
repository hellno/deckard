//! Shield — the privacy hero's trigger flow (T5). Three states over one centered card:
//! **compose** (amount + 0zk recipient) → **review** (a clear-signing card: amount /
//! recipient / 0.25% fee + the three honesty lines + a deliberate hold-to-confirm) →
//! **done** (the deposit is broadcast and on its way to a private note).
//!
//! The honesty + deliberate-hold model is DESIGN's clear-signing engine (plain language,
//! exact mono figures, danger early, confirm is a hold never a tap). The hold-to-confirm is
//! hand-built (no existing widget): `on_mouse_down`/`up` drive an epoch-guarded timer in
//! `shell.rs`, and an amber `theme::amber_tint` fill-sweep animates over the same
//! `SHIELD_HOLD` span so the bar fills exactly as the deposit signs.

use gpui::{
    div, px, relative, Animation, AnimationExt, ClipboardItem, Context, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Disableable, Icon, IconName,
};

use deckard_core::U256;

use crate::money::money;
use crate::shell::{Shell, ShieldProposal, Surface, SHIELD_HOLD};
use crate::theme;

/// The Railgun shield fee, 25 bps (0.25%) — matches `deckard_core::shield`'s on-chain
/// deduction (`value - value*25/10000`). Shown so the review card never hides the haircut.
fn shield_fee(value: U256) -> U256 {
    value * U256::from(25u64) / U256::from(10_000u64)
}

/// Middle-truncate a long address (0zk… / 0x…) for a tight row.
fn short_mid(s: &str) -> String {
    if s.len() >= 16 {
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

impl Shell {
    /// Dispatch to the active shield state: done (broadcast) → review (proposed) → compose.
    pub fn render_shield(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(tx) = self.shield_tx {
            return self
                .render_shield_done(tx.to_string(), cx)
                .into_any_element();
        }
        if let Some(proposal) = self.shield_proposal.clone() {
            return self.render_shield_review(proposal, cx).into_any_element();
        }
        self.render_shield_compose(cx).into_any_element()
    }

    /// Compose: amount (ETH) + 0zk recipient, then Review. The shield glyph is a neutral,
    /// low-chroma mark (DESIGN: private ≠ cyan/agent and ≠ amber/human — it stays off the
    /// actor axis).
    fn render_shield_compose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let busy = self.shield_busy;

        // Validity drives the Review button's disabled state (DESIGN: disable a primary
        // action on incomplete/invalid input). Re-evaluated live via the input subscriptions.
        let amount_raw = self.shield_amount.read(cx).value().to_string();
        let recipient_raw = self.shield_recipient.read(cx).value().to_string();
        let can_review = crate::signer::parse_eth_to_wei(&amount_raw)
            .map(|w| w > U256::ZERO)
            .unwrap_or(false)
            && !recipient_raw.trim().is_empty();

        self.shield_shell(
            v_flex()
                .w_full()
                .gap_5()
                .child(self.shield_heading(
                    "Shield to private",
                    "Move public ETH into a Railgun private balance. The deposit itself is visible on Ethereum; the balance after is not.",
                    cx,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label("Amount", muted))
                        .child(Input::new(&self.shield_amount).w_full()),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label("Recipient (your 0zk address)", muted))
                        .child(Input::new(&self.shield_recipient).w_full()),
                )
                .children(self.shield_error.as_ref().map(|e| error_line(e, cx)))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("shield-review")
                                .primary()
                                .label(if busy { "Reviewing…" } else { "Review deposit" })
                                .disabled(busy || !can_review)
                                .on_click(cx.listener(|this, _, _, cx| this.review_shield(cx))),
                        )
                        .child(
                            Button::new("shield-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open(Surface::Home, cx)
                                })),
                        ),
                )
                .child(
                    // Only call the recipient "your own 0zk address" when it actually matches the
                    // wallet's auto-filled address — a user-typed/edited recipient gets neutral copy
                    // so the line never misrepresents where the deposit is going.
                    div().text_xs().text_color(muted).child({
                        let recipient = recipient_raw.trim();
                        let is_own_address =
                            self.railgun_address.as_deref().map(str::trim) == Some(recipient);
                        if recipient.is_empty() {
                            "Enter the 0zk address that will receive the private balance."
                        } else if is_own_address {
                            "Pre-filled with your own 0zk address — edit it to shield to a different recipient."
                        } else {
                            "Shielding to the 0zk address above — double-check it before you continue."
                        }
                    }),
                )
                .into_any_element(),
        )
    }

    /// Review: the clear-signing card (amount / recipient / fee / net) + the three honesty
    /// lines + a deliberate hold-to-confirm. `intent` carries the gross (pre-fee) value.
    fn render_shield_review(
        &self,
        proposal: ShieldProposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono = theme.mono_font_family.clone();

        // Render from the proposal SNAPSHOT — the amount + recipient that are actually inside
        // the signed intent — never the live input (which the user could have since edited).
        let gross = proposal.intent.value;
        let fee = shield_fee(gross);
        let net = gross.saturating_sub(fee);
        let recipient = proposal.recipient.clone();

        // One key/value row: label left (muted), value right (mono-for-money or mono text).
        let kv_money = |label: &'static str, wei: U256| {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .py_1p5()
                .child(div().text_sm().text_color(muted).child(label))
                .child(div().text_sm().child(money(
                    wei,
                    18,
                    6,
                    Some("ETH"),
                    false,
                    mono.clone(),
                    fg,
                    muted,
                )))
        };

        self.shield_shell(
            v_flex()
                .w_full()
                .gap_4()
                .child(self.shield_heading(
                    "Review deposit",
                    "Confirm what leaves, where it goes, and the fee. Hold to shield.",
                    cx,
                ))
                // The clear-signing card: one frame, no interior grid lines.
                .child(
                    v_flex()
                        .w_full()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .child(kv_money("Amount", gross))
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .py_1p5()
                                .child(div().text_sm().text_color(muted).child("To"))
                                .child(
                                    div()
                                        .font_family(mono.clone())
                                        .text_sm()
                                        .text_color(fg)
                                        .child(short_mid(recipient.trim())),
                                ),
                        )
                        .child(kv_money("Railgun fee · 0.25%", fee))
                        .child(kv_money("You'll receive (private)", net)),
                )
                .child(self.shield_honesty(cx))
                .children(self.shield_error.as_ref().map(|e| error_line(e, cx)))
                .child(self.hold_to_confirm(cx))
                .child(
                    Button::new("shield-edit")
                        .ghost()
                        .w_full()
                        .label("Edit")
                        .on_click(cx.listener(|this, _, _, cx| this.open_shield(cx))),
                )
                .into_any_element(),
        )
    }

    /// Done: the deposit broadcast — on its way to a private note (reassurance copy mirrors
    /// `ShieldStatus`). The full lifecycle drive lands in Wave 2.
    fn render_shield_done(&self, tx: String, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let success = theme.success;
        let mono = theme.mono_font_family.clone();

        self.shield_shell(
            v_flex()
                .w_full()
                .items_center()
                .gap_4()
                .child(
                    Icon::new(IconName::CircleCheck)
                        .text_color(success)
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child("Deposit broadcast"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .text_center()
                        .child("Your deposit is on its way to a private balance. It becomes spendable after on-chain confirmation and a private sync."),
                )
                .child(
                    div()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .font_family(mono)
                        .text_xs()
                        .text_color(muted)
                        .child(short_mid(&tx)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("shield-copy-tx")
                                .ghost()
                                .label("Copy tx hash")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(tx.clone()));
                                })),
                        )
                        .child(
                            Button::new("shield-done")
                                .primary()
                                .label("Done")
                                .on_click(cx.listener(|this, _, _, cx| this.open(Surface::Home, cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The three honesty lines, in DESIGN's caution frame (neutral surface + a 2px amber
    /// left keyline). Calm, not a filled warm block.
    fn shield_honesty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let surface = theme.secondary;
        let amber = theme::amber(theme.is_dark());

        v_flex()
            .w_full()
            .gap_1p5()
            .px_3()
            .py_2p5()
            .rounded_lg()
            .bg(surface)
            .border_l_2()
            .border_color(amber)
            .child(
                div()
                    .text_xs()
                    .text_color(fg)
                    .child("This deposit is public on Ethereum."),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(fg)
                    .child("Avoid round or unusual amounts."),
            )
            .child(div().text_xs().text_color(muted).child(
                "A 0.25% Railgun fee is deducted; your private balance will read slightly less.",
            ))
    }

    /// The hand-built hold-to-confirm: an amber fill sweeps the button width over
    /// [`SHIELD_HOLD`] while held; completing the hold fires `confirm_shield`, releasing
    /// early resets it. The label sits above the sweep.
    fn hold_to_confirm(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let amber_tint = theme::amber_tint(theme.is_dark());
        let holding = self.shield_holding;
        let busy = self.shield_busy;

        let label = if busy {
            "Shielding…"
        } else if holding {
            "Keep holding…"
        } else {
            "Hold to shield"
        };

        // The amber fill: 0→full width over SHIELD_HOLD while holding; empty otherwise.
        let fill = if holding {
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .bg(amber_tint)
                .with_animation("shield-fill", Animation::new(SHIELD_HOLD), |el, delta| {
                    el.w(relative(delta))
                })
                .into_any_element()
        } else {
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(0.0))
                .into_any_element()
        };

        div()
            .id("shield-hold")
            .relative()
            .overflow_hidden()
            .w_full()
            .h(px(44.0))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(surface)
            .cursor_pointer()
            .child(fill)
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(label),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.shield_hold_start(cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.shield_hold_cancel(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.shield_hold_cancel(cx)),
            )
    }

    /// The shared centered shell for every shield state (mirrors `render_receive`'s layout).
    fn shield_shell(&self, inner: gpui::AnyElement) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_8()
            .child(v_flex().w(px(460.0)).items_start().child(inner))
    }

    /// The shield heading: a neutral low-chroma shield glyph + H1 + muted subtitle. The
    /// glyph is deliberately NOT cyan/amber — privacy sits off the actor axis (DESIGN).
    fn shield_heading(
        &self,
        title: &str,
        subtitle: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let shield_tone = theme::shield(theme.is_dark());

        h_flex()
            .w_full()
            .items_start()
            .gap_3()
            // A small neutral shield mark (no shield icon ships in the kit): a rounded
            // square in the low-chroma shield tone.
            .child(
                div()
                    .size(px(28.0))
                    .rounded(px(6.0))
                    .bg(shield_tone)
                    .flex_shrink_0(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(title.to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(subtitle.to_string()),
                    ),
            )
    }
}

/// A tiny uppercase field label (matches the sidebar/section label treatment).
fn field_label(text: &'static str, muted: gpui::Hsla) -> impl IntoElement {
    div().text_xs().text_color(muted).child(text)
}

/// A one-line shield error, in `danger`.
fn error_line(msg: &str, cx: &mut Context<Shell>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().danger)
        .child(format!("⚠ {msg}"))
}
