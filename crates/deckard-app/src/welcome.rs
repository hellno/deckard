//! Portfolio — Deckard's home screen. A calm, dense balance view: the native ETH
//! balance up top, a Send / Receive / Swap action row, then real token holdings read
//! live over Multicall3 (`deckard-core`). Renders instantly from the last cached
//! snapshot; the only loading state is the very first sync.

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, relative, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex, v_flex, ActiveTheme, Disableable, IconName,
};

use deckard_core::U256;

use crate::money::money;
use crate::shell::{Shell, Surface};
use crate::shell_chrome::agent_squircle;
use crate::theme;

/// One row in the holdings table. Carries the raw balance (not a pre-formatted
/// string) so the amount column can render mono-for-money with dimmed decimals.
struct Holding {
    mark: String,
    name: String,
    symbol: String,
    raw: U256,
    decimals: u8,
    max_frac: usize,
}

/// The primary modifier label, per platform (⌘ on macOS, "Ctrl " elsewhere).
#[cfg(target_os = "macos")]
const MOD: &str = "⌘";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl ";

/// Middle-truncate an address string for a tight pill, e.g. `0xA1b2…9F3c`.
fn short_addr(a: &str) -> String {
    if a.len() >= 12 {
        format!("{}…{}", &a[..6], &a[a.len() - 4..])
    } else {
        a.to_string()
    }
}

/// The agent policy card's rows, built from the daemon's LIVE policy — the same fence
/// `deckard_policy_get` shows an MCP client. Pure so the mapping is testable: an empty
/// allowlist honestly reads "any", the approval mode is spelled out, and a STOP
/// (`revoked`) is never hidden. `masked` bullets only the spent-today figure — that is
/// activity; the caps are config the user set, not a balance to hide.
fn agent_policy_rows(p: &deckard_contract::Policy, masked: bool) -> Vec<(&'static str, String)> {
    use deckard_contract::ApprovalMode;
    let eth = |wei: U256| format!("{} ETH", deckard_core::format_amount(wei, 18, 6));
    vec![
        ("Per-transaction cap", eth(p.per_tx_cap_wei)),
        ("Daily budget", format!("{} / day", eth(p.daily_cap_wei))),
        (
            "Spent today",
            crate::money::mask_money(masked, &eth(p.spent_today_wei)),
        ),
        (
            "Recipients",
            if p.allow_to.is_empty() {
                "any (no allowlist)".to_string()
            } else {
                format!("{} allowed", p.allow_to.len())
            },
        ),
        (
            "Auto-shield inbound",
            format!("≥ {}", eth(p.auto_shield_min_wei)),
        ),
        (
            "Approval",
            match p.require_approval {
                ApprovalMode::Never => "auto within caps · over cap denied",
                ApprovalMode::OverCap => "auto within caps · ask over cap",
                ApprovalMode::Always => "ask for every move",
            }
            .to_string(),
        ),
        (
            "STOP brake",
            if p.revoked {
                "engaged — unlock to re-arm".to_string()
            } else {
                "ready".to_string()
            },
        ),
    ]
}

impl Shell {
    /// The selected wallet's home — a left-anchored, scrollable main pane:
    /// wallet-name H1 + address subtitle, the balance hero, Send/Receive/Swap,
    /// then live holdings. The synced/trust status line lives in the bottom
    /// status strip (`shell_chrome.rs`), not here.
    pub fn render_wallet_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono: SharedString = theme.mono_font_family.clone();

        // A small bordered key-hint chip, e.g. ⌘K.
        let chip = move |keys: String, label: String| {
            h_flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .text_color(fg)
                        .text_xs()
                        .child(keys),
                )
                .child(div().text_xs().text_color(muted).child(label))
        };

        // --- Derive view state from the live portfolio. ---
        let addr_str = self.display_address.to_string();
        let account_pill = short_addr(&addr_str);
        let first_sync = self.portfolio_loading && self.portfolio.is_none();

        // The native ETH balance for the hero — `None` until the first sync lands.
        let native_wei = self.portfolio.as_ref().map(|p| p.native_wei);

        // Holdings rows: ETH first, then each listed token. Each row carries the
        // raw value + decimals + frac so the amount column renders mono-for-money
        // (dimmed decimals) rather than a single-color string.
        let mut holdings: Vec<Holding> = Vec::new();
        if let Some(p) = &self.portfolio {
            holdings.push(Holding {
                mark: "Ξ".into(),
                name: "Ethereum".into(),
                symbol: "ETH".into(),
                raw: p.native_wei,
                decimals: 18,
                max_frac: 4,
            });
            for t in &p.tokens {
                let frac = if t.decimals <= 6 { 2 } else { 4 };
                let mark = t.symbol.chars().next().unwrap_or('•').to_string();
                holdings.push(Holding {
                    mark,
                    name: t.name.to_string(),
                    symbol: t.symbol.to_string(),
                    raw: t.raw,
                    decimals: t.decimals,
                    max_frac: frac,
                });
            }
        }
        let has_tokens = self
            .portfolio
            .as_ref()
            .map(|p| !p.tokens.is_empty())
            .unwrap_or(false);

        // Wallet identity for the header: a desaturated, tinted-neutral square
        // (DESIGN rule 4 — identity squares avoid the warm/amber band).
        let id_square = theme::identity_square(theme.is_dark());
        let wallet_name = if self.viewing_watch {
            "Watched account".to_string()
        } else {
            "Personal".to_string()
        };

        div()
            .size_full()
            .p_8()
            // TODO(scroll): restore a scrollable main pane via a Stateful
            // `div().id(..).overflow_y_scroll()` (the agent draft mis-ordered
            // gpui-component's `overflow_y_scrollbar`). Content is short for now.
            .child(
                v_flex()
                    .items_start()
                    .max_w(px(680.0))
                    .gap_6()
                    // Page header (DESIGN §Page header): identity square + wallet-name
                    // H1 (text.primary, weight 600 — NEVER cyan) + a muted mono,
                    // middle-truncated address subtitle.
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(div().size(px(28.0)).rounded(px(6.0)).bg(id_square))
                                    .child(
                                        v_flex()
                                            .gap_0p5()
                                            .child(
                                                div()
                                                    .text_xl()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(fg)
                                                    .child(wallet_name),
                                            )
                                            .child(
                                                div()
                                                    .font_family(mono.clone())
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .child(account_pill),
                                            ),
                                    ),
                            )
                            .child(
                                Button::new("refresh")
                                    .ghost()
                                    .icon(IconName::Replace)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_portfolio(cx)),
                                    ),
                            ),
                    )
                    // Balance hero: the merged Total (public + private), a Private/Public
                    // allocation bar, and the composition lines (Wave 2 T10).
                    .child(self.render_shielded_hero(native_wei, cx))
                    // Primary actions. Shield (the privacy hero) is the live, primary CTA; Send
                    // is now live too (native ETH). Both sign from YOUR wallet, so both are
                    // disabled while viewing a watched read-only account. Swap stays gated to
                    // the next release and shown disabled rather than inert-but-active.
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            // Shield signs from YOUR wallet, so it's disabled while viewing a
                            // watched read-only account (don't show a funds-moving action in a
                            // someone-else's-address context).
                            .child(
                                Button::new("shield")
                                    .primary()
                                    .label("Shield")
                                    .disabled(self.viewing_watch)
                                    .on_click(cx.listener(|this, _, _, cx| this.open_shield(cx))),
                            )
                            .child(Button::new("receive").ghost().label("Receive").on_click(
                                cx.listener(|this, _, _, cx| this.open(Surface::Receive, cx)),
                            ))
                            .child(
                                Button::new("send")
                                    .ghost()
                                    .label("Send")
                                    .disabled(self.viewing_watch)
                                    .on_click(cx.listener(|this, _, _, cx| this.open_send(cx))),
                            )
                            .child(Button::new("swap").ghost().label("Swap").disabled(true)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Swap arrives in the next release."),
                    )
                    // Holdings, or a state.
                    .child(self.render_holdings(first_sync, has_tokens, holdings, cx))
                    // "What Atlas may do" — the agent policy fence. Atlas is key-less automation
                    // ON this same wallet (same EOA), not a separate account, so its limits live
                    // here in the wallet cockpit. The rows are the daemon's LIVE policy (the same
                    // fence `deckard_policy_get` shows an MCP client), never invented.
                    .child(self.render_agent_fence(cx))
                    // Keyboard hints — the Superhuman/Linear signal.
                    .child(
                        h_flex()
                            .gap_4()
                            .pt_1()
                            .child(chip(format!("{MOD}K"), "Command palette".into()))
                            .child(chip(format!("{MOD}["), "Back".into()))
                            .child(chip(format!("{MOD},"), "Settings".into())),
                    ),
            )
    }

    /// The agent policy fence card ("What Atlas may do") for the wallet home. Atlas is key-less
    /// automation on the SAME wallet EOA — not a separate account — so its limits belong here in
    /// the wallet cockpit, not on a standalone page. The card shows the daemon's LIVE policy fence
    /// (per-tx cap, daily budget, spent-today, recipients, auto-shield minimum, approval mode, the
    /// STOP brake) via `agent_policy_rows` — the same numbers `deckard_policy_get` returns to an
    /// MCP client, never invented. `None` until the first `PolicyGet` lands → a quiet "loading…".
    /// The cyan squircle is the static two-signal identity marker (no pulse).
    fn render_agent_fence(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let surface = theme.secondary;
        let mono: SharedString = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();
        let agent = theme::agent(is_dark);
        let agent_tint = theme::agent_tint(is_dark);

        // One policy row: label left (muted), value right (mono, primary). No per-row hairline —
        // grouping is whitespace.
        let mono_for_row = mono.clone();
        let policy_row = move |label: &'static str, value: String| {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .py_1p5()
                .child(div().text_sm().text_color(muted).child(label))
                .child(
                    div()
                        .font_family(mono_for_row.clone())
                        .text_sm()
                        .text_color(fg)
                        .child(value),
                )
        };

        v_flex()
            .w_full()
            .gap_3()
            // Section header: the cyan squircle (the ONLY cyan here — static identity, no pulse)
            // + a plain title. The agent name H1 (text.primary, NEVER cyan) sits beside it.
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(agent_squircle(px(20.0), px(5.0), agent, agent_tint))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("What Atlas may do"),
                    ),
            )
            // Policy card: one faint frame, no interior grid lines. The rows are the daemon's live
            // fence; until the first fetch lands the card says so instead of showing invented numbers.
            .child(
                v_flex()
                    .w_full()
                    .gap_0()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .bg(surface)
                    .map(|card| match self.agent_policy.as_ref() {
                        Some(p) => card.children(
                            agent_policy_rows(p, self.mask)
                                .into_iter()
                                .map(|(label, value)| policy_row(label, value)),
                        ),
                        None => card.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child("Reading the signer's policy…"),
                        ),
                    }),
            )
            // The fence is read-only here; it's enforced by the signer, edited via policy.json.
            .child(div().text_xs().text_color(muted).child(
                "Atlas acts through the key-less deckard-mcp sidecar, signing from this same \
                     wallet. The signer checks every move against this fence — edit policy.json in \
                     the Deckard config dir to change it.",
            ))
    }

    /// The merged Total hero (Wave 2 T10): `Total = public + private` when both are known, a
    /// Private/Public allocation bar (Private first, neutral shield tone — off the actor axis),
    /// and the composition lines. While the private sync runs the total stays the known public
    /// (never `public + 0`) and the private line reads "syncing…". Clicking the Total masks it.
    fn render_shielded_hero(
        &self,
        native_wei: Option<U256>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let mono: SharedString = theme.mono_font_family.clone();
        let masked = self.mask;
        let is_dark = theme.is_dark();
        let public_tone = theme::identity_square(is_dark);
        let shield_tone = theme::shield(is_dark);

        let snap = self.shielded.as_ref().map(|h| h.snapshot());
        let private_wei = snap.as_ref().and_then(|s| s.shielded_wei);
        let syncing = snap.as_ref().map(|s| s.syncing).unwrap_or(false);
        let public = native_wei;

        // Total: sum only when the private side is known; never `public + 0` while syncing.
        let total = match (public, private_wei) {
            (Some(p), Some(s)) => Some(p.saturating_add(s)),
            (Some(p), None) => Some(p),
            _ => None,
        };

        let hero = div()
            .id("balance-hero")
            .cursor_pointer()
            .text_3xl()
            .font_weight(FontWeight::SEMIBOLD)
            .map(|el| match total {
                Some(wei) => el.child(money(
                    wei,
                    18,
                    4,
                    Some("ETH"),
                    masked,
                    mono.clone(),
                    fg,
                    muted,
                )),
                None => el.font_family(mono.clone()).text_color(muted).child("—"),
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_mask(cx)));

        // No balance yet (first sync): just the placeholder hero.
        let Some(pub_wei) = public else {
            return v_flex().w_full().gap_3().child(hero).into_any_element();
        };

        // A real Private/Public split once the private side is known, else a single Public bar.
        let bar = match private_wei {
            Some(priv_wei) => {
                let total_wei = pub_wei.saturating_add(priv_wei);
                allocation_bar(
                    vec![
                        AllocSegment {
                            label: "Private".into(),
                            fraction: fraction(priv_wei, total_wei),
                            tone: shield_tone,
                        },
                        AllocSegment {
                            label: "Public".into(),
                            fraction: fraction(pub_wei, total_wei),
                            tone: public_tone,
                        },
                    ],
                    masked,
                    border,
                    muted,
                    fg,
                )
            }
            None => allocation_bar(
                vec![AllocSegment {
                    label: "Public".into(),
                    fraction: 1.0,
                    tone: public_tone,
                }],
                masked,
                border,
                muted,
                fg,
            ),
        };

        // Label the hero honestly: a real Total once private is known, otherwise public-only
        // (so the big figure is never read as "public + 0" while the private side syncs).
        let caption = match (private_wei, snap.is_some()) {
            (Some(_), _) => "Total",
            (None, true) => "Public · private balance still syncing",
            (None, false) => "",
        };

        v_flex()
            .w_full()
            .gap_3()
            .child(hero)
            .children((!caption.is_empty()).then(|| div().text_xs().text_color(muted).child(caption)))
            .child(bar)
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(composition_line(
                        "Private",
                        shield_tone,
                        private_wei,
                        syncing,
                        masked,
                        mono.clone(),
                        fg,
                        muted,
                    ))
                    .child(composition_line(
                        "Public",
                        public_tone,
                        Some(pub_wei),
                        false,
                        masked,
                        mono.clone(),
                        fg,
                        muted,
                    )),
            )
            .child(div().text_xs().text_color(muted).child(
                "Private is WETH-equivalent, net of the 0.25% fee, and synced over raw RPC (not independently verified).",
            ))
            .into_any_element()
    }

    /// The holdings region: skeleton on first sync, empty-state when nothing held,
    /// otherwise the live rows plus the listed-tokens-only caveat.
    fn render_holdings(
        &self,
        first_sync: bool,
        has_tokens: bool,
        holdings: Vec<Holding>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let mono: SharedString = theme.mono_font_family.clone();
        let masked = self.mask;

        let mut col = v_flex().w_full().gap_2();

        if first_sync {
            return col
                .child(skeleton_row(theme.secondary, theme.border))
                .child(skeleton_row(theme.secondary, theme.border))
                .into_any_element();
        }

        if holdings.is_empty() {
            return col
                .child(
                    div()
                        .w_full()
                        .px_4()
                        .py_8()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.secondary)
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child("No balances yet"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("Receive funds, or watch an address in Settings."),
                        ),
                )
                .into_any_element();
        }

        for h in holdings {
            col = col.child(render_row(
                theme.foreground,
                theme.muted_foreground,
                theme.border,
                theme.secondary,
                theme.muted,
                h.mark,
                h.name,
                h.symbol,
                h.raw,
                h.decimals,
                h.max_frac,
                masked,
                mono.clone(),
            ));
        }
        if !has_tokens {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .pt_1()
                    .child("Only listed tokens are shown — long-tail tokens may be missing."),
            );
        } else {
            col = col.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .pt_1()
                    .child("Listed tokens only."),
            );
        }
        col.into_any_element()
    }

    /// Project home — the aggregate-of-one for the demo's single project: the
    /// wallet's balance plus a one-line composition (1 wallet · 1 agent). Real
    /// multi-wallet aggregation is fast-follow.
    pub fn render_project_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let mono: SharedString = theme.mono_font_family.clone();
        let id_square = theme::identity_square(theme.is_dark());
        let masked = self.mask;

        let native_wei = self.portfolio.as_ref().map(|p| p.native_wei);

        div()
            .size_full()
            .p_8()
            // TODO(scroll): restore a scrollable main pane via a Stateful
            // `div().id(..).overflow_y_scroll()` (the agent draft mis-ordered
            // gpui-component's `overflow_y_scrollbar`). Content is short for now.
            .child(
                v_flex()
                    .items_start()
                    .max_w(px(680.0))
                    .gap_6()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(div().size(px(28.0)).rounded(px(6.0)).bg(id_square))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(fg)
                                    .child("Personal"),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .gap_3()
                            .child(
                                div()
                                    .id("project-balance-hero")
                                    .cursor_pointer()
                                    .text_3xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .map(|el| match native_wei {
                                        Some(wei) => el.child(money(
                                            wei,
                                            18,
                                            4,
                                            Some("ETH"),
                                            masked,
                                            mono.clone(),
                                            fg,
                                            muted,
                                        )),
                                        None => el
                                            .font_family(mono.clone())
                                            .text_color(muted)
                                            .child("—"),
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mask(cx))),
                            )
                            .children(native_wei.map(|_| {
                                allocation_bar(
                                    vec![AllocSegment {
                                        label: "Public".into(),
                                        fraction: 1.0,
                                        tone: id_square,
                                    }],
                                    masked,
                                    border,
                                    muted,
                                    fg,
                                )
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("1 wallet · 1 agent"),
                    ),
            )
    }
}

/// One segment of the [`allocation_bar`]: a label, its share of the whole (0..=1),
/// and a low-chroma tone. Wave 2 feeds it Private/Public; v1 passes a single Public.
struct AllocSegment {
    label: SharedString,
    fraction: f32,
    tone: Hsla,
}

/// A thin Splits-style allocation bar (DESIGN §Balance hero): low-chroma tonal
/// segments (never amber, rule 5) over a neutral track, each non-zero segment kept
/// ≥3px wide so it stays visible, with a small legend below. When `masked`, the
/// composition is itself private — the bar collapses to one flat neutral track with no
/// legend (part of what the privacy mask hides).
fn allocation_bar(
    segments: Vec<AllocSegment>,
    masked: bool,
    track: Hsla,
    muted: Hsla,
    fg: Hsla,
) -> impl IntoElement {
    // Masked → a single flat neutral bar, no segments, no legend.
    if masked {
        return div()
            .w_full()
            .h(px(8.0))
            .rounded(px(3.0))
            .bg(track)
            .into_any_element();
    }

    // The tonal bar: each segment a fraction of the width, ≥3px, clipped to the rounding.
    let mut bar = h_flex()
        .w_full()
        .h(px(8.0))
        .rounded(px(3.0))
        .bg(track)
        .overflow_hidden();
    for seg in &segments {
        let frac = seg.fraction.clamp(0.0, 1.0);
        // A zero-value segment is omitted — the ≥3px minimum is only for a NON-zero share
        // (DESIGN §Balance hero), so an empty Private slice never shows a phantom sliver.
        if frac <= 0.0 {
            continue;
        }
        bar = bar.child(
            div()
                .h_full()
                .flex_shrink_0()
                .min_w(px(3.0))
                .w(relative(frac))
                .bg(seg.tone),
        );
    }

    // The legend: a tone chip + label + percentage per segment.
    let mut legend = h_flex().gap_4();
    for seg in &segments {
        let pct = (seg.fraction.clamp(0.0, 1.0) * 100.0).round() as u32;
        legend = legend.child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(div().size(px(8.0)).rounded(px(2.0)).bg(seg.tone))
                .child(div().text_xs().text_color(fg).child(seg.label.clone()))
                .child(div().text_xs().text_color(muted).child(format!("{pct}%"))),
        );
    }

    v_flex()
        .w_full()
        .gap_2()
        .child(bar)
        .child(legend)
        .into_any_element()
}

/// `part / total` as a 0..=1 fraction, via integer (bps) math — f32 only at the edge so a
/// huge `U256` can't lose precision in the ratio. Zero `total` → 0.
fn fraction(part: U256, total: U256) -> f32 {
    if total.is_zero() {
        return 0.0;
    }
    let bps = (part.saturating_mul(U256::from(10_000u64)) / total).min(U256::from(10_000u64));
    let bps: u64 = bps.try_into().unwrap_or(0);
    bps as f32 / 10_000.0
}

/// One composition line: a tone chip + label + the (maskable) amount, or "syncing…" while the
/// private side hasn't landed (never a fake zero).
#[allow(clippy::too_many_arguments)]
fn composition_line(
    label: &'static str,
    tone: Hsla,
    wei: Option<U256>,
    syncing: bool,
    masked: bool,
    mono: SharedString,
    fg: Hsla,
    muted: Hsla,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(div().size(px(8.0)).rounded(px(2.0)).bg(tone))
        .child(div().flex_1().text_xs().text_color(muted).child(label))
        .child(div().text_xs().map(|el| match wei {
            Some(w) => el.child(money(w, 18, 4, Some("ETH"), masked, mono, fg, muted)),
            None if syncing => el.text_color(muted).child("syncing…"),
            None => el.text_color(muted).child("—"),
        }))
}

/// A shimmer-free skeleton placeholder row for the first-sync state.
fn skeleton_row(surface: gpui::Hsla, border: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .rounded_lg()
        .border_1()
        .border_color(border)
        .bg(surface)
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(div().size(px(34.0)).rounded_full().bg(border))
                .child(div().w(px(120.0)).h(px(12.0)).rounded_md().bg(border)),
        )
        .child(div().w(px(64.0)).h(px(12.0)).rounded_md().bg(border))
}

/// A single holding row (free fn so the closure-type plumbing stays simple).
#[allow(clippy::too_many_arguments)]
fn render_row(
    fg: gpui::Hsla,
    muted: gpui::Hsla,
    border: gpui::Hsla,
    _surface: gpui::Hsla,
    mark_bg: gpui::Hsla,
    mark: String,
    name: String,
    symbol: String,
    raw: U256,
    decimals: u8,
    max_frac: usize,
    masked: bool,
    mono: SharedString,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        // DESIGN §Holdings table: tight rows, hairline row separators only —
        // no per-row card frame, no fill.
        .border_b_1()
        .border_color(border)
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .size(px(34.0))
                        // DESIGN §Radii: a desaturated token SQUARE (6px), not a
                        // round identicon (round is reserved for the human principal).
                        .rounded(px(6.0))
                        .bg(mark_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child(mark),
                        ),
                )
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(fg)
                                .child(name),
                        )
                        .child(div().text_xs().text_color(muted).child(symbol)),
                ),
        )
        .child(div().text_sm().child(money(
            raw, decimals, max_frac, None, masked, mono, fg, muted,
        )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckard_contract::{ApprovalMode, Policy};

    fn policy() -> Policy {
        Policy {
            per_tx_cap_wei: U256::from(100_000_000_000_000_000u128), // 0.1 ETH
            daily_cap_wei: U256::from(500_000_000_000_000_000u128),  // 0.5 ETH
            spent_today_wei: U256::from(20_000_000_000_000_000u128), // 0.02 ETH
            allow_to: vec![],
            auto_shield_min_wei: U256::from(10_000_000_000_000_000u128), // 0.01 ETH
            require_approval: ApprovalMode::OverCap,
            revoked: false,
            allow_swap_tokens: vec![],
        }
    }

    /// The card maps the live policy honestly: real numbers, "any" for an empty
    /// allowlist, the approval mode spelled out, the brake ready.
    #[test]
    fn policy_rows_render_the_live_fence() {
        let rows = agent_policy_rows(&policy(), false);
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| *l == label)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("missing row {label:?}"))
        };
        assert_eq!(get("Per-transaction cap"), "0.1 ETH");
        assert_eq!(get("Daily budget"), "0.5 ETH / day");
        assert_eq!(get("Spent today"), "0.02 ETH");
        assert_eq!(get("Recipients"), "any (no allowlist)");
        assert_eq!(get("Auto-shield inbound"), "≥ 0.01 ETH");
        assert_eq!(get("Approval"), "auto within caps · ask over cap");
        assert_eq!(get("STOP brake"), "ready");
    }

    /// The mask bullets ONLY the spent-today activity figure — caps are the user's own
    /// config and stay readable — and a STOP is surfaced, never hidden.
    #[test]
    fn policy_rows_mask_activity_and_surface_a_stop() {
        let mut p = policy();
        p.revoked = true;
        p.allow_to = vec![deckard_core::Address::repeat_byte(0x22)];
        let rows = agent_policy_rows(&p, true);
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| *l == label)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("missing row {label:?}"))
        };
        assert_eq!(get("Spent today"), crate::money::MASK_BULLETS);
        assert_eq!(get("Per-transaction cap"), "0.1 ETH"); // config, not a balance
        assert_eq!(get("Recipients"), "1 allowed");
        assert_eq!(get("STOP brake"), "engaged — unlock to re-arm");
    }
}
