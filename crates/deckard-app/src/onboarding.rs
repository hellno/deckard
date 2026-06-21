//! Onboarding + unlock — the auth gate that wraps the app until the keystore is
//! unlocked. Implements the create / import / migrate / unlock flows defined in
//! `specs/keystore-design.md`: mandatory passphrase, hold-to-reveal recovery phrase,
//! confirm-a-subset backup, and the (Phase-2) Touch ID affordance shown disabled.

use gpui::{
    div, px, relative, Context, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Disableable, IconName, TitleBar,
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
            AuthStep::CreateVerify => self.render_create_verify(cx).into_any_element(),
            AuthStep::CreateDone => self.render_create_done(cx).into_any_element(),
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
                "Your money on autopilot. You can see and stop everything.",
                "A self-custodial Ethereum wallet. Your keys are generated and encrypted on this device, and they never leave it.",
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
        let (amber, danger, warning, success, track, muted, fg) = {
            let t = cx.theme();
            (
                crate::theme::amber(t.is_dark()),
                t.danger,
                t.warning,
                t.success,
                t.border,
                t.muted_foreground,
                t.foreground,
            )
        };
        // Live strength meter — shown only once there's something to score. Length dominates; the
        // value never leaves the input (we read length + classes, never store or log it).
        let (frac, label) = passphrase_strength(&self.create_pass.read(cx).value());
        let meter = (frac > 0.0).then(|| {
            let bar_color = if frac < 0.5 {
                danger
            } else if frac < 0.72 {
                warning
            } else {
                success
            };
            v_flex()
                .w_full()
                .gap_1p5()
                .child(strength_bar(frac, bar_color, track))
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(crate::widgets::section_label("Passphrase strength", muted))
                        .child(div().text_xs().text_color(bar_color).child(label)),
                )
        });
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Set a passphrase",
                "You'll enter this each time you open Deckard. Choose something long you'll remember — length matters more than symbols.",
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.create_pass).w_full())
                    .child(Input::new(&self.create_pass2).w_full()),
            )
            .children(meter)
            .child(crate::widgets::caution_line(
                amber,
                fg,
                false,
                "If you forget it, no one can reset it — not even us.",
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Encrypted at rest with Argon2id + XChaCha20-Poly1305."),
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
                        Button::new("continue")
                            .primary()
                            .label(busy_label)
                            .on_click(cx.listener(|this, _, _, cx| this.do_create(cx))),
                    ),
            )
    }

    /// Step 1 of backup: reveal the phrase (read-only). Verification is the *next*, separate step
    /// (DESIGN §Onboarding) — there's no quiz input on this screen.
    fn render_create_backup(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (border, surface, fg, muted, amber) = {
            let t = cx.theme();
            (
                t.border,
                t.secondary,
                t.foreground,
                t.muted_foreground,
                crate::theme::amber(t.is_dark()),
            )
        };

        // The 12-word grid: each cell shows the word only while held-to-reveal; otherwise dots.
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

        // Press-and-hold to reveal; releasing (even off the button) re-hides, and it auto-hides
        // after `SEED_REVEAL_TIMEOUT` even while held.
        let reveal_btn = div()
            .id("hold-reveal")
            .flex()
            .items_center()
            .justify_center()
            .gap_2()
            .flex_1()
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

        // The demoted Copy — explicit click only, never auto-copied (DESIGN §Seed reveal).
        let copy_btn = Button::new("copy-phrase")
            .ghost()
            .label(if self.seed_copied {
                "Copied ✓"
            } else {
                "Copy"
            })
            .on_click(cx.listener(|this, _, _, cx| this.copy_recovery_phrase(cx)));

        v_flex()
            .gap_4()
            .child(self.auth_heading(
                "Back up your recovery phrase",
                "These 12 words are the only way to restore your wallet. Write them down and store them offline — anyone who has them controls your funds, and Deckard can't recover them for you.",
                cx,
            ))
            .child(grid)
            .child(crate::widgets::caution_line(
                amber,
                fg,
                false,
                "Make sure nobody can see your screen before you reveal.",
            ))
            .child(h_flex().w_full().gap_2().child(reveal_btn).child(copy_btn))
            .child(
                Button::new("written-down")
                    .primary()
                    .w_full()
                    .label("I've written it down")
                    .on_click(cx.listener(|this, _, _, cx| this.advance_to_verify(cx))),
            )
    }

    /// Step 2 of backup: verify by retyping requested words, with the grid hidden. The primary
    /// stays disabled until the typed words match (DESIGN §Onboarding).
    fn render_create_verify(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let busy_label = if self.auth_busy {
            "Saving…"
        } else {
            "Confirm & finish"
        };
        let matches = self.backup_words_match(cx);

        // The prompt: 1-indexed word positions.
        let positions: Vec<String> = self
            .confirm_positions
            .iter()
            .map(|i| format!("#{}", i + 1))
            .collect();
        let prompt = format!("Enter words {}", positions.join(", "));

        v_flex()
            .gap_4()
            .child(self.auth_heading(
                "Verify your backup",
                "Type the words below to confirm you saved them. Your recovery phrase is hidden now.",
                cx,
            ))
            .child(div().pt_1().text_sm().text_color(muted).child(prompt))
            .child(Input::new(&self.confirm_words).w_full())
            .child(self.error_line(cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Button::new("back-to-backup")
                            .ghost()
                            .label("Back")
                            .on_click(cx.listener(|this, _, _, cx| this.back_to_backup(cx))),
                    )
                    .child(
                        Button::new("finish")
                            .primary()
                            .label(busy_label)
                            .disabled(self.auth_busy || !matches)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_backup(cx))),
                    ),
            )
    }

    /// The "you're ready" interstitial: the vault is sealed + unlocked; the user steps into the
    /// live app deliberately (DESIGN §Onboarding: "Ready — a real screen").
    fn render_create_done(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (muted, fg, slate) = {
            let t = cx.theme();
            (
                t.muted_foreground,
                t.foreground,
                crate::theme::identity_square(t.is_dark()),
            )
        };
        let addr = self.wallet_address_string();
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Your wallet is ready",
                "It's encrypted on this device. Only your passphrase can open it — and your recovery phrase can restore it.",
                cx,
            ))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(crate::widgets::identity_mark(&addr, px(22.0), px(6.0), slate, fg))
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_sm()
                            .text_color(muted)
                            .child(crate::widgets::short_addr(&addr)),
                    ),
            )
            .child(
                Button::new("enter-deckard")
                    .primary()
                    .w_full()
                    .label("Enter Deckard")
                    .on_click(cx.listener(|this, _, _, cx| this.enter_after_create(cx))),
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
            .child(crate::widgets::error_line(
                danger,
                "This key was previously in plaintext on disk. For full safety, create a fresh wallet afterward and move your funds.",
            ))
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
                    .child("Touch ID unlock: available after the signed build (Phase 2)"),
            )
    }

    /// A one-line error, or nothing.
    fn error_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        div().children(
            self.auth_error
                .as_ref()
                .map(|e| crate::widgets::error_line(danger, e.clone())),
        )
    }
}

/// A calm, advisory passphrase-strength estimate as `(fill 0–1, one-word label)`. Length is the
/// dominant factor; using more character classes nudges it up within a band. Advisory only — the
/// real defence is the Argon2id KDF — and deliberately dictionary-free so it pulls in no new
/// dependency. Reads only the length + which classes are present; never stores or logs the value.
fn passphrase_strength(pass: &str) -> (f32, &'static str) {
    let len = pass.chars().count();
    if len == 0 {
        return (0.0, "");
    }
    let lower = pass.chars().any(|c| c.is_lowercase());
    let upper = pass.chars().any(|c| c.is_uppercase());
    let digit = pass.chars().any(|c| c.is_ascii_digit());
    let symbol = pass.chars().any(|c| !c.is_alphanumeric());
    let classes = [lower, upper, digit, symbol].iter().filter(|&&b| b).count();
    let len_score = match len {
        0..=7 => 0.18,
        8..=11 => 0.46,
        12..=15 => 0.70,
        16..=19 => 0.88,
        _ => 1.0,
    };
    let variety_bonus = classes.saturating_sub(1) as f32 * 0.05;
    let frac = (len_score + variety_bonus).min(1.0);
    let label = if len < 8 {
        "Too short"
    } else if frac < 0.5 {
        "Weak"
    } else if frac < 0.72 {
        "Fair"
    } else if frac < 0.9 {
        "Good"
    } else {
        "Strong"
    };
    (frac, label)
}

/// The thin strength bar: a 4px track with a fill whose width tracks `frac` and whose color is
/// chosen by the caller (danger → warning → success as strength rises). Mirrors the
/// [`crate::widgets::budget_gauge`] track/fill shape, inverted in meaning.
fn strength_bar(frac: f32, fill: Hsla, track: Hsla) -> impl IntoElement {
    div().w_full().h(px(4.0)).rounded(px(2.0)).bg(track).child(
        div()
            .h(px(4.0))
            .w(relative(frac.clamp(0.06, 1.0)))
            .rounded(px(2.0))
            .bg(fill),
    )
}

#[cfg(test)]
mod tests {
    use super::passphrase_strength;

    #[test]
    fn empty_passphrase_is_unscored() {
        let (frac, label) = passphrase_strength("");
        assert_eq!(frac, 0.0);
        assert_eq!(label, "");
    }

    #[test]
    fn below_minimum_length_reads_too_short() {
        // Under the 8-char floor `do_create` enforces — the meter must say so, never "Weak".
        let (frac, label) = passphrase_strength("short");
        assert!(frac < 0.5);
        assert_eq!(label, "Too short");
    }

    #[test]
    fn length_dominates_the_band() {
        // Same single class (lowercase), longer → strictly stronger.
        let (weak, _) = passphrase_strength("abcdefgh"); // 8
        let (fair, _) = passphrase_strength("abcdefghijkl"); // 12
        let (good, _) = passphrase_strength("abcdefghijklmnop"); // 16
        assert!(weak < fair, "8 < 12 chars");
        assert!(fair < good, "12 < 16 chars");
    }

    #[test]
    fn variety_lifts_within_a_length() {
        // Same length (12), more character classes → higher score + a better label.
        let (plain, plain_label) = passphrase_strength("abcdefghijkl");
        let (varied, varied_label) = passphrase_strength("Abcdefgh1jkl");
        assert!(varied > plain);
        assert_eq!(plain_label, "Fair");
        assert_eq!(varied_label, "Good");
    }

    #[test]
    fn long_and_varied_reads_strong() {
        let (frac, label) = passphrase_strength("Abcdefghijklmnop1!");
        assert!(frac >= 0.9);
        assert_eq!(label, "Strong");
    }

    #[test]
    fn frac_never_exceeds_one() {
        // The variety bonus must never push the bar past a full track.
        let (frac, _) = passphrase_strength("Abcdefghijklmnopqrstuvwxyz0123456789!@#");
        assert!(frac <= 1.0);
    }
}
