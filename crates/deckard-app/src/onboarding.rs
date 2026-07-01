//! Onboarding + unlock — the auth gate that wraps the app until the keystore is
//! unlocked. Implements the create / import / migrate / unlock flows defined in
//! `specs/keystore-design.md`: mandatory passphrase, hold-to-reveal recovery phrase,
//! confirm-a-subset backup, and the (Phase-2) Touch ID affordance shown disabled.

use gpui::{
    div, prelude::FluentBuilder, px, relative, Context, FontWeight, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
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
        // The create flow is a stepped sequence — show a small progress rail above the content so
        // the user always knows where they are. Only the four create steps are part of it.
        let active_step = match self.auth {
            AuthStep::CreateSetup => Some(0),
            AuthStep::CreateBackup => Some(1),
            AuthStep::CreateVerify => Some(2),
            AuthStep::CreateDone => Some(3),
            _ => None,
        };
        let column = v_flex()
            .w(crate::tokens::CONFIRM_W)
            .gap_6()
            .when_some(active_step, |c, step| {
                c.child(self.auth_step_rail(step, cx))
            });
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(column.child(inner))
    }

    /// The create-flow progress rail: the four step labels in a row, the active one in amber, the
    /// rest muted (DESIGN: amber only on the active step). Labels only — no dots or chrome.
    fn auth_step_rail(&self, active: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let (amber, muted) = {
            let t = cx.theme();
            (crate::theme::amber(t.is_dark()), t.muted_foreground)
        };
        let labels = ["Secure", "Back up", "Verify", "Ready"];
        let mut row = h_flex().w_full().items_center().gap_2();
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                row = row.child(div().text_xs().text_color(muted).child("·"));
            }
            let is_active = i == active;
            row = row.child(
                div()
                    .text_xs()
                    .when(is_active, |d| d.font_weight(FontWeight::MEDIUM))
                    .text_color(if is_active { amber } else { muted })
                    .child(*label),
            );
        }
        row
    }

    // --- screens ---

    fn render_choose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_6()
            .child(self.auth_heading(
                "Deckard, your new favorite wallet.",
                "A self-custodial Ethereum wallet. Your keys never leave this device.",
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
        // value never leaves the input (we read length + classes, never store or log it). The band
        // is the single source of truth for BOTH the colour and the word, so they can't drift.
        let (frac, band) = passphrase_strength(&self.create_pass.read(cx).value());
        let meter = (frac > 0.0).then(|| {
            let bar_color = band.fill(danger, warning, success);
            v_flex()
                .w_full()
                .gap_1p5()
                .child(strength_bar(frac, bar_color, track))
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(crate::widgets::section_label("Passphrase strength", muted))
                        .child(div().text_xs().text_color(bar_color).child(band.label())),
                )
        });
        // Reserve the meter's height from the start so nothing below shifts when it appears on the
        // first keystroke — the slot is always present, the bar+label fill it once there's input.
        let meter_slot = div().w_full().min_h(px(28.0)).children(meter);
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Set a passphrase",
                "You'll enter this each time you open Deckard. Choose something long you'll remember.",
                cx,
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(Input::new(&self.create_pass).w_full())
                    .child(Input::new(&self.create_pass2).w_full()),
            )
            .child(meter_slot)
            .child(crate::widgets::caution_line(
                amber,
                fg,
                false,
                "If you forget it, no one can reset it, not even us.",
            ))
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
                "These 12 words are the only way to restore your wallet. Write them down and keep them offline. Anyone who has them controls your funds.",
                cx,
            ))
            .child(grid)
            .child(crate::widgets::caution_line(
                amber,
                fg,
                false,
                "Make sure nobody can see your screen before you reveal.",
            ))
            // Reveal is the primary affordance (own full-width row); Copy is demoted to a centered
            // ghost action beneath it (DESIGN §Seed reveal: "Copy demoted below reveal").
            .child(reveal_btn)
            .child(h_flex().w_full().justify_center().child(copy_btn))
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
                "Type the words below to confirm you saved them.",
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
                            // Pinned during the seal/unlock write so the user can't navigate away
                            // mid-flight and get force-jumped when the result lands.
                            .disabled(self.auth_busy)
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
        let (muted, fg, slate, mono, success) = {
            let t = cx.theme();
            (
                t.muted_foreground,
                t.foreground,
                crate::theme::identity_square(t.is_dark()),
                t.mono_font_family.clone(),
                t.success,
            )
        };
        let addr = self.wallet_address_string();
        v_flex()
            .gap_5()
            .child(self.auth_heading(
                "Your wallet is ready",
                "Only your passphrase can open it. Your recovery phrase can restore it.",
                cx,
            ))
            // The canonical address treatment (identicon + mono short_addr) — the same widget
            // every other address row uses (DESIGN §Trust) — made one-click-copy with inline
            // "Copied ✓", since every address in the app is copyable.
            .child(
                h_flex()
                    .id("copy-ready-address")
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(crate::widgets::truncated_address(
                        &addr, None, mono, slate, fg, fg, muted,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if self.address_copied { success } else { muted })
                            .child(if self.address_copied {
                                "Copied ✓"
                            } else {
                                "Copy"
                            }),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.copy_wallet_address(cx))),
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
                "Paste a 12 or 24-word recovery phrase, or a 0x private key.",
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
                "Enter your passphrase to unlock your wallet.",
                cx,
            ))
            .child(Input::new(&self.pass_input).w_full())
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
        // The locked scale (DESIGN §Typography): screen-title H1 = `.text_xl` (20, the size every
        // other screen title uses), body/subtitle = `tokens::TEXT_BODY` (13). Onboarding was the
        // lone 22px outlier; v3 aligns it with the rest.
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title.to_string()),
            )
            .child(
                div()
                    .text_size(crate::tokens::TEXT_BODY)
                    .text_color(theme.muted_foreground)
                    .child(subtitle.to_string()),
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

/// One band of passphrase strength. The SINGLE source of truth for both the meter's word and its
/// colour — derived once in [`passphrase_strength`] so the label and the bar colour can never drift
/// apart (the threshold cut-points live in exactly one place).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PassBand {
    Empty,
    TooShort,
    Weak,
    Fair,
    Good,
    Strong,
}

impl PassBand {
    fn label(self) -> &'static str {
        match self {
            PassBand::Empty => "",
            PassBand::TooShort => "Too short",
            PassBand::Weak => "Weak",
            PassBand::Fair => "Fair",
            PassBand::Good => "Good",
            PassBand::Strong => "Strong",
        }
    }

    /// danger for too-short/weak, warning (amber) for fair, success (green) for good/strong.
    fn fill(self, danger: Hsla, warning: Hsla, success: Hsla) -> Hsla {
        match self {
            PassBand::Empty | PassBand::TooShort | PassBand::Weak => danger,
            PassBand::Fair => warning,
            PassBand::Good | PassBand::Strong => success,
        }
    }
}

/// A calm, advisory passphrase-strength estimate as `(fill 0–1, band)`. Length is the dominant
/// factor; using more character classes nudges it up within a band. Advisory only — the real
/// defence is the Argon2id KDF — and deliberately dictionary-free so it pulls in no new dependency.
/// Reads only the length + which classes are present; never stores or logs the value.
fn passphrase_strength(pass: &str) -> (f32, PassBand) {
    let len = pass.chars().count();
    if len == 0 {
        return (0.0, PassBand::Empty);
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
    let band = if len < 8 {
        PassBand::TooShort
    } else if frac < 0.5 {
        PassBand::Weak
    } else if frac < 0.72 {
        PassBand::Fair
    } else if frac < 0.9 {
        PassBand::Good
    } else {
        PassBand::Strong
    };
    (frac, band)
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
    use super::{passphrase_strength, PassBand};

    #[test]
    fn empty_passphrase_is_unscored() {
        let (frac, band) = passphrase_strength("");
        assert_eq!(frac, 0.0);
        assert_eq!(band, PassBand::Empty);
        assert_eq!(band.label(), "");
    }

    #[test]
    fn below_minimum_length_reads_too_short() {
        // Under the 8-char floor `do_create` enforces — the meter must say so, never "Weak".
        let (frac, band) = passphrase_strength("short");
        assert!(frac < 0.5);
        assert_eq!(band, PassBand::TooShort);
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
        // Same length (12), more character classes → higher score + a better band.
        let (plain, plain_band) = passphrase_strength("abcdefghijkl");
        let (varied, varied_band) = passphrase_strength("Abcdefgh1jkl");
        assert!(varied > plain);
        assert_eq!(plain_band, PassBand::Fair);
        assert_eq!(varied_band, PassBand::Good);
    }

    #[test]
    fn long_and_varied_reads_strong() {
        let (frac, band) = passphrase_strength("Abcdefghijklmnop1!");
        assert!(frac >= 0.9);
        assert_eq!(band, PassBand::Strong);
    }

    #[test]
    fn frac_never_exceeds_one() {
        // The variety bonus must never push the bar past a full track.
        let (frac, _) = passphrase_strength("Abcdefghijklmnopqrstuvwxyz0123456789!@#");
        assert!(frac <= 1.0);
    }
}
