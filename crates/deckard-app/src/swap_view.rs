//! Swap — the CoW Protocol swap flow (#25)'s bespoke render + its [`CommitView`] descriptor.
//!
//! Unlike Send/Shield (which share the generic `render_commit` driver), Swap's compose screen has
//! two **token pickers** and a live **quote summary** the generic key/value money rows can't
//! express, and its review card shows buy / min-receive rows in *token* units (not ETH). So the
//! compose / quote-summary / review / done arms are hand-written here in [`Shell::render_swap`],
//! while the parts that ARE identical to every commit surface — the centered card frame
//! ([`Shell::commit_shell`]), the neutral glyph + H1 + subtitle ([`Shell::commit_heading`]), and
//! the amber hold-to-confirm sweep ([`Shell::hold_to_confirm`]) — are reused verbatim.
//!
//! The clear-signing contract is unchanged: plain language, exact mono figures, the honest "this
//! order is public" / "you receive at least the minimum" lines, and confirm is a **hold** (the
//! amber sweep), never a tap. The heading glyph is the neutral low-chroma `shield` tone — a swap
//! sits *off* the cyan/agent + amber/human actor axis; the human signal lives on the hold.
//!
//! The token swatch is a small rounded square in the cool `identity_square` neutral, never gold or
//! amber (DESIGN §Color: identity colors avoid the warm band so they never read as actor signal).
//!
//! [`SWAP_VIEW`] is the descriptor `commit_heading` / `hold_to_confirm` read for this surface's copy
//! and its hold handler routing; the compose/review descriptor text fields just carry swap copy so
//! the shared widgets have something to render. The actual swap state + handlers live on `Shell`
//! (`shell.rs`), and the snapshot-based orchestrator lives in `swap.rs`.

use gpui::{
    div, prelude::FluentBuilder, px, ClipboardItem, Context, FontWeight, Hsla, IntoElement,
    ParentElement, SharedString, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex, ActiveTheme, Disableable, Icon, IconName,
};

use deckard_core::{tokens_for, Address, U256};

use crate::commit_view::{CommitView, HonestyLine};
use crate::money::money;
use crate::shell::{Shell, Surface};
use crate::theme;

/// The Swap surface descriptor. Unlike Send/Shield it does NOT feed `render_commit` (Swap's
/// compose + review are bespoke — see [`Shell::render_swap`]); it carries the copy, ids, glyph tone,
/// and the hold handler routing that the SHARED widgets ([`Shell::commit_heading`],
/// [`Shell::hold_to_confirm`]) read for this surface. The compose/review/done text fields hold the
/// swap copy so the shared heading widget renders the right strings; the `extra_rows` /
/// `compose_hint*` slots are unused by the bespoke render and left empty.
pub static SWAP_VIEW: CommitView = CommitView {
    // The swap flow's live state + the neutral "shield / private" glyph tone (a swap is neither an
    // agent nor a human signal — it sits off the actor axis like Shield).
    flow: swap_flow,
    glyph_tone: theme::shield,

    // --- compose (read by `commit_heading` on the compose arm) ---
    compose_title: "Swap",
    compose_subtitle:
        "Trade one token for another via CoW Protocol. Your wallet receives the bought token; the order is public on the orderbook.",
    // Unused by the bespoke compose (token pickers replace the single recipient input), but the
    // descriptor field is non-optional; carry a sensible label rather than an empty string.
    recipient_label: "Receiver",
    review_button_id: "swap-review",
    review_label: "Review order",
    cancel_button_id: "swap-cancel",
    // The bespoke compose draws its own hints inline; no generic hint hook.
    compose_hint: None,
    compose_hint_dynamic: None,

    // --- review (read by `commit_heading` on the review arm) ---
    review_title: "Review swap",
    review_subtitle: "Confirm what you sell, the minimum you receive, and where it goes, then swap with ⌘↵.",
    // The bespoke review card builds its own token-denominated rows; the generic ETH money rows
    // don't apply.
    extra_rows: &[],
    honesty: &[
        HonestyLine {
            text: "This order is public on the CoW orderbook.",
            emphasized: true,
            danger: false,
        },
        HonestyLine {
            text: "You receive at least the minimum shown. A worse price never settles.",
            emphasized: false,
            danger: false,
        },
    ],
    hold_id: "swap-hold",
    hold_label_idle: "Swap",
    hold_label_busy: "Swapping…",
    edit_button_id: "swap-edit",

    // --- done (read by `commit_heading` is N/A; the bespoke done draws its own copy) ---
    done_title: "Order submitted",
    done_body:
        "Your order is open on the CoW orderbook. It settles when a solver fills it at or above your minimum.",
    copy_button_id: "swap-copy-uid",
    done_button_id: "swap-done",

    // --- handlers (route to the surface's existing `impl Shell` swap methods) ---
    on_review: review_swap,
    on_edit: open_swap,
    on_cancel: open_home,
    on_done: open_home,
    on_hold_start: swap_hold_start,
};

/// Re-acquire the swap flow's state from the shell (the descriptor's `flow` selector).
fn swap_flow(shell: &Shell) -> &crate::commit_flow::CommitFlow {
    &shell.swap
}

// Thin free-function adapters so the descriptor's `fn(&mut Shell, &mut Context<Shell>)` slots can
// name the surface's handlers (a `&'static` descriptor can't hold a closure, and the methods take
// `&mut self`). Each is a one-line forward to the existing handler in `shell.rs`.
fn review_swap(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.review_swap(cx);
}
fn open_swap(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open_swap(cx);
}
fn open_home(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.open(Surface::Home, cx);
}
fn swap_hold_start(shell: &mut Shell, cx: &mut Context<Shell>) {
    shell.swap_hold_start(cx);
}

impl Shell {
    /// The Swap surface: a bespoke dispatch over the swap flow's state — done (submitted, has a
    /// uid) → review (a proposal is installed) → compose. NOT `render_commit`: compose has token
    /// pickers + a quote summary and review shows token-denominated buy/min-receive rows the
    /// generic renderer can't express. Reuses `commit_shell` / `commit_heading` / `hold_to_confirm`.
    pub fn render_swap(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(uid) = self.swap_uid.clone() {
            return self.render_swap_done(uid, cx).into_any_element();
        }
        if let Some(proposal) = self.swap.proposal.clone() {
            return self.render_swap_review(proposal, cx).into_any_element();
        }
        self.render_swap_compose(cx).into_any_element()
    }

    /// Compose: a sell amount + a sell-token picker + a buy-token picker + Get quote, then the live
    /// quote summary card once a quote is in hand. The pickers open a small inline token list from
    /// `tokens_for(chain_id)`; `open_swap` seeds the first two distinct tokens so the picker is
    /// never empty on first paint.
    fn render_swap_compose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let mono = theme.mono_font_family.clone();
        let chain_id = self.chain_id();
        let busy = self.swap.busy;
        let quoting = self.swap_quoting;

        let amount_raw = self.swap.amount.read(cx).value().to_string();
        let amount_ok = crate::signer::parse_eth_to_wei(&amount_raw)
            .map(|w| w > U256::ZERO)
            .unwrap_or(false);
        let tokens_ok = match (self.swap_sell_token, self.swap_buy_token) {
            (Some(s), Some(b)) => s != b,
            _ => false,
        };
        let can_quote = amount_ok && tokens_ok && !quoting && !busy;

        // The two token pickers. Each is a labeled row: a swatch + symbol button per token; the
        // active one is primary, the rest ghost. Tapping sets the side (and clears any stale quote,
        // handled in the shell handler).
        let sell_picker = self.render_token_picker(
            chain_id,
            self.swap_sell_token,
            self.swap_buy_token,
            true,
            cx,
        );
        let buy_picker = self.render_token_picker(
            chain_id,
            self.swap_buy_token,
            self.swap_sell_token,
            false,
            cx,
        );

        // The receive-side figure: the post-slippage minimum the wallet receives, shown as the
        // editorial large-mono hero for the "Receive at least" field. Derived from the live quote;
        // before a quote exists it's a dimmed placeholder so the field still reads as an amount slot.
        let receive_hero = self
            .swap_quote
            .as_ref()
            .map(|q| {
                let buy_tok = q.quote.buy_token;
                let buy_dec = crate::swap::token_decimals(chain_id, buy_tok);
                let min_receive = deckard_core::apply_slippage(
                    q.quote.buy_amount,
                    deckard_core::DEFAULT_SLIPPAGE_BPS,
                );
                div()
                    .text_3xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(money(
                        min_receive,
                        buy_dec,
                        6,
                        None,
                        false,
                        mono.clone(),
                        fg,
                        muted,
                    ))
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .font_family(mono.clone())
                    .text_3xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child("—")
                    .into_any_element()
            });

        self.commit_shell(
            &SWAP_VIEW,
            v_flex()
                .w_full()
                .gap_5()
                .child(self.commit_heading(
                    &SWAP_VIEW,
                    SWAP_VIEW.compose_title,
                    SWAP_VIEW.compose_subtitle,
                    cx,
                ))
                // SELL field — section label, the editable mono amount input, the sell-token chips.
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(crate::widgets::section_label("Sell", muted))
                        .child(Input::new(&self.swap.amount).w_full())
                        .child(sell_picker),
                )
                // A thin direction divider: a hairline on each side of a small neutral swap knob
                // (DESIGN: hierarchy from hairlines + whitespace, not a card).
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(div().flex_1().h(px(1.0)).bg(border))
                        .child(
                            Icon::new(IconName::Replace)
                                .text_color(muted)
                                .flex_shrink_0(),
                        )
                        .child(div().flex_1().h(px(1.0)).bg(border)),
                )
                // RECEIVE field — section label, the large-mono minimum-receive hero, the buy chips.
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(crate::widgets::section_label("Receive at least", muted))
                        .child(receive_hero)
                        .child(buy_picker),
                )
                .children(self.swap.error.as_ref().map(|e| error_line(e, cx)))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new("swap-get-quote")
                                .primary()
                                .label(if quoting { "Getting quote…" } else { "Get quote" })
                                .disabled(!can_quote)
                                .on_click(cx.listener(|this, _, _, cx| this.get_swap_quote(cx))),
                        )
                        .child(
                            Button::new(SWAP_VIEW.cancel_button_id)
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.open(Surface::Home, cx))),
                        ),
                )
                // Once a quote is in, the inline quote/route strip (rate · via CoW · slippage · fee)
                // below a hairline — NOT a bordered card (DESIGN editorial: state facts inline).
                .children(
                    self.swap_quote
                        .as_ref()
                        .map(|q| self.render_quote_summary(q, chain_id, fg, muted, cx)),
                )
                // Once a quote is in, the primary CTA to proceed to the clear-signing order review +
                // hold-to-confirm. (Re-clicking "Get quote" above re-prices.)
                .children(self.swap_quote.as_ref().map(|_| {
                    Button::new(SWAP_VIEW.review_button_id)
                        .primary()
                        .w_full()
                        .label(SWAP_VIEW.review_label)
                        .disabled(self.swap.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.review_swap(cx)))
                }))
                .child(
                    div().text_xs().text_color(muted).child(
                        "A quote is good for about 30 minutes; we re-check the price the moment you confirm.",
                    ),
                )
                .into_any_element(),
        )
    }

    /// A single token-side picker: an inline row of token chips for the chain's curated list. The
    /// active token's chip is `primary`, the rest `ghost`; the *other* side's current token is
    /// disabled (you can't sell and buy the same token). Each chip carries the cool
    /// `identity_square` swatch + the ticker. The grouping label ("Sell" / "Receive at least") is
    /// owned by the enclosing field in `render_swap_compose`, so the picker draws only the chips.
    fn render_token_picker(
        &self,
        chain_id: u64,
        active: Option<Address>,
        other: Option<Address>,
        is_sell: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let swatch = theme::identity_square(theme.is_dark());

        let mut row = h_flex().w_full().gap_2().flex_wrap();
        for (i, tok) in tokens_for(chain_id).iter().enumerate() {
            let addr = tok.address;
            let is_active = active == Some(addr);
            let is_other = other == Some(addr);
            // Stable, side-scoped id so the two pickers never collide on a shared ticker.
            let id = SharedString::from(format!(
                "swap-tok-{}-{}",
                if is_sell { "sell" } else { "buy" },
                i
            ));
            let chip = Button::new(id)
                .when(is_active, |b| b.primary())
                .when(!is_active, |b| b.ghost())
                .disabled(is_other)
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(token_swatch(swatch))
                        .child(div().text_sm().child(tok.symbol)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if is_sell {
                        this.set_swap_sell_token(addr, cx);
                    } else {
                        this.set_swap_buy_token(addr, cx);
                    }
                }));
            row = row.child(chip);
        }

        row
    }

    /// The live quote/route strip (shown once a quote is fetched): the indicative rate, the route
    /// (CoW Protocol), the max slippage, the network fee, and the order's freshness — an INLINE strip
    /// below a hairline (DESIGN editorial: facts stated inline, never a bordered summary card). This
    /// is indicative only; the binding figures live on the review screen, built from the re-quote at
    /// confirm time. The sell/min-receive figures live as the field heroes above, so they're not
    /// restated here (state each figure once).
    fn render_quote_summary(
        &self,
        quote: &deckard_core::QuoteResponse,
        chain_id: u64,
        fg: Hsla,
        muted: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let border = theme.border;
        let mono = theme.mono_font_family.clone();

        let sell_tok = quote.quote.sell_token;
        let buy_tok = quote.quote.buy_token;
        let sell_dec = crate::swap::token_decimals(chain_id, sell_tok);
        let buy_dec = crate::swap::token_decimals(chain_id, buy_tok);
        let sell_sym = crate::swap::token_symbol(chain_id, sell_tok);
        let buy_sym = crate::swap::token_symbol(chain_id, buy_tok);

        // The gross sell (after-fee + fee) and the network fee, in the sell token's atoms.
        let gross_sell = quote
            .quote
            .sell_amount
            .saturating_add(quote.quote.fee_amount);
        let fee = quote.quote.fee_amount;

        // The indicative rate "1 SELL = N BUY": N in buy-token units per ONE sell-token unit. Scaled
        // so `format_amount(.., buy_dec)` renders it directly — N·10^buy_dec = buy_amount·10^sell_dec
        // / gross_sell (zero when the gross is degenerate, so a malformed quote never divides by 0).
        let rate_label = if gross_sell.is_zero() {
            None
        } else {
            // `sell_dec` is a curated token's decimals (<= 18), so 10^sell_dec fits a u128.
            let scale = U256::from(10u128.pow(sell_dec as u32));
            let rate_atoms = quote.quote.buy_amount.saturating_mul(scale) / gross_sell;
            let rate = deckard_core::format_amount(rate_atoms, buy_dec, 4);
            Some(format!("1 {sell_sym} = {rate} {buy_sym}"))
        };

        // Each fact is a small secondary-text span; mono is reserved for the figures. Wraps so the
        // strip never overflows the pane. The fee uses `money()` (an `h_flex` that carries its own
        // mono font + color-dimming), so it sits inline beside its label.
        let strip = h_flex()
            .w_full()
            .flex_wrap()
            .gap_3()
            .text_xs()
            .text_color(muted)
            .when_some(rate_label, |el, r| {
                el.child(div().font_family(mono.clone()).text_color(fg).child(r))
            })
            .child(div().child("via CoW Protocol"))
            .child(
                h_flex()
                    .gap_1()
                    .child("max slippage")
                    .child(div().font_family(mono.clone()).text_color(fg).child("0.5%")),
            )
            .child(h_flex().gap_1().child("network fee").child(money(
                fee,
                sell_dec,
                6,
                Some(sell_sym),
                false,
                mono,
                muted,
                muted,
            )))
            .child(div().child(format!("valid until {}", short_clock(quote.quote.valid_to))));

        // The hairline above the strip is the only rule — no box, no fill.
        v_flex()
            .w_full()
            .pt_3()
            .border_t_1()
            .border_color(border)
            .child(strip)
    }

    /// Review: the clear-signing card for the bound order — what you sell (gross), the minimum you
    /// receive, the receiver (your own wallet), the max slippage, and the order's expiry — plus the
    /// honesty lines and the amber hold-to-confirm. Built from the proposal SNAPSHOT (the bound
    /// order's request_id rides the proposal; the display summary rides `proposal.recipient`).
    fn render_swap_review(
        &self,
        proposal: crate::commit_flow::Proposal,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let mark_fill = theme::identity_square(theme.is_dark());
        let mono = theme.mono_font_family.clone();
        let chain_id = self.chain_id();

        // Pull the bound figures off the last quote (the quote that produced this proposal). The
        // proposal's `intent` carries the swap's value, but the token-denominated sell/buy figures
        // come from the quote snapshot still held on the shell — cleared only on a compose edit,
        // which `review_swap` blocks once a proposal is live.
        let (sell_row, recv_row, rate_row, valid_row) = match self.swap_quote.as_ref() {
            Some(q) => {
                let sell_tok = q.quote.sell_token;
                let buy_tok = q.quote.buy_token;
                let sell_dec = crate::swap::token_decimals(chain_id, sell_tok);
                let buy_dec = crate::swap::token_decimals(chain_id, buy_tok);
                let sell_sym = crate::swap::token_symbol(chain_id, sell_tok);
                let buy_sym = crate::swap::token_symbol(chain_id, buy_tok);
                let gross_sell = q.quote.sell_amount.saturating_add(q.quote.fee_amount);
                let min_receive = deckard_core::apply_slippage(
                    q.quote.buy_amount,
                    deckard_core::DEFAULT_SLIPPAGE_BPS,
                );
                // The indicative rate "1 SELL = N BUY" (same scaling as the compose strip).
                let rate = if gross_sell.is_zero() {
                    None
                } else {
                    let scale = U256::from(10u128.pow(sell_dec as u32));
                    let rate_atoms = q.quote.buy_amount.saturating_mul(scale) / gross_sell;
                    let r = deckard_core::format_amount(rate_atoms, buy_dec, 4);
                    Some(format!("1 {sell_sym} = {r} {buy_sym}"))
                };
                (
                    Some(token_money_row(
                        "You sell",
                        gross_sell,
                        sell_dec,
                        sell_sym,
                        mono.clone(),
                        fg,
                        muted,
                    )),
                    Some(token_money_row(
                        "You receive at least",
                        min_receive,
                        buy_dec,
                        buy_sym,
                        mono.clone(),
                        fg,
                        muted,
                    )),
                    rate,
                    Some(short_clock(q.quote.valid_to)),
                )
            }
            None => (None, None, None, None),
        };

        // The receiver is always your own wallet (a swap never sends elsewhere). Rendered with the
        // trust-grade identicon + truncation so the destination is verifiable at the moment of sign.
        let receiver = self.wallet_address_string();

        // The quiet supporting facts as a flat hairline-ruled kv list — top + bottom hairline, a
        // single rule between rows, NO bordered card (DESIGN editorial: state each fact once below a
        // hairline). Each row is built then folded so only rows after the first carry a top hairline.
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        if let Some(r) = sell_row {
            rows.push(r.into_any_element());
        }
        if let Some(r) = recv_row {
            rows.push(r.into_any_element());
        }
        if let Some(rate) = rate_row {
            rows.push(kv_text_row("Rate", rate, mono.clone(), fg, muted).into_any_element());
        }
        rows.push(
            kv_address_row(
                "Receiver",
                receiver.trim(),
                mono.clone(),
                mark_fill,
                fg,
                muted,
            )
            .into_any_element(),
        );
        rows.push(
            kv_text_row(
                "Max slippage",
                "0.5%".to_string(),
                mono.clone(),
                muted,
                muted,
            )
            .into_any_element(),
        );
        rows.push(
            kv_text_row(
                "Route",
                "CoW Protocol".to_string(),
                mono.clone(),
                muted,
                muted,
            )
            .into_any_element(),
        );
        if let Some(valid) = valid_row {
            rows.push(kv_text_row("Valid until", valid, mono, muted, muted).into_any_element());
        }

        let mut kvlist = v_flex()
            .w_full()
            .border_t_1()
            .border_b_1()
            .border_color(border);
        for (i, row) in rows.into_iter().enumerate() {
            kvlist = kvlist.child(
                div()
                    .w_full()
                    .when(i > 0, |d| d.border_t_1().border_color(border))
                    .child(row),
            );
        }

        self.commit_shell(
            &SWAP_VIEW,
            v_flex()
                .w_full()
                .gap_4()
                .child(self.commit_heading(
                    &SWAP_VIEW,
                    SWAP_VIEW.review_title,
                    SWAP_VIEW.review_subtitle,
                    cx,
                ))
                // A faint reminder of the human-readable summary the proposal snapshot carries
                // (e.g. "0.05 WETH → at least 92.1 COW"), so the list matches what was reviewed.
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(proposal.recipient.clone()),
                )
                .child(kvlist)
                .child(self.commit_honesty_swap(cx))
                .children(self.swap.error.as_ref().map(|e| error_line(e, cx)))
                .child(self.hold_to_confirm(&SWAP_VIEW, cx))
                .child(
                    Button::new(SWAP_VIEW.edit_button_id)
                        .ghost()
                        .w_full()
                        .label("Edit")
                        .on_click(cx.listener(|this, _, _, cx| this.open_swap(cx))),
                )
                .into_any_element(),
        )
    }

    /// Done: the order is submitted — open on the orderbook. Shows a check, the success copy, the
    /// CoW uid in a mono chip with Copy, and Done. The uid is a string (not a tx hash), so this is
    /// bespoke rather than `render_commit_done`.
    ///
    /// TODO(swap-lifecycle): a Track-status / Cancel affordance belongs here once the open-order
    /// poll loop + in-app cancel land (the daemon `cancel_order` / `pending_list` paths already
    /// exist; wiring them is deferred for this increment).
    fn render_swap_done(&self, uid: String, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let success = theme.success;
        let mono = theme.mono_font_family.clone();
        let uid_for_copy = uid.clone();

        self.commit_shell(
            &SWAP_VIEW,
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
                        .child(SWAP_VIEW.done_title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .text_center()
                        .child(SWAP_VIEW.done_body),
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
                        .child(crate::widgets::short_addr(&uid)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(SWAP_VIEW.copy_button_id)
                                .ghost()
                                .label("Copy uid")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        uid_for_copy.clone(),
                                    ));
                                })),
                        )
                        .child(
                            Button::new(SWAP_VIEW.done_button_id)
                                .primary()
                                .label("Done")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.open(Surface::Home, cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The swap honesty lines as inline caution lines (DESIGN §Color rule 7: a `TriangleAlert`
    /// icon + risk text, no box, no keyline). A tiny local copy reading [`SWAP_VIEW`]'s `honesty`
    /// slice; each line tints the icon amber (the caution register) with the emphasized line at
    /// medium weight so the "this order is public" caution leads.
    fn commit_honesty_swap(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let amber = theme::amber(theme.is_dark());

        let mut col = v_flex().w_full().gap_2();
        for line in SWAP_VIEW.honesty {
            col = col.child(crate::widgets::caution_line(
                amber,
                muted,
                line.emphasized,
                line.text,
            ));
        }
        col
    }
}

/// A small rounded square token swatch in a cool neutral (DESIGN: identity colors avoid the warm /
/// amber band, never gold). Sized to sit inline with a ticker in a chip.
fn token_swatch(tone: Hsla) -> impl IntoElement {
    div()
        .size(px(16.0))
        .rounded(crate::tokens::RADIUS_INPUT)
        .bg(tone)
        .flex_shrink_0()
}

/// A label/value money row in token units: label left (muted), the amount + ticker right (mono),
/// dimming the fraction + ticker by color only via [`money`]. The swap analogue of
/// `commit_view::kv_money_row`, but parameterized by token decimals + symbol instead of ETH.
fn token_money_row(
    label: &'static str,
    raw: U256,
    decimals: u8,
    symbol: &str,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    let unit = if symbol.is_empty() {
        None
    } else {
        Some(symbol)
    };
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_3()
        .child(div().text_sm().text_color(muted).child(label))
        .child(
            div()
                .text_sm()
                // 6 fractional places, matching the ETH money rows; full precision lives on-chain.
                .child(money(raw, decimals, 6, unit, false, mono, fg, muted)),
        )
}

/// A label/value text row (mono value) — for the rate, slippage, route, and validity rows that
/// aren't money figures.
fn kv_text_row(
    label: &'static str,
    value: String,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_3()
        .child(div().text_sm().text_color(muted).child(label))
        .child(
            div()
                .font_family(mono)
                .text_sm()
                .text_color(fg)
                .child(value),
        )
}

/// The Receiver row in the review kv list: label left (muted), a trust-grade truncated address
/// right (identicon + the canonical `short_addr` mono truncation) via the shared
/// [`crate::widgets::truncated_address`], so the destination is verifiable at the moment of sign.
/// (A swap's receiver is always your own wallet, hence no ENS.)
fn kv_address_row(
    label: &'static str,
    addr: &str,
    mono: SharedString,
    mark_fill: Hsla,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .py_3()
        .child(div().text_sm().text_color(muted).child(label))
        .child(crate::widgets::truncated_address(
            addr, None, mono, mark_fill, muted, muted, fg,
        ))
}

/// A one-line error, in `danger` (matches `commit_view::error_line`). Delegates to the shared
/// `widgets::error_line` (the Lucide `TriangleAlert` icon + danger text) so every error register
/// reads identically across surfaces.
fn error_line(msg: &str, cx: &mut Context<Shell>) -> impl IntoElement {
    crate::widgets::error_line(cx.theme().danger, msg.to_string())
}

/// Render a unix-seconds expiry as a short, human clock for the review card (e.g. `14:32 UTC`).
/// A `validTo` is always near-future (≈30 min out), so a time-of-day clock reads clearer than a
/// full timestamp; we keep it UTC to avoid a misleading local-time read of an on-chain field.
fn short_clock(valid_to: u32) -> String {
    let secs = valid_to as u64;
    let day_secs = secs % 86_400;
    let hh = day_secs / 3_600;
    let mm = (day_secs % 3_600) / 60;
    format!("{hh:02}:{mm:02} UTC")
}
