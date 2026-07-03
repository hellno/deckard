//! Shell chrome — the two-pane shell's hand-built furniture (DESIGN §Information
//! architecture): the 248px sidebar tree, the 44px breadcrumb top bar, and the
//! 25px bottom status strip, plus the shared neutral network pill.
//!
//! These are deliberately NOT gpui-component's heavyweight `Sidebar`/`Breadcrumb`
//! components — the demo scope is a single project / wallet, so the tree is a plain
//! `v_flex` of rows. Color law (DESIGN §Color): ~95% grayscale; the selected row is a
//! brightness lift (`secondary`), NEVER a colored keyline; amber is reserved for
//! Receive's keyline + focus rings; cyan appears ONLY on the agent mark (the cyan
//! squircle, `widgets::agent_mark`) — the sidebar/breadcrumb agent, its surface, the feed.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use crate::money::mask_money;
use crate::settings::ThemeModePref;
use crate::shell::{Selection, Shell, Surface};
use crate::theme;

impl Shell {
    /// The trailing view segment of the breadcrumb, for a full-pane action surface opened over the
    /// selected wallet (`Meridian › Send`). `Home` has no trailing segment — its breadcrumb names
    /// the focused entity alone (see [`Shell::breadcrumb_entity`]).
    fn view_label(&self) -> &'static str {
        match self.surface {
            Surface::Settings => "Settings",
            Surface::Receive => "Receive",
            Surface::Send => "Send",
            Surface::Shield => "Shield",
            Surface::Activity => "Activity",
            Surface::Swap => "Swap",
            Surface::Home => "",
        }
    }

    /// The entity the breadcrumb names (E2, #182): the agent handle on the agent home, "Personal"
    /// on the project home, otherwise the wallet's name — the wallet is the entity every action
    /// surface (Send/Receive/Shield/Swap/Settings) acts on. Drops the old `Personal ›` prefix and
    /// the literal word Wallet. `is_agent` picks the cyan agent mark over the neutral identity mark.
    fn breadcrumb_entity(&self) -> (String, bool) {
        match (self.surface, self.selection) {
            (Surface::Home, Selection::Agent) => (self.agent_handle(), true),
            (Surface::Home, Selection::Project) => ("Personal".to_string(), false),
            _ => (self.wallet_name(), false),
        }
    }

    /// The hand-built sidebar tree: a PROJECTS label, one project row, a Wallets group + one
    /// named wallet row, an Agents group + the first-class agent row (its cyan `agent_mark` +
    /// handle + status), a flex spacer, an Activity ledger row, and a footer gear that opens
    /// Settings. Neutral throughout except the agent's cyan mark.
    pub fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let lift = theme.secondary; // selected/active = brightness lift (not a keyline)
        let mono = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();
        let id_square = theme::identity_square(is_dark);
        let amber = theme::amber(is_dark);
        let amber_tint = theme::amber_tint(is_dark);
        // The live "needs you" count (amber = "awaiting you") — the still-proposed rows in the
        // Activity feed, which form its NEEDS YOU triage band. Surfaced on the Activity nav row.
        let needs_you_count = crate::activity_view::activity_pending(&self.activity).len();
        let activity_active = self.surface == Surface::Activity;

        let project_selected =
            self.surface == Surface::Home && self.selection == Selection::Project;
        let wallet_selected = self.surface == Surface::Home && self.selection == Selection::Wallet;
        let agent_selected = self.surface == Surface::Home && self.selection == Selection::Agent;
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        // Identity is named (E2, #182): the sidebar names the wallet + agent, not a raw address.
        let wallet_name = self.wallet_name();
        let agent_handle = self.agent_handle();
        let balance = self
            .portfolio
            .as_ref()
            .map(|p| mask_money(self.mask, &deckard_core::format_amount(p.native_wei, 18, 4)))
            .unwrap_or_else(|| "—".to_string());

        // A tiny uppercase section label (DESIGN §Typography label tier) via the shared
        // `section_label` widget; the wrapper carries only the row's padding.
        let group_label = |text: &'static str| {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .child(crate::widgets::section_label(text, muted))
        };

        v_flex()
            .w(crate::tokens::SIDEBAR_W)
            .flex_shrink_0()
            .h_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(border)
            // PROJECTS header.
            .child(group_label("PROJECTS"))
            // Project row.
            .child(
                div()
                    .id("nav-project")
                    .mx_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .when(project_selected, |e| e.bg(lift))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(crate::widgets::identity_mark(
                                "Personal",
                                crate::tokens::MARK_SM,
                                crate::tokens::RADIUS_ROW,
                                id_square,
                                fg,
                            ))
                            .child(div().text_sm().text_color(fg).child("Personal")),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.select(Selection::Project, cx))),
            )
            // Wallets group.
            .child(group_label("Wallets"))
            .child(
                div()
                    .id("nav-wallet")
                    .mx_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .when(wallet_selected, |e| e.bg(lift))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(crate::widgets::identity_mark(
                                        &wallet_name,
                                        crate::tokens::MARK_SM,
                                        crate::tokens::RADIUS_ROW,
                                        id_square,
                                        fg,
                                    ))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .text_color(fg)
                                            .child(wallet_name),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .text_color(muted)
                                    .child(balance),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.select(Selection::Wallet, cx))),
            )
            // Agents group — first-class (DESIGN.md v2 §The agent interaction model): the agent is
            // its own entity with its own surface, no longer folded into the wallet home.
            .child(group_label("Agents"))
            .child(
                div()
                    .id("nav-agent")
                    .mx_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .when(agent_selected, |e| e.bg(lift))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(crate::widgets::agent_mark(
                                &agent_handle,
                                crate::tokens::MARK_SM,
                                crate::tokens::RADIUS_ROW,
                                agent,
                                agent_tint,
                            ))
                            .child(div().text_sm().text_color(fg).child(agent_handle))
                            .child(div().flex_1())
                            .child(div().size(px(6.0)).rounded_full().bg(agent)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.select(Selection::Agent, cx))),
            )
            // Spacer pushes the footer rows to the bottom.
            .child(div().flex_1())
            // Activity ledger — a sibling of Settings (bottom): the full cross-agent record AND
            // the triage queue for what still needs you (its NEEDS YOU band). A live amber count
            // badge ("awaiting you") surfaces the still-proposed rows; ⌘⇧A summons it from anywhere.
            .child(
                div()
                    .id("nav-activity")
                    .mx_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .when(activity_active, |e| e.bg(lift))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(div().text_sm().text_color(muted).child("Activity"))
                            .when(needs_you_count > 0, |row| {
                                row.child(
                                    div()
                                        .px_1p5()
                                        .rounded_md()
                                        .bg(amber_tint)
                                        .text_xs()
                                        .text_color(amber)
                                        .child(format!("{needs_you_count}")),
                                )
                            }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_activity(window, cx))),
            )
            // Footer gear → Settings.
            .child(
                div()
                    .id("nav-settings")
                    .mx_2()
                    .mb_2()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .when(self.surface == Surface::Settings, |e| e.bg(lift))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::Settings).text_color(muted))
                            .child(div().text_sm().text_color(muted).child("Settings")),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open(Surface::Settings, cx))),
            )
    }

    /// The 44px breadcrumb bar: `[mark] <entity>` on the left — the entity the current view is
    /// about (`Meridian`, the agent `Kyoto`, or `Personal`), plus `› <view>` on an action surface
    /// (`Meridian › Send`). Identity is named (E2, #182): no `Personal ›` prefix, no literal Wallet.
    /// The neutral network pill + ⌘K affordance + theme toggle sit on the right.
    pub fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let is_dark = theme.is_dark();
        let id_square = theme::identity_square(is_dark);
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        // The focused entity + its mark (cyan agent mark vs. neutral identity mark). On an action
        // surface the trailing `› <view>` is appended; Home shows the entity name alone.
        let (entity_name, is_agent_entity) = self.breadcrumb_entity();
        let entity_mark = if is_agent_entity {
            crate::widgets::agent_mark(
                &entity_name,
                crate::tokens::MARK_MD,
                crate::tokens::RADIUS_ROW,
                agent,
                agent_tint,
            )
        } else {
            crate::widgets::identity_mark(
                &entity_name,
                crate::tokens::MARK_MD,
                crate::tokens::RADIUS_ROW,
                id_square,
                fg,
            )
        };
        let on_home = self.surface == Surface::Home;
        let theme_icon = if self.settings.theme_mode == ThemeModePref::Dark {
            IconName::Sun
        } else {
            IconName::Moon
        };
        // The eye glyph reflects current state: slashed eye = balances hidden.
        let mask_icon = if self.mask {
            IconName::EyeOff
        } else {
            IconName::Eye
        };

        h_flex()
            .h(px(44.0))
            .flex_shrink_0()
            .w_full()
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(entity_mark)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(fg)
                            .child(entity_name),
                    )
                    // An action surface (Send/Receive/Shield/Swap/Activity/Settings) appends
                    // `› <view>`; Home names the entity alone.
                    .when(!on_home, |el| {
                        el.child(div().flex_shrink_0().text_sm().text_color(muted).child("›"))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_sm()
                                    .text_color(fg)
                                    .child(self.view_label()),
                            )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(self.network_pill(cx))
                    .child(
                        // ⌘K affordance — opens the command palette.
                        div()
                            .id("breadcrumb-cmdk")
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .text_xs()
                            .text_color(muted)
                            .cursor_pointer()
                            .child("⌘K")
                            // Route through the shared toggle so the breadcrumb opens the palette
                            // exactly like ⌘K does — capturing focus, recomputing results, and
                            // focusing the panel (a bare `palette_open = true` renders it inert).
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_palette(window, cx);
                            })),
                    )
                    .child(
                        // Eye glyph → toggle the privacy mask (⌘⇧M / click-the-Total /
                        // palette all route to the same `toggle_mask`).
                        Button::new("toggle-mask")
                            .ghost()
                            .icon(mask_icon)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mask(cx))),
                    )
                    .child(
                        Button::new("toggle-theme")
                            .ghost()
                            .icon(theme_icon)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx))),
                    ),
            )
    }

    /// The 25px bottom status strip: the synced-block + trust label on the left
    /// (migrated out of welcome.rs), the network name on the right. A balance is
    /// never shown as quietly trusted — a non-Verified read surfaces here.
    pub fn render_status_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;

        let first_sync = self.portfolio_loading && self.portfolio.is_none();

        // Trust label suffix (DESIGN: never silently "trusted").
        let trust_tag = match &self.read_status {
            Some(deckard_core::ReadStatus::Verified) => " · verified",
            Some(deckard_core::ReadStatus::Degraded { .. }) => " · degraded",
            Some(deckard_core::ReadStatus::Unsynced { .. }) => " · NOT VERIFIED",
            None => "",
        };
        // A read failure is the loud danger register: the shared `error_line` widget (Lucide
        // TriangleAlert + danger text), with the raw provider error humanized to one calm line.
        let error_el = self.portfolio_error.as_ref().map(|err| {
            crate::widgets::error_line(theme.danger, crate::errors::humanize_read_error(err))
        });

        let status_line = if first_sync {
            "Syncing over Ethereum…".to_string()
        } else if let Some(block) = self.synced_block {
            let watching = if self.viewing_watch {
                "watching · "
            } else {
                ""
            };
            format!("{watching}synced · block {block}{trust_tag}")
        } else {
            "Ethereum mainnet".to_string()
        };

        // An unverified read is a soft warning (not trustless), not a hard error.
        let unverified = matches!(
            self.read_status,
            Some(deckard_core::ReadStatus::Unsynced { .. })
        );
        let status_color = if unverified { theme.warning } else { muted };

        // An active shield's lifecycle (the "where's my money?" reassurance line). The glyph
        // token maps to a small colored dot: amber in-flight, success done, danger failed.
        let shield_chip = self.shield_status.as_ref().map(|st| {
            let token = st.glyph();
            let color = match token {
                "check-filled" => theme.success,
                "x-ring" => theme.danger,
                _ => theme::amber(theme.is_dark()),
            };
            // DESIGN status-as-glyph: filled check / x-ring from the icon kit; pending has no
            // clock icon in the kit, so it's a small amber dot.
            let glyph = match token {
                "check-filled" => Icon::new(IconName::CircleCheck)
                    .text_color(color)
                    .small()
                    .into_any_element(),
                "x-ring" => Icon::new(IconName::CircleX)
                    .text_color(color)
                    .small()
                    .into_any_element(),
                _ => div()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(color)
                    .into_any_element(),
            };
            h_flex()
                .items_center()
                .gap_1p5()
                .child(glyph)
                .child(div().text_color(color).child(st.to_string()))
        });

        // Fork-mode caution (DESIGN §Color rule 7 / Trust): an amber alert icon + the risk
        // text, inline — NO keyline or banner box. Per rule 7 the amber ICON carries the signal
        // and the text stays NEUTRAL (matching the canonical Receive network warning), so amber
        // keeps its <1% discipline. Surfaced only on a local dev fork so the operator can never
        // mistake demo funds for the real mainnet wallet.
        let fork_caution = self.fork_mode().then(|| {
            let amber = theme::amber(theme.is_dark());
            h_flex()
                .items_center()
                .gap_1p5()
                .child(Icon::new(IconName::TriangleAlert).text_color(amber).small())
                .child(
                    div()
                        .text_color(theme.foreground)
                        .child("Demo fork: not mainnet"),
                )
        });

        h_flex()
            .h(crate::tokens::STATUS_H)
            .flex_shrink_0()
            .w_full()
            .px_3()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(border)
            .text_xs()
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .children(fork_caution)
                    .children(shield_chip)
                    // A read failure renders the danger `error_line`; otherwise the calm status line.
                    .children(error_el)
                    .when(self.portfolio_error.is_none(), |row| {
                        row.child(div().text_color(status_color).child(status_line))
                    }),
            )
            .child(div().text_color(muted).child("Ethereum"))
    }

    /// The shared neutral network chip ("Ethereum"). Bordered, NOT amber (DESIGN
    /// §Color rule 4) and NOT a fully-rounded pill (rounded_md, ~6px).
    pub fn network_pill(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let border = theme.border;
        let surface = theme.secondary;

        h_flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_0p5()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(surface)
            .child(Icon::new(IconName::Globe).text_color(fg).small())
            .child(div().text_xs().text_color(fg).child("Ethereum"))
    }
}
