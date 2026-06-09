//! Onboarding + unlock — the auth gate that wraps the app until the keystore is
//! unlocked. Implements the create / import / migrate / unlock flows defined in
//! `specs/keystore-design.md`: mandatory passphrase, hold-to-reveal recovery phrase,
//! confirm-a-subset backup, and the (Phase-2) Touch ID affordance shown disabled.

use gpui::{
    div, px, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, IconName, TitleBar,
};

use crate::settings::ThemeModePref;
use crate::shell::{AuthStep, Shell};
use crate::APP_NAME;

impl Shell {
    /// A minimal title bar for the auth screens: the app name + a theme toggle.
    pub fn render_auth_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
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
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(APP_NAME),
                )
                .child(
                    Button::new("auth-theme")
                        .ghost()
                        .icon(theme_icon)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx))),
                ),
        )
    }

    /// Dispatch to the active auth step.
    pub fn render_auth(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let inner = match self.auth {
            AuthStep::Choose => self.render_choose(cx).into_any_element(),
            AuthStep::CreateSetup => self.render_create_setup(cx).into_any_element(),
            AuthStep::CreateBackup => self.render_create_backup(cx).into_any_element(),
            AuthStep::Import => self.render_import(cx).into_any_element(),
            AuthStep::Migrate => self.render_migrate(cx).into_any_element(),
            AuthStep::Unlock => self.render_unlock(cx).into_any_element(),
            AuthStep::Ready => div().into_any_element(),
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(div().w(px(460.0)).child(inner))
    }

    // --- screens ---

    fn render_choose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_6()
            .child(self.auth_heading(
                "Welcome to Deckard",
                "A self-custodial Ethereum wallet. Your keys are generated and encrypted on this device — they never leave it.",
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        Button::new("create")
                            .primary()
                            .w_full()
                            .label("Create a new wallet")
                            .on_click(cx.listener(|this, _, _, cx| this.start_create(cx))),
                    )
                    .child(
                        Button::new("import")
                            .ghost()
                            .w_full()
                            .label("Import an existing wallet")
                            .on_click(cx.listener(|this, _, _, cx| this.start_import(cx))),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Open source · AGPL-3.0 · no telemetry"),
            )
    }

    fn render_create_setup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy_label = if self.auth_busy {
            "Encrypting…"
        } else {
            "Continue"
        };
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Set a passphrase",
                "This encrypts your wallet at rest with Argon2id + XChaCha20-Poly1305. You'll enter it each time you open Deckard.",
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.create_pass).w_full())
                    .child(Input::new(&self.create_pass2).w_full()),
            )
            .child(self.touch_id_note(cx))
            .child(self.error_line(cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("back")
                            .ghost()
                            .label("Back")
                            .on_click(cx.listener(|this, _, _, cx| this.auth_back_to_choose(cx))),
                    )
                    .child(
                        Button::new("continue")
                            .primary()
                            .label(busy_label)
                            .on_click(cx.listener(|this, _, _, cx| this.do_create(cx))),
                    ),
            )
    }

    fn render_create_backup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, surface, fg, muted) = {
            let t = cx.theme();
            (t.border, t.secondary, t.foreground, t.muted_foreground)
        };
        let busy_label = if self.auth_busy {
            "Saving…"
        } else {
            "Confirm & finish"
        };

        // The 12-word grid: each cell shows the word only while held-to-reveal.
        let words: Vec<String> = self
            .pending_phrase
            .as_ref()
            .map(|p| p.split_whitespace().map(|w| w.to_string()).collect())
            .unwrap_or_default();
        let mut grid = v_flex().w_full().gap_2();
        for (row_i, chunk) in words.chunks(3).enumerate() {
            let mut row = h_flex().w_full().gap_2();
            for (col_i, word) in chunk.iter().enumerate() {
                let n = row_i * 3 + col_i + 1;
                let shown = if self.reveal_seed {
                    word.clone()
                } else {
                    "••••••".to_string()
                };
                row = row.child(
                    h_flex()
                        .flex_1()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .child(div().text_xs().text_color(muted).child(format!("{n}")))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg)
                                .child(shown),
                        ),
                );
            }
            grid = grid.child(row);
        }

        // Press-and-hold to reveal; releasing (even off the button) re-hides.
        let reveal_btn = div()
            .id("hold-reveal")
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .w_full()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(surface)
            .text_sm()
            .text_color(fg)
            .child(if self.reveal_seed {
                "Release to hide"
            } else {
                "Hold to reveal"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.set_reveal_seed(true, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.set_reveal_seed(false, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.set_reveal_seed(false, cx)),
            );

        // The confirm prompt: 1-indexed word positions.
        let positions: Vec<String> = self
            .confirm_positions
            .iter()
            .map(|i| format!("#{}", i + 1))
            .collect();
        let prompt = format!("Confirm words {}", positions.join(", "));

        v_flex()
            .gap_4()
            .child(self.auth_heading(
                "Back up your recovery phrase",
                "Write these 12 words down and store them offline. Anyone with them controls your funds — Deckard can't recover them for you.",
                cx,
            ))
            .child(grid)
            .child(reveal_btn)
            .child(div().pt_1().text_sm().text_color(muted).child(prompt))
            .child(Input::new(&self.confirm_words).w_full())
            .child(self.error_line(cx))
            .child(
                Button::new("finish")
                    .primary()
                    .w_full()
                    .label(busy_label)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_backup(cx))),
            )
    }

    fn render_import(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy_label = if self.auth_busy {
            "Importing…"
        } else {
            "Import"
        };
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Import a wallet",
                "Paste a 12 or 24-word recovery phrase, or a 0x private key. It's encrypted on this device with your passphrase.",
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.import_secret).w_full())
                    .child(Input::new(&self.import_pass).w_full()),
            )
            .child(self.error_line(cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("back")
                            .ghost()
                            .label("Back")
                            .on_click(cx.listener(|this, _, _, cx| this.auth_back_to_choose(cx))),
                    )
                    .child(
                        Button::new("do-import")
                            .primary()
                            .label(busy_label)
                            .on_click(cx.listener(|this, _, _, cx| this.do_import(cx))),
                    ),
            )
    }

    fn render_migrate(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let busy_label = if self.auth_busy {
            "Encrypting…"
        } else {
            "Encrypt & continue"
        };
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Secure your existing wallet",
                "Deckard found a wallet stored without encryption from an earlier build. Set a passphrase to encrypt it now.",
                cx,
            ))
            .child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_xs()
                    .text_color(danger)
                    .child("This key was previously in plaintext on disk. For full safety, create a fresh wallet afterward and move your funds."),
            )
            .child(Input::new(&self.pass_input).w_full())
            .child(self.error_line(cx))
            .child(
                Button::new("do-migrate")
                    .primary()
                    .w_full()
                    .label(busy_label)
                    .on_click(cx.listener(|this, _, _, cx| this.do_migrate(cx))),
            )
    }

    fn render_unlock(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy_label = if self.auth_busy {
            "Unlocking…"
        } else {
            "Unlock"
        };
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Unlock Deckard",
                "Enter your passphrase to decrypt your wallet on this device.",
                cx,
            ))
            .child(Input::new(&self.pass_input).w_full())
            .child(self.touch_id_note(cx))
            .child(self.error_line(cx))
            .child(
                Button::new("do-unlock")
                    .primary()
                    .w_full()
                    .label(busy_label)
                    .on_click(cx.listener(|this, _, _, cx| this.do_unlock(cx))),
            )
    }

    // --- small shared pieces ---

    fn auth_heading(
        &self,
        title: &str,
        subtitle: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(subtitle.to_string()),
            )
    }

    /// The Phase-2 Touch ID affordance — shown disabled, with an honest reason.
    fn touch_id_note(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .size(px(16.0))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.muted_foreground),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Touch ID unlock — available after the signed build (Phase 2)"),
            )
    }

    /// A one-line error, or nothing.
    fn error_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().children(self.auth_error.as_ref().map(|e| {
            div()
                .text_sm()
                .text_color(theme.danger)
                .child(format!("⚠ {e}"))
        }))
    }
}
