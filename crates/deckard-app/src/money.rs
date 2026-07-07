//! Money — mono-for-money rendering (DESIGN.md §Typography).
//!
//! Every figure that is money / a balance renders in **JetBrains Mono**, tabular,
//! full precision. The fractional part **and** the ticker are dimmed *by color
//! only* (`text.muted`) — never by a size step, which would produce the
//! superscript look DESIGN rejects. The integer carries `text.primary`.
//!
//! Three entry points wrap the single canonical formatter
//! `deckard_core::format_amount(raw, decimals, max_frac)`:
//! - [`money`] — an asset amount with an optional trailing ticker (`1,934.5 ETH`).
//! - [`usd`] — a USD figure carrying the `$` prefix; zero renders `$0`, never
//!   `$0.0` (the `$` discipline + the zero rule).
//! - [`money_cell`] — a decimal-point-aligned ledger cell (integer right-aligned to a
//!   fixed seam, `.decimals` left-aligned after it) so a holdings column's points line up.

use gpui::{
    div, prelude::FluentBuilder, px, Hsla, IntoElement, ParentElement, Pixels, SharedString, Styled,
};
use gpui_component::h_flex;

use deckard_core::U256;

/// The fixed width of a ledger money cell's fractional sub-column — it holds `.decimals` (up to the
/// four-digit hero precision) left-aligned. The integer sub-column takes the rest of the cell and is
/// right-aligned, so the decimal seam sits at a constant `x` across every row in a column and the
/// points line up (the E4 "decimals aligned on the point" rule, #184). Sized to clear `.0000`.
const FRAC_SEAM_W: Pixels = px(52.0);

/// The fixed-length privacy mask: **always six bullets**, never `real.len()`, so a
/// masked figure leaks neither its value nor its digit count (the magnitude-safe rule,
/// per MetaMask's `SensitiveText`). One glyph for every money surface.
pub const MASK_BULLETS: &str = "••••••";

/// String-level mask for callers that render a plain balance string rather than the
/// `money()` spans (e.g. the sidebar wallet balance). Fixed six bullets when masked.
pub fn mask_money(masked: bool, real: &str) -> String {
    if masked {
        MASK_BULLETS.to_string()
    } else {
        real.to_string()
    }
}

/// Render an asset amount as mono spans: integer in `primary`, decimals + ticker
/// dimmed to `dim` by color only. `unit` is the trailing ticker (e.g. `"ETH"`),
/// or `None` for a bare number. `mono` is `cx.theme().mono_font_family`. When
/// `masked`, renders the fixed [`MASK_BULLETS`] in `dim` instead (no value, no unit).
#[allow(clippy::too_many_arguments)]
pub fn money(
    raw: U256,
    decimals: u8,
    max_frac: usize,
    unit: Option<&str>,
    masked: bool,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    if masked {
        // Magnitude-safe: a single dimmed bullet span, no decimals, no ticker.
        return spans(
            MASK_BULLETS.to_string(),
            String::new(),
            None,
            mono,
            dim,
            dim,
        );
    }
    let s = deckard_core::format_amount(raw, decimals, max_frac);
    let (int_part, frac) = split_amount(&s);
    spans(int_part, frac, unit, mono, primary, dim)
}

/// Render a USD figure with the `$` prefix. Integer (incl. `$`) in `primary`,
/// decimals dimmed to `dim`. Zero renders `"$0"` (no `.00`, no `$0.0k`).
// reason: consumed by the Wave-2 shielded-balance / fiat view (Total + Private/
// Public lines carry `$`); kept now as the companion to `money`.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn usd(
    raw: U256,
    decimals: u8,
    max_frac: usize,
    masked: bool,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    if masked {
        return spans(
            MASK_BULLETS.to_string(),
            String::new(),
            None,
            mono,
            dim,
            dim,
        );
    }
    let s = deckard_core::format_amount(raw, decimals, max_frac);
    let (int_part, frac) = split_amount(&s);
    spans(format!("${int_part}"), frac, None, mono, primary, dim)
}

/// A **decimal-point-aligned** money cell for a holdings / ledger column (E4, #184). The integer
/// part is **right-aligned** up to a fixed seam and the `.decimals` are **left-aligned** after it, so
/// the decimal points line up down a column no matter how many fractional digits each row carries.
/// Dimming stays color-only (the decimals in `dim`), size held flat — never a superscript step. When
/// `masked`, renders the fixed [`MASK_BULLETS`] right-aligned to the seam (no value, no decimals).
///
/// The caller sets the column's width (this fills it, `w_full`); the seam is at `col_w -
/// FRAC_SEAM_W`, identical for every row, which is what makes the points align.
pub fn money_cell(
    raw: U256,
    decimals: u8,
    max_frac: usize,
    masked: bool,
    mono: SharedString,
    primary: Hsla,
    dim: Hsla,
) -> impl IntoElement {
    let (int_part, frac, int_color) = if masked {
        (MASK_BULLETS.to_string(), String::new(), dim)
    } else {
        let s = deckard_core::format_amount(raw, decimals, max_frac);
        let (int_part, frac) = split_amount(&s);
        (int_part, frac, primary)
    };

    h_flex()
        .w_full()
        .font_family(mono)
        // Integer: fills the row up to the seam, right-aligned so its ones-digit meets the point.
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .justify_end()
                .child(div().flex_shrink_0().text_color(int_color).child(int_part)),
        )
        // Fractional seam: a fixed-width, left-aligned sub-column so the point starts at a constant x.
        .child(
            h_flex()
                .w(FRAC_SEAM_W)
                .flex_shrink_0()
                .when(!frac.is_empty(), |e| {
                    e.child(div().text_color(dim).child(format!(".{frac}")))
                }),
        )
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
