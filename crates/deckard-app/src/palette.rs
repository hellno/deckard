//! Command palette — the ⌘K control plane. A self-managed fuzzy query (nucleo) over the
//! static `palette_commands::COMMANDS` registry, ordered by frecency, run keyboard-first.
//!
//! It does NOT use a gpui-component `InputState`: a focused single-line input binds
//! `up`→MoveUp / `down`→MoveDown and CONSUMES them (GPUI dispatches action bindings before
//! key-down listeners), so the arrow keys could never move the selection. Instead the panel
//! is a `track_focus`'d div with `key_context("CommandPalette")` and an `on_key_down`
//! handler (`on_palette_key`) that owns the query `String` directly — every key reaches us.
//!
//! Color law (DESIGN §Color): the selected row is a brightness lift (`secondary`), NEVER a
//! colored keyline; matched chars are a weight/brightness lift, NOT a new hue; cyan appears
//! only on the agent squircle. Curated Lucide icons sit in a fixed gutter so labels stay
//! aligned even when a command has no glyph.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, rgba, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, SharedString, Styled, Window,
};
use gpui_component::{
    h_flex, scroll::ScrollableElement, v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use crate::palette_commands::{self, COMMANDS};
use crate::settings::ThemeModePref;
use crate::shell::{Selection, Shell};
use crate::shell_chrome::{agent_squircle, short_addr};
use crate::theme;

/// The live display label for a command — the registry title, except mask/theme/agent flip
/// to reflect the current state (e.g. "Show balances" while masked). Ranking always uses the
/// static title (handled in `palette_commands`), so the alternate sense stays reachable.
fn display_label(this: &Shell, id: &str, title: &str) -> String {
    match id {
        "mask" => {
            if this.mask {
                "Show balances".to_string()
            } else {
                "Mask balances".to_string()
            }
        }
        "agent" => {
            if this.agent_acting {
                "Stop agent activity (demo)".to_string()
            } else {
                "Simulate agent activity (demo)".to_string()
            }
        }
        _ => title.to_string(),
    }
}

impl Shell {
    /// The ⌘K overlay: a scrim (click-out to close) with a focused, keyed panel — a query row,
    /// a context line showing the active scope, and the ranked results. Rendered from
    /// `shell.rs`'s `render` when `palette_open`.
    pub fn render_palette(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let popover = theme.popover;
        let lift = theme.secondary; // selected / hover = brightness lift (never a keyline)
        let mono = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();
        let id_square = theme::identity_square(is_dark);
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        // --- Query row: a muted Search glyph, the live query (or placeholder), a thin caret. ---
        let query_empty = self.palette_query.is_empty();
        let query_row = h_flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2p5()
            .child(Icon::new(IconName::Search).text_color(muted).small())
            .child(if query_empty {
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(muted)
                    .child("Type a command…")
            } else {
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(fg)
                    .child(self.palette_query.clone())
            })
            // A thin blinking caret keeps the field reading as a live input even with no widget.
            .child(div().w(px(1.0)).h(px(15.0)).bg(fg));

        // --- Context line: the active scope (wallet identity / agent squircle / project). ---
        let context_line =
            self.palette_context_line(id_square, agent, agent_tint, muted, mono.clone());

        // --- Results: the ranked rows (or an empty-state line). ---
        let results = if self.palette_results.is_empty() {
            v_flex().px_4().py_2().child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("No matching commands"),
            )
        } else {
            let mut list = v_flex().py_1().gap_0p5();
            for (ix, ranked) in self.palette_results.iter().enumerate() {
                let Some(cmd) = COMMANDS.get(ranked.cmd_index) else {
                    continue;
                };
                let selected = ix == self.palette_selected;
                let id = cmd.id;
                list = list.child(
                    self.palette_row(
                        ix,
                        cmd,
                        &ranked.positions,
                        ranked.matched_alias,
                        selected,
                        fg,
                        muted,
                        lift,
                        agent,
                        agent_tint,
                        mono.clone(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.palette_selected = ix;
                            this.run_palette_command(id, window, cx);
                        }),
                    ),
                );
            }
            list
        };

        // Scrim covering the whole window; click-out closes. The panel stops propagation so a
        // click INSIDE it never reaches the scrim's close handler.
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .bg(rgba(0x00000080))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
            )
            .child(
                v_flex()
                    .mt(px(96.0))
                    .w(px(560.0))
                    .bg(popover)
                    .border_1()
                    .border_color(border)
                    .rounded(px(10.0))
                    .shadow_lg()
                    .track_focus(&self.palette_focus)
                    .key_context("CommandPalette")
                    .on_key_down(cx.listener(Self::on_palette_key))
                    // Swallow clicks inside the panel so they don't bubble to the scrim's close.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(query_row)
                    .child(context_line)
                    // Hairline under the query/context header.
                    .child(div().h(px(1.0)).w_full().bg(border))
                    .child(
                        div()
                            .id("palette-scroll")
                            .max_h(px(360.0))
                            .overflow_y_scrollbar()
                            .child(results),
                    ),
            )
    }

    /// The context line under the query: the active scope. A wallet selection shows the
    /// identity square + truncated mono address; an agent selection shows the cyan squircle +
    /// "Atlas"; the project shows the identity square + "Personal".
    fn palette_context_line(
        &self,
        id_square: gpui::Hsla,
        agent: gpui::Hsla,
        agent_tint: gpui::Hsla,
        muted: gpui::Hsla,
        mono: SharedString,
    ) -> impl IntoElement {
        let row = h_flex().items_center().gap_2().px_4().pb_2().text_xs();
        match self.selection {
            Selection::Agent => row
                .child(agent_squircle(
                    px(14.0),
                    px(4.0),
                    self.agent_acting,
                    agent,
                    agent_tint,
                    "palette-context-agent",
                ))
                .child(div().text_color(muted).child("Atlas")),
            Selection::Wallet => {
                let addr = short_addr(&self.wallet_address_string());
                let label = if addr.is_empty() {
                    "Wallet".to_string()
                } else {
                    addr
                };
                row.child(div().size(px(14.0)).rounded(px(4.0)).bg(id_square))
                    .child(div().font_family(mono).text_color(muted).child(label))
            }
            Selection::Project => row
                .child(div().size(px(14.0)).rounded(px(4.0)).bg(id_square))
                .child(div().text_color(muted).child("Personal")),
        }
    }

    /// One ranked result row: a fixed icon gutter, the (live) title with matched chars lifted,
    /// an optional ` (alias)` tag, and the right-aligned shortcut hint. `selected`/hover lift the
    /// background (`secondary`) — never a colored keyline (DESIGN §Color).
    #[allow(clippy::too_many_arguments)]
    fn palette_row(
        &self,
        ix: usize,
        cmd: &palette_commands::Command,
        positions: &[usize],
        matched_alias: Option<&'static str>,
        selected: bool,
        fg: gpui::Hsla,
        muted: gpui::Hsla,
        lift: gpui::Hsla,
        agent: gpui::Hsla,
        agent_tint: gpui::Hsla,
        mono: SharedString,
    ) -> gpui::Stateful<gpui::Div> {
        let label = display_label(self, cmd.id, cmd.title);

        // Fixed ~20px gutter so labels stay aligned whether or not a command has a glyph.
        let gutter = div()
            .w(px(20.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(self.palette_icon(cmd, muted, agent, agent_tint));

        // The title row: matched chars (positions) are a brightness/weight lift, the rest muted.
        // Positions index the STATIC title; a dynamic relabel (mask/theme/agent) can differ in
        // length, so only apply them when the displayed label IS the title — otherwise render flat
        // (a title-match highlight on the shorter "Stop agent activity" label would bold stray chars).
        let title_positions: &[usize] = if label == cmd.title { positions } else { &[] };
        let label_row = self.highlighted_label(&label, title_positions, fg, muted);

        let mut text_col = h_flex().items_center().gap_1p5().min_w_0().child(label_row);
        if matched_alias.is_some() {
            // An alias outscored the title — tag it muted (the specific alias isn't shown).
            text_col = text_col.child(div().text_xs().text_color(muted).child("(alias)"));
        }

        let mut row = div()
            .id(("palette-row", ix))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px_4()
            .py_1p5()
            .cursor_pointer()
            .when(selected, |e| e.bg(lift))
            .hover(|e| e.bg(lift))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(gutter)
                    .child(text_col),
            );

        if let Some(shortcut) = cmd.shortcut {
            row = row.child(
                div()
                    .font_family(mono)
                    .text_xs()
                    .text_color(muted)
                    .child(shortcut),
            );
        }
        row
    }

    /// The curated glyph for a row's gutter: the command's static icon, the live Eye/EyeOff for
    /// `mask`, the live Sun/Moon for `theme`, or the cyan agent squircle for `agent`. Commands
    /// with no curated glyph render an empty (but still fixed-width) gutter.
    fn palette_icon(
        &self,
        cmd: &palette_commands::Command,
        muted: gpui::Hsla,
        agent: gpui::Hsla,
        agent_tint: gpui::Hsla,
    ) -> gpui::AnyElement {
        match cmd.id {
            "agent" => agent_squircle(
                px(16.0),
                px(5.0),
                self.agent_acting,
                agent,
                agent_tint,
                "palette-row-agent",
            ),
            "mask" => {
                // Match the breadcrumb's state-reflecting glyph: a slashed eye while masked.
                let icon = if self.mask {
                    IconName::EyeOff
                } else {
                    IconName::Eye
                };
                Icon::new(icon).text_color(muted).small().into_any_element()
            }
            "theme" => {
                let icon = if self.settings.theme_mode == ThemeModePref::Dark {
                    IconName::Sun
                } else {
                    IconName::Moon
                };
                Icon::new(icon).text_color(muted).small().into_any_element()
            }
            _ => match &cmd.icon {
                Some(icon) => Icon::new(icon.clone())
                    .text_color(muted)
                    .small()
                    .into_any_element(),
                None => div().into_any_element(),
            },
        }
    }

    /// Build the label as a row of char spans: chars at `positions` render brighter/semibold
    /// (DESIGN: highlight is a brightness/weight lift, NOT a new hue), the rest stay muted.
    fn highlighted_label(
        &self,
        label: &str,
        positions: &[usize],
        fg: gpui::Hsla,
        muted: gpui::Hsla,
    ) -> impl IntoElement {
        if positions.is_empty() {
            // No title match (empty query or an alias hit) — one flat span, primary text.
            return h_flex()
                .text_sm()
                .child(div().text_color(fg).child(label.to_string()));
        }
        let mut row = h_flex().text_sm();
        for (i, ch) in label.chars().enumerate() {
            let matched = positions.contains(&i);
            let span = div()
                .text_color(if matched { fg } else { muted })
                .when(matched, |e| e.font_weight(FontWeight::SEMIBOLD))
                .child(ch.to_string());
            row = row.child(span);
        }
        row
    }

    /// The palette's own key handler. The panel uses `key_context("CommandPalette")` (no
    /// `"Input"` context), so this listener receives every keystroke — including ↑/↓, which a
    /// focused `InputState` would otherwise consume via the action system.
    fn on_palette_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;
        match key {
            "escape" => {
                self.close_palette(window, cx);
            }
            "up" => {
                self.palette_selected = self.palette_selected.saturating_sub(1);
                cx.notify();
            }
            "down" => {
                let n = self.palette_results.len();
                if n > 0 {
                    self.palette_selected = (self.palette_selected + 1).min(n - 1);
                }
                cx.notify();
            }
            "enter" => {
                if let Some(r) = self.palette_results.get(self.palette_selected) {
                    if let Some(cmd) = COMMANDS.get(r.cmd_index) {
                        let id = cmd.id;
                        self.run_palette_command(id, window, cx);
                    }
                }
            }
            "backspace" => {
                self.palette_query.pop();
                self.palette_selected = 0;
                self.repalette(cx);
                cx.notify();
            }
            _ => {
                if m.platform && key == "v" {
                    // ⌘V paste: append the clipboard text (best-effort; ignore a non-text item).
                    if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                        self.palette_query.push_str(&t);
                        self.palette_selected = 0;
                        self.repalette(cx);
                        cx.notify();
                    }
                } else if !m.platform && !m.control && !m.function {
                    // A printable key (shift is fine — it's part of the typed char). Append the
                    // character the platform resolved; modifier-only events carry no `key_char`.
                    if let Some(ch) = ks.key_char.as_ref() {
                        self.palette_query.push_str(ch);
                        self.palette_selected = 0;
                        self.repalette(cx);
                        cx.notify();
                    }
                } else {
                    // Let ⌘K (toggle) and other global shortcuts through — don't stop them.
                    return;
                }
            }
        }
        // Every branch we handled above stops here so the keystroke doesn't also drive a
        // global action (e.g. a bare letter that happens to match a binding).
        cx.stop_propagation();
    }
}
