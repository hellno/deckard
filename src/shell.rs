//! Shell — the single root view. Owns the persisted `Settings`, the current
//! route (Welcome vs Settings), and a stateful text input. It renders the title
//! bar plus whichever page is active. The page bodies live in `welcome.rs` and
//! `settings_view.rs` as `impl Shell` methods (Rust lets you split an inherent
//! impl across modules), so this file stays focused on state + navigation.

use gpui::{
    div, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    v_flex, ActiveTheme, IconName, TitleBar,
};

use crate::settings::{Settings, ThemeModePref};
use crate::theme::{self, Accent};
use crate::{GoBack, NewItem, OpenSettings, ToggleTheme, APP_NAME};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Welcome,
    Settings,
}

pub struct Shell {
    pub focus_handle: FocusHandle,
    pub route: Route,
    pub settings: Settings,
    pub name_input: Entity<InputState>,
    pub created: usize,
}

impl Shell {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Your name")
                .default_value(settings.display_name.clone())
        });

        // Persist the text field as the user types (and on blur).
        cx.subscribe(&name_input, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change | InputEvent::Blur) {
                this.settings.display_name = state.read(cx).value().to_string();
                this.settings.save();
            }
        })
        .detach();

        Self {
            focus_handle,
            route: Route::Welcome,
            settings,
            name_input,
            created: 0,
        }
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.route = route;
        cx.notify();
    }

    /// Re-install the theme from the current settings (accent + mode).
    fn apply_theme(&self, cx: &mut Context<Self>) {
        theme::install(cx, self.settings.accent, self.settings.theme_mode.to_gpui());
    }

    pub fn set_accent(&mut self, accent: Accent, cx: &mut Context<Self>) {
        self.settings.accent = accent;
        self.settings.save();
        self.apply_theme(cx);
        // Keep the menu-bar tray icon (if running) in sync with the accent.
        #[cfg(feature = "tray")]
        crate::tray::set_accent(cx, accent);
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: ThemeModePref, cx: &mut Context<Self>) {
        self.settings.theme_mode = mode;
        self.settings.save();
        self.apply_theme(cx);
        cx.notify();
    }

    pub fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.settings.theme_mode {
            ThemeModePref::Dark => ThemeModePref::Light,
            ThemeModePref::Light => ThemeModePref::Dark,
        };
        self.set_mode(next, cx);
    }

    // --- Action handlers (wired in `render`) ---

    fn on_new_item(&mut self, _: &NewItem, _: &mut Window, cx: &mut Context<Self>) {
        self.created += 1;
        cx.notify();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, cx);
    }

    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.route != Route::Welcome {
            self.navigate(Route::Welcome, cx);
        }
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mode(cx);
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let is_settings = self.route == Route::Settings;
        let theme_icon = if self.settings.theme_mode == ThemeModePref::Dark {
            IconName::Sun
        } else {
            IconName::Moon
        };

        TitleBar::new().child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .children(is_settings.then(|| {
                            Button::new("back")
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.navigate(Route::Welcome, cx)),
                                )
                        }))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(muted)
                                .child(if is_settings { "Settings" } else { APP_NAME }),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .children((!is_settings).then(|| {
                            Button::new("open-settings")
                                .ghost()
                                .icon(IconName::Settings)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.navigate(Route::Settings, cx)
                                    }),
                                )
                        }))
                        .child(
                            Button::new("toggle-theme")
                                .ghost()
                                .icon(theme_icon)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx))),
                        ),
                ),
        )
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let title_bar = self.render_title_bar(cx);
        let content = match self.route {
            Route::Welcome => self.render_welcome(cx).into_any_element(),
            Route::Settings => self.render_settings(window, cx).into_any_element(),
        };

        v_flex()
            .size_full()
            .bg(background)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_item))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_toggle_theme))
            .child(title_bar)
            .child(content)
    }
}
