//! Shell chrome — the two-pane shell's hand-built furniture (DESIGN §Information
//! architecture): the 248px sidebar tree, the 44px breadcrumb top bar, and the
//! 25px bottom status strip, plus the shared neutral network pill.
//!
//! These are deliberately NOT gpui-component's heavyweight `Sidebar`/`Breadcrumb`
//! components — the demo scope is a single project / wallet, so the tree is a plain
//! `v_flex` of rows. Color law (DESIGN §Color): ~95% grayscale; the selected row is a
//! brightness lift (`secondary`), NEVER a colored keyline; amber is reserved for
//! Receive's keyline + focus rings; cyan appears ONLY on the agent squircle glyph
//! (in the wallet home's policy fence + the activity feed).

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    Pixels, StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable,
};

use crate::money::mask_money;
use crate::settings::ThemeModePref;
use crate::shell::{Selection, Shell, Surface};
use crate::theme;

/// Middle-truncate an address for a tight row, e.g. `0xA1b2…9F3c`. Re-exported from
/// [`crate::widgets`] so existing call sites (`crate::shell_chrome::short_addr`) keep working
/// while the single canonical definition lives in the shared widget vocabulary.
pub(crate) use crate::widgets::short_addr;

/// The agent's cyan squircle monogram — the ONE cyan surface (DESIGN §Actor model): a
/// rounded square (NEVER `rounded_full`) with the "A" monogram. Always static — the cyan
/// glyph is the two-signal identity marker; it carries no pulse or ambient motion. Shared
/// by the sidebar row, the wallet-home fence header, the palette, and the activity feed.
pub(crate) fn agent_squircle(
    size: Pixels,
    radius: Pixels,
    agent: Hsla,
    agent_tint: Hsla,
) -> AnyElement {
    div()
        .size(size)
        .rounded(radius)
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
        )
        .into_any_element()
}

impl Shell {
    /// The current view's human label, for the breadcrumb's trailing segment.
    fn view_label(&self) -> &'static str {
        match self.surface {
            Surface::Settings => "Settings",
            Surface::Receive => "Receive",
            Surface::Send => "Send",
            Surface::Shield => "Shield",
            Surface::Activity => "Activity",
            Surface::Swap => "Swap",
            Surface::Home => match self.selection {
                Selection::Project => "Personal",
                Selection::Wallet => "Wallet",
            },
        }
    }

    /// The hand-built sidebar tree: a PROJECTS label, one project row, a Wallets
    /// group + one wallet row, a flex spacer, an Activity ledger row, and a footer
    /// gear that opens Settings. Neutral throughout. (Atlas is key-less automation ON
    /// the wallet — same EOA — so it lives in the wallet home's policy fence, not as a
    /// separate sidebar entity.)
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

        let addr = short_addr(&self.wallet_address_string());
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
            .w(px(248.0))
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
                                px(16.0),
                                px(4.0),
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
                                        &addr,
                                        px(16.0),
                                        px(4.0),
                                        id_square,
                                        fg,
                                    ))
                                    .child(
                                        div()
                                            .font_family(mono.clone())
                                            .text_xs()
                                            .text_color(fg)
                                            .child(addr),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .text_color(muted)
                                    .child(balance),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.select(Selection::Wallet, cx))),
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

    /// The 44px breadcrumb bar: `[identity square] Personal › <view>` on the left,
    /// and the neutral network pill + ⌘K affordance + theme toggle on the right
    /// (the controls lifted out of the old title bar).
    pub fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let id_square = theme::identity_square(theme.is_dark());
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
                    .child(crate::widgets::identity_mark(
                        "Personal",
                        px(16.0),
                        px(4.0),
                        id_square,
                        fg,
                    ))
                    .child(div().text_sm().text_color(fg).child("Personal"))
                    // Skip the trailing "› <view>" when it would just repeat the project name
                    // (Project Home's label is "Personal" → avoid "Personal › Personal").
                    .when(
                        !(self.surface == Surface::Home && self.selection == Selection::Project),
                        |el| {
                            el.child(div().text_sm().text_color(muted).child("›"))
                                .child(div().text_sm().text_color(fg).child(self.view_label()))
                        },
                    ),
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
            .h(px(25.0))
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
