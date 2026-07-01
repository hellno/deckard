//! Design tokens — the sizing scale from DESIGN.md §"Spacing, sizing, radii,
//! motion — the token layer", as Rust consts.
//!
//! Only the values gpui does **not** already name live here. gpui's own utilities
//! ARE the rest of the scale and stay the idiom:
//! - **spacing** — `.gap_N` / `.p_N` / `.m_N` (the 4px grid; `N` units = `N*4`px),
//! - **the h1 / section / small type steps** — `.text_xl` (20) / `.text_sm` (14) /
//!   `.text_xs` (12), plus `.text_3xl` (30) for a swap compose amount.
//!
//! A view carries a const or a gpui utility, never a raw `px(...)` that duplicates
//! one; the `no_raw_text_size_px` source-scan test enforces the type half.
//!
//! The object-size ladder (marks, gauges), the divider stroke, and the remaining
//! motion timings are named in DESIGN.md but land here as they gain call sites —
//! the widget work (`page_header` / `divider` / `budget_gauge`) is where they'll
//! be consumed, so they arrive with it rather than as dead consts now.

use std::time::Duration;

use gpui::{px, Pixels};

// ── Display + body/label type sizes (h1/section/small are `.text_xl`/`.text_sm`/`.text_xs`) ──
/// Balance hero on the wallet home.
pub const TEXT_HERO: Pixels = px(64.0);
/// The oversized amount on a clear-signing confirm (Send / Shield / Approve). Was a 40px send hero;
/// swap *compose* amounts are a step below at `.text_3xl` (30) — a compose screen is not a confirm.
pub const TEXT_TX_HERO: Pixels = px(44.0);
/// Body / values. gpui has no 13px utility (`.text_xs` 12 / `.text_sm` 14 bracket it).
pub const TEXT_BODY: Pixels = px(13.0);
/// The tiny uppercase group label (`section_label`); tracking approximated by size + uppercase.
pub const TEXT_LABEL: Pixels = px(10.0);

// ── Radii ──
pub const RADIUS_INPUT: Pixels = px(4.0);
pub const RADIUS_ROW: Pixels = px(6.0);
pub const RADIUS_MODAL: Pixels = px(10.0);

// ── Stroke ──
/// Dividers, input outlines, the focus ring — the one hairline width.
pub const STROKE_HAIRLINE: Pixels = px(1.0);

// ── Chrome dimensions ──
pub const SIDEBAR_W: Pixels = px(248.0);
pub const STATUS_H: Pixels = px(25.0);
/// The reading column on a main surface.
pub const CONTENT_MAX_W: Pixels = px(760.0);
/// The centered clear-signing / confirm card.
pub const CONFIRM_W: Pixels = px(460.0);

// ── Motion ──
/// The confirm's inert window — a queued/held keypress from the previous screen
/// can't carry through and fire (DESIGN §The confirm pattern).
pub const ARM_DELAY: Duration = Duration::from_millis(450);

#[cfg(test)]
mod lint {
    //! Source-scan guard: no view may set a type size with a raw pixel literal.
    //! Every type size is a `tokens::TEXT_*` const or a gpui `.text_*` utility — a
    //! raw `text_size(px(..))` is exactly the drift DESIGN.md's token layer replaces.
    use std::fs;
    use std::path::Path;

    #[test]
    fn no_raw_text_size_px_in_views() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        for entry in fs::read_dir(&src).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            // This module's own source names the forbidden pattern (in the check and
            // the message); skip it so the guard doesn't flag itself.
            if name == "tokens.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).expect("read source");
            for (i, raw) in text.lines().enumerate() {
                let line = raw.trim_start();
                if line.starts_with("//") {
                    continue;
                }
                if line.contains("text_size(px(") {
                    offenders.push(format!("{name}:{}", i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "raw text_size(px(..)) is a review failure — use a tokens::TEXT_* const or a \
             gpui .text_* utility (DESIGN.md visual definition of done):\n  {}",
            offenders.join("\n  ")
        );
    }
}
