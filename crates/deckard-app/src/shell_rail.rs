//! The always-on right metadata rail (E3, #183; DESIGN §Information architecture) — the third pane
//! of the shell, contextual to the focused object. It is **always on, never collapsible**: a
//! selected wallet shows its holdings/status + the agent cap ledger, a selected agent its fence, the
//! Activity surface the selected request's clear-signing or the latest transaction's receipt (that
//! last dispatch lives in `activity_view.rs`, beside the feed's own summary helpers, so it reuses
//! them rather than re-deriving).
//!
//! Composed entirely from the E1 rail primitives (`meta_rail`/`meta_section`/`meta_obj`/`kv_row`):
//! the rail clamps (each row `min_w_0` + truncate, the column `flex_shrink_0`), so content can never
//! run off the pane. It only ever *reads* state — no action lives here (approve/deny stay on the
//! feed), so it needs no ⌘K command of its own.

use gpui::{div, AnyElement, Context, IntoElement, ParentElement, Styled};
use gpui_component::{v_flex, ActiveTheme};

use crate::shell::{Selection, Shell, Surface};
use crate::theme;
use crate::widgets::{kv_row, meta_obj, meta_rail, meta_section, KvValue};

impl Shell {
    /// The right metadata rail, dispatched on what the shell has in focus. Always returns a body —
    /// there is no "off" state (the rail is not collapsible) — so the wallet home, an action
    /// surface, the agent, and Activity each get contextual detail.
    pub fn render_meta_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, body): (&'static str, AnyElement) = match (self.selection, self.surface) {
            // Activity owns its own dispatch (pending request → transaction → empty), next to the
            // feed's summary helpers it reuses.
            (_, Surface::Activity) => self.activity_rail(cx),
            (Selection::Agent, Surface::Home) => ("This agent", self.agent_rail_body(cx)),
            // The wallet is the focused entity on its home and on every action surface it hosts
            // (Send/Receive/Shield/Swap/Settings) — the rail stays useful throughout a value move.
            _ => ("This wallet", self.wallet_rail_body(cx)),
        };
        let theme = cx.theme();
        meta_rail(title, body, theme)
    }

    /// The focused wallet's rail: identity object + the honest facts the app actually holds
    /// (balance, verified-read status, network — never an invented USD figure, DESIGN §Trust) + the
    /// live agent cap ledger (the SAME `PolicyGet` fence the daemon enforces).
    fn wallet_rail_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let success = theme.success;
        let warn = theme.warning;
        let mono = theme.mono_font_family.clone();
        let id_square = theme::identity_square(theme.is_dark());

        let name = self.wallet_name();
        let addr = crate::widgets::short_addr(&self.display_address.to_checksum(None));
        let mark = crate::widgets::identity_mark(
            &name,
            crate::tokens::MARK_LG,
            crate::tokens::RADIUS_ROW,
            id_square,
            fg,
        );
        let obj = meta_obj(mark, &name, &addr, theme);

        let balance = self
            .portfolio
            .as_ref()
            .map(|p| {
                crate::money::mask_money(
                    self.mask,
                    &format!("{} ETH", deckard_core::format_amount(p.native_wei, 18, 4)),
                )
            })
            .unwrap_or_else(|| "—".to_string());
        let synced = match self.synced_block {
            Some(b) => format!("block {b}"),
            None => "syncing…".to_string(),
        };
        // Never claim "Verified" the read doesn't back, and never render a downgrade quiet (DESIGN
        // §Trust rule 9): `Verified` is success-tinted, a `Degraded`/`Unsynced` read is the LOUD
        // `warn` tag (matching the status strip), and a not-yet-synced read is a neutral dash.
        let status_kv = match &self.read_status {
            Some(deckard_core::ReadStatus::Verified) => KvValue::Ok("Verified"),
            Some(deckard_core::ReadStatus::Degraded { .. }) => KvValue::Warn("Degraded"),
            Some(deckard_core::ReadStatus::Unsynced { .. }) => KvValue::Warn("Not verified"),
            None => KvValue::Sans("—"),
        };
        let network = deckard_core::for_chain(self.chain_id())
            .map(|c| c.network_name)
            .unwrap_or("Unknown network");

        let kv = |label: &str, value: KvValue| {
            kv_row(label, value, muted, fg, success, warn, mono.clone())
        };
        let facts = v_flex()
            .w_full()
            .gap_2()
            .child(kv("Balance", KvValue::Mono(&balance)))
            .child(kv("Synced", KvValue::Sans(&synced)))
            .child(kv("Status", status_kv))
            .child(kv("Network", KvValue::Sans(network)))
            .into_any_element();

        let mut col = v_flex().w_full().gap_4().child(obj).child(facts);

        // Agent caps here — the live fence, never a hardcoded number (DESIGN §cap enforcement is
        // real). Only when the daemon's policy has landed (it answers `PolicyGet` even while locked).
        if let Some(p) = self.agent_policy.as_ref() {
            // 6dp trims trailing zeros (`format_amount`), matching the daemon's canonical cap
            // display in `agent_policy_rows`.
            let eth = |wei| format!("{} ETH", deckard_core::format_amount(wei, 18, 6));
            let handle = self.agent_handle();
            let daily_label = format!("{handle} daily");
            let daily_val = format!(
                "{} / {}",
                crate::money::mask_money(self.mask, &eth(p.spent_today_wei)),
                eth(p.daily_cap_wei)
            );
            // Honest per-tx: "no limit" / "denied" instead of a false "0 ETH" (shared helper).
            let per_tx = crate::welcome::per_tx_cap_display(p);
            let per_tx_val = if per_tx.ends_with("ETH") {
                KvValue::Mono(&per_tx)
            } else {
                KvValue::Sans(&per_tx)
            };
            let caps = v_flex()
                .w_full()
                .gap_2()
                .child(kv(&daily_label, KvValue::Mono(&daily_val)))
                .child(kv("Per-transaction", per_tx_val))
                .into_any_element();
            col = col.child(meta_section(Some("Agent caps"), caps, theme));

            // The golden ref's trailing composition metasec. Only "Agents" is engine-backed today —
            // the app models one agent, armed unless a STOP revoked it — so the "Connections" count
            // is omitted (not invented) until the browser bridge lands (ADR-0001 / #44).
            let agents = if p.revoked { "1 stopped" } else { "1 active" };
            let summary = v_flex()
                .w_full()
                .gap_2()
                .child(kv("Agents", KvValue::Sans(agents)))
                .into_any_element();
            col = col.child(meta_section(None, summary, theme));
        }

        col.into_any_element()
    }

    /// The focused agent's rail (DESIGN keeps the agent surface light this pass): identity object +
    /// the live policy fence, reusing the exact `agent_policy_rows` mapping the wallet-home agent
    /// card renders, so the two never drift.
    fn agent_rail_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let success = theme.success;
        let warn = theme.warning;
        let mono = theme.mono_font_family.clone();
        let is_dark = theme.is_dark();

        let handle = self.agent_handle();
        let mark = crate::widgets::agent_mark(
            &handle,
            crate::tokens::MARK_LG,
            crate::tokens::RADIUS_ROW,
            theme::agent(is_dark),
            theme::agent_tint(is_dark),
        );
        let status = match self.agent_policy.as_ref() {
            Some(p) if p.revoked => "stopped",
            Some(_) => "acting",
            None => "idle",
        };
        let obj = meta_obj(mark, &handle, status, theme);

        let body = if let Some(p) = self.agent_policy.as_ref() {
            let mut facts = v_flex().w_full().gap_2();
            for (label, value) in crate::welcome::agent_policy_rows(p, self.mask) {
                facts = facts.child(kv_row(
                    label,
                    KvValue::Sans(&value),
                    muted,
                    fg,
                    success,
                    warn,
                    mono.clone(),
                ));
            }
            facts.into_any_element()
        } else {
            div()
                .text_sm()
                .text_color(muted)
                .child("Policy not loaded yet.")
                .into_any_element()
        };

        v_flex()
            .w_full()
            .gap_4()
            .child(obj)
            .child(body)
            .into_any_element()
    }
}
