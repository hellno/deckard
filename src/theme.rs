//! Theme — a refined dark/light palette with a selectable brand accent.
//!
//! gpui-component's stock dark theme is near pure-black-on-white, which reads
//! harsh. The common pattern (Linear, GitHub, Zed) is: a *soft* near-black with
//! slightly-elevated surfaces, muted secondary text, and a single saturated
//! **accent** that carries the brand. We build that by cloning the built-in
//! `ThemeConfig` and overriding ~20 color tokens, so it survives light/dark
//! toggles (gpui-component re-applies the config on every `Theme::change`).

use std::rc::Rc;

use gpui::{App, SharedString};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use serde::{Deserialize, Serialize};

/// The brand accent. This is the knob that makes the app feel like *yours* — the
/// settings page lets the user pick one and it re-themes the whole app live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Indigo,
    Blue,
    Violet,
    Emerald,
    Amber,
    Rose,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Indigo,
        Accent::Blue,
        Accent::Violet,
        Accent::Emerald,
        Accent::Amber,
        Accent::Rose,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Indigo => "Indigo",
            Accent::Blue => "Blue",
            Accent::Violet => "Violet",
            Accent::Emerald => "Emerald",
            Accent::Amber => "Amber",
            Accent::Rose => "Rose",
        }
    }

    /// `(base, hover, active)` hex for the accent. `base` is also the swatch color.
    fn ramp(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Accent::Indigo => ("#6E78F0", "#828AF2", "#5A64E0"),
            Accent::Blue => ("#3B82F6", "#5A95F7", "#2F6FE0"),
            Accent::Violet => ("#8B5CF6", "#9D74F7", "#7A47E6"),
            Accent::Emerald => ("#10B981", "#2DC894", "#0E9E6F"),
            Accent::Amber => ("#F59E0B", "#F7AE2F", "#DB8C06"),
            Accent::Rose => ("#F43F5E", "#F65A75", "#E02A4A"),
        }
    }

    /// The swatch / mark color as an `0xRRGGBB` value, for `gpui::rgb(..)` in the UI.
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Indigo => 0x6E78F0,
            Accent::Blue => 0x3B82F6,
            Accent::Violet => 0x8B5CF6,
            Accent::Emerald => 0x10B981,
            Accent::Amber => 0xF59E0B,
            Accent::Rose => 0xF43F5E,
        }
    }
}

/// Install (or re-install) the refined theme for `accent`, then apply `mode`.
/// Call once at startup and again whenever the user changes accent or mode.
pub fn install(cx: &mut App, accent: Accent, mode: ThemeMode) {
    // Ensure the Theme global exists (first call seeds it from the registry).
    Theme::change(mode, None, cx);

    let registry = ThemeRegistry::global(cx);
    let mut dark = (**registry.default_dark_theme()).clone();
    let mut light = (**registry.default_light_theme()).clone();
    refine(&mut dark, accent, true);
    refine(&mut light, accent, false);

    let theme = Theme::global_mut(cx);
    theme.dark_theme = Rc::new(dark);
    theme.light_theme = Rc::new(light);

    // Re-apply so the (possibly already-open) windows pick up the new config.
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}

fn refine(config: &mut ThemeConfig, accent: Accent, dark: bool) {
    let (primary, primary_hover, primary_active) = accent.ramp();
    let c = &mut config.colors;
    let set = |slot: &mut Option<SharedString>, hex: &str| *slot = Some(hex.to_string().into());

    // Brand accent (shared across modes). `primary` is the brand color; the
    // `accent` token is a *subtle surface* (ghost-button / menu hover), not the brand.
    set(&mut c.primary, primary);
    set(&mut c.primary_hover, primary_hover);
    set(&mut c.primary_active, primary_active);
    set(&mut c.primary_foreground, "#FFFFFF");
    set(&mut c.ring, primary);

    if dark {
        set(&mut c.background, "#0C0D11"); // soft near-black, faint blue cast
        set(&mut c.foreground, "#E6E7EB"); // soft white, not #FFF
        set(&mut c.secondary, "#16171D"); // cards / surfaces
        set(&mut c.secondary_foreground, "#E6E7EB");
        set(&mut c.muted, "#1B1C23");
        set(&mut c.muted_foreground, "#8A8C99"); // secondary text
        set(&mut c.border, "#262833");
        set(&mut c.input, "#1B1C23");
        set(&mut c.popover, "#16171D");
        set(&mut c.popover_foreground, "#E6E7EB");
        set(&mut c.title_bar, "#0C0D11");
        set(&mut c.title_bar_border, "#1B1C23");
        set(&mut c.sidebar, "#0F1014");
        set(&mut c.accent, "#1F2029"); // subtle hover surface
        set(&mut c.accent_foreground, "#E6E7EB");
    } else {
        set(&mut c.background, "#FBFBFC");
        set(&mut c.foreground, "#16171D");
        set(&mut c.secondary, "#F1F2F4");
        set(&mut c.secondary_foreground, "#16171D");
        set(&mut c.muted, "#F1F2F4");
        set(&mut c.muted_foreground, "#6B6D78");
        set(&mut c.border, "#E4E5EA");
        set(&mut c.input, "#FFFFFF");
        set(&mut c.popover, "#FFFFFF");
        set(&mut c.popover_foreground, "#16171D");
        set(&mut c.title_bar, "#FBFBFC");
        set(&mut c.title_bar_border, "#E4E5EA");
        set(&mut c.sidebar, "#F6F7F9");
        set(&mut c.accent, "#F1F2F4");
        set(&mut c.accent_foreground, "#16171D");
    }
}
