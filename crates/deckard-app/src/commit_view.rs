//! commit_view — the generic "compose → review → done" renderer that drives every
//! [`CommitFlow`](crate::commit_flow) surface (Send now; Shield joins in Step 2). A single
//! [`CommitView`] descriptor (a `&'static` table of copy, button ids, the heading glyph, and a
//! few per-surface hooks) feeds [`Shell::render_commit`], which renders the shared compose →
//! review → done surface. The review step is the editorial transaction-as-hero clear-signing
//! statement (DESIGN §Clear-signing review): action label, oversized mono amount, recipient,
//! danger/caution lines, then quiet supporting facts — driven entirely by the descriptor.
//!
//! The clear-signing contract is unchanged from `shield_view`/`send_view`: plain language, exact
//! mono figures, danger early, and confirm is a hold (never a tap) — the hand-built
//! [`Shell::hold_to_confirm`] sweep animates an amber fill over [`SHIELD_HOLD`] as the action
//! signs (amber = the human-confirm signal).
//!
//! Step 1 migrates ONLY Send onto this renderer; Shield still uses its flat `shield_*` fields and
//! `shield_view.rs`. The descriptor already carries the slots Shield needs (optional fee/net rows,
//! a variable honesty-line list, an optional conditional compose-hint hook) so Step 2 is a pure
//! descriptor + handler swap with no renderer changes.

use gpui::{
    div, px, relative, Animation, AnimationExt, ClipboardItem, Context, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Disableable, Icon, IconName,
};

use deckard_core::U256;

use crate::commit_flow::{CommitFlow, Proposal};
use crate::money::money;
use crate::shell::{Shell, SHIELD_HOLD};

/// A single label/value money row in the review's quiet supporting-facts list: label left
/// (muted), value right (mono). One signature shared by every commit surface.
fn kv_money_row(
    label: &'static str,
    wei: U256,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_1p5()
        .child(div().text_sm().text_color(muted).child(label))
        .child(
            div()
                .text_sm()
                .child(money(wei, 18, 6, Some("ETH"), false, mono, fg, muted)),
        )
}

/// A money figure derived from the proposal's gross value, rendered as one quiet supporting-fact
/// row below the review hero (e.g. Shield's "Railgun fee" and "You'll receive (private)").
/// `compute` turns the gross intent value into the row's wei figure.
pub struct MoneyRow {
    pub label: &'static str,
    pub compute: fn(gross: U256) -> U256,
}

/// One honesty line in the calm neutral surface: `emphasized` lines use the foreground tone, the
/// rest are muted (matches `send_honesty` / `shield_honesty` exactly).
pub struct HonestyLine {
    pub text: &'static str,
    pub emphasized: bool,
}

/// The `&'static` descriptor that turns the generic renderer into a specific surface. Every field
/// reproduces a hand-written value from the corresponding `*_view.rs`; the function-pointer hooks
/// re-acquire the live [`CommitFlow`] and route the buttons to the surface's existing handlers, so
/// the renderer never needs to know which surface it is drawing.
pub struct CommitView {
    // --- per-surface state access ---
    /// Re-acquire this surface's flow from the shell (`&mut self.send`, later `&mut self.shield`).
    /// Read-only here; the renderer only reads flow state.
    pub flow: fn(&Shell) -> &CommitFlow,
    /// The neutral, low-chroma heading glyph tone (NOT cyan/amber — these surfaces sit off the
    /// actor axis). `dark` is `theme.is_dark()`.
    pub glyph_tone: fn(dark: bool) -> Hsla,

    // --- compose ---
    pub compose_title: &'static str,
    pub compose_subtitle: &'static str,
    pub recipient_label: &'static str,
    /// The Review button id + its idle/busy labels (busy reuses `"Reviewing…"` across surfaces).
    pub review_button_id: &'static str,
    pub review_label: &'static str,
    pub cancel_button_id: &'static str,
    /// The static compose hint (Send). `None` when a surface drives its hint conditionally via
    /// `compose_hint`.
    pub compose_hint: Option<&'static str>,
    /// The conditional compose hint (Shield's 3-way line). Takes precedence over `compose_hint`
    /// when set; picks the line from live shell state + the recipient text the renderer already
    /// read (`recipient_raw`, passed so the hook needs no `cx`). `None` for a static-hint surface.
    pub compose_hint_dynamic: Option<fn(&Shell, recipient_raw: &str) -> &'static str>,

    // --- review ---
    pub review_title: &'static str,
    pub review_subtitle: &'static str,
    /// Quiet supporting-fact money rows, demoted between hairlines below the hero (Shield's fee +
    /// net). Empty for Send, which has nothing to demote.
    pub extra_rows: &'static [MoneyRow],
    /// The honesty lines (2 for Send, 3 for Shield), in render order.
    pub honesty: &'static [HonestyLine],
    /// The hold-to-confirm widget + fill-animation ids, and the idle/holding/busy labels.
    pub hold_id: &'static str,
    pub hold_fill_id: &'static str,
    pub hold_label_idle: &'static str,
    pub hold_label_holding: &'static str,
    pub hold_label_busy: &'static str,
    pub edit_button_id: &'static str,

    // --- done ---
    pub done_title: &'static str,
    pub done_body: &'static str,
    pub copy_button_id: &'static str,
    pub done_button_id: &'static str,

    // --- handlers (route to the surface's existing `impl Shell` methods) ---
    pub on_review: fn(&mut Shell, &mut Context<Shell>),
    pub on_edit: fn(&mut Shell, &mut Context<Shell>),
    pub on_cancel: fn(&mut Shell, &mut Context<Shell>),
    pub on_done: fn(&mut Shell, &mut Context<Shell>),
    pub on_hold_start: fn(&mut Shell, &mut Context<Shell>),
    pub on_hold_cancel: fn(&mut Shell, &mut Context<Shell>),
}

impl Shell {
    /// Dispatch to the active commit state: done (broadcast) → review (proposed) → compose.
    /// Reads the surface's flow via `view.flow`. The render arms below are byte-identical to the
    /// hand-written `render_send` (and, in Step 2, `render_shield`).
    pub fn render_commit(
        &self,
        view: &'static CommitView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let flow = (view.flow)(self);
        if let Some(tx) = flow.tx {
            return self
                .render_commit_done(view, tx.to_string(), cx)
                .into_any_element();
        }
        if let Some(proposal) = flow.proposal.clone() {
            return self
                .render_commit_review(view, proposal, cx)
                .into_any_element();
        }
        self.render_commit_compose(view, cx).into_any_element()
    }

    /// Compose: amount (ETH) + a recipient, then Review. Validity drives the Review button's
    /// disabled state, re-evaluated live via the input subscriptions.
    fn render_commit_compose(
        &self,
        view: &'static CommitView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let flow = (view.flow)(self);
        let busy = flow.busy;

        let amount_raw = flow.amount.read(cx).value().to_string();
        let recipient_raw = flow.recipient.read(cx).value().to_string();
        let can_review = crate::signer::parse_eth_to_wei(&amount_raw)
            .map(|w| w > U256::ZERO)
            .unwrap_or(false)
            && !recipient_raw.trim().is_empty();

        // The compose hint: a dynamic (Shield 3-way) hook takes precedence over the static line.
        // The dynamic hook reuses the `recipient_raw` the renderer already read (no second `cx`
        // borrow of the input).
        let hint: &'static str = match view.compose_hint_dynamic {
            Some(f) => f(self, &recipient_raw),
            None => view.compose_hint.unwrap_or(""),
        };

        self.commit_shell(
            view,
            v_flex()
                .w_full()
                .gap_5()
                .child(self.commit_heading(view, view.compose_title, view.compose_subtitle, cx))
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(crate::widgets::section_label("Amount", muted))
                        .child(Input::new(&flow.amount).w_full()),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(crate::widgets::section_label(view.recipient_label, muted))
                        .child(Input::new(&flow.recipient).w_full()),
                )
                .children(
                    flow.error
                        .as_ref()
                        .map(|e| crate::widgets::error_line(danger, e.clone())),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new(view.review_button_id)
                                .primary()
                                .label(if busy {
                                    "Reviewing…"
                                } else {
                                    view.review_label
                                })
                                .disabled(busy || !can_review)
                                .on_click(cx.listener(|this, _, _, cx| (view.on_review)(this, cx))),
                        )
                        .child(
                            Button::new(view.cancel_button_id)
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| (view.on_cancel)(this, cx))),
                        ),
                )
                .child(div().text_xs().text_color(muted).child(hint))
                .into_any_element(),
        )
    }

    /// Review: the clear-signing statement (DESIGN §Clear-signing review — transaction-as-hero).
    /// NOT a bordered card: a tiny action label, then the AMOUNT as the oversized mono hero
    /// (dimmed decimals), then `TO` + the recipient via `truncated_address`, then the danger /
    /// caution lines, then the quiet supporting facts (any `extra_rows`) demoted between
    /// hairlines, then the unchanged hold-to-confirm + Edit. Rendered from the proposal SNAPSHOT —
    /// never the live input.
    fn render_commit_review(
        &self,
        view: &'static CommitView,
        proposal: Proposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let secondary = theme.secondary;
        let border = theme.border;
        let danger = theme.danger;
        let is_dark = theme.is_dark();
        let mono = theme.mono_font_family.clone();
        let flow = (view.flow)(self);

        let gross = proposal.intent.value;
        let recipient = proposal.recipient.clone();

        // The action label noun ("Sending" / "Shielding") — derived from the descriptor's own
        // busy verb (the `&'static` `CommitView` is shared with the off-limits swap descriptor, so
        // a new noun field can't be added; the busy label is the descriptor's authoritative verb).
        let noun = view.hold_label_busy.trim_end_matches('…');

        // The transaction-as-hero block: a tiny action label, then the amount as the oversized
        // mono hero (integer `fg`, decimals + ticker dimmed by color via `money`, no size step).
        let hero = v_flex()
            .w_full()
            .gap_1()
            .child(crate::widgets::section_label(noun, muted))
            .child(
                div()
                    .text_size(px(40.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(money(
                        gross,
                        18,
                        6,
                        Some("ETH"),
                        false,
                        mono.clone(),
                        fg,
                        muted,
                    )),
            );

        // The recipient: a tiny `To` label + the trust-grade identicon/address (no ENS plumbed
        // through the proposal yet, so `None`).
        let to = v_flex()
            .w_full()
            .gap_2()
            .child(crate::widgets::section_label("To", muted))
            .child(crate::widgets::truncated_address(
                recipient.trim(),
                None,
                mono.clone(),
                crate::theme::identity_square(is_dark),
                fg,
                secondary,
                muted,
            ));

        // The quiet supporting facts (Shield's Railgun fee + net), demoted between two hairlines.
        // Empty for a public send, where there is nothing to demote.
        let facts = (!view.extra_rows.is_empty()).then(|| {
            let mut col = v_flex()
                .w_full()
                .child(div().w_full().h(px(1.0)).bg(border));
            for row in view.extra_rows {
                col = col.child(kv_money_row(
                    row.label,
                    (row.compute)(gross),
                    mono.clone(),
                    fg,
                    muted,
                ));
            }
            col.child(div().w_full().h(px(1.0)).bg(border))
        });

        self.commit_shell(
            view,
            v_flex()
                .w_full()
                .gap_4()
                .child(self.commit_heading(view, view.review_title, view.review_subtitle, cx))
                .child(hero)
                .child(to)
                .child(self.commit_honesty(view, cx))
                .children(
                    flow.error
                        .as_ref()
                        .map(|e| crate::widgets::error_line(danger, e.clone())),
                )
                .children(facts)
                .child(self.hold_to_confirm(view, cx))
                .child(
                    Button::new(view.edit_button_id)
                        .ghost()
                        .w_full()
                        .label("Edit")
                        .on_click(cx.listener(|this, _, _, cx| (view.on_edit)(this, cx))),
                )
                .into_any_element(),
        )
    }

    /// Done: the action broadcast — on its way. The success copy is per-surface.
    fn render_commit_done(
        &self,
        view: &'static CommitView,
        tx: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let success = theme.success;
        let mono = theme.mono_font_family.clone();

        self.commit_shell(
            view,
            v_flex()
                .w_full()
                .items_center()
                .gap_4()
                .child(
                    Icon::new(IconName::CircleCheck)
                        .text_color(success)
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(view.done_title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .text_center()
                        .child(view.done_body),
                )
                .child(
                    div()
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .font_family(mono)
                        .text_xs()
                        .text_color(muted)
                        .child(crate::widgets::short_addr(&tx)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(view.copy_button_id)
                                .ghost()
                                .label("Copy tx hash")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(tx.clone()));
                                })),
                        )
                        .child(
                            Button::new(view.done_button_id)
                                .primary()
                                .label("Done")
                                .on_click(cx.listener(|this, _, _, cx| (view.on_done)(this, cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The honesty lines as caution affordances (DESIGN §Color rule 7): an inline
    /// TriangleAlert icon + risk text, NO box and NO keyline. The irreversible /
    /// funds-are-lost line takes the loud `danger` register; the softer cautions take
    /// amber, with `emphasized` driving the text weight.
    fn commit_honesty(
        &self,
        view: &'static CommitView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let is_dark = theme.is_dark();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let amber = crate::theme::amber(is_dark);

        let mut col = v_flex().w_full().gap_2();
        for line in view.honesty {
            // The irreversible / funds-are-lost line is the danger register; the rest are amber.
            let danger_line = line.text.contains("can't be undone") || line.text.contains("lost");
            let el = if danger_line {
                crate::widgets::error_line(danger, line.text)
            } else {
                crate::widgets::caution_line(amber, muted, line.emphasized, line.text)
            };
            col = col.child(el);
        }
        col
    }

    /// The hand-built hold-to-confirm: an amber fill sweeps the button width over [`SHIELD_HOLD`]
    /// while held; completing the hold fires the surface's confirm (via the surface-checked
    /// timer in `*_hold_start`), releasing early resets it. The label sits above the sweep.
    ///
    /// `pub(crate)` so Swap's bespoke review screen (`swap_view.rs`) can reuse the exact same
    /// amber hold widget without re-implementing the sweep + mouse wiring (it routes to
    /// [`SWAP_VIEW`](crate::swap_view::SWAP_VIEW)'s hold handlers).
    pub(crate) fn hold_to_confirm(
        &self,
        view: &'static CommitView,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let amber_tint = crate::theme::amber_tint(theme.is_dark());
        let flow = (view.flow)(self);
        let holding = flow.holding;
        let busy = flow.busy;

        let label = if busy {
            view.hold_label_busy
        } else if holding {
            view.hold_label_holding
        } else {
            view.hold_label_idle
        };

        let fill = if holding {
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .bg(amber_tint)
                .with_animation(
                    view.hold_fill_id,
                    Animation::new(SHIELD_HOLD),
                    |el, delta| el.w(relative(delta)),
                )
                .into_any_element()
        } else {
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(0.0))
                .into_any_element()
        };

        div()
            .id(view.hold_id)
            .relative()
            .overflow_hidden()
            .w_full()
            .h(px(44.0))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(surface)
            .cursor_pointer()
            .child(fill)
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(label),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| (view.on_hold_start)(this, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| (view.on_hold_cancel)(this, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| (view.on_hold_cancel)(this, cx)),
            )
    }

    /// The shared centered shell for every commit state (mirrors `send_shell` / `shield_shell`).
    /// `pub(crate)` so Swap's bespoke compose/review/done can sit in the same centered card frame.
    pub(crate) fn commit_shell(
        &self,
        _view: &'static CommitView,
        inner: gpui::AnyElement,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_8()
            .child(v_flex().w(px(460.0)).items_start().child(inner))
    }

    /// The commit heading: a neutral low-chroma glyph + H1 + muted subtitle. The glyph is
    /// deliberately NOT cyan/amber — these surfaces sit off the actor axis (DESIGN); the human
    /// signal lives on the hold-to-confirm. `pub(crate)` so Swap's bespoke screens share the
    /// identical heading treatment.
    pub(crate) fn commit_heading(
        &self,
        view: &'static CommitView,
        title: &'static str,
        subtitle: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let glyph_tone = (view.glyph_tone)(theme.is_dark());

        h_flex()
            .w_full()
            .items_start()
            .gap_3()
            .child(
                div()
                    .size(px(28.0))
                    .rounded(px(6.0))
                    .bg(glyph_tone)
                    .flex_shrink_0(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child(title),
                    )
                    .child(div().text_sm().text_color(muted).child(subtitle)),
            )
    }
}
