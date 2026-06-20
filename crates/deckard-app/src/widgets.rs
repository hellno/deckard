//! Widgets — the shared component vocabulary (DESIGN.md §Enforcement model).
//!
//! Every view composes from these primitives instead of hand-rolling leaf elements
//! per file. That is the whole point: the audit found 3-4 divergent copies of every
//! helper (`short_addr` vs `short_mid` vs `short_address`; `field_label` x2;
//! `error_line` x3 all using a `⚠` emoji; `section_label` x2), which is the root
//! cause of the visual drift. A primitive bakes in the correct DESIGN value so a
//! screen *cannot* drift.
//!
//! Style matches the rest of the crate: pure functions that take explicit theme
//! colors (`Hsla`) and return an `AnyElement` (see `shell_chrome::agent_squircle`).
//! No raw hex; callers pass `cx.theme().*` / `theme::amber(is_dark)`.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, FontWeight, Hsla, IntoElement, ParentElement, Pixels, SharedString, Styled,
};
use gpui_component::{h_flex, Icon, IconName};

/// Middle-truncate an address/hash to the ONE canonical rule: first-6 + last-4
/// (DESIGN §Trust: "show enough of each address that two are distinguishable").
/// This replaces `short_mid` (first-10+last-6) and `short_address` everywhere.
pub(crate) fn short_addr(a: &str) -> String {
    if a.len() >= 12 {
        format!("{}…{}", &a[..6], &a[a.len() - 4..])
    } else {
        a.to_string()
    }
}

/// The ONE caution / danger affordance (DESIGN §Color rule 7): a Lucide
/// `TriangleAlert` icon + inline risk text, **no box, no keyline**. The icon
/// carries the signal. `accent` tints the icon (amber for caution, `danger` for
/// danger); `text_color` tints the risk text (`text.secondary` for caution,
/// `danger` for the irreversible-loss line). Replaces every `format!("⚠ {..}")`.
pub(crate) fn caution_line(
    accent: Hsla,
    text_color: Hsla,
    strong: bool,
    msg: impl Into<SharedString>,
) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .child(
            Icon::new(IconName::TriangleAlert)
                .text_color(accent)
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(text_color)
                .when(strong, |d| d.font_weight(FontWeight::MEDIUM))
                .child(msg.into()),
        )
        .into_any_element()
}

/// A danger line: `TriangleAlert` + text both in `danger`, medium weight. The
/// loud-red, irreversible-action register (DESIGN §Color rule 6). Convenience over
/// [`caution_line`].
pub(crate) fn error_line(danger: Hsla, msg: impl Into<SharedString>) -> AnyElement {
    caution_line(danger, danger, true, msg)
}

/// The ONE section/group label (DESIGN §Typography label tier): tiny, uppercase,
/// muted. Replaces the divergent `group_label` / `section_label` / `field_label`
/// copies. (Letter-spacing is not yet exposed by gpui's `Styled`; size + uppercase
/// + muted carry the treatment until it is.)
pub(crate) fn section_label(text: &str, muted: Hsla) -> AnyElement {
    div()
        .text_size(px(10.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(muted)
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

/// A project / wallet **identity mark** with a deterministic monogram (DESIGN
/// §Actor model: shape is the accessibility backup; never a blank fill). Square
/// (rounded) for projects/wallets; pass `radius = size / 2` for the round human
/// principal. `fill` is the desaturated identity slate; `glyph` tints the monogram.
/// The agent uses `shell_chrome::agent_squircle` instead (cyan, the actor signal).
pub(crate) fn identity_mark(
    seed: &str,
    size: Pixels,
    radius: Pixels,
    fill: Hsla,
    glyph: Hsla,
) -> AnyElement {
    let ch: SharedString = seed
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".to_string())
        .into();
    div()
        .size(size)
        .rounded(radius)
        .bg(fill)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(size * 0.46)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(glyph)
                .child(ch),
        )
        .into_any_element()
}
