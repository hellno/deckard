//! Portfolio — Deckard's home screen. A calm, dense balance view: the native ETH
//! balance up top, a Send / Receive / Swap action row, then real token holdings read
//! live over Multicall3 (`deckard-core`). Renders instantly from the last cached
//! snapshot; the only loading state is the very first sync.

use gpui::{div, px, Context, FontWeight, IntoElement, ParentElement, Styled};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Disableable, IconName,
};

use deckard_core::format_amount;

use crate::shell::{Route, Shell};

/// The primary modifier label, per platform (⌘ on macOS, "Ctrl " elsewhere).
#[cfg(target_os = "macos")]
const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl ";

/// Middle-truncate an address string for a tight pill, e.g. `0xA1b2…9F3c`.
fn short_addr(a: &str) -> String {
    if a.len() >= 12 {
        format!("{}…{}", &a[..6], &a[a.len() - 4..])
    } else {
        a.to_string()
    }
}

impl Shell {
    pub fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let accent = theme.primary;

        // A small bordered key-hint chip, e.g. ⌘K.
        let chip = move |keys: String, label: String| {
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
                        .child(keys),
                )
                .child(div().text_xs().text_color(muted).child(label))
        };

        // --- Derive view state from the live portfolio. ---
        let addr_str = self.display_address.to_string();
        let account_pill = short_addr(&addr_str);
        let first_sync = self.portfolio_loading && self.portfolio.is_none();

        let native_str = self
            .portfolio
            .as_ref()
            .map(|p| format_amount(p.native_wei, 18, 4))
            .unwrap_or_else(|| "—".to_string());

        // Holdings rows: ETH first, then each non-zero listed token.
        let mut holdings: Vec<(String, String, String, String)> = Vec::new();
        if let Some(p) = &self.portfolio {
            holdings.push((
                "Ξ".into(),
                "Ethereum".into(),
                "ETH".into(),
                format_amount(p.native_wei, 18, 4),
            ));
            for t in &p.tokens {
                let frac = if t.decimals <= 6 { 2 } else { 4 };
                let mark = t.symbol.chars().next().unwrap_or('•').to_string();
                holdings.push((
                    mark,
                    t.name.to_string(),
                    t.symbol.to_string(),
                    format_amount(t.raw, t.decimals, frac),
                ));
            }
        }
        let has_tokens = self
            .portfolio
            .as_ref()
            .map(|p| !p.tokens.is_empty())
            .unwrap_or(false);

        // Status sub-line: synced block, watching tag, or an error. When a read carries a
        // non-Verified trust label, surface it: a balance is never shown as quietly trusted.
        let trust_tag = match &self.read_status {
            Some(deckard_core::ReadStatus::Verified) => " · verified",
            Some(deckard_core::ReadStatus::Degraded { .. }) => " · degraded",
            Some(deckard_core::ReadStatus::Unsynced { .. }) => " · NOT VERIFIED",
            None => "",
        };
        let status_line = if let Some(err) = &self.portfolio_error {
            format!("⚠ {err}")
        } else if first_sync {
            "Syncing over Ethereum…".to_string()
        } else if let Some(block) = self.synced_block {
            let net = if self.viewing_watch {
                "watching · "
            } else {
                ""
            };
            format!("{net}synced · block {block}{trust_tag}")
        } else {
            "Ethereum mainnet".to_string()
        };
        // An unverified read is a soft warning (the value may not be trustless), not a hard error.
        let unverified = matches!(
            self.read_status,
            Some(deckard_core::ReadStatus::Unsynced { .. })
        );
        let status_color = if self.portfolio_error.is_some() {
            theme.danger
        } else if unverified {
            theme.warning
        } else {
            muted
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
                    // Header: section label + account pill + refresh.
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().text_color(muted).child("Portfolio"))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .px_2p5()
                                            .py_1()
                                            .rounded_full()
                                            .border_1()
                                            .border_color(if self.viewing_watch {
                                                accent
                                            } else {
                                                border
                                            })
                                            .bg(surface)
                                            .text_xs()
                                            .text_color(fg)
                                            .child(account_pill),
                                    )
                                    .child(
                                        Button::new("refresh")
                                            .ghost()
                                            .icon(IconName::Replace)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.refresh_portfolio(cx)
                                            })),
                                    ),
                            ),
                    )
                    // Total: native ETH balance + status sub-line.
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_3xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(fg)
                                            .child(native_str),
                                    )
                                    .child(div().text_lg().text_color(muted).child("ETH")),
                            )
                            .child(div().text_sm().text_color(status_color).child(status_line)),
                    )
                    // Primary actions. Send + Swap are gated to the next release (Chunk 4,
                    // testnet-first), so they're shown disabled rather than inert-but-active.
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(Button::new("send").primary().label("Send").disabled(true))
                            .child(Button::new("receive").ghost().label("Receive").on_click(
                                cx.listener(|this, _, _, cx| this.navigate(Route::Receive, cx)),
                            ))
                            .child(Button::new("swap").ghost().label("Swap").disabled(true)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Send & Swap arrive in the next release."),
                    )
                    // Holdings, or a state.
                    .child(self.render_holdings(first_sync, has_tokens, holdings, cx))
                    // Keyboard hints — the Superhuman/Linear signal.
                    .child(
                        h_flex()
                            .gap_4()
                            .pt_1()
                            .child(chip(format!("{MOD}K"), "Command palette".into()))
                            .child(chip(format!("{MOD}["), "Back".into()))
                            .child(chip(format!("{MOD},"), "Settings".into())),
                    ),
            )
    }

    /// The holdings region: skeleton on first sync, empty-state when nothing held,
    /// otherwise the live rows plus the listed-tokens-only caveat.
    fn render_holdings(
        &self,
        first_sync: bool,
        has_tokens: bool,
        holdings: Vec<(String, String, String, String)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;

        let mut col = v_flex().w_full().gap_2();

        if first_sync {
            return col
                .child(skeleton_row(theme.secondary, theme.border))
                .child(skeleton_row(theme.secondary, theme.border))
                .into_any_element();
        }

        if holdings.is_empty() {
            return col
                .child(
                    div()
                        .w_full()
                        .px_4()
                        .py_8()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.secondary)
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child("No balances yet"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("Receive funds, or watch an address in Settings."),
                        ),
                )
                .into_any_element();
        }

        for (mark, name, symbol, amount) in holdings {
            col = col.child(render_row(
                theme.foreground,
                theme.muted_foreground,
                theme.border,
                theme.secondary,
                theme.muted,
                mark,
                name,
                symbol,
                amount,
            ));
        }
        if !has_tokens {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .pt_1()
                    .child("Only listed tokens are shown — long-tail tokens may be missing."),
            );
        } else {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .pt_1()
                    .child("Listed tokens only."),
            );
        }
        col.into_any_element()
    }
}

/// A shimmer-free skeleton placeholder row for the first-sync state.
fn skeleton_row(surface: gpui::Hsla, border: gpui::Hsla) -> impl IntoElement {
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
                .child(div().size(px(34.0)).rounded_full().bg(border))
                .child(div().w(px(120.0)).h(px(12.0)).rounded_md().bg(border)),
        )
        .child(div().w(px(64.0)).h(px(12.0)).rounded_md().bg(border))
}

/// A single holding row (free fn so the closure-type plumbing stays simple).
#[allow(clippy::too_many_arguments)]
fn render_row(
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    surface: gpui::Hsla,
    mark_bg: gpui::Hsla,
    mark: String,
    name: String,
    symbol: String,
    amount: String,
) -> impl IntoElement {
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
                                .child(mark),
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
                                .child(name),
                        )
                        .child(div().text_xs().text_color(muted).child(symbol)),
                ),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg)
                .child(amount),
        )
}
