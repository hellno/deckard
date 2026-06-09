//! Command palette — cmd-K overlay with clickable commands. v0 is a fixed
//! command list (navigate, copy address, toggle theme); type-to-filter fuzzy
//! search is the next refinement. This is the keyboard-first signature surface.

use gpui::{
    div, px, rgba, ClipboardItem, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use gpui_component::{v_flex, ActiveTheme};

use crate::shell::{Selection, Shell, Surface};

impl Shell {
    pub fn render_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let popover = theme.popover;

        // A command row: label left, key hint right. Caller attaches `.on_click`.
        let row = move |id: &'static str, label: &str, hint: &str| {
            div()
                .id(id)
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .rounded_lg()
                .child(div().text_sm().text_color(fg).child(label.to_string()))
                .child(div().text_xs().text_color(muted).child(hint.to_string()))
        };

        // Scrim covering the whole window; the panel floats near the top.
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .bg(rgba(0x00000080))
            .child(
                v_flex()
                    .mt(px(96.0))
                    .w(px(540.0))
                    .p_2()
                    .gap_1()
                    .bg(popover)
                    .border_1()
                    .border_color(border)
                    .rounded_xl()
                    .shadow_lg()
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .text_xs()
                            .text_color(muted)
                            .child("Commands"),
                    )
                    .child(
                        row("cmd-portfolio", "Go to Portfolio", "").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.palette_open = false;
                                this.select(Selection::Wallet, cx);
                                this.open(Surface::Home, cx);
                            },
                        )),
                    )
                    .child(row("cmd-receive", "Receive", "").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.palette_open = false;
                            this.open(Surface::Receive, cx);
                        },
                    )))
                    .child(row("cmd-settings", "Settings", "⌘,").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.palette_open = false;
                            this.open(Surface::Settings, cx);
                        },
                    )))
                    .child(row("cmd-copy", "Copy address", "").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.palette_open = false;
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                this.wallet_address_string(),
                            ));
                            cx.notify();
                        },
                    )))
                    .child(
                        row("cmd-theme", "Toggle theme", "⌘⇧D").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.palette_open = false;
                                this.toggle_mode(cx);
                            },
                        )),
                    )
                    .child(
                        row(
                            "cmd-mask",
                            if self.mask {
                                "Show balances"
                            } else {
                                "Mask balances"
                            },
                            "⌘⇧M",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.palette_open = false;
                            this.toggle_mask(cx);
                        })),
                    )
                    .child(
                        row(
                            "cmd-agent-acting",
                            if self.agent_acting {
                                "Stop agent activity (demo)"
                            } else {
                                "Simulate agent activity (demo)"
                            },
                            "",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.palette_open = false;
                            this.toggle_agent_acting(cx);
                        })),
                    )
                    .child(row("cmd-lock", "Lock wallet", "").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.palette_open = false;
                            this.lock(cx);
                        },
                    ))),
            )
    }
}
