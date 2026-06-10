//! The Settings page — rendered by `Shell` when the route is `Settings`.
//! Every control writes straight back into `self.settings` and calls `.save()`,
//! and theme changes apply live. This is the template for your own settings.

use gpui::{div, px, AnyElement, Context, FontWeight, IntoElement, ParentElement, Styled, Window};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    switch::Switch,
    v_flex, ActiveTheme,
};

use crate::settings::{Settings, ThemeModePref};
use crate::shell::Shell;

impl Shell {
    pub fn render_settings(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;

        let mode = self.settings.theme_mode;

        // One settings row: title + description on the left, a control on the right.
        let row = move |title: &str, desc: &str, control: AnyElement| {
            h_flex()
                .w_full()
                .py_3()
                .gap_4()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .text_color(foreground)
                                .child(title.to_string()),
                        )
                        .child(div().text_xs().text_color(muted).child(desc.to_string())),
                )
                .child(control)
        };

        // A bordered card that groups rows with separators.
        let card = move || {
            v_flex()
                .w_full()
                .px_4()
                .rounded_xl()
                .border_1()
                .border_color(border)
                .bg(surface)
        };

        // Theme mode: two buttons, the active one is `primary`.
        let mode_button = |id: &'static str, label: &'static str, value: ThemeModePref| {
            let button = Button::new(id)
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_mode(value, cx)));
            if mode == value {
                button.primary()
            } else {
                button.ghost()
            }
        };
        let theme_control = h_flex()
            .gap_1()
            .child(mode_button("mode-dark", "Dark", ThemeModePref::Dark))
            .child(mode_button("mode-light", "Light", ThemeModePref::Light))
            .into_any_element();

        let name_control = Input::new(&self.name_input).w(px(220.0)).into_any_element();
        let rpc_control = Input::new(&self.rpc_input).w(px(260.0)).into_any_element();
        let watch_control = Input::new(&self.watch_input)
            .w(px(260.0))
            .into_any_element();

        let launch_control = Switch::new("launch-min")
            .checked(self.settings.launch_minimized)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.settings.launch_minimized = *checked;
                this.settings.save();
                cx.notify();
            }))
            .into_any_element();

        // Privacy: the mask is security-relevant, so its switch is allowed to read amber
        // (DESIGN §Toggle). It mirrors `self.mask` (the live, persisted state).
        let mask_control = Switch::new("mask-balances")
            .checked(self.mask)
            .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_mask(*checked, cx)))
            .into_any_element();
        let capture_control = Switch::new("capture-block")
            .checked(self.settings.capture_block)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.settings.capture_block = *checked;
                this.settings.save();
                cx.notify();
            }))
            .into_any_element();

        v_flex().flex_1().items_center().p_8().child(
            v_flex()
                .w(px(540.0))
                .gap_6()
                .child(section_label("Appearance", muted))
                .child(card().child(row("Theme", "Light or dark interface", theme_control)))
                .child(section_label("Privacy", muted))
                .child(
                    card()
                        .child(row(
                            "Mask balances",
                            "Hide every balance behind fixed bullets — persists until you turn it off (⌘⇧M, or click the Total)",
                            mask_control,
                        ))
                        .child(divider(border))
                        .child(row(
                            "Block screen capture",
                            "While masked, remove Deckard's windows from screen recordings (macOS, tray build) — off for demos",
                            capture_control,
                        )),
                )
                .child(section_label("Network", muted))
                .child(
                    card()
                        .child(row(
                            "Custom RPC",
                            "Bring your own Ethereum RPC — blank uses the bundled default",
                            rpc_control,
                        ))
                        .child(divider(border))
                        .child(row(
                            "Watch address",
                            "View any address or ENS read-only — blank shows your wallet",
                            watch_control,
                        )),
                )
                .child(section_label("Profile", muted))
                .child(
                    card()
                        .child(row(
                            "Display name",
                            "Used to greet you on the home screen",
                            name_control,
                        ))
                        .child(divider(border))
                        .child(row(
                            "Start in menu bar",
                            "Launch minimized (needs the `tray` feature)",
                            launch_control,
                        )),
                )
                .child(div().pt_2().text_xs().text_color(muted).child(format!(
                    "Preferences are stored at {}",
                    Settings::config_path_display()
                ))),
        )
    }
}

fn section_label(title: &str, color: gpui::Hsla) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(title.to_uppercase())
}

fn divider(color: gpui::Hsla) -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(color)
}
