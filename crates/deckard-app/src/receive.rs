//! Receive — the active account's real EIP-55 address as a scannable QR, the full
//! string, and a working copy-to-clipboard. The address is a real keypair
//! (see wallet.rs); anyone can send to it.

use gpui::{div, px, rgb, ClipboardItem, Context, FontWeight, IntoElement, ParentElement, Styled};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Icon, IconName,
};
use qrcode::{Color, QrCode};

use crate::shell::{Shell, Surface};
use crate::theme;

impl Shell {
    pub fn render_receive(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;

        let is_dark = theme.is_dark();
        let amber = theme::amber(is_dark);

        let address = self.wallet_address_string();

        // Real QR on a white card (QR must be dark-on-light to scan).
        let qr = match QrCode::new(address.as_bytes()) {
            Ok(code) => {
                let width = code.width();
                let colors = code.to_colors();
                let m = 5.0_f32;
                let mut grid = v_flex().bg(rgb(0xFFFFFF)).p_4().rounded_xl();
                for y in 0..width {
                    let mut line = h_flex();
                    for x in 0..width {
                        let dark = colors[y * width + x] == Color::Dark;
                        line = line.child(div().w(px(m)).h(px(m)).bg(if dark {
                            rgb(0x000000)
                        } else {
                            rgb(0xFFFFFF)
                        }));
                    }
                    grid = grid.child(line);
                }
                grid.into_any_element()
            }
            Err(_) => div()
                .text_sm()
                .text_color(muted)
                .child("Could not render QR")
                .into_any_element(),
        };

        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(420.0))
                    .items_center()
                    .gap_5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted)
                            .child("Receive on Ethereum"),
                    )
                    .child(qr)
                    .child(
                        div()
                            .w_full()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .text_xs()
                            .text_color(fg)
                            .child(address),
                    )
                    // Network warning — the one caution moment (DESIGN §236): a
                    // neutral surface with a 2px amber LEFT keyline + amber icon/text.
                    // Not a filled warm block; the risk word carries the emphasis.
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap_2()
                            .px_3()
                            .py_2p5()
                            .rounded_lg()
                            .bg(surface)
                            .border_l_2()
                            .border_color(amber)
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .text_color(amber)
                                    .flex_shrink_0(),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(fg)
                                    .child("Only send Ethereum-network assets to this address. Funds sent on the wrong network may be lost."),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("copy-address")
                                    .primary()
                                    .label("Copy address")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            this.wallet_address_string(),
                                        ));
                                    })),
                            )
                            .child(Button::new("receive-back").ghost().label("Back").on_click(
                                cx.listener(|this, _, _, cx| this.open(Surface::Home, cx)),
                            )),
                    ),
            )
    }
}
