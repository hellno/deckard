//! Portfolio — Deckard's home screen. A calm, dense balance view: total at the
//! top, a Send / Receive / Swap action row, then the token holdings. Rendered by
//! `Shell` for the `Welcome` route. v0 uses representative data; wiring live
//! balances over the light client / RPC is the next step.

use gpui::{div, px, Context, FontWeight, IntoElement, ParentElement, Styled};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme,
};

use crate::shell::{Route, Shell};

/// The primary modifier label, per platform (⌘ on macOS, "Ctrl " elsewhere).
#[cfg(target_os = "macos")]
const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl ";

impl Shell {
    pub fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mark_bg = theme.muted;

        // A token holding row: mark + name/symbol on the left, amount + USD right.
        let row = move |mark: &str, name: &str, symbol: &str, amount: &str, usd: &str| {
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .rounded_lg()
                .border_1()
                .border_color(border)
                .bg(surface)
                .child(
                    h_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(34.0))
                                .rounded_full()
                                .bg(mark_bg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(fg)
                                        .child(mark.to_string()),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(fg)
                                        .child(name.to_string()),
                                )
                                .child(div().text_xs().text_color(muted).child(symbol.to_string())),
                        ),
                )
                .child(
                    v_flex()
                        .items_end()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg)
                                .child(amount.to_string()),
                        )
                        .child(div().text_xs().text_color(muted).child(usd.to_string())),
                )
        };

        // A small bordered key-hint chip, e.g. ⌘K.
        let chip = move |keys: &str, label: &str| {
            h_flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .text_color(fg)
                        .text_xs()
                        .child(keys.to_string()),
                )
                .child(div().text_xs().text_color(muted).child(label.to_string()))
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(460.0))
                    .gap_6()
                    // Header: section label + active account pill.
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().text_color(muted).child("Portfolio"))
                            .child(
                                div()
                                    .px_2p5()
                                    .py_1()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(border)
                                    .bg(surface)
                                    .text_xs()
                                    .text_color(fg)
                                    .child(self.wallet.short()),
                            ),
                    )
                    // Total balance.
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_3xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(fg)
                                    .child("$5,471.20"),
                            )
                            .child(div().text_sm().text_color(muted).child("≈ 1.934 ETH on Ethereum")),
                    )
                    // Primary actions.
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(Button::new("send").primary().label("Send").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.created += 1;
                                    cx.notify();
                                }),
                            ))
                            .child(Button::new("receive").ghost().label("Receive").on_click(
                                cx.listener(|this, _, _, cx| this.navigate(Route::Receive, cx)),
                            ))
                            .child(Button::new("swap").ghost().label("Swap")),
                    )
                    // Holdings.
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(row("Ξ", "Ethereum", "ETH", "1.934", "$4,180.55"))
                            .child(row("$", "USD Coin", "USDC", "1,200.00", "$1,200.00"))
                            .child(row("◈", "Dai", "DAI", "90.65", "$90.65")),
                    )
                    // Keyboard hints — the Superhuman/Linear signal.
                    .child(
                        h_flex()
                            .gap_4()
                            .pt_1()
                            .child(chip(&format!("{MOD}K"), "Command palette"))
                            .child(chip(&format!("{MOD}S"), "Send"))
                            .child(chip(&format!("{MOD},"), "Settings")),
                    ),
            )
    }
}
