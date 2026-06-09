//! Money — mono-for-money rendering (DESIGN.md §Typography).
//!
//! Every figure that is money / a balance renders in **JetBrains Mono**, tabular,
//! full precision. The fractional part **and** the ticker are dimmed *by color
//! only* (`text.muted`) — never by a size step, which would produce the
//! superscript look DESIGN rejects. The integer carries `text.primary`.
//!
//! Two entry points wrap the single canonical formatter
//! `deckard_core::format_amount(raw, decimals, max_frac)`:
//! - [`money`] — an asset amount with an optional trailing ticker (`1,934.5 ETH`).
//! - [`usd`] — a USD figure carrying the `$` prefix; zero renders `$0`, never
//!   `$0.0` (the `$` discipline + the zero rule).

use gpui::{div, prelude::FluentBuilder, Hsla, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::h_flex;

use deckard_core::U256;

/// Render an asset amount as mono spans: integer in `primary`, decimals + ticker
/// dimmed to `dim` by color only. `unit` is the trailing ticker (e.g. `"ETH"`),
/// or `None` for a bare number. `mono` is `cx.theme().mono_font_family`.
pub fn money(
    raw: U256,
    decimals: u8,
    max_frac: usize,
    unit: Option<&str>,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    let s = deckard_core::format_amount(raw, decimals, max_frac);
    let (int_part, frac) = split_amount(&s);
    spans(int_part, frac, unit, mono, primary, dim)
}

/// Render a USD figure with the `$` prefix. Integer (incl. `$`) in `primary`,
/// decimals dimmed to `dim`. Zero renders `"$0"` (no `.00`, no `$0.0k`).
// reason: consumed by the Wave-2 shielded-balance / fiat view (Total + Private/
// Public lines carry `$`); kept now as the companion to `money`.
#[allow(dead_code)]
pub fn usd(
    raw: U256,
    decimals: u8,
    max_frac: usize,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    let s = deckard_core::format_amount(raw, decimals, max_frac);
    let (int_part, frac) = split_amount(&s);
    spans(format!("${int_part}"), frac, None, mono, primary, dim)
}

/// Split a formatted amount (`"1,934.5"`) into its integer and (possibly empty)
/// fractional parts. `format_amount` already strips trailing zeros, so the frac
/// is absent for whole numbers (zero renders `"0"` → `("0", "")`).
fn split_amount(s: &str) -> (String, String) {
    match s.split_once('.') {
        Some((int_part, frac)) => (int_part.to_string(), frac.to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// The shared row: a baseline-aligned mono flex with up to three colored spans
/// (integer · `.decimals` · ` ticker`). Dimming is color-only, size held flat.
fn spans(
    int_part: impl Into<String>,
    frac: String,
    unit: Option<&str>,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    let unit = unit.map(|u| format!(" {u}"));
    h_flex()
        .items_baseline()
        .font_family(mono)
        .child(div().text_color(primary).child(int_part.into()))
        .when(!frac.is_empty(), |e| {
            e.child(div().text_color(dim).child(format!(".{frac}")))
        })
        .when_some(unit, |e, u| e.child(div().text_color(dim).child(u)))
}
