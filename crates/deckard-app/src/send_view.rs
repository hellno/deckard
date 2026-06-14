//! Send — the native-ETH transfer flow. Three states over one centered card: **compose**
//! (amount + a `0x…`/ENS recipient) → **review** (a clear-signing card: amount / recipient + an
//! honesty line + a deliberate hold-to-confirm) → **done** (the transfer is broadcast and on
//! its way).
//!
//! Mirrors `shield_view`: the honesty + deliberate-hold model is DESIGN's clear-signing engine
//! (plain language, exact mono figures, danger early, confirm is a hold never a tap). The amber
//! fill-sweep animates over the same [`SHIELD_HOLD`] span so the bar fills exactly as the
//! transfer signs. A send has no Railgun fee and no private side — so there is no fee row and no
//! net line, just what leaves and where it goes.

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
use crate::shell::{SendProposal, Shell, Surface, SHIELD_HOLD};
use crate::theme;

/// Middle-truncate a long address (`0x…`) for a tight row (matches `shield_view`).
fn short_mid(s: &str) -> String {
    if s.len() >= 16 {
        format!("{}…{}", &s[..10], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

impl Shell {
    /// Dispatch to the active send state: done (broadcast) → review (proposed) → compose.
    pub fn render_send(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(tx) = self.send_tx {
            return self.render_send_done(tx.to_string(), cx).into_any_element();
        }
        if let Some(proposal) = self.send_proposal.clone() {
            return self.render_send_review(proposal, cx).into_any_element();
        }
        self.render_send_compose(cx).into_any_element()
    }

    /// Compose: amount (ETH) + a `0x…`/ENS recipient, then Review. The send glyph is a neutral,
    /// low-chroma "public" mark (DESIGN: a public transfer sits off the cyan/agent axis; the
    /// human signal lives on the amber hold-to-confirm, not the heading).
    fn render_send_compose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let busy = self.send_busy;

        // Validity drives the Review button's disabled state, re-evaluated live via the input
        // subscriptions (same as the shield compose screen).
        let amount_raw = self.send_amount.read(cx).value().to_string();
        let recipient_raw = self.send_recipient.read(cx).value().to_string();
        let can_review = crate::signer::parse_eth_to_wei(&amount_raw)
            .map(|w| w > U256::ZERO)
            .unwrap_or(false)
            && !recipient_raw.trim().is_empty();

        self.send_shell(
            v_flex()
                .w_full()
                .gap_5()
                .child(self.send_heading(
                    "Send ETH",
                    "Transfer native ETH from your wallet. This transaction is public on Ethereum and can't be undone.",
                    cx,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label("Amount", muted))
                        .child(Input::new(&self.send_amount).w_full()),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(field_label("Recipient (0x address or ENS name)", muted))
                        .child(Input::new(&self.send_recipient).w_full()),
                )
                .children(self.send_error.as_ref().map(|e| error_line(e, cx)))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("send-review")
                                .primary()
                                .label(if busy { "Reviewing…" } else { "Review transfer" })
                                .disabled(busy || !can_review)
                                .on_click(cx.listener(|this, _, _, cx| this.review_send(cx))),
                        )
                        .child(
                            Button::new("send-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.open(Surface::Home, cx)),
                                ),
                        ),
                )
                .child(
                    div().text_xs().text_color(muted).child(
                        "An ENS name is resolved when you review — you'll confirm the exact address before sending.",
                    ),
                )
                .into_any_element(),
        )
    }

    /// Review: the clear-signing card (amount / recipient) + an honesty line + a deliberate
    /// hold-to-confirm. Rendered from the proposal SNAPSHOT — the amount + resolved recipient
    /// that are actually inside the signed intent — never the live input.
    fn render_send_review(
        &self,
        proposal: SendProposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono = theme.mono_font_family.clone();

        let amount = proposal.intent.value;
        let recipient = proposal.recipient.clone();

        self.send_shell(
            v_flex()
                .w_full()
                .gap_4()
                .child(self.send_heading(
                    "Review transfer",
                    "Confirm the amount and the destination address. Hold to send.",
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
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .py_1p5()
                                .child(div().text_sm().text_color(muted).child("Amount"))
                                .child(div().text_sm().child(money(
                                    amount,
                                    18,
                                    6,
                                    Some("ETH"),
                                    false,
                                    mono.clone(),
                                    fg,
                                    muted,
                                ))),
                        )
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
                        ),
                )
                .child(self.send_honesty(cx))
                .children(self.send_error.as_ref().map(|e| error_line(e, cx)))
                .child(self.send_hold_to_confirm(cx))
                .child(
                    Button::new("send-edit")
                        .ghost()
                        .w_full()
                        .label("Edit")
                        .on_click(cx.listener(|this, _, _, cx| this.open_send(cx))),
                )
                .into_any_element(),
        )
    }

    /// Done: the transfer broadcast — on its way. Mirrors `render_shield_done`, minus the
    /// private-sync reassurance (a public send has no note to settle).
    fn render_send_done(&self, tx: String, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let success = theme.success;
        let mono = theme.mono_font_family.clone();

        self.send_shell(
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
                        .child("Transfer broadcast"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .text_center()
                        .child("Your ETH is on its way. It settles after on-chain confirmation; your balance updates on the next sync."),
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
                            Button::new("send-copy-tx")
                                .ghost()
                                .label("Copy tx hash")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(tx.clone()));
                                })),
                        )
                        .child(
                            Button::new("send-done")
                                .primary()
                                .label("Done")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.open(Surface::Home, cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The honesty lines in a calm neutral surface (no keyline): a send is public and final.
    fn send_honesty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let surface = theme.secondary;

        v_flex()
            .w_full()
            .gap_1p5()
            .px_3()
            .py_2p5()
            .rounded_lg()
            .bg(surface)
            .child(
                div()
                    .text_xs()
                    .text_color(fg)
                    .child("This transfer is public on Ethereum and can't be undone."),
            )
            .child(div().text_xs().text_color(muted).child(
                "Double-check the destination address — funds sent to the wrong address are lost.",
            ))
    }

    /// The hand-built hold-to-confirm: an amber fill sweeps the button width over
    /// [`SHIELD_HOLD`] while held; completing the hold fires `confirm_send`, releasing early
    /// resets it. Mirrors `shield_view::hold_to_confirm` (the amber = human-confirm signal).
    fn send_hold_to_confirm(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let amber_tint = theme::amber_tint(theme.is_dark());
        let holding = self.send_holding;
        let busy = self.send_busy;

        let label = if busy {
            "Sending…"
        } else if holding {
            "Keep holding…"
        } else {
            "Hold to send"
        };

        let fill = if holding {
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .bg(amber_tint)
                .with_animation("send-fill", Animation::new(SHIELD_HOLD), |el, delta| {
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
            .id("send-hold")
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
                cx.listener(|this, _, _, cx| this.send_hold_start(cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.send_hold_cancel(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.send_hold_cancel(cx)),
            )
    }

    /// The shared centered shell for every send state (mirrors `shield_shell`).
    fn send_shell(&self, inner: gpui::AnyElement) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_8()
            .child(v_flex().w(px(460.0)).items_start().child(inner))
    }

    /// The send heading: a neutral low-chroma "public" glyph + H1 + muted subtitle. The glyph
    /// is the desaturated identity tone (the public/your-wallet tone used by the balance hero),
    /// deliberately NOT cyan/amber — the human signal lives on the hold-to-confirm.
    fn send_heading(
        &self,
        title: &str,
        subtitle: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let public_tone = theme::identity_square(theme.is_dark());

        h_flex()
            .w_full()
            .items_start()
            .gap_3()
            .child(
                div()
                    .size(px(28.0))
                    .rounded(px(6.0))
                    .bg(public_tone)
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

/// A tiny uppercase field label (matches the shield/sidebar section-label treatment).
fn field_label(text: &'static str, muted: gpui::Hsla) -> impl IntoElement {
    div().text_xs().text_color(muted).child(text)
}

/// A one-line send error, in `danger`.
fn error_line(msg: &str, cx: &mut Context<Shell>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().danger)
        .child(format!("⚠ {msg}"))
}
