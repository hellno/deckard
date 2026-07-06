//! commit_view — the generic "compose → review → done" renderer that drives every self-initiated
//! [`CommitFlow`](crate::commit_flow) surface (Send + Shield; Swap reuses the confirm + shell). A
//! single [`CommitView`] descriptor (a `&'static` table of copy, button ids, the heading glyph, and
//! a few per-surface hooks) feeds [`Shell::render_commit`], which renders the shared compose →
//! review → done surface.
//!
//! The review step is the ONE shared clear-signing Review (DESIGN §Clear-signing — E5, #185),
//! rendered for EVERY origin (here a self Send/Shield; an agent proposal and a dapp request route
//! through the same body from `activity_view`). Only the request-origin **rail** changes; the body
//! is identical: `origin_header` → the transaction-as-hero amount ([`tx_hero`]) → the full recipient
//! ([`tx_recipient`]) → the one danger line "This can't be undone." → amber cautions → the quiet
//! facts (From · Network · fee/net · **Allowed by** the rule + cap-after, via [`Shell::review_authority_row`]).
//!
//! Confirm is the platform-aware `⌘↵` key-cap (DESIGN §The confirm pattern — a deliberate click or
//! chord, NOT a hold; the arm-delay gates it), rendered by [`Shell::hold_to_confirm`] and reused by
//! Swap's bespoke review. The clear-signing contract is unchanged: plain language, exact mono
//! figures, danger early (amber = the human-confirm signal).

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, AnyElement, ClipboardItem, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Disableable, Icon, IconName,
};

use deckard_contract::IntentKind;
use deckard_core::U256;

use crate::commit_flow::{CommitFlow, Proposal};
use crate::money::money;
use crate::shell::Shell;
use crate::widgets::{key_cap, kv_row, origin_header, KeyCap, KvValue, Origin};

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

/// The transaction-as-hero amount block shared by every Tx-shaped Review — a self Send/Shield
/// (`render_commit_review`) AND an agent's proposed Tx (`render_activity_review`). A tiny action
/// label, then the amount as the oversized mono hero (integer `fg`, decimals + ticker dimmed by
/// color, no size step). ONE builder so the self review and the agent review render the amount
/// identically — the "one review, header-rail-only difference" invariant (DESIGN §Clear-signing).
/// Never masked: at the moment of authorization you must SEE the figure you are approving.
pub(crate) fn tx_hero(
    noun: &str,
    value: U256,
    unit: Option<&str>,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .child(crate::widgets::section_label(noun, muted))
        .child(
            div()
                .text_size(crate::tokens::TEXT_TX_HERO)
                .font_weight(FontWeight::SEMIBOLD)
                .child(money(value, 18, 6, unit, false, mono, fg, muted)),
        )
        .into_any_element()
}

/// The security-critical recipient block shared by every Tx-shaped Review: a tiny `To` label +
/// identicon + the FULL address (every character, `fg`, not dimmed, wraps for a long 0zk address) —
/// maximal distinguishability at the moment of authorization. ONE builder so the self review and
/// the agent review show the destination identically.
pub(crate) fn tx_recipient(
    recipient: &str,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
    id_fill: Hsla,
) -> gpui::AnyElement {
    let r = recipient.trim();
    v_flex()
        .w_full()
        .gap_2()
        .child(crate::widgets::section_label("To", muted))
        .child(
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(crate::widgets::identity_mark(
                    r,
                    px(16.0),
                    px(4.0),
                    id_fill,
                    fg,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono)
                        .text_sm()
                        .text_color(fg)
                        .child(SharedString::from(r.to_string())),
                ),
        )
        .into_any_element()
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
    /// Stronger weight within the calm caution (amber) register.
    pub emphasized: bool,
    /// The loud-red DANGER register (irreversible / funds-are-lost), not amber caution. Explicit
    /// per line so severity never depends on substring-sniffing the copy: a reword can't silently
    /// downgrade a danger line (DESIGN §Color rule 6).
    pub danger: bool,
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
    // (No review title/subtitle: the shared Review leads with the request-origin rail, not a
    // heading — DESIGN §Clear-signing. The action verb lives in the hero's `SENDING`/`SHIELDING`
    // label, derived from `hold_label_busy`.)
    /// Quiet supporting-fact money rows, demoted between hairlines below the hero (Shield's fee +
    /// net). Empty for Send, which has nothing to demote.
    pub extra_rows: &'static [MoneyRow],
    /// The honesty lines (2 for Send, 3 for Shield), in render order.
    pub honesty: &'static [HonestyLine],
    /// The key-cap confirm button's id, its idle label (the confirm verb, e.g. "Send"), and the
    /// busy label shown while signing (e.g. "Sending…").
    pub hold_id: &'static str,
    pub hold_label_idle: &'static str,
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
    /// The confirm trigger (button click or ⌘↵). Routes to `confirm_send`/`shield`/`swap` via the
    /// surface's `*_hold_start` adapter, which re-checks the surface + the arm-delay.
    pub on_hold_start: fn(&mut Shell, &mut Context<Shell>),
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
        // The wallet's spendable native balance gates the amount. Both Send and Shield reduce
        // native ETH, so an amount over the balance is invalid for either (DESIGN §Required states:
        // amount > balance disables the action with an inline error, rather than a late provider
        // failure). The strict `>` still lets an exactly-full-balance amount through; the daemon's
        // gas-aware check + humanized error is the backstop for the gas-leaves-nothing edge (a
        // precise up-front reserve needs a gas estimate we don't have here). `None` balance
        // (pre-sync) leaves the gate open — we can't claim over-balance.
        let native_wei = self.portfolio.as_ref().map(|p| p.native_wei);
        let parsed = crate::signer::parse_eth_to_wei(&amount_raw)
            .ok()
            .filter(|w| *w > U256::ZERO);
        let over_balance = matches!((parsed, native_wei), (Some(w), Some(bal)) if w > bal);
        let can_review = parsed.is_some() && !recipient_raw.trim().is_empty() && !over_balance;

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
                        .child(Input::new(&flow.amount).w_full())
                        .when(over_balance, |c| {
                            c.child(crate::widgets::error_line(
                                danger,
                                "More than your wallet holds.",
                            ))
                        }),
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

    /// The shared Review's **Allowed by** authority line (DESIGN §Clear-signing): the rule that
    /// permits this move + the daily budget left AFTER it, read from the daemon's live policy via
    /// [`Policy::authority_for`](deckard_contract::Policy::authority_for) so the figure is the SAME
    /// one `evaluate` enforces — the UI never recomputes cap math, so it can't drift. Returns `None`
    /// (line omitted) when the policy is unavailable, no rule governs the action, or the move is
    /// OVER cap: there is then no truthful headroom to cite, and the danger line carries that story
    /// (never show an enforcement claim the engine doesn't back). Shared by every Tx-shaped origin.
    pub(crate) fn review_authority_row(
        &self,
        kind: IntentKind,
        value: U256,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let auth = self.agent_policy.as_ref()?.authority_for(kind, value)?;
        if auth.over_cap {
            return None;
        }
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let primary = theme.foreground;
        let mono = theme.mono_font_family.clone();
        let remaining = deckard_core::format_amount(auth.daily_remaining_after_wei, 18, 4);
        let total = deckard_core::format_amount(auth.daily_cap_wei, 18, 4);
        Some(
            h_flex()
                .w_full()
                .items_baseline()
                .justify_between()
                .gap_4()
                .py_1p5()
                .text_size(crate::tokens::TEXT_BODY)
                .child(div().flex_shrink_0().text_color(muted).child("Allowed by"))
                .child(
                    h_flex()
                        .min_w_0()
                        .items_baseline()
                        .gap_1()
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(primary)
                                .child(SharedString::from(auth.rule_label)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono)
                                .text_color(muted)
                                .child(SharedString::from(format!(
                                    "· {remaining} of {total} ETH daily left after this"
                                ))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Review: the ONE shared clear-signing statement (DESIGN §Clear-signing — transaction-as-hero),
    /// rendered here for a self-initiated Send/Shield. NOT a bordered card: the request-origin rail
    /// (`You are sending`, amber) — the ONLY thing that changes across origins — then the AMOUNT as
    /// the oversized mono hero, then `TO` + the full recipient, one danger line `This can't be
    /// undone.`, the descriptor's amber cautions, then the quiet facts (From · Network · any
    /// fee/net · **Allowed by**) below a hairline, then the ⌘↵ key-cap confirm + Edit. Rendered from
    /// the proposal SNAPSHOT — never the live input.
    fn render_commit_review(
        &self,
        view: &'static CommitView,
        proposal: Proposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let danger = theme.danger;
        let success = theme.success;
        let warn = theme.warning; // kv_row's loud-downgrade tone (unused by the Sans facts here)
        let is_dark = theme.is_dark();
        let mono = theme.mono_font_family.clone();
        let flow = (view.flow)(self);

        let gross = proposal.intent.value;
        let kind = proposal.intent.kind.clone();
        let recipient = proposal.recipient.clone();

        // The action label noun ("Sending" / "Shielding") — derived from the descriptor's own
        // busy verb (the `&'static` `CommitView` is shared with the off-limits swap descriptor, so
        // a new noun field can't be added; the busy label is the descriptor's authoritative verb).
        let noun = view.hold_label_busy.trim_end_matches('…');
        let verb = noun.to_lowercase();

        // Quiet-fact inputs (owned; no theme borrow) — the From identity and the network name.
        let wallet_name = self.wallet_name();
        let from_value = format!(
            "{} · {}",
            wallet_name,
            crate::widgets::short_addr(&self.wallet_address_string())
        );
        let network_name = deckard_core::for_chain(self.chain_id())
            .map(|c| c.network_name)
            .unwrap_or("—");

        // The request-origin rail: a self-initiated move is always You (amber "You are sending").
        // This rail is the ONLY thing that differs across origins — the body below is identical.
        let origin_rail = origin_header(
            Origin::You {
                account: &wallet_name,
                verb: &verb,
            },
            None,
            theme,
        );
        // `theme` is not borrowed past this point — the `self.*(cx)` calls below re-borrow it.

        // The **Allowed by** authority line (rule + cap-after from the live policy). Built now,
        // rendered inside the quiet facts; `None` when there's no truthful headroom to claim.
        let authority = self.review_authority_row(kind, gross, cx);

        // The transaction-as-hero amount + the security-critical recipient — the shared builders,
        // so an agent's proposed Tx renders these identically (the "one review" invariant).
        let hero = tx_hero(noun, gross, Some("ETH"), mono.clone(), fg, muted);
        let to = tx_recipient(
            &recipient,
            mono.clone(),
            fg,
            muted,
            crate::theme::identity_square(is_dark),
        );

        // The ONE danger line for every value move (DESIGN §Clear-signing) — plain and declarative,
        // no textbook blockchain explainer. The descriptor's amber cautions follow it.
        let danger_line = crate::widgets::error_line(danger, "This can't be undone.");

        // Quiet supporting facts, demoted between two hairlines: From · Network · any fee/net rows
        // (Shield's Railgun fee + private net) · the Allowed-by authority line. State each once.
        let mut quiet = v_flex()
            .w_full()
            .child(crate::widgets::divider(border))
            .child(kv_row(
                "From",
                KvValue::Sans(&from_value),
                muted,
                fg,
                success,
                warn,
                mono.clone(),
            ))
            .child(kv_row(
                "Network",
                KvValue::Sans(network_name),
                muted,
                fg,
                success,
                warn,
                mono.clone(),
            ));
        for row in view.extra_rows {
            quiet = quiet.child(kv_money_row(
                row.label,
                (row.compute)(gross),
                mono.clone(),
                fg,
                muted,
            ));
        }
        let quiet = quiet
            .children(authority)
            .child(crate::widgets::divider(border));

        self.commit_shell(
            view,
            v_flex()
                .w_full()
                .gap_4()
                .child(origin_rail)
                .child(hero)
                .child(to)
                .child(danger_line)
                .child(self.commit_honesty(view, cx))
                .children(
                    flow.error
                        .as_ref()
                        .map(|e| crate::widgets::error_line(danger, e.clone())),
                )
                .child(quiet)
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
            // Danger (loud red) vs caution (amber) is an EXPLICIT per-line flag, never sniffed from
            // the copy, so an editorial reword can't silently downgrade a danger line.
            let el = if line.danger {
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
        let fill = theme.muted; // bg.raise2 — the neutral primary fill
        let mono = theme.mono_font_family.clone();
        let amber = crate::theme::amber(theme.is_dark());
        let muted = theme.muted_foreground;
        let flow = (view.flow)(self);
        let busy = flow.busy;

        // The key-cap arms ~450ms after the review appears (the spam-guard): `key_cap`'s `armed`
        // flag renders it amber once `commit_armed()`, dim before, so a too-early click / ⌘↵ reads
        // as "not ready yet" rather than a silently-dead button. The arm timer (`arm_commit`) wakes
        // a re-render at the boundary so it visibly brightens.
        let armed = self.commit_armed();

        let label = if busy {
            view.hold_label_busy
        } else {
            view.hold_label_idle
        };

        // A keyboard-first key-cap confirm (DESIGN.md §The confirm pattern) via the shared,
        // platform-aware `key_cap` widget (⌘↵ on macOS, Ctrl↵ on Linux, the chord as ONE cap). A
        // deliberate click — or ⌘↵ — confirms; this is NOT a hold (the press-and-hold gesture was an
        // anti-pattern). The ⌘↵ chord plus the arm-delay keep it spam-proof. The confirm handler is
        // `on_hold_start` (kept as the trigger slot).
        div()
            .id(view.hold_id)
            .w_full()
            .h(px(48.0))
            .rounded(crate::tokens::RADIUS_MODAL)
            .border_1()
            .border_color(border)
            .bg(fill)
            .flex()
            .items_center()
            .px_4()
            .cursor_pointer()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_sm()
                    .text_color(fg)
                    .child(label),
            )
            .child(div().flex_1())
            .when(!busy, |b| {
                b.child(key_cap(KeyCap::CmdEnter, armed, border, muted, amber, mono))
            })
            .on_click(cx.listener(|this, _, _, cx| (view.on_hold_start)(this, cx)))
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
            .child(
                v_flex()
                    .w(crate::tokens::CONFIRM_W)
                    .items_start()
                    .child(inner),
            )
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
                    .rounded(crate::tokens::RADIUS_ROW)
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
