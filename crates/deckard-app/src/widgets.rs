//! Widgets — the shared component vocabulary (DESIGN.md §Enforcement model).
//!
//! Every view composes from these primitives instead of hand-rolling leaf elements
//! per file. That is the whole point: the audit found 3-4 divergent copies of every
//! helper (`short_addr` vs `short_mid` vs `short_address`; `field_label` x2;
//! `error_line` x3 all using a `⚠` emoji; `section_label` x2), which is the root
//! cause of the visual drift. A primitive bakes in the correct DESIGN value so a
//! screen *cannot* drift.
//!
//! Style matches the rest of the crate: the atomic primitives are pure functions that take
//! explicit theme colors (`Hsla`) and return an `AnyElement` (see `identity_mark` / `agent_mark`).
//! No raw hex; callers pass `cx.theme().*` / `theme::amber(is_dark)`.
//!
//! The v4 *composite* widgets (`origin_header`, `status_glyph`, `balance_diff`, the `meta_rail`
//! family, `stop_brake`) pull many theme tokens at once, so they take `theme: &Theme` and resolve
//! colors internally — the convention DESIGN.md §Build notes sanctions ("a widget reads them from
//! `cx.theme()`"). Their pure decision logic (the platform key-cap label, the action-tag label,
//! the status→icon map) is factored into small `fn`s that ARE unit-tested. Every v4 primitive is
//! the foundation the request-origin views (E2–E7, epic #179) consume; until a child wires its
//! call site, each carries a scoped `#[allow(dead_code)]` with a `// reason:` naming that consumer
//! (the `money::usd` precedent), so `-D warnings` stays green without a one-off view edit here.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, relative, AnyElement, FontWeight, Hsla, IntoElement, ParentElement, Pixels,
    SharedString, Styled,
};
use gpui_component::{h_flex, v_flex, Icon, IconName, Theme};

use deckard_core::U256;

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
        .text_size(crate::tokens::TEXT_LABEL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(muted)
        .child(SharedString::from(text.to_uppercase()))
        .into_any_element()
}

/// The ONE full-width hairline rule (DESIGN §Visual language: a single hairline only where a list
/// needs row separation). Replaces the inline `div().h(px(1.)).w_full().bg(..)` copies and the
/// per-file local `divider` in `settings_view`.
pub(crate) fn divider(color: Hsla) -> AnyElement {
    div()
        .w_full()
        .h(crate::tokens::STROKE_HAIRLINE)
        .bg(color)
        .into_any_element()
}

/// The deterministic single-char monogram for an identity `seed` (DESIGN §Actor model: shape is
/// the accessibility backup; never a blank fill). The first char, uppercased, or `•` when empty.
fn monogram(seed: &str) -> SharedString {
    seed.trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".to_string())
        .into()
}

/// A project / wallet **identity mark** with a deterministic monogram (DESIGN
/// §Actor model: shape is the accessibility backup; never a blank fill). Square
/// (rounded) for projects/wallets; pass `radius = size / 2` for the round human
/// principal. `fill` is the desaturated identity slate; `glyph` tints the monogram.
/// The agent uses [`agent_mark`] instead (cyan-bordered, the actor signal).
pub(crate) fn identity_mark(
    seed: &str,
    size: Pixels,
    radius: Pixels,
    fill: Hsla,
    glyph: Hsla,
) -> AnyElement {
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
                .child(monogram(seed)),
        )
        .into_any_element()
}

/// The **cyan agent squircle**, handle-aware (DESIGN §Actor model: agent = a cyan squircle monogram
/// — the ONE cyan surface): a cyan-tint fill + the **cyan border that defines the squircle** + a
/// cyan monogram, seeded so it renders the agent's handle initial (`K` for `Kyoto`). The bordered
/// cyan mark IS the two-signal actor signal, so it keeps the border `identity_mark` omits. The one
/// agent mark for the sidebar, breadcrumb, wallet-home presence, agent surface, and activity feed.
pub(crate) fn agent_mark(
    seed: &str,
    size: Pixels,
    radius: Pixels,
    agent: Hsla,
    agent_tint: Hsla,
) -> AnyElement {
    div()
        .size(size)
        .rounded(radius)
        .bg(agent_tint)
        .border_1()
        .border_color(agent)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(size * 0.46)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(agent)
                .child(monogram(seed)),
        )
        .into_any_element()
}

/// The thin budget / utilization gauge (DESIGN §Color rule 8 + §Components Policy
/// card): a 4px neutral track with a fill whose color escalates with pressure —
/// neutral/cyan at rest, amber at >=90%, red at >=100%. `frac` is spent/cap. The
/// label row carries `left` (e.g. "Spent today 0.32 ETH") and `right` (e.g.
/// "64% of 0.50"). Over-cap can never render calm.
#[allow(clippy::too_many_arguments)]
pub(crate) fn budget_gauge(
    frac: f32,
    cyan: Hsla,
    amber: Hsla,
    danger: Hsla,
    track: Hsla,
    muted: Hsla,
    left: impl Into<SharedString>,
    right: impl Into<SharedString>,
) -> AnyElement {
    let fill = if frac >= 1.0 {
        danger
    } else if frac >= 0.9 {
        amber
    } else {
        cyan
    };
    v_flex()
        .w_full()
        .gap_1p5()
        .child(
            div().w_full().h(px(4.0)).rounded(px(2.0)).bg(track).child(
                div()
                    .h(px(4.0))
                    .w(relative(frac.clamp(0.0, 1.0)))
                    .rounded(px(2.0))
                    .bg(fill),
            ),
        )
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .text_xs()
                .text_color(muted)
                .child(left.into())
                .child(right.into()),
        )
        .into_any_element()
}

/// A trust-grade address: identicon mark + ENS (when known) + the canonical
/// `short_addr` mono truncation (DESIGN §Trust: "paired with identicon + ENS").
/// Use on every confirm/recipient row so two addresses are distinguishable at the
/// moment of authorization. `mono` is `cx.theme().mono_font_family`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn truncated_address(
    addr: &str,
    ens: Option<&str>,
    mono: SharedString,
    mark_fill: Hsla,
    glyph: Hsla,
    ens_color: Hsla,
    addr_color: Hsla,
) -> AnyElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(identity_mark(
            ens.unwrap_or(addr),
            px(16.0),
            px(4.0),
            mark_fill,
            glyph,
        ))
        .when_some(ens, |el, e| {
            el.child(
                div()
                    .text_sm()
                    .text_color(ens_color)
                    .child(SharedString::from(e)),
            )
        })
        .child(
            div()
                .font_family(mono)
                .text_sm()
                .text_color(addr_color)
                .child(short_addr(addr)),
        )
        .into_any_element()
}

// ─────────────────────────────────────────────────────────────────────────────
// v4 request-origin primitives (epic #179). Each is the shared foundation the
// origin views consume; the `#[allow(dead_code)]` + `// reason:` lands the widget
// ahead of its E2–E7 consumer (mirrors `money::usd`) so no later view hand-rolls it.
// ─────────────────────────────────────────────────────────────────────────────

/// A chord a [`key_cap`] can render. The `⌘↵` chord renders as ONE cap (DESIGN §The confirm
/// pattern) — a chord can't be fat-fingered like a bare Enter.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // reason: variants selected per confirm tier by E5/E6 (#185/#186).
pub(crate) enum KeyCap {
    /// The irreversible-move confirm chord: `⌘↵` on macOS, `Ctrl↵` elsewhere.
    CmdEnter,
    /// A routine forward step (Continue / Review / Next).
    Enter,
    /// A literal single key (e.g. `X` to deny).
    Key(&'static str),
}

/// The primary-modifier label for `os` (pass `std::env::consts::OS`): `⌘` on macOS, `Ctrl`
/// everywhere else. A pure fn so the platform mapping is unit-testable without a window.
/// The platform half of `key_cap`, now consumed by the shared Review (E5) via `key_cap`.
fn primary_mod_label(os: &str) -> &'static str {
    if os == "macos" {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// The glyphs a [`KeyCap`] renders for `os`. Pure (no rendering) so it is unit-testable.
/// The label half of `key_cap`, now consumed by the shared Review (E5) via `key_cap`.
fn key_cap_label(cap: KeyCap, os: &str) -> String {
    match cap {
        KeyCap::CmdEnter => format!("{}↵", primary_mod_label(os)),
        KeyCap::Enter => "↵".to_string(),
        KeyCap::Key(k) => k.to_string(),
    }
}

/// A quiet, flat key-cap chip (DESIGN §The confirm pattern + §Command palette): a bordered,
/// rounded, mono glyph. **Platform-aware** — `⌘` on macOS, `Ctrl` on Linux (via
/// `std::env::consts::OS`), the `⌘↵` chord as ONE cap. `armed` renders the amber border + amber
/// text (no fill) of the live confirm; at rest it is `border.strong` + `text.muted`. The one
/// key-cap so no view hardcodes a `⌘`/`Ctrl` glyph.
// Consumed by the v4 confirm button (E5, #185 — the shared Review's ⌘↵) and, later, activity /
// needs-you key hints (E6, #186); the one platform-aware glyph so no view re-rolls a ⌘/Ctrl.
pub(crate) fn key_cap(
    cap: KeyCap,
    armed: bool,
    border_strong: Hsla,
    muted: Hsla,
    amber: Hsla,
    mono: SharedString,
) -> AnyElement {
    let (border, text) = if armed {
        (amber, amber)
    } else {
        (border_strong, muted)
    };
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .px_1p5()
        .py_0p5()
        .rounded(crate::tokens::RADIUS_INPUT)
        .border_1()
        .border_color(border)
        .font_family(mono)
        .text_xs()
        .text_color(text)
        .child(SharedString::from(key_cap_label(cap, std::env::consts::OS)))
        .into_any_element()
}

/// A value-move verb, for [`action_tag`].
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // reason: variants selected per row by the Activity feed / Review (E5/E6).
pub(crate) enum ActionKind {
    Send,
    Swap,
    Shield,
    Supply,
    Receive,
    Approve,
    Revoke,
}

/// The uppercase label for an [`ActionKind`]. Pure so it is unit-testable.
#[allow(dead_code)] // reason: the label half of `action_tag`; consumed via `action_tag` (E6).
fn action_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Send => "SEND",
        ActionKind::Swap => "SWAP",
        ActionKind::Shield => "SHIELD",
        ActionKind::Supply => "SUPPLY",
        ActionKind::Receive => "RECEIVE",
        ActionKind::Approve => "APPROVE",
        ActionKind::Revoke => "REVOKE",
    }
}

/// The uppercase action chip (`SWAP` / `SHIELD` / `SEND` / `SUPPLY`) that leads a scannable
/// activity or needs-you row (DESIGN §The request-origin model: an action *tag*, not prose). A
/// NEUTRAL chip — `bg.raise` fill, hairline border, `text.secondary` — never a signal color: the
/// origin identity carries the color, the tag carries the verb.
// reason: consumed by the v4 Activity feed + needs-you queue (E6, #186) and the Review verb (E5);
// E1 lands it so no row hand-rolls a `format!`-uppercased span.
#[allow(dead_code)]
pub(crate) fn action_tag(kind: ActionKind, raise: Hsla, border: Hsla, text: Hsla) -> AnyElement {
    div()
        .flex_shrink_0()
        .px_1p5()
        .py_0p5()
        .rounded(crate::tokens::RADIUS_ROW)
        .bg(raise)
        .border_1()
        .border_color(border)
        .text_size(crate::tokens::TEXT_LABEL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(text)
        .child(SharedString::from(action_label(kind)))
        .into_any_element()
}

/// The state a [`status_glyph`] carries.
#[derive(Clone, Copy, PartialEq, Eq)]
// reason: `Confirmed` is wired by the E3 transaction rail (#183); `Failed`/`Pending`/`Live`/
// `Neutral` land with the E6/E7 feed + full receipt (#186/#187).
#[allow(dead_code)]
pub(crate) enum StatusGlyph {
    /// Confirmed / approved / executed — a `success` check.
    Confirmed,
    /// Failed / declined / revoked — a `danger` x.
    Failed,
    /// Awaiting you — an amber ring.
    Pending,
    /// An agent currently acting — a cyan ring.
    Live,
    /// No status — a muted minus.
    Neutral,
}

/// The Lucide icon for a [`StatusGlyph`]. Pure so it is unit-testable. No `clock` ships in the
/// icon set, so `Pending`/`Live` use the loader ring — the DESIGN "clock-ring = pending" intent
/// (a ring, not a checkmark); the color separates awaiting-you (amber) from an agent (cyan).
fn status_icon(state: StatusGlyph) -> IconName {
    match state {
        StatusGlyph::Confirmed => IconName::CircleCheck,
        StatusGlyph::Failed => IconName::CircleX,
        StatusGlyph::Pending | StatusGlyph::Live => IconName::LoaderCircle,
        StatusGlyph::Neutral => IconName::Minus,
    }
}

/// The ONE status vocabulary (DESIGN §Component primitives: "one status vocabulary across feed +
/// chips"): a circular glyph whose color carries the state — `success` check = confirmed, `danger`
/// x = failed, amber ring = pending/awaiting-you, cyan ring = an agent acting, muted minus = none.
/// The icon shape backs the color, so it survives grayscale.
// reason: consumed by the v4 Activity feed + Transaction receipt (E6/E7, #186/#187); E1 lands one
// glyph set so the feed + receipt stop re-rolling per-file status SVGs.
pub(crate) fn status_glyph(state: StatusGlyph, theme: &Theme) -> AnyElement {
    let is_dark = theme.is_dark();
    let tone = match state {
        StatusGlyph::Confirmed => theme.success,
        StatusGlyph::Failed => theme.danger,
        StatusGlyph::Pending => crate::theme::amber(is_dark),
        StatusGlyph::Live => crate::theme::agent(is_dark),
        StatusGlyph::Neutral => theme.muted_foreground,
    };
    Icon::new(status_icon(state))
        .size(crate::tokens::ICON_MD)
        .text_color(tone)
        .into_any_element()
}

/// A [`kv_row`] value: mono by default, sans for a human phrase ("Ethereum · mainnet"),
/// `success`-tinted for a verified / OK state, or `warn`-tinted for a loud trust downgrade
/// ("Not verified" — DESIGN §Trust rule 9: a downgrade is never rendered quiet).
pub(crate) enum KvValue<'a> {
    Mono(&'a str),
    Sans(&'a str),
    Ok(&'a str),
    Warn(&'a str),
}

/// The ONE key/value row (DESIGN §Widget vocabulary): label-left `text.muted`, value-right (mono
/// `text.primary`, or sans, or `success`, or `warn` for a loud downgrade), the row clamped so a long
/// value truncates rather than overflowing. Shared by clear-signing quiet-facts, the policy ledger,
/// and the metadata rail.
// Consumed by the Review quiet facts (E5, #185 — From / Network / Allowed by) AND the v4 metadata
// rail (E3, #183).
pub(crate) fn kv_row(
    label: &str,
    value: KvValue,
    muted: Hsla,
    primary: Hsla,
    success: Hsla,
    warn: Hsla,
    mono: SharedString,
) -> AnyElement {
    let (text, is_mono, color) = match value {
        KvValue::Mono(v) => (v, true, primary),
        KvValue::Sans(v) => (v, false, primary),
        KvValue::Ok(v) => (v, false, success),
        KvValue::Warn(v) => (v, false, warn),
    };
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .gap_4()
        .text_size(crate::tokens::TEXT_BODY)
        .child(
            div()
                .flex_shrink_0()
                .text_color(muted)
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(color)
                .when(is_mono, |d| d.font_family(mono))
                .child(SharedString::from(text.to_string())),
        )
        .into_any_element()
}

/// The ONE page-header anatomy (DESIGN §Component primitives): a caller-built identity `mark`
/// (`identity_mark` / `agent_mark`) + an H1 title at ONE size (`text_xl`, `text.primary`, 600) +
/// an optional muted one-line subtitle. Kills the hand-rolled headers at three sizes. Pass
/// `subtitle_mono = Some(theme.mono_font_family)` when the subtitle is an address (DESIGN §Trust:
/// addresses are mono), `None` for prose.
// The one header anatomy consumed by the v4 view headers (E2 wires the wallet-home masthead;
// E4/E6/E7 follow) so no view re-rolls a header at its own size.
pub(crate) fn page_header(
    mark: AnyElement,
    title: &str,
    subtitle: Option<&str>,
    subtitle_mono: Option<SharedString>,
    primary: Hsla,
    muted: Hsla,
) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .child(mark)
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(primary)
                        .child(SharedString::from(title.to_string())),
                )
                .when_some(subtitle, |d, s| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(muted)
                            .when_some(subtitle_mono, |d, mono| d.font_family(mono))
                            .child(SharedString::from(s.to_string())),
                    )
                }),
        )
        .into_any_element()
}

/// Who a shared-Review request came FROM (DESIGN §The request-origin model): the human, an agent,
/// or a dapp. The verb is the human's action (`You are sending`); agents `propose`, dapps `request`.
/// Constructed per request by the shared Review (E5, #185 — You / Agent / Dapp, the last for a dapp
/// message) and the E3 request rail (#183, You / Agent). Every variant is now built, so no allow.
pub(crate) enum Origin<'a> {
    /// The human principal — a round identity mark (`account` seeds it) + amber `You are {verb}`.
    You { account: &'a str, verb: &'a str },
    /// An agent (MCP) — a cyan mark (`handle` seeds it) + `{handle} proposes`, the cyan signal.
    Agent { handle: &'a str },
    /// A dapp origin — a NEUTRAL favicon mark + `{domain} requests`; never a third signal color.
    Dapp { domain: &'a str },
}

/// A per-origin trust badge (DESIGN §The request-origin model): it borrows the STATE colors, never
/// a signal hue. `Verified` = success, `FirstSeen` = amber caution, `Flagged` = danger.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // reason: attached per origin by the shared Review (E5, #185).
pub(crate) enum Trust {
    Verified,
    FirstSeen,
    Flagged,
}

/// The small state-color trust badge for the [`origin_header`] rail.
/// The badge half of `origin_header`, consumed via `origin_header` (E5, #185).
fn trust_badge(trust: Trust, theme: &Theme) -> AnyElement {
    let is_dark = theme.is_dark();
    // The tint is hue-keyed (DESIGN §Opacity): the amber caution reads weaker at equal alpha, so it
    // gets the `alpha-tint-warm` .14 (via `theme::amber_tint`); the success/danger states take the
    // standard `ALPHA_TINT` .12.
    let (label, tone, tint) = match trust {
        Trust::Verified => (
            "Verified",
            theme.success,
            theme.success.opacity(crate::tokens::ALPHA_TINT),
        ),
        Trust::FirstSeen => (
            "New site",
            crate::theme::amber(is_dark),
            crate::theme::amber_tint(is_dark),
        ),
        Trust::Flagged => (
            "Flagged",
            theme.danger,
            theme.danger.opacity(crate::tokens::ALPHA_TINT),
        ),
    };
    div()
        .flex_shrink_0()
        .px_1p5()
        .py_0p5()
        .rounded(crate::tokens::RADIUS_ROW)
        .bg(tint)
        .border_1()
        .border_color(tone)
        .text_size(crate::tokens::TEXT_LABEL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(tone)
        .child(SharedString::from(label))
        .into_any_element()
}

/// The request-origin header rail on the shared Review (DESIGN §Clear-signing): identity mark +
/// who-line + an optional state-color trust badge, over a bottom hairline. This is the ONLY thing
/// that changes across origins — the review body below is identical, so there is one review to be
/// fooled by. A dapp/external origin is a neutral identity + a state-color badge, NEVER a third
/// signal color; the agent mark is the bordered cyan squircle ([`agent_mark`], handle-aware).
/// `You` is amber, an agent is cyan, a dapp is neutral.
// Consumed by the ONE shared Review (E5, #185) for every origin (self Send/Shield/Swap, an agent
// proposal, a dapp request) AND the rail's compact clear-signing (E3, #183).
pub(crate) fn origin_header(origin: Origin, trust: Option<Trust>, theme: &Theme) -> AnyElement {
    let is_dark = theme.is_dark();
    let amber = crate::theme::amber(is_dark);
    let agent = crate::theme::agent(is_dark);
    let agent_tint = crate::theme::agent_tint(is_dark);
    let id_fill = crate::theme::identity_square(is_dark);
    let primary = theme.foreground;
    let muted = theme.muted_foreground;
    let border = theme.border;
    let round = crate::tokens::MARK_LG * 0.5;

    // (mark, who-line) per origin. The dapp who-line is neutral (dim domain + primary verb);
    // You/Agent tint the whole phrase with their signal color.
    let (mark, who): (AnyElement, AnyElement) = match origin {
        Origin::You { account, verb } => (
            identity_mark(account, crate::tokens::MARK_LG, round, id_fill, primary),
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(amber)
                .child(SharedString::from(format!("You are {verb}")))
                .into_any_element(),
        ),
        Origin::Agent { handle } => (
            agent_mark(
                handle,
                crate::tokens::MARK_LG,
                crate::tokens::RADIUS_ROW,
                agent,
                agent_tint,
            ),
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(agent)
                .child(SharedString::from(format!("{handle} proposes")))
                .into_any_element(),
        ),
        Origin::Dapp { domain } => (
            identity_mark(
                domain,
                crate::tokens::MARK_LG,
                crate::tokens::RADIUS_ROW,
                id_fill,
                primary,
            ),
            h_flex()
                .min_w_0()
                .items_baseline()
                .gap_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(muted)
                        .child(SharedString::from(domain.to_string())),
                )
                .child(div().flex_shrink_0().text_color(primary).child("requests"))
                .into_any_element(),
        ),
    };

    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .pb_4()
        .border_b_1()
        .border_color(border)
        .child(mark)
        .child(div().flex_1().min_w_0().child(who))
        .when_some(trust, |el, t| el.child(trust_badge(t, theme)))
        .into_any_element()
}

/// A direction for a [`DiffRow`]: what the tx does to a balance.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // reason: selected per net effect by the receipt / multi-effect Review (E7/E5).
pub(crate) enum DiffDir {
    /// You pay — `−`, `danger`.
    Out,
    /// You receive — `+`, `success`.
    In,
}

/// One row of a [`balance_diff`] — a net effect on one asset.
#[allow(dead_code)] // reason: constructed per net effect by the receipt / Review (E7/E5).
pub(crate) struct DiffRow<'a> {
    /// The ticker — seeds the row's token mark and is its label (e.g. `ETH`).
    pub token: &'a str,
    /// The magnitude in base units.
    pub amount: U256,
    /// Decimals for `amount` (18 for ETH).
    pub decimals: u8,
    /// The direction of the effect.
    pub dir: DiffDir,
}

/// The Rabby-style "what changes" balance diff (DESIGN §Clear-signing): hairline rows, no card,
/// each `[token mark] [ticker] ···· [signed amount]`. Kept ONLY for a multi-effect tx whose net
/// effects aren't obvious from the hero (a simple swap states its amount once, no diff). `Out`
/// renders `−` in `danger`, `In` renders `+` in `success`; the amount is mono via `money.rs`.
// reason: consumed by the read-only Transaction receipt + multi-effect Review (E7/E5, #187/#185).
#[allow(dead_code)]
pub(crate) fn balance_diff(rows: &[DiffRow], theme: &Theme) -> AnyElement {
    let muted = theme.muted_foreground;
    let border = theme.border;
    let raise = theme.secondary; // bg.raise — the token mark fill
    let success = theme.success;
    let danger = theme.danger;
    let mono = theme.mono_font_family.clone();

    let mut col = v_flex().w_full().border_b_1().border_color(border);
    for row in rows {
        let (sign, tone) = match row.dir {
            DiffDir::Out => ("−", danger),
            DiffDir::In => ("+", success),
        };
        col = col.child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .py_3()
                .border_t_1()
                .border_color(border)
                .child(identity_mark(
                    row.token,
                    crate::tokens::MARK_MD,
                    crate::tokens::RADIUS_ROW,
                    raise,
                    muted,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(crate::tokens::TEXT_BODY)
                        .text_color(muted)
                        .child(SharedString::from(row.token.to_string())),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .items_baseline()
                        .child(div().font_family(mono.clone()).text_color(tone).child(sign))
                        .child(crate::money::money(
                            row.amount,
                            row.decimals,
                            6,
                            Some(row.token),
                            false,
                            mono.clone(),
                            tone,
                            tone,
                        )),
                ),
        );
    }
    col.into_any_element()
}

/// The always-on right metadata rail container (DESIGN §IA: "~300px, hairline-left, not
/// collapsible; contextual to the focused object"). A fixed-width column — a titled 48px head over
/// a scrollable body. E3 fills `body` with `meta_section` / `meta_obj` / `kv_row` blocks.
// reason: consumed by the three-pane shell (E3, #183) — home / request / transaction rail bodies.
pub(crate) fn meta_rail(title: &str, body: AnyElement, theme: &Theme) -> AnyElement {
    let border = theme.border;
    let rail_bg = theme.sidebar; // bg.rail
    let fg = theme.foreground;
    v_flex()
        .w(crate::tokens::RAIL_W)
        .h_full()
        .flex_shrink_0()
        .min_h_0()
        .bg(rail_bg)
        .border_l_1()
        .border_color(border)
        .child(
            h_flex()
                .flex_shrink_0()
                .h(px(48.0))
                .px_4()
                .items_center()
                .border_b_1()
                .border_color(border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(SharedString::from(title.to_string())),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .p_4()
                .gap_4()
                .child(body),
        )
        .into_any_element()
}

/// A ruled sub-section inside [`meta_rail`] (DESIGN: a `.metasec` — a top hairline + an optional
/// `section_label` + body). Groups a set of `kv_row`s under a quiet label.
// reason: consumed by the three-pane shell rail bodies (E3, #183).
pub(crate) fn meta_section(label: Option<&str>, body: AnyElement, theme: &Theme) -> AnyElement {
    let border = theme.border;
    let muted = theme.muted_foreground;
    v_flex()
        .w_full()
        .pt_4()
        .gap_3()
        .border_t_1()
        .border_color(border)
        .when_some(label, |d, l| d.child(section_label(l, muted)))
        .child(body)
        .into_any_element()
}

/// The identity object at the top of a rail body (DESIGN: a `.metaobj` — a caller-built `mark` +
/// name (600) + a mono sub-line, e.g. the truncated address or `shield · confirmed`).
// reason: consumed by the three-pane shell rail bodies (E3, #183).
pub(crate) fn meta_obj(mark: AnyElement, name: &str, sub: &str, theme: &Theme) -> AnyElement {
    let fg = theme.foreground;
    let muted = theme.muted_foreground;
    let mono = theme.mono_font_family.clone();
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .child(mark)
        .child(
            // `flex_1` so the text column fills the row's remaining width: a `truncate` sub whose
            // min-content is 0 would otherwise let the column shrink to the (shorter) name and clip
            // the wider mono address to a second ellipsis (the E2 masthead bug, #192). Now a short
            // address renders in full and only genuinely over-wide content clamps.
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(SharedString::from(name.to_string())),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono)
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(sub.to_string())),
                ),
        )
        .into_any_element()
}

/// The STOP brake state (DESIGN §Widget vocabulary + §Agent model).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BrakeState {
    /// An agent is active, not yet armed — amber, `Stop all agents`.
    Ready,
    /// Armed after the first press — danger, `Confirm STOP`.
    Armed,
    /// No agent running — neutral, `No agents running` (never a start/stop toggle).
    Idle,
}

/// The one kill-switch treatment (DESIGN §Widget vocabulary): amber-idle → danger-armed, reused
/// across Activity + the agent surface. Text-only (no `power` icon ships, and the app's existing
/// STOP is text), the always-reachable panic brake; the view owns the `⌘↵`/Esc arm handlers and
/// the STOP-zeroizes-the-key logic — this renders only its state.
/// Map the two view flags to a brake state. **Arming wins over everything** — once you've started
/// a STOP you can always confirm it, even if the agent goes away underneath you — then Ready only
/// while an agent is actually live, else the disabled Idle marker. Pure so the precedence is
/// unit-testable and can't silently drift under a refactor.
pub(crate) fn brake_state(arming: bool, has_active_agent: bool) -> BrakeState {
    if arming {
        BrakeState::Armed
    } else if has_active_agent {
        BrakeState::Ready
    } else {
        BrakeState::Idle
    }
}

// reason: consumed by the v4 Activity header (E6, #186); the agent surface keeps its own
// hand-rolled STOP until #167 folds it onto this widget too.
pub(crate) fn stop_brake(state: BrakeState, theme: &Theme) -> AnyElement {
    let is_dark = theme.is_dark();
    let amber = crate::theme::amber(is_dark);
    let amber_tint = crate::theme::amber_tint(is_dark);
    let danger = theme.danger;
    let border = theme.border;
    let muted = theme.muted_foreground;
    let (label, edge, text, fill) = match state {
        BrakeState::Ready => ("Stop all agents", amber, amber, Some(amber_tint)),
        BrakeState::Armed => (
            "Confirm STOP: revoke & lock signing · Esc to cancel",
            danger,
            danger,
            Some(danger.opacity(crate::tokens::ALPHA_TINT)),
        ),
        BrakeState::Idle => ("No agents running", border, muted, None),
    };
    h_flex()
        .flex_shrink_0()
        .items_center()
        .px_3()
        .py_1p5()
        .rounded(crate::tokens::RADIUS_ROW)
        .border_1()
        .border_color(edge)
        .when_some(fill, |d, f| d.bg(f))
        .text_size(crate::tokens::TEXT_BODY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(text)
        .child(SharedString::from(label))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_cap_is_platform_aware() {
        // AC #2: `Ctrl` on a forced-Linux path, `⌘` on macOS. The public `key_cap` reads
        // `std::env::consts::OS`; the pure helpers let us assert BOTH platforms from any host.
        assert_eq!(primary_mod_label("macos"), "⌘");
        assert_eq!(primary_mod_label("linux"), "Ctrl");
        // A non-macOS `os` string always resolves to `Ctrl` (Linux/Windows/other).
        assert_eq!(primary_mod_label("windows"), "Ctrl");
    }

    #[test]
    fn key_cap_chord_renders_as_one_cap() {
        // The `⌘↵` / `Ctrl↵` chord is a single cap's text, platform-aware.
        assert_eq!(key_cap_label(KeyCap::CmdEnter, "macos"), "⌘↵");
        assert_eq!(key_cap_label(KeyCap::CmdEnter, "linux"), "Ctrl↵");
        // Routine forward step is a bare Enter; a literal key is passed through.
        assert_eq!(key_cap_label(KeyCap::Enter, "macos"), "↵");
        assert_eq!(key_cap_label(KeyCap::Key("X"), "linux"), "X");
    }

    #[test]
    fn brake_state_arm_beats_agent_state() {
        // Arming wins even if the agent went away underneath you — you can always confirm a STOP
        // you started (the irreversible one must never get stuck half-armed).
        assert_eq!(brake_state(true, true), BrakeState::Armed);
        assert_eq!(brake_state(true, false), BrakeState::Armed);
        // Not arming: amber Ready only while an agent is live, else the disabled Idle marker.
        assert_eq!(brake_state(false, true), BrakeState::Ready);
        assert_eq!(brake_state(false, false), BrakeState::Idle);
    }

    #[test]
    fn action_tags_are_uppercase_verbs() {
        assert_eq!(action_label(ActionKind::Swap), "SWAP");
        assert_eq!(action_label(ActionKind::Shield), "SHIELD");
        assert_eq!(action_label(ActionKind::Send), "SEND");
        assert_eq!(action_label(ActionKind::Supply), "SUPPLY");
    }

    #[test]
    fn status_glyphs_map_to_shipped_icons() {
        // No `clock` ships → pending/live share the loader ring; the color (resolved at the
        // render site) separates them. Confirmed/failed/neutral map to their circle glyphs.
        // `IconName` derives only `IntoElement, Clone` (no `PartialEq`), so match on the variant.
        assert!(matches!(
            status_icon(StatusGlyph::Confirmed),
            IconName::CircleCheck
        ));
        assert!(matches!(
            status_icon(StatusGlyph::Failed),
            IconName::CircleX
        ));
        assert!(matches!(
            status_icon(StatusGlyph::Pending),
            IconName::LoaderCircle
        ));
        assert!(matches!(
            status_icon(StatusGlyph::Live),
            IconName::LoaderCircle
        ));
        assert!(matches!(status_icon(StatusGlyph::Neutral), IconName::Minus));
    }
}
