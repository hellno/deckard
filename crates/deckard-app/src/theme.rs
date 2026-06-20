//! Theme — the locked Deckard dark/light palette (DESIGN.md §Color).
//!
//! gpui-component's stock dark theme is near pure-black-on-white, which reads
//! harsh. Deckard's language is ~95% grayscale: a *soft* near-black with
//! slightly-elevated surfaces, muted secondary text, **neutral** primary buttons,
//! and two signal colors spent sparingly — **amber = the human / where-you-are /
//! caution / focus ring**, **cyan = the agent class**. We build the surface
//! palette by cloning the built-in `ThemeConfig` and overriding the color tokens,
//! so it survives light/dark toggles (gpui-component re-applies the config on
//! every `Theme::change`).
//!
//! The two signal colors are *not* gpui-component theme slots — the cyan slot is
//! private and the live `cyan` is the kit's own base color. So `amber()` /
//! `agent()` live here as app-level helpers returning `gpui::Hsla`, consumed at
//! render sites with `.text_color(..)` / `.bg(..)`.

use std::rc::Rc;

use gpui::{App, Hsla, Rgba, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};

/// Parse a `#RRGGBB` / `#RRGGBBAA` hex string into an `Hsla`. Falls back to a
/// transparent default on a malformed literal (the inputs here are all const).
pub fn hex(s: &str) -> Hsla {
    Rgba::try_from(s).map(Into::into).unwrap_or_default()
}

/// **amber** — the human / "where you are" / caution / the sanctioned focus ring.
/// <1% of pixels. Never a primary-button fill, never a chart segment.
pub fn amber(dark: bool) -> Hsla {
    hex(if dark { "#F2A43B" } else { "#A8650C" })
}

/// Amber at low alpha — the T5 shield hold-to-confirm fill-sweep wash.
/// `rgba(242,164,59,.14)` dark; the deepened light amber at the same alpha. (The caution
/// banner correctly uses a NEUTRAL surface + keyline per DESIGN rule 7, not a tint fill.)
pub fn amber_tint(dark: bool) -> Hsla {
    amber(dark).opacity(0.14)
}

/// **cyan** — the agent class only (the squircle glyph + agent status). Low-chroma.
/// **Never** a page title, body, or link color.
pub fn agent(dark: bool) -> Hsla {
    hex(if dark { "#3CC9BC" } else { "#0C7E75" })
}

/// Cyan at low alpha — the agent-identity chip / "currently acting" wash.
/// `rgba(60,201,188,.12)` dark; the deepened light teal at the same alpha.
pub fn agent_tint(dark: bool) -> Hsla {
    agent(dark).opacity(0.12)
}

/// A project/wallet **identity square** fill — a desaturated, tinted-neutral chip
/// (DESIGN §Color rule 4: identity colors avoid the warm/amber band entirely and
/// sit off the semantic `success` hue, so they never read as actor signal or
/// status). A cool slate-neutral, distinct from both amber and cyan.
pub fn identity_square(dark: bool) -> Hsla {
    hex(if dark { "#3A4250" } else { "#A7AEBA" })
}

/// The **shield / private** tone — a neutral, LOW-chroma cool slate for the private
/// (shielded) balance segment + the shield glyph. Deliberately NOT cyan and NOT amber:
/// privacy sits *off* the actor axis (cyan = agent, amber = human), so the shield mark
/// never reads as an actor signal (deckard-demo-ux-locked + DESIGN §Color). A touch
/// cooler/dimmer than `identity_square` so the two neutrals stay distinguishable.
pub fn shield(dark: bool) -> Hsla {
    hex(if dark { "#33424C" } else { "#94A2AC" })
}

/// Install (or re-install) the refined theme, then apply `mode`. Call once at
/// startup and again whenever the user toggles light/dark.
pub fn install(cx: &mut App, mode: ThemeMode) {
    // Ensure the Theme global exists (first call seeds it from the registry).
    Theme::change(mode, None, cx);

    let registry = ThemeRegistry::global(cx);
    let mut dark = (**registry.default_dark_theme()).clone();
    let mut light = (**registry.default_light_theme()).clone();
    refine(&mut dark, true);
    refine(&mut light, false);

    let theme = Theme::global_mut(cx);
    theme.dark_theme = Rc::new(dark);
    theme.light_theme = Rc::new(light);

    // Re-apply so the (possibly already-open) windows pick up the new config.
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}

fn refine(config: &mut ThemeConfig, dark: bool) {
    // Bundled offline fonts (registered in `main.rs`). The string is the OS /
    // registered family name, not a path; GPUI silently falls back to the system
    // font if the family isn't installed. `Root` applies `font_family` app-wide;
    // mono is per-element via `cx.theme().mono_font_family`.
    config.font_family = Some("Schibsted Grotesk".into());
    config.mono_font_family = Some("JetBrains Mono".into());
    // NB: deliberately do NOT set `font.size` — the views rely on the relative
    // `.text_*` utilities off gpui's 16px base; lowering it would rescale every
    // screen (DESIGN body=13 is honored per-element, not via the base rem).

    let c = &mut config.colors;
    let set = |slot: &mut Option<SharedString>, hex: &str| *slot = Some(hex.to_string().into());

    // PRIMARY buttons are NEUTRAL (DESIGN: amber is never a primary fill — it
    // appears only as the hold-sweep on irreversible confirms). The focus ring is
    // the one sanctioned amber surface.
    if dark {
        set(&mut c.primary, "#161922"); // bg.raise2
        set(&mut c.primary_hover, "#1B1E25");
        set(&mut c.primary_active, "#121419"); // bg.raise
        set(&mut c.primary_foreground, "#E7E9EC"); // text.primary (never pure white)
        set(&mut c.ring, "#F2A43B"); // amber focus ring
    } else {
        set(&mut c.primary, "#FFFFFF"); // raise
        set(&mut c.primary_hover, "#ECEBE4"); // hover
        set(&mut c.primary_active, "#F6F5F1"); // base
        set(&mut c.primary_foreground, "#17191E"); // text.primary
        set(&mut c.ring, "#A8650C"); // deepened amber for AA
    }

    // SEMANTIC tokens — set explicitly so they match DESIGN (warning = amber).
    set(&mut c.success, if dark { "#4FB463" } else { "#2F8F47" });
    set(&mut c.danger, if dark { "#E5565B" } else { "#C23B40" });
    set(&mut c.warning, if dark { "#F2A43B" } else { "#A8650C" });

    if dark {
        set(&mut c.background, "#0A0B0D"); // bg.base — near-black, faint cool cast
        set(&mut c.foreground, "#E7E9EC"); // text.primary
        set(&mut c.secondary, "#121419"); // bg.raise — cards / surfaces
        set(&mut c.secondary_foreground, "#E7E9EC");
        set(&mut c.muted, "#161922"); // bg.raise2
        set(&mut c.muted_foreground, "#9298A2"); // text.secondary (the kit's "muted text")
        set(&mut c.border, "#1B1E25"); // border.hairline
        set(&mut c.input, "#262A33"); // border.strong
        set(&mut c.popover, "#161922"); // bg.raise2
        set(&mut c.popover_foreground, "#E7E9EC");
        set(&mut c.title_bar, "#0B0C0F"); // bg.rail
        set(&mut c.title_bar_border, "#1B1E25"); // hairline
        set(&mut c.sidebar, "#0B0C0F"); // bg.rail
        set(&mut c.accent, "#14161B"); // bg.hover — subtle hover surface
        set(&mut c.accent_foreground, "#E7E9EC");
    } else {
        set(&mut c.background, "#F6F5F1"); // bg.base
        set(&mut c.foreground, "#17191E"); // text.primary
        set(&mut c.secondary, "#FFFFFF"); // raise
        set(&mut c.secondary_foreground, "#17191E");
        set(&mut c.muted, "#ECEBE4"); // hover
        set(&mut c.muted_foreground, "#6B7280"); // text.muted (>=4.5:1 on base)
        set(&mut c.border, "#DDDBD2"); // border.hairline
        set(&mut c.input, "#CFCCC2"); // border.strong
        set(&mut c.popover, "#FFFFFF"); // raise
        set(&mut c.popover_foreground, "#17191E");
        set(&mut c.title_bar, "#EEEDE6"); // bg.rail
        set(&mut c.title_bar_border, "#DDDBD2");
        set(&mut c.sidebar, "#EEEDE6"); // bg.rail
        set(&mut c.accent, "#ECEBE4"); // hover
        set(&mut c.accent_foreground, "#17191E");
    }
}
