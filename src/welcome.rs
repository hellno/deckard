//! The Welcome page — a Linear-style centered card. Rendered by `Shell` when the
//! route is `Welcome`. Replace this with your app's home screen.

use gpui::{div, px, Context, FontWeight, IntoElement, ParentElement, Styled};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, IconName,
};

use crate::shell::Shell;
use crate::APP_NAME;

/// The primary modifier label, per platform (⌘ on macOS, "Ctrl " elsewhere).
#[cfg(target_os = "macos")]
const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl ";

impl Shell {
    pub fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let primary = theme.primary;
        let primary_fg = theme.primary_foreground;
        let border = theme.border;
        let surface = theme.secondary;

        let name = self.settings.display_name.trim();
        let title = if name.is_empty() {
            format!("Welcome to {APP_NAME}")
        } else {
            format!("Welcome back, {name}")
        };

        // A small bordered key-hint chip, e.g. ⌘N.
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
                        .text_color(foreground)
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
                    .w(px(440.0))
                    .items_center()
                    .gap_7()
                    // Logo mark — uses the accent (theme primary), so it restyles
                    // live when the user changes accent in settings.
                    .child(
                        div()
                            .size(px(68.0))
                            .rounded_2xl()
                            .bg(primary)
                            .shadow_lg()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_color(primary_fg)
                                    .text_3xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(APP_NAME.chars().next().unwrap_or('D').to_string()),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .max_w(px(360.0))
                                    .text_center()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(
                                        "A tiny, native desktop starter built on GPUI. \
                                         Fork it, rename it, and wire in your own agent.",
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Button::new("get-started")
                                    .primary()
                                    .label("Get Started")
                                    .icon(IconName::ArrowRight)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.created += 1;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("docs")
                                    .ghost()
                                    .label("Documentation")
                                    .icon(IconName::BookOpen),
                            ),
                    )
                    .child(if self.created > 0 {
                        div()
                            .text_sm()
                            .text_color(foreground)
                            .child(format!(
                                "✓ Created {} item{}",
                                self.created,
                                if self.created == 1 { "" } else { "s" }
                            ))
                            .into_any_element()
                    } else {
                        h_flex()
                            .gap_4()
                            .child(chip(&format!("{MOD}N"), "New"))
                            .child(chip(&format!("{MOD},"), "Settings"))
                            .child(chip(&format!("{MOD}⇧D"), "Theme"))
                            .into_any_element()
                    }),
            )
    }
}
