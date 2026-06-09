//! Portfolio — Deckard's home screen. A calm, dense balance view: the native ETH
//! balance up top, a Send / Receive / Swap action row, then real token holdings read
//! live over Multicall3 (`deckard-core`). Renders instantly from the last cached
//! snapshot; the only loading state is the very first sync.

use gpui::prelude::FluentBuilder;
use gpui::{div, px, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Disableable, IconName,
};

use deckard_core::U256;

use crate::money::money;
use crate::shell::{Shell, Surface};
use crate::theme;

/// One row in the holdings table. Carries the raw balance (not a pre-formatted
/// string) so the amount column can render mono-for-money with dimmed decimals.
struct Holding {
    mark: String,
    name: String,
    symbol: String,
    raw: U256,
    decimals: u8,
    max_frac: usize,
}

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
    /// The selected wallet's home — a left-anchored, scrollable main pane:
    /// wallet-name H1 + address subtitle, the balance hero, Send/Receive/Swap,
    /// then live holdings. The synced/trust status line lives in the bottom
    /// status strip (`shell_chrome.rs`), not here.
    pub fn render_wallet_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono: SharedString = theme.mono_font_family.clone();

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

        // The native ETH balance for the hero — `None` until the first sync lands.
        let native_wei = self.portfolio.as_ref().map(|p| p.native_wei);

        // Holdings rows: ETH first, then each listed token. Each row carries the
        // raw value + decimals + frac so the amount column renders mono-for-money
        // (dimmed decimals) rather than a single-color string.
        let mut holdings: Vec<Holding> = Vec::new();
        if let Some(p) = &self.portfolio {
            holdings.push(Holding {
                mark: "Ξ".into(),
                name: "Ethereum".into(),
                symbol: "ETH".into(),
                raw: p.native_wei,
                decimals: 18,
                max_frac: 4,
            });
            for t in &p.tokens {
                let frac = if t.decimals <= 6 { 2 } else { 4 };
                let mark = t.symbol.chars().next().unwrap_or('•').to_string();
                holdings.push(Holding {
                    mark,
                    name: t.name.to_string(),
                    symbol: t.symbol.to_string(),
                    raw: t.raw,
                    decimals: t.decimals,
                    max_frac: frac,
                });
            }
        }
        let has_tokens = self
            .portfolio
            .as_ref()
            .map(|p| !p.tokens.is_empty())
            .unwrap_or(false);

        // Wallet identity for the header: a desaturated, tinted-neutral square
        // (DESIGN rule 4 — identity squares avoid the warm/amber band).
        let id_square = theme::identity_square(theme.is_dark());
        let wallet_name = if self.viewing_watch {
            "Watched account".to_string()
        } else {
            "Personal".to_string()
        };

        div()
            .size_full()
            .p_8()
            // TODO(scroll): restore a scrollable main pane via a Stateful
            // `div().id(..).overflow_y_scroll()` (the agent draft mis-ordered
            // gpui-component's `overflow_y_scrollbar`). Content is short for now.
            .child(
                v_flex()
                    .items_start()
                    .max_w(px(680.0))
                    .gap_6()
                    // Page header (DESIGN §Page header): identity square + wallet-name
                    // H1 (text.primary, weight 600 — NEVER cyan) + a muted mono,
                    // middle-truncated address subtitle.
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(div().size(px(28.0)).rounded(px(6.0)).bg(id_square))
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_xl()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg)
                                                    .child(wallet_name),
                                            )
                                            .child(
                                                div()
                                                    .font_family(mono.clone())
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(account_pill),
                                            ),
                                    ),
                            )
                            .child(
                                Button::new("refresh")
                                    .ghost()
                                    .icon(IconName::Replace)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_portfolio(cx)),
                                    ),
                            ),
                    )
                    // Balance hero: native ETH balance (mono-for-money, dimmed
                    // decimals + ticker, weight 600 not 700).
                    .child(v_flex().gap_1().child(
                        div().text_3xl().font_weight(FontWeight::SEMIBOLD).map(
                            |el| match native_wei {
                                Some(wei) => el.child(money(
                                    wei,
                                    18,
                                    4,
                                    Some("ETH"),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )),
                                None => el.font_family(mono.clone()).text_color(muted).child("—"),
                            },
                        ),
                    ))
                    // Primary actions. Send + Swap are gated to the next release (Chunk 4,
                    // testnet-first), so they're shown disabled rather than inert-but-active.
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(Button::new("send").primary().label("Send").disabled(true))
                            .child(Button::new("receive").ghost().label("Receive").on_click(
                                cx.listener(|this, _, _, cx| this.open(Surface::Receive, cx)),
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
        holdings: Vec<Holding>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let mono: SharedString = theme.mono_font_family.clone();

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

        for h in holdings {
            col = col.child(render_row(
                theme.foreground,
                theme.muted_foreground,
                theme.border,
                theme.secondary,
                theme.muted,
                h.mark,
                h.name,
                h.symbol,
                h.raw,
                h.decimals,
                h.max_frac,
                mono.clone(),
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

    /// Project home — the aggregate-of-one for the demo's single project: the
    /// wallet's balance plus a one-line composition (1 wallet · 1 agent). Real
    /// multi-wallet aggregation is fast-follow.
    pub fn render_project_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let mono: SharedString = theme.mono_font_family.clone();
        let id_square = theme::identity_square(theme.is_dark());

        let native_wei = self.portfolio.as_ref().map(|p| p.native_wei);

        div()
            .size_full()
            .p_8()
            // TODO(scroll): restore a scrollable main pane via a Stateful
            // `div().id(..).overflow_y_scroll()` (the agent draft mis-ordered
            // gpui-component's `overflow_y_scrollbar`). Content is short for now.
            .child(
                v_flex()
                    .items_start()
                    .max_w(px(680.0))
                    .gap_6()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(div().size(px(28.0)).rounded(px(6.0)).bg(id_square))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg)
                                    .child("Personal"),
                            ),
                    )
                    .child(
                        div().text_3xl().font_weight(FontWeight::SEMIBOLD).map(
                            |el| match native_wei {
                                Some(wei) => el.child(money(
                                    wei,
                                    18,
                                    4,
                                    Some("ETH"),
                                    mono.clone(),
                                    fg,
                                    muted,
                                )),
                                None => el.font_family(mono.clone()).text_color(muted).child("—"),
                            },
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("1 wallet · 1 agent"),
                    ),
            )
    }

    /// Agent home — a static, demo-scoped policy-card placeholder (DESIGN §Policy
    /// card): 2-column label/value pairs grouped by whitespace in one faint frame,
    /// no interior grid lines. Agent "Atlas" is the openly-narrated manual stand-in
    /// for v1 (real Claude-Desktop-via-MCP is fast-follow), so the values are
    /// static. The agent identity (cyan) lives only on the squircle glyph.
    pub fn render_agent_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono: SharedString = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        // One policy row: label left (muted), value right (mono, primary). No
        // per-row hairline — grouping is whitespace.
        let mono_for_row = mono.clone();
        let policy_row = move |label: &'static str, value: &'static str| {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .py_1p5()
                .child(div().text_sm().text_color(muted).child(label))
                .child(
                    div()
                        .font_family(mono_for_row.clone())
                        .text_sm()
                        .text_color(fg)
                        .child(value),
                )
        };

        div()
            .size_full()
            .p_8()
            // TODO(scroll): restore a scrollable main pane via a Stateful
            // `div().id(..).overflow_y_scroll()` (the agent draft mis-ordered
            // gpui-component's `overflow_y_scrollbar`). Content is short for now.
            .child(
                v_flex()
                    .items_start()
                    .max_w(px(680.0))
                    .gap_6()
                    // Header: cyan squircle monogram (the ONLY cyan on the surface) +
                    // agent name H1 (text.primary, NEVER cyan).
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(28.0))
                                    .rounded(px(6.0))
                                    .bg(agent_tint)
                                    .border_1()
                                    .border_color(agent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(agent)
                                            .child("A"),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(fg)
                                            .child("Atlas"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child("Delegated agent · idle"),
                                    ),
                            ),
                    )
                    // Policy card: one faint frame, no interior grid lines.
                    .child(
                        v_flex()
                            .w_full()
                            .gap_0()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .child(policy_row("Per-transaction cap", "0.10 ETH"))
                            .child(policy_row("Period budget", "1.00 ETH / week"))
                            .child(policy_row("Allowed assets", "ETH"))
                            .child(policy_row("Session key", "expires in 6d"))
                            .child(policy_row("Autonomy", "act < $50 · ask above")),
                    )
                    .child(
                        div().text_xs().text_color(muted).child(
                            "Atlas is a manual stand-in for the demo. Controls land with MCP.",
                        ),
                    ),
            )
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
    _surface: gpui::Hsla,
    mark_bg: gpui::Hsla,
    mark: String,
    name: String,
    symbol: String,
    raw: U256,
    decimals: u8,
    max_frac: usize,
    mono: SharedString,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        // DESIGN §Holdings table: tight rows, hairline row separators only —
        // no per-row card frame, no fill.
        .border_b_1()
        .border_color(border)
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .size(px(34.0))
                        // DESIGN §Radii: a desaturated token SQUARE (6px), not a
                        // round identicon (round is reserved for the human principal).
                        .rounded(px(6.0))
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
                .child(money(raw, decimals, max_frac, None, mono, fg, muted)),
        )
}
