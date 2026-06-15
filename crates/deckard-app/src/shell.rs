//! Shell — the single root view. Owns the persisted `Settings`, the current
//! route (Welcome vs Settings), and a stateful text input. It renders the title
//! bar plus whichever page is active. The page bodies live in `welcome.rs` and
//! `settings_view.rs` as `impl Shell` methods (Rust lets you split an inherent
//! impl across modules), so this file stays focused on state + navigation.

use std::time::Duration;

use gpui::{
    div, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, Styled, Window,
};
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex, ActiveTheme, TitleBar,
};

use alloy_primitives::B256;
use deckard_contract::{
    Decision, ExecuteResult, Intent, Policy, RequestId, ShieldStatus, SignerRequest, SignerResponse,
};
use deckard_core::{
    Address, EthProvider, KdfParams, Portfolio, ReadStatus, ShieldedHandle, Vault, WordCount, U256,
};
use zeroize::Zeroizing;

use deckard_signerd::SignerClient;

use crate::settings::{Settings, ThemeModePref};
use crate::signer::{self, AppSigner};
use crate::theme;
use crate::wallet;
use crate::{
    GoBack, NewItem, OpenApprovals, OpenSettings, PaletteNext, PalettePrev, ToggleMask,
    TogglePalette, ToggleTheme, APP_NAME,
};

/// How long the user must hold the shield confirm before it signs — the deliberate-gesture
/// duration (DESIGN: confirm is a hold, never a tap). The amber fill-sweep (`shield_view`)
/// runs for the same span so the bar fills exactly as the action fires.
pub(crate) const SHIELD_HOLD: Duration = Duration::from_millis(900);

/// Trim a noisy provider error down to one short line for the UI.
fn short_err(e: impl std::fmt::Display) -> String {
    let line = e.to_string();
    let line = line.lines().next().unwrap_or("").trim();
    line.chars().take(140).collect()
}

/// Map a daemon deny/`reason` tag to a calm, user-facing line (the wire tags are terse +
/// machine-readable; the UI shouldn't show `chain_mismatch` raw).
fn humanize_deny(reason: &str) -> String {
    // The broadcast error carries a variable RPC suffix, so match it by prefix.
    if reason.starts_with("broadcast_failed") {
        return "the deposit couldn't be broadcast — check your network, then review again".into();
    }
    match reason {
        "locked" => "unlock your wallet first".into(),
        "revoked" => "the signer is paused (STOP is active)".into(),
        "chain_mismatch" => {
            "the signer is on a different chain than this deposit — reconcile the chain first"
                .into()
        }
        "over_cap" | "cap_exceeded" => "it exceeds the agent's spending cap".into(),
        "off_allowlist" => "the recipient isn't on the allowlist".into(),
        "undecodable" => "the deposit calldata didn't validate".into(),
        "shield_to_mismatch" => {
            "the deposit doesn't target the Railgun contract for this chain".into()
        }
        "not_approved" => "this deposit hasn't been approved yet — review it again".into(),
        "unknown_request" => {
            "the signer session was reset — review the deposit again".into()
        }
        "erc20_unsupported_v1" => "only native-ETH shields are supported in v1".into(),
        "unsupported_v1" => "that action isn't supported in v1".into(),
        "broadcast_timeout" => {
            "the network didn't confirm in time — your deposit may already be in flight, so check your activity before retrying"
                .into()
        }
        "already_executed" => "this deposit was already submitted".into(),
        other => other.to_string(),
    }
}

/// True for a daemon `reason` that means the unlock **session ended** — the key was zeroized
/// by a STOP (an external `RevokeAll` from an MCP client, or the daemon is otherwise `Locked`).
/// The app must return to the unlock gate, not just show an inline error: a propose against a
/// locked daemon answers `locked`; an execute of a prior request answers `revoked`.
fn is_session_ended(reason: &str) -> bool {
    matches!(reason, "locked" | "revoked")
}

/// Run `prework` (which seals + writes the keystore for create/import/migrate, or is a no-op
/// for a plain unlock), then unlock OVER THE DAEMON SOCKET — the key is decrypted in the
/// daemon, never here. Returns the wallet address or a one-line, user-facing error. Always
/// called from a background thread (it blocks on Argon2 + a socket round-trip).
fn write_then_unlock(
    client: &SignerClient,
    passphrase: &str,
    prework: impl FnOnce() -> anyhow::Result<()>,
) -> Result<Address, String> {
    prework().map_err(short_err)?;
    let outcome = client.unlock_blocking(passphrase).map_err(short_err)?;
    signer::address_or_error(outcome)
}

/// What the sidebar tree selects — the contextual-view driver. The home surface
/// renders differently per selection (wallet / project / agent). Demo scope is a
/// single project, wallet, and agent (see deckard-demo-ux-locked.md).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Project,
    Wallet,
    Agent,
}

/// Transient full-pane surfaces opened FROM a selection. `Home` = the contextual
/// view for the current `Selection`; `Receive`/`Settings` are actions, not nav
/// destinations (DESIGN §Information architecture).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Home,
    Receive,
    /// The native-ETH send flow: compose (amount + 0x/ENS recipient) → review card →
    /// hold-to-confirm. Mirrors `Shield`; the daemon decides + signs, the app holds no key.
    Send,
    /// The shield trigger flow (T5): compose a deposit → review card → hold-to-confirm.
    Shield,
    /// The agent-approval queue (+ its clear-signing review): pending writes an agent breached
    /// its budget/scope on, waiting on a human approve/deny. Keyboard-first (j/k/x/Enter/⌘Enter).
    Approvals,
    /// The session activity feed: terminal records (allowed / denied / expired), grouped by day.
    Activity,
    Settings,
}

/// A reviewed-and-allowed shield, ready to sign. Carries a **recipient snapshot** taken at
/// review time so the clear-signing card always shows the recipient that is actually inside
/// `intent` — never a value the user edited in the input after `propose` landed.
#[derive(Clone)]
pub struct ShieldProposal {
    pub intent: Intent,
    pub request_id: RequestId,
    pub recipient: String,
    /// True when the daemon answered `NeedsApproval` (over-cap, or the mainnet guardrail
    /// downgrading an auto-allow). The completed hold-to-confirm IS the human approval —
    /// the app is the wire contract's designated resolver — so confirm sends
    /// `Resolve{approved: true}` before `Execute`.
    pub needs_resolve: bool,
}

/// A reviewed-and-allowed native send, ready to sign. Carries a **recipient snapshot** (the
/// resolved, checksummed destination address that is actually inside `intent.to`) so the
/// clear-signing card always shows where the ETH is going — never the raw `0x…`/ENS text the
/// user could have since edited. The `needs_resolve` flag mirrors [`ShieldProposal`]: an
/// over-cap (or `Always`-approval) send returns `NeedsApproval`, and the completed
/// hold-to-confirm IS that human approval, so confirm sends `Resolve{approved: true}` first.
#[derive(Clone)]
pub struct SendProposal {
    pub intent: Intent,
    pub request_id: RequestId,
    pub recipient: String,
    pub needs_resolve: bool,
}

/// The auth gate that wraps the whole app. Until it reaches `Ready`, the portfolio and
/// every funds-touching surface are hidden behind onboarding or the unlock screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthStep {
    /// First run, no vault: choose to create or import.
    Choose,
    /// Create: set the passphrase.
    CreateSetup,
    /// Create: reveal the recovery phrase and confirm a subset.
    CreateBackup,
    /// Import an existing phrase or raw key.
    Import,
    /// A legacy plaintext key was found — set a passphrase to encrypt it.
    Migrate,
    /// A vault exists — enter the passphrase to unlock.
    Unlock,
    /// Unlocked; the normal app is live.
    Ready,
}

pub struct Shell {
    pub focus_handle: FocusHandle,
    /// Which sidebar entity is selected (drives the Home contextual view).
    pub selection: Selection,
    /// The active full-pane surface (Home = the selection's contextual view).
    pub surface: Surface,
    pub settings: Settings,
    pub name_input: Entity<InputState>,
    pub rpc_input: Entity<InputState>,
    pub watch_input: Entity<InputState>,
    pub created: usize,
    pub palette_open: bool,
    /// The palette's self-managed fuzzy query (NOT a gpui-component `InputState` — a focused
    /// single-line input would consume ↑/↓ before our key handler ever sees them). Owned as a
    /// plain `String`, edited in `palette.rs`'s `on_palette_key`.
    pub palette_query: String,
    /// The highlighted result row (0-based into `palette_results`); ↑/↓ move it, Enter runs it.
    pub palette_selected: usize,
    /// The ranked, displayable results for the current query — recomputed by `repalette` on
    /// every query edit (and on open) via the pure `palette_commands::rank`.
    pub palette_results: Vec<crate::palette_commands::Ranked>,
    /// The palette panel's focus handle — `track_focus`'d so the panel (with its
    /// `key_context("CommandPalette")`) receives every keystroke while open.
    pub palette_focus: gpui::FocusHandle,
    /// Whatever had focus when the palette opened, so closing can restore it (codex m7: a
    /// command that doesn't change focus shouldn't strand the user in the dismissed overlay).
    pub palette_prev_focus: Option<gpui::FocusHandle>,
    /// Persisted per-command frecency so the empty palette surfaces the most-used actions first.
    /// `record`'d when a command runs (best-effort disk write; never load-bearing).
    pub palette_usage: crate::palette_usage::PaletteUsage,
    /// The reused nucleo matcher (its scratch buffers amortize across queries). Pure-data ranking
    /// borrows it; it holds no palette state of its own.
    pub palette_matcher: nucleo_matcher::Matcher,
    /// Privacy mask: when true, every money surface renders fixed bullets instead of a
    /// figure (DESIGN §Trust). Initialised from `Settings.mask_balances` and persisted on
    /// every toggle — the inverse of the seed reveal's momentary, default-hidden model.
    pub mask: bool,
    /// Demo stand-in for "Atlas is currently acting": drives the one sanctioned ambient
    /// motion (the ~1.2s breathing pulse on the agent squircle). Not persisted — it's a
    /// narrated demo toggle until the daemon exposes an activity feed.
    pub agent_acting: bool,
    /// The signer's live policy fence, rendered on the agent home so the card shows the
    /// SAME numbers `deckard_policy_get` returns. Fetched from the daemon (`PolicyGet`
    /// deliberately succeeds while locked — the fence is config, not a secret); `None`
    /// until the first fetch lands or when the daemon is unreachable.
    pub agent_policy: Option<Policy>,

    // --- agent approvals queue + activity ---
    /// The latest `PendingList` snapshot from the daemon — every in-flight + recently-terminal
    /// record with its payload. The Approvals queue derives its pending rows from this
    /// (`approvals_queue`); Activity derives its terminal rows (`activity_days`). Refreshed by
    /// `refresh_pending` and the poller; read by both views.
    pub pending: Vec<deckard_contract::PendingRecord>,
    /// The highlighted queue row (0-based into `approvals_queue(&self.pending)`). j/k move it;
    /// clamped to the queue length on every refresh so it never points past a now-shorter list.
    pub approvals_selected: usize,
    /// True while a `PendingList` round-trip is in flight (drives the loading skeletons, but
    /// only when there's no prior snapshot to render).
    pub approvals_loading: bool,
    /// A fail-loud one-liner when a `PendingList` fetch fails (the queue shows it in `danger`
    /// rather than silently render an empty/stale inbox).
    pub approvals_error: Option<String>,
    /// `Some` ⇒ the clear-signing review is open for that request id (the queue dispatches to
    /// the review card). Cleared on cancel / approve / deny and on opening the surface fresh.
    /// `pub(crate)` so the read-only `approvals_view` can dispatch on it (it never mutates it).
    pub(crate) approvals_reviewing: Option<deckard_contract::RequestId>,
    /// Bumped on each `refresh_pending` so a slow reply for a since-superseded fetch can't
    /// install a stale snapshot (mirrors the send/shield review epochs).
    approvals_epoch: u64,
    /// The Approvals surface's focus handle — `track_focus`'d so its `key_context("Approvals")`
    /// listener receives the in-queue keys (j/k/x/Enter/⌘Enter/Esc). Focused on open.
    approvals_focus: gpui::FocusHandle,
    /// True while the recurring pending-list poller loop is alive, so opening a surface twice
    /// can't spawn a second loop (the loop self-terminates when neither surface is active).
    pending_poller_running: bool,

    // --- activity feed (#60: the see-and-stop ledger) ---
    /// The latest `ActivityFeed` snapshot — every tracked action (auto-allowed, pending, denied,
    /// executed), newest-first, with `tx_hash`/`timestamp_ms`/breached-cap. The Activity surface
    /// renders this; refreshed by `refresh_activity` + the shared poller. Distinct from `pending`
    /// (the queue's pending-only view) because the feed retains executed/auto-allowed rows.
    pub activity: Vec<deckard_contract::ActivityRecord>,
    /// The highlighted feed row, 0-based into the APPROVABLE (still-proposed) subset of
    /// `activity` — clamped on every refresh so it never points past a now-shorter list. j/k move
    /// it; the feed renders all rows but only proposed ones are selectable/approvable.
    pub activity_selected: usize,
    /// `Some` ⇒ the inline clear-signing review is open on the feed for that request id. Cleared
    /// on cancel / approve / deny. `pub(crate)` so the read-only `activity_view` can dispatch on it.
    pub(crate) activity_reviewing: Option<deckard_contract::RequestId>,
    /// True while an `ActivityFeed` round-trip is in flight (drives a first-load skeleton only).
    pub activity_loading: bool,
    /// Fail-loud one-liner when an `ActivityFeed` fetch fails (shown in `danger`, never a silent
    /// empty feed).
    pub activity_error: Option<String>,
    /// Bumped on each `refresh_activity` so a slow reply for a superseded fetch can't install a
    /// stale snapshot (mirrors `approvals_epoch`).
    activity_epoch: u64,
    /// The Activity surface's focus handle — `track_focus`'d so its `key_context("Activity")`
    /// listener owns the in-feed keys (j/k/x/Enter/⌘Enter/Esc) without colliding with Approvals.
    activity_focus: gpui::FocusHandle,
    /// STOP confirm arming: a first click on the feed's STOP control arms it (shows "confirm"),
    /// a second confirms — so the irreversible key-zeroize is never a single click. Esc disarms.
    activity_stop_arming: bool,
    /// Set once a STOP from the feed succeeded — drives the "Stopped — key zeroized, unlock to
    /// re-arm" banner. The feed stays visible (the daemon answers `ActivityFeed` while locked) so
    /// the revoked rows are seen; the next unlock clears it.
    activity_stopped: bool,

    /// The capture-block state last pushed to the OS, so `render` only re-issues the
    /// native `setSharingType` call when `capture_block && mask` actually changes.
    capture_applied: bool,
    /// Recording override (`DECKARD_ALLOW_SCREEN_CAPTURE`): when set, force the capture block
    /// OFF regardless of the `capture_block` setting, so an automated agent can record the demo
    /// GIF without touching the settings UI. Resolved once at launch; default false (the setting
    /// governs) — a normal build never disables the trust feature behind the user's back.
    pub allow_screen_capture: bool,

    // --- shield trigger flow (T5) ---
    /// Deposit amount (ETH, free text) and the `0zk…` recipient. Free-text recipient is v1;
    /// auto-filling the user's OWN railgun address is Wave 2.
    pub shield_amount: Entity<InputState>,
    pub shield_recipient: Entity<InputState>,
    /// Set once `propose` returns `Allow`. `Some` means the review card + hold-to-confirm are
    /// live; it carries a recipient snapshot so the card can't show a since-edited address.
    pub shield_proposal: Option<ShieldProposal>,
    /// Bumped on each `review_shield` (and on reset) so a slow propose reply for a
    /// since-cancelled/re-issued review can't install a stale proposal.
    shield_review_epoch: u64,
    /// True while a `propose`/`execute` round-trip runs on a background thread.
    pub shield_busy: bool,
    /// One-line, user-facing shield error (parse / build / deny / broadcast).
    pub shield_error: Option<String>,
    /// Set on a successful `execute` broadcast — the demo's "deposit is moving private" state.
    pub shield_tx: Option<B256>,
    /// True while the confirm button is being held; drives the amber fill-sweep.
    pub shield_holding: bool,
    /// Bumped on each hold-start so a stale hold timer can't fire a later confirm.
    shield_hold_epoch: u64,

    // --- native-ETH send flow (mirrors the shield trigger flow above) ---
    /// Send amount (ETH, free text) and the recipient (a `0x…` address or an ENS name, which
    /// is forward-resolved at review time — reuse of the watch-address resolve path).
    pub send_amount: Entity<InputState>,
    pub send_recipient: Entity<InputState>,
    /// Set once `propose` returns `Allow`/`NeedsApproval`; `Some` means the review card +
    /// hold-to-confirm are live. Carries the resolved-recipient snapshot for the card.
    pub send_proposal: Option<SendProposal>,
    /// Bumped on each `review_send` (and on reset) so a slow propose/resolve reply for a
    /// since-cancelled/re-issued review can't install a stale proposal.
    send_review_epoch: u64,
    /// True while a `resolve`/`propose`/`execute` round-trip runs on a background thread.
    pub send_busy: bool,
    /// One-line, user-facing send error (parse / resolve / deny / broadcast).
    pub send_error: Option<String>,
    /// Set on a successful `execute` broadcast — the send's "on its way" confirmation state.
    pub send_tx: Option<B256>,
    /// True while the confirm button is being held; drives the amber fill-sweep.
    pub send_holding: bool,
    /// Bumped on each hold-start so a stale hold timer can't fire a later confirm.
    send_hold_epoch: u64,

    // --- shielded balance (Wave 2: T9 sync + T10 lifecycle) ---
    /// The read-only Railgun sync actor (None until the view grant is fetched post-unlock,
    /// and only if the derivation gate passes). Holds the viewing key, never the spending key.
    pub shielded: Option<ShieldedHandle>,
    /// The user's own 0zk address — the shield recipient auto-fill (None until granted).
    pub railgun_address: Option<String>,
    /// True once the shield recipient input has been auto-filled with `railgun_address`.
    recipient_autofilled: bool,
    /// The active shield's lifecycle, surfaced in the status strip (None when idle).
    pub shield_status: Option<ShieldStatus>,
    /// Bumped on every unlock/lock so a slow grant fetch from a prior session can't install a
    /// stale handle/address after the wallet locked or a different wallet unlocked.
    auth_epoch: u64,
    /// Set on lock so the next Ready render clears the shield inputs (which a listener can't —
    /// `set_value` needs a `Window`), preventing a prior wallet's 0zk recipient from lingering.
    pending_shield_clear: bool,

    // --- auth / keystore (Chunk 3) ---
    pub auth: AuthStep,
    pub auth_error: Option<String>,
    /// True while an Argon2 create/unlock runs on a background thread.
    pub auth_busy: bool,
    /// The unlocked wallet's own address (for Receive / copy). `None` until unlocked. This is
    /// the ONLY wallet identity the app holds — the key lives in the daemon, never here.
    pub wallet_address: Option<Address>,
    /// The key-less bridge to the process-isolated signer daemon: the app spawns + supervises
    /// it and talks over the socket (unlock / propose / execute). Unlock happens *in the
    /// daemon*; the app only learns the address. Dropping this kills the daemon child.
    signer: AppSigner,
    /// During create: the sealed-but-unwritten vault and its phrase, pending backup.
    pending_vault: Option<Vault>,
    pub pending_phrase: Option<Zeroizing<String>>,
    pending_pass: Option<Zeroizing<String>>,
    /// Which word positions (0-indexed) the backup-confirm step quizzes.
    pub confirm_positions: Vec<usize>,
    /// Hold-to-reveal state for the recovery phrase.
    pub reveal_seed: bool,
    /// The auth step whose primary input we've already auto-focused (focus once per step).
    focused_step: Option<AuthStep>,
    // Auth inputs (passphrases are never persisted).
    pub create_pass: Entity<InputState>,
    pub create_pass2: Entity<InputState>,
    pub confirm_words: Entity<InputState>,
    pub import_secret: Entity<InputState>,
    pub import_pass: Entity<InputState>,
    pub pass_input: Entity<InputState>,

    // --- live network state (Chunks 1 & 2) ---
    /// The async bridge to the single tokio network thread.
    pub eth: EthProvider,
    /// The address currently being viewed — the wallet, or a watched address/ENS.
    pub display_address: Address,
    /// True when `display_address` is a watched address rather than the wallet.
    pub viewing_watch: bool,
    /// Last good portfolio snapshot; rendered from cache while a refresh runs.
    pub portfolio: Option<Portfolio>,
    /// True only during the first sync (the one allowed loading state).
    pub portfolio_loading: bool,
    pub portfolio_error: Option<String>,
    /// Trust label for the last portfolio/block read: Helios-`Verified` vs visibly
    /// `Unsynced`/`Degraded`. Never silently "trusted" — surfaced in the status line.
    pub read_status: Option<ReadStatus>,
    /// Latest block height — a liveness/sync indicator for the status line.
    pub synced_block: Option<u64>,
    /// Bumped on every `retarget`; a slow ENS resolution checks it before applying so a
    /// stale reply for a since-changed target can't clobber the current view.
    view_epoch: u64,
    /// The RPC URL the current worker was spawned with — so we don't tear it down on a
    /// no-op blur of the RPC field.
    current_rpc: String,
    /// The chain the supervised daemon signs for, resolved ONCE at startup
    /// (`DECKARD_CHAIN_ID` env > settings > [`settings::DEFAULT_CHAIN_ID`]). Threaded to the
    /// daemon launch, the shield builder (`propose` denies `chain_mismatch` otherwise), and the
    /// Railgun sync so they never disagree. `just demo` sets it to Sepolia (11155111).
    chain_id: u64,
}

impl Shell {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Your name")
                .default_value(settings.display_name.clone())
        });

        // Persist the text field as the user types (and on blur).
        cx.subscribe(&name_input, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change | InputEvent::Blur) {
                this.settings.display_name = state.read(cx).value().to_string();
                this.settings.save();
            }
        })
        .detach();

        // Custom RPC URL: persist as typed; apply (re-spawn the provider) on blur.
        let rpc_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://…  (default: bundled public RPC)")
                .default_value(settings.rpc_url.clone())
        });
        cx.subscribe(
            &rpc_input,
            |this, state, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.settings.rpc_url = state.read(cx).value().to_string();
                    this.settings.save();
                }
                InputEvent::Blur => {
                    this.settings.rpc_url = state.read(cx).value().to_string();
                    this.settings.save();
                    this.respawn_provider(cx);
                }
                _ => {}
            },
        )
        .detach();

        // Watch address / ENS: persist as typed; re-target the portfolio on blur.
        let watch_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("0x… or name.eth  (blank = your wallet)")
                .default_value(settings.watch_address.clone())
        });
        cx.subscribe(
            &watch_input,
            |this, state, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.settings.watch_address = state.read(cx).value().to_string();
                    this.settings.save();
                }
                InputEvent::Blur => {
                    this.settings.watch_address = state.read(cx).value().to_string();
                    this.settings.save();
                    this.retarget(cx);
                }
                _ => {}
            },
        )
        .detach();

        // Auth inputs — passphrases are masked and NEVER persisted to disk.
        let masked = |window: &mut Window, cx: &mut Context<Self>, ph: &str| {
            let ph = ph.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(ph).masked(true))
        };
        let create_pass = masked(window, cx, "Choose a passphrase (min 8 characters)");
        let create_pass2 = masked(window, cx, "Confirm passphrase");
        let import_pass = masked(window, cx, "Choose a passphrase (min 8 characters)");
        let pass_input = masked(window, cx, "Passphrase");
        let confirm_words = cx.new(|cx| {
            InputState::new(window, cx).placeholder("the requested words, space-separated")
        });
        let import_secret = cx.new(|cx| {
            InputState::new(window, cx).placeholder("12 / 24-word phrase, or a 0x private key")
        });

        // Shield flow inputs (T5): amount in ETH + the 0zk recipient (free text in v1).
        let shield_amount =
            cx.new(|cx| InputState::new(window, cx).placeholder("Amount in ETH, e.g. 0.05"));
        let shield_recipient =
            cx.new(|cx| InputState::new(window, cx).placeholder("0zk… recipient address"));
        // Re-render on edits so the Review button's disabled state tracks validity live;
        // Enter on the recipient reviews the deposit (keyboard-first).
        cx.subscribe(&shield_amount, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(
            &shield_recipient,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.review_shield(cx),
                _ => {}
            },
        )
        .detach();

        // Send flow inputs: amount in ETH + a `0x…`/ENS recipient. Same live-validity +
        // Enter-to-review wiring as the shield fields above.
        let send_amount =
            cx.new(|cx| InputState::new(window, cx).placeholder("Amount in ETH, e.g. 0.05"));
        let send_recipient = cx
            .new(|cx| InputState::new(window, cx).placeholder("0x… address or name.eth recipient"));
        cx.subscribe(&send_amount, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(
            &send_recipient,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.review_send(cx),
                _ => {}
            },
        )
        .detach();

        // Submit-on-Enter for each auth field (keyboard-first).
        cx.subscribe(&create_pass2, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.do_create(cx);
            }
        })
        .detach();
        cx.subscribe(&confirm_words, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_backup(cx);
            }
        })
        .detach();
        cx.subscribe(&import_pass, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.do_import(cx);
            }
        })
        .detach();
        cx.subscribe(&pass_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit_passphrase(cx);
            }
        })
        .detach();

        // Decide the initial gate: existing vault → unlock; legacy plaintext key →
        // migrate; otherwise first-run onboarding.
        let auth = if wallet::vault_exists() {
            AuthStep::Unlock
        } else if wallet::legacy_key_hex().is_some() {
            AuthStep::Migrate
        } else {
            AuthStep::Choose
        };

        // The network worker is always live (it serves the watch-only path too), but the
        // wallet portfolio isn't fetched until the vault is unlocked.
        let current_rpc = settings.effective_rpc();
        // One resolved runtime chain id (env > settings > default), threaded to the daemon
        // launch, the shield builder, and the Railgun sync.
        let chain_id = settings.effective_chain_id();
        let eth = EthProvider::spawn(current_rpc.clone());

        // Log the resolved runtime config once (the RPC is REDACTED to scheme://host — it may
        // carry an API key). Makes "which chain / RPC / mode am I on?" answerable from the log,
        // which matters most exactly when an env override re-points the demo off mainnet.
        eprintln!(
            "deckard: runtime — chain {chain_id} · rpc {} · verified-reads {} · fork-mode {}",
            deckard_signerd::config::redact_url(&current_rpc),
            if deckard_core::verified_reads_enabled() {
                "on"
            } else {
                "off"
            },
            if crate::settings::is_fork_mode(&current_rpc, chain_id) {
                "yes (DEMO FORK)"
            } else {
                "no"
            },
        );

        // Recording override: disabling a trust feature must never be silent. Resolve it once
        // here and say so loudly when active (it only flips on for an explicit env opt-in).
        let allow_screen_capture = deckard_core::screen_capture_allowed();
        if allow_screen_capture {
            eprintln!(
                "deckard: screen-capture block DISABLED via DECKARD_ALLOW_SCREEN_CAPTURE — \
                 recording mode (the privacy capture-block is held off this session)"
            );
        }

        // Spawn + supervise the process-isolated signer daemon. It owns the key; the app is a
        // key-less client that unlocks/signs over the socket. The daemon broadcasts via the
        // same RPC the app reads from, on the same resolved chain.
        let signer = AppSigner::launch(current_rpc.clone(), chain_id);

        // The mask is a persisted preference (default off); seed it from settings.
        let mask = settings.mask_balances;

        Self {
            focus_handle,
            selection: Selection::Wallet,
            surface: Surface::Home,
            settings,
            name_input,
            rpc_input,
            watch_input,
            created: 0,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_results: Vec::new(),
            palette_focus: cx.focus_handle(),
            palette_prev_focus: None,
            palette_usage: crate::palette_usage::PaletteUsage::load(),
            palette_matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT),
            mask,
            agent_acting: false,
            agent_policy: None,
            pending: Vec::new(),
            approvals_selected: 0,
            approvals_loading: false,
            approvals_error: None,
            approvals_reviewing: None,
            approvals_epoch: 0,
            approvals_focus: cx.focus_handle(),
            pending_poller_running: false,
            activity: Vec::new(),
            activity_selected: 0,
            activity_reviewing: None,
            activity_loading: false,
            activity_error: None,
            activity_epoch: 0,
            activity_focus: cx.focus_handle(),
            activity_stop_arming: false,
            activity_stopped: false,
            capture_applied: false,
            allow_screen_capture,
            shield_amount,
            shield_recipient,
            shield_proposal: None,
            shield_review_epoch: 0,
            shield_busy: false,
            shield_error: None,
            shield_tx: None,
            shield_holding: false,
            shield_hold_epoch: 0,
            send_amount,
            send_recipient,
            send_proposal: None,
            send_review_epoch: 0,
            send_busy: false,
            send_error: None,
            send_tx: None,
            send_holding: false,
            send_hold_epoch: 0,
            shielded: None,
            railgun_address: None,
            recipient_autofilled: false,
            shield_status: None,
            auth_epoch: 0,
            pending_shield_clear: false,
            auth,
            auth_error: None,
            auth_busy: false,
            wallet_address: None,
            signer,
            pending_vault: None,
            pending_phrase: None,
            pending_pass: None,
            confirm_positions: Vec::new(),
            reveal_seed: false,
            focused_step: None,
            create_pass,
            create_pass2,
            confirm_words,
            import_secret,
            import_pass,
            pass_input,
            eth,
            display_address: Address::ZERO,
            viewing_watch: false,
            portfolio: None,
            portfolio_loading: false,
            portfolio_error: None,
            read_status: None,
            synced_block: None,
            view_epoch: 0,
            current_rpc,
            chain_id,
        }
    }

    // --- auth / keystore actions (Chunk 3) ---

    pub fn start_create(&mut self, cx: &mut Context<Self>) {
        self.auth = AuthStep::CreateSetup;
        self.auth_error = None;
        cx.notify();
    }

    pub fn start_import(&mut self, cx: &mut Context<Self>) {
        self.auth = AuthStep::Import;
        self.auth_error = None;
        cx.notify();
    }

    pub fn auth_back_to_choose(&mut self, cx: &mut Context<Self>) {
        self.auth = AuthStep::Choose;
        self.auth_error = None;
        cx.notify();
    }

    pub fn set_reveal_seed(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.reveal_seed = reveal;
        cx.notify();
    }

    /// Enter pressed on the single passphrase field → unlock or migrate.
    fn submit_passphrase(&mut self, cx: &mut Context<Self>) {
        match self.auth {
            AuthStep::Unlock => self.do_unlock(cx),
            AuthStep::Migrate => self.do_migrate(cx),
            _ => {}
        }
    }

    /// Lock the wallet: tell the daemon to zeroize the key (best-effort, off the UI thread)
    /// and return to the unlock gate. The app held no key to drop — locking is the daemon's job.
    pub fn lock(&mut self, cx: &mut Context<Self>) {
        let client = self.signer.client();
        cx.background_spawn(async move {
            let _ = client.lock_blocking();
        })
        .detach();
        self.wallet_address = None;
        self.portfolio = None;
        // Dropping the handle closes its channel → the sync worker thread exits.
        self.shielded = None;
        self.railgun_address = None;
        self.recipient_autofilled = false;
        self.shield_status = None;
        // Invalidate any in-flight grant fetch and clear shield inputs on the next render.
        self.auth_epoch = self.auth_epoch.wrapping_add(1);
        self.pending_shield_clear = true;
        self.reset_shield();
        self.reset_send();
        // Clear the feed's STOP banner + arming + any open inline review (a fresh unlock re-arms
        // the wallet, so a stale "Stopped" banner or half-open review must not survive).
        self.activity_stopped = false;
        self.activity_stop_arming = false;
        self.activity_reviewing = None;
        self.auth = AuthStep::Unlock;
        self.palette_open = false;
        cx.notify();
    }

    /// CreateSetup → generate a fresh vault + phrase (Argon2 runs off the UI thread).
    pub fn do_create(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let p1 = self.create_pass.read(cx).value().to_string();
        let p2 = self.create_pass2.read(cx).value().to_string();
        if p1.chars().count() < 8 {
            self.auth_error = Some("Passphrase must be at least 8 characters".into());
            cx.notify();
            return;
        }
        if p1 != p2 {
            self.auth_error = Some("Passphrases don't match".into());
            cx.notify();
            return;
        }
        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        let pass = Zeroizing::new(p1);
        let task = cx.background_spawn(async move {
            let made = Vault::create(&pass, WordCount::Twelve, KdfParams::PRODUCTION);
            made.map(|(v, phrase)| (v, phrase, pass))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok((vault, phrase, pass)) => {
                        let wc = phrase.split_whitespace().count();
                        this.confirm_positions = deckard_core::random_word_positions(wc, 3);
                        this.pending_vault = Some(vault);
                        this.pending_phrase = Some(phrase);
                        this.pending_pass = Some(pass);
                        this.reveal_seed = false;
                        this.auth = AuthStep::CreateBackup;
                    }
                    Err(e) => this.auth_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// CreateBackup → check the quizzed words, then write + unlock the vault.
    pub fn confirm_backup(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let (Some(phrase), Some(vault), Some(pass)) = (
            self.pending_phrase.clone(),
            self.pending_vault.clone(),
            self.pending_pass.clone(),
        ) else {
            return;
        };
        let words: Vec<&str> = phrase.split_whitespace().collect();
        let expected: Vec<String> = self
            .confirm_positions
            .iter()
            .map(|&i| words.get(i).copied().unwrap_or("").to_lowercase())
            .collect();
        let entered: Vec<String> = self
            .confirm_words
            .read(cx)
            .value()
            .split_whitespace()
            .map(|s| s.trim().to_lowercase())
            .collect();
        if entered != expected {
            self.auth_error = Some("Those words don't match your backup — try again".into());
            cx.notify();
            return;
        }

        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        let Some(path) = wallet::vault_path() else {
            self.auth_error = Some("no config directory available".into());
            self.auth_busy = false;
            cx.notify();
            return;
        };
        let client = self.signer.client();
        let task = cx.background_spawn(async move {
            write_then_unlock(&client, pass.as_str(), move || vault.write_atomic(&path))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(addr) => {
                        wallet::delete_legacy_key();
                        this.pending_phrase = None;
                        this.pending_pass = None;
                        this.pending_vault = None;
                        this.finish_unlock(addr, cx);
                    }
                    Err(msg) => {
                        this.auth_error = Some(msg);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Import → seal a phrase or raw key into a new vault, then unlock it.
    pub fn do_import(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let secret = self.import_secret.read(cx).value().to_string();
        let pass = self.import_pass.read(cx).value().to_string();
        if secret.trim().is_empty() {
            self.auth_error = Some("Enter a recovery phrase or a private key".into());
            cx.notify();
            return;
        }
        if pass.chars().count() < 8 {
            self.auth_error = Some("Passphrase must be at least 8 characters".into());
            cx.notify();
            return;
        }
        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        let Some(path) = wallet::vault_path() else {
            self.auth_error = Some("no config directory available".into());
            self.auth_busy = false;
            cx.notify();
            return;
        };
        let secret = Zeroizing::new(secret);
        let pass = Zeroizing::new(pass);
        let seal_pass = pass.clone();
        let client = self.signer.client();
        let task = cx.background_spawn(async move {
            write_then_unlock(&client, pass.as_str(), move || {
                let trimmed = secret.trim();
                // Route by shape, not word count: a pure-hex string (optional 0x) is a raw
                // key; anything with spaces/words is a mnemonic, so a short/long phrase gets a
                // real BIP-39 error rather than a misleading "must be 32 bytes".
                let h = trimmed.strip_prefix("0x").unwrap_or(trimmed);
                let looks_like_hex_key = !trimmed.contains(char::is_whitespace)
                    && !h.is_empty()
                    && h.chars().all(|c| c.is_ascii_hexdigit());
                let vault = if looks_like_hex_key {
                    Vault::import_raw_key(trimmed, seal_pass.as_str(), KdfParams::PRODUCTION)?
                } else {
                    Vault::import_mnemonic(trimmed, seal_pass.as_str(), KdfParams::PRODUCTION)?
                };
                vault.write_atomic(&path)?;
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(addr) => {
                        wallet::delete_legacy_key();
                        this.finish_unlock(addr, cx);
                    }
                    Err(msg) => {
                        this.auth_error = Some(msg);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Unlock an existing vault with the entered passphrase (Argon2 off the UI thread).
    pub fn do_unlock(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let pass = self.pass_input.read(cx).value().to_string();
        if pass.is_empty() {
            self.auth_error = Some("Enter your passphrase".into());
            cx.notify();
            return;
        }
        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        // No vault write: the daemon reads the existing keystore and decrypts it.
        let pass = Zeroizing::new(pass);
        let client = self.signer.client();
        let task = cx
            .background_spawn(async move { write_then_unlock(&client, pass.as_str(), || Ok(())) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(addr) => this.finish_unlock(addr, cx),
                    Err(msg) => {
                        this.auth_error = Some(msg);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Migrate the legacy plaintext key into an encrypted vault under a new passphrase.
    pub fn do_migrate(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let pass = self.pass_input.read(cx).value().to_string();
        if pass.chars().count() < 8 {
            self.auth_error = Some("Passphrase must be at least 8 characters".into());
            cx.notify();
            return;
        }
        let Some(hex) = wallet::legacy_key_hex() else {
            self.auth = AuthStep::Choose;
            cx.notify();
            return;
        };
        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        let Some(path) = wallet::vault_path() else {
            self.auth_error = Some("no config directory available".into());
            self.auth_busy = false;
            cx.notify();
            return;
        };
        let pass = Zeroizing::new(pass);
        let seal_pass = pass.clone();
        let hex = Zeroizing::new(hex);
        let client = self.signer.client();
        let task = cx.background_spawn(async move {
            write_then_unlock(&client, pass.as_str(), move || {
                let vault =
                    Vault::import_raw_key(hex.as_str(), seal_pass.as_str(), KdfParams::PRODUCTION)?;
                vault.write_atomic(&path)?;
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(addr) => {
                        wallet::delete_legacy_key();
                        this.finish_unlock(addr, cx);
                    }
                    Err(msg) => {
                        this.auth_error = Some(msg);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Land in the unlocked app: stash the address the daemon returned and fetch the
    /// portfolio. The key stays in the daemon — the app only holds this address.
    fn finish_unlock(&mut self, address: Address, cx: &mut Context<Self>) {
        self.wallet_address = Some(address);
        self.auth = AuthStep::Ready;
        self.auth_error = None;
        self.selection = Selection::Wallet;
        self.surface = Surface::Home;
        self.auth_epoch = self.auth_epoch.wrapping_add(1);
        self.retarget(cx);
        self.kick_railgun_grant(cx);
        self.kick_agent_policy(cx);
    }

    /// Fetch the daemon's live policy for the agent home (off the UI thread). Key-less:
    /// `PolicyGet` is a read of the fence the daemon enforces — the daemon answers it even
    /// while locked, so this works from the unlock gate too. On any failure the card keeps
    /// its previous snapshot (or honestly shows none); it never fabricates numbers.
    fn kick_agent_policy(&mut self, cx: &mut Context<Self>) {
        let client = self.signer.client();
        let task = cx.background_spawn(async move {
            match client.request_blocking(&SignerRequest::PolicyGet) {
                Ok(SignerResponse::Policy(p)) => Some(p),
                _ => None,
            }
        });
        cx.spawn(async move |this, cx| {
            let policy = task.await;
            this.update(cx, |this, cx| {
                if policy.is_some() {
                    this.agent_policy = policy;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// After unlock, ask the daemon for the read-only Railgun view grant (it gates this on the
    /// derivation known-answer test), then spawn the shielded-balance sync over the app's RPC
    /// and start watching it. If the daemon refuses (locked / gate failed), there's simply no
    /// shielded UI — honest, never a fabricated private balance.
    fn kick_railgun_grant(&mut self, cx: &mut Context<Self>) {
        // Belt-and-suspenders: the daemon already gates the grant on the derivation KAT, but
        // never show a shielded balance unless the app independently re-verifies it too.
        if !deckard_core::known_answer_ok() {
            return;
        }
        let client = self.signer.client();
        let chain_id = self.chain_id;
        let rpc = self.settings.effective_rpc();
        let epoch = self.auth_epoch;
        let task =
            cx.background_spawn(async move { client.railgun_view_grant_blocking(chain_id, 0) });
        cx.spawn(async move |this, cx| {
            let grant = task.await;
            this.update(cx, |this, cx| {
                // Drop a reply for a session that has since locked / re-unlocked.
                if this.auth_epoch != epoch || this.auth != AuthStep::Ready {
                    return;
                }
                if let Ok(grant) = grant {
                    this.railgun_address = Some(grant.address.clone());
                    this.recipient_autofilled = false;
                    this.shielded = Some(ShieldedHandle::spawn(rpc, chain_id, grant));
                    this.watch_shielded_sync(false, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Poll the shielded snapshot while a sync runs so the UI reflects progress (the actor's
    /// cached state isn't a GPUI entity, so we tick it). Capped so a hung sync can't loop
    /// forever. With `drive_lifecycle`, once the sync SETTLES it advances `ShieldStatus`
    /// honestly: a clean synced balance → `PrivateSpendable(wei)`, a sync error → `Failed`, a
    /// timeout → stays in-flight (never claims "spendable" with a fabricated zero).
    fn watch_shielded_sync(&self, drive_lifecycle: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut timed_out = true;
            for _ in 0..90 {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let syncing = this.update(cx, |this, cx| {
                    cx.notify();
                    this.shielded.as_ref().is_some_and(|h| h.snapshot().syncing)
                });
                match syncing {
                    Ok(true) => continue,
                    Ok(false) => {
                        timed_out = false;
                        break;
                    }
                    Err(_) => return, // the view is gone
                }
            }
            if !drive_lifecycle || timed_out {
                return; // an initial/refresh watch, or a hung sync — don't touch the lifecycle
            }
            this.update(cx, |this, cx| {
                // Only advance an in-flight shield, and only on a real settled result.
                if !matches!(this.shield_status, Some(ShieldStatus::Sending)) {
                    return;
                }
                let snap = this.shielded.as_ref().map(|h| h.snapshot());
                this.shield_status = match snap {
                    Some(s) if s.error.is_some() => Some(ShieldStatus::Failed {
                        reason: s.error.unwrap_or_default(),
                    }),
                    Some(s) => match s.shielded_wei {
                        Some(wei) => Some(ShieldStatus::PrivateSpendable { shielded_wei: wei }),
                        None => return, // settled without a value — leave it in-flight
                    },
                    None => return,
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The unlocked wallet's own address as an EIP-55 string (empty until unlocked).
    pub fn wallet_address_string(&self) -> String {
        self.wallet_address
            .map(|a| a.to_string())
            .unwrap_or_default()
    }

    /// Whether the app is pointed at a local development fork rather than a public network —
    /// drives the status-strip "DEMO FORK — not mainnet" caution (rendered in `shell_chrome`).
    pub(crate) fn fork_mode(&self) -> bool {
        crate::settings::is_fork_mode(&self.current_rpc, self.chain_id)
    }

    /// An external STOP (e.g. an MCP client calling `revoke_all`) zeroized the key — this
    /// unlock session is dead. Tear down to the unlock gate with clear copy rather than leave a
    /// Ready screen that would silently fail every funds action. Reachable because the app and
    /// an MCP client share one daemon (the two-client reality); a re-unlock re-arms.
    fn handle_session_revoked(&mut self, cx: &mut Context<Self>) {
        self.lock(cx);
        self.auth_error =
            Some("The signer was stopped (STOP). Unlock your wallet to continue.".into());
        cx.notify();
    }

    /// Auto-focus the primary input for the current auth step (so the user — and the
    /// keyboard — can type immediately). Called once per step from `render`.
    fn focus_auth_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.auth {
            AuthStep::CreateSetup => Some(&self.create_pass),
            AuthStep::CreateBackup => Some(&self.confirm_words),
            AuthStep::Import => Some(&self.import_secret),
            AuthStep::Migrate | AuthStep::Unlock => Some(&self.pass_input),
            AuthStep::Choose | AuthStep::Ready => None,
        };
        if let Some(input) = target {
            // Clear any stale text (e.g. a passphrase left after lock) before focusing, so
            // secrets don't linger in the field across an auth-step change.
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
    }

    // --- live network plumbing (Chunks 1 & 2) ---

    /// Spawn a portfolio fetch for `addr`; fold the result into `self` on the UI thread.
    fn kick_portfolio(eth: &EthProvider, addr: Address, cx: &mut Context<Self>) {
        let rx = eth.portfolio(addr);
        cx.spawn(async move |this, cx| {
            let res = rx.recv_async().await;
            this.update(cx, |this, cx| {
                this.portfolio_loading = false;
                match res {
                    Ok(Ok(read)) => {
                        // Ignore a stale reply for an address we're no longer viewing.
                        if read.value.address == this.display_address {
                            this.portfolio = Some(read.value);
                            this.portfolio_error = None;
                            // Surface the trust label (Helios-verified vs unsynced).
                            this.read_status = Some(read.status);
                        }
                    }
                    Ok(Err(e)) => this.portfolio_error = Some(short_err(e)),
                    Err(_) => this.portfolio_error = Some("network worker stopped".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Refresh the latest block height for the status line.
    fn kick_block_number(eth: &EthProvider, cx: &mut Context<Self>) {
        let rx = eth.block_number();
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(read)) = rx.recv_async().await {
                this.update(cx, |this, cx| {
                    this.synced_block = Some(read.value);
                    this.read_status = Some(read.status);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Re-fetch the portfolio for the current `display_address` (manual or post-change).
    pub fn refresh_portfolio(&mut self, cx: &mut Context<Self>) {
        if self.portfolio.is_none() {
            self.portfolio_loading = true;
        }
        self.portfolio_error = None;
        Self::kick_portfolio(&self.eth, self.display_address, cx);
        Self::kick_block_number(&self.eth, cx);
        // An MCP/CLI agent shields through the daemon WITHOUT this app in the loop, so a
        // manual refresh must re-scan the shielded balance too — otherwise an agent-path
        // deposit stays invisible until the next unlock.
        if let Some(h) = &self.shielded {
            h.resync();
            self.watch_shielded_sync(false, cx);
        }
        cx.notify();
    }

    /// Point the portfolio at the wallet, a raw address, or an ENS name (per settings).
    pub fn retarget(&mut self, cx: &mut Context<Self>) {
        // Each retarget supersedes the last; a slow ENS resolve checks this before applying.
        self.view_epoch += 1;
        let epoch = self.view_epoch;
        let target = self.settings.watch_address.trim().to_string();
        if target.is_empty() {
            self.display_address = self.wallet_address.unwrap_or(Address::ZERO);
            self.viewing_watch = false;
            self.portfolio = None;
            self.refresh_portfolio(cx);
        } else if let Ok(addr) = target.parse::<Address>() {
            self.display_address = addr;
            self.viewing_watch = true;
            self.portfolio = None;
            self.refresh_portfolio(cx);
        } else {
            // Treat as an ENS name: resolve first, then fetch.
            self.viewing_watch = true;
            self.portfolio = None;
            self.portfolio_loading = true;
            self.portfolio_error = None;
            cx.notify();
            let rx = self.eth.resolve_name(target);
            cx.spawn(async move |this, cx| {
                let res = rx.recv_async().await;
                this.update(cx, |this, cx| {
                    // A newer retarget happened while we were resolving — drop this reply.
                    if this.view_epoch != epoch {
                        return;
                    }
                    match res {
                        Ok(Ok(addr)) => {
                            this.display_address = addr;
                            this.refresh_portfolio(cx);
                        }
                        Ok(Err(e)) => {
                            this.portfolio_loading = false;
                            this.portfolio_error =
                                Some(format!("couldn't resolve name — {}", short_err(e)));
                            cx.notify();
                        }
                        Err(_) => {
                            this.portfolio_loading = false;
                            this.portfolio_error = Some("network worker stopped".into());
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Re-spawn the network worker against the RPC URL, but only if it actually changed —
    /// so a no-op blur of the RPC field doesn't tear down the live worker and refetch.
    ///
    /// v1 limitation: this re-points only the *reader*. The signer daemon's RPC + chain are
    /// fixed at launch (mainnet-first), so changing the RPC here does NOT re-point where the
    /// daemon would broadcast. There is no send UI yet (T-UX), so nothing broadcasts through a
    /// diverged endpoint; re-pointing the daemon (and forcing a re-unlock) lands with the send
    /// screen.
    pub fn respawn_provider(&mut self, cx: &mut Context<Self>) {
        let url = self.settings.effective_rpc();
        if url == self.current_rpc {
            return;
        }
        self.current_rpc = url.clone();
        self.eth = EthProvider::spawn(url);
        self.retarget(cx);
        // Re-point the shielded sync at the new RPC too (drops the old worker, clears stale
        // private state) so public and private reads can't diverge across endpoints.
        if self.auth == AuthStep::Ready {
            self.shielded = None;
            self.kick_railgun_grant(cx);
        }
    }

    /// Select a sidebar entity: switch the selection and reset to its Home view.
    pub fn select(&mut self, sel: Selection, cx: &mut Context<Self>) {
        self.selection = sel;
        self.surface = Surface::Home;
        // The agent home renders the daemon's live policy fence — re-fetch on every visit
        // so an out-of-band edit to policy.json (or a STOP) shows up without a relaunch.
        if sel == Selection::Agent {
            self.kick_agent_policy(cx);
        }
        cx.notify();
    }

    /// Open a full-pane surface (Home / Receive / Settings) over the current selection.
    pub fn open(&mut self, surface: Surface, cx: &mut Context<Self>) {
        // Leaving Shield (back, palette, a nav click) cancels any in-progress hold so its
        // timer can't fire a confirm after the screen is gone.
        if surface != Surface::Shield && self.shield_holding {
            self.shield_holding = false;
            self.shield_hold_epoch = self.shield_hold_epoch.wrapping_add(1);
        }
        // Same for the send hold: leaving the Send surface must cancel an in-progress hold so
        // its timer can't fire a confirm after the screen is gone.
        if surface != Surface::Send && self.send_holding {
            self.send_holding = false;
            self.send_hold_epoch = self.send_hold_epoch.wrapping_add(1);
        }
        self.surface = surface;
        cx.notify();
    }

    /// Set the privacy mask to an explicit value (the Settings switch), persisting it.
    pub fn set_mask(&mut self, masked: bool, cx: &mut Context<Self>) {
        if self.mask == masked {
            return;
        }
        self.mask = masked;
        self.settings.mask_balances = masked;
        self.settings.save();
        cx.notify();
    }

    /// Toggle the privacy mask (the ⌘⇧M action, the eye glyph, the click-the-Total
    /// gesture, and the palette row all route here). Persists the new state.
    pub fn toggle_mask(&mut self, cx: &mut Context<Self>) {
        self.set_mask(!self.mask, cx);
    }

    /// Flip the demo "agent currently acting" state (the breathing-pulse driver). Not
    /// persisted — a narrated demo toggle until the daemon exposes an activity feed the
    /// pulse can bind to (the policy card itself is already live via `PolicyGet`).
    pub fn toggle_agent_acting(&mut self, cx: &mut Context<Self>) {
        self.agent_acting = !self.agent_acting;
        cx.notify();
    }

    // --- shield trigger flow (T5) ---

    /// Open the shield flow with a clean slate (clears any prior proposal/error/result; the
    /// typed amount/recipient are left intact). No-op while viewing a watched read-only
    /// account — a shield signs from YOUR wallet, so it must not be initiated from a
    /// someone-else's-address context.
    pub fn open_shield(&mut self, cx: &mut Context<Self>) {
        if self.viewing_watch {
            return;
        }
        self.reset_shield();
        self.open(Surface::Shield, cx);
    }

    /// Clear all transient shield state (proposal, error, broadcast, hold). Bumps the hold +
    /// review epochs so any in-flight hold timer or propose reply lands as a no-op.
    fn reset_shield(&mut self) {
        self.shield_proposal = None;
        self.shield_error = None;
        self.shield_tx = None;
        self.shield_busy = false;
        self.shield_holding = false;
        self.shield_hold_epoch = self.shield_hold_epoch.wrapping_add(1);
        self.shield_review_epoch = self.shield_review_epoch.wrapping_add(1);
    }

    /// Build + `propose` the shield off-thread. On `Allow`, stash the proposal so the review
    /// card + hold-to-confirm appear; on `NeedsApproval`/`Deny`/parse error, surface a clear
    /// line. Mirrors `do_unlock` (build off-thread, fold the result on the UI thread).
    pub fn review_shield(&mut self, cx: &mut Context<Self>) {
        if self.shield_busy {
            return;
        }
        let amount = self.shield_amount.read(cx).value().to_string();
        let recipient = self.shield_recipient.read(cx).value().to_string();
        let value_wei = match signer::parse_eth_to_wei(&amount) {
            Ok(w) if w > U256::ZERO => w,
            Ok(_) => {
                self.shield_error = Some("Enter an amount greater than zero".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.shield_error = Some(e);
                cx.notify();
                return;
            }
        };
        if recipient.trim().is_empty() {
            self.shield_error = Some("Enter a 0zk recipient address".into());
            cx.notify();
            return;
        }
        self.shield_error = None;
        self.shield_proposal = None;
        self.shield_busy = true;
        // Each review supersedes the last; a slow reply for a since-cancelled/re-issued
        // review checks this epoch before installing (and before touching `busy`).
        self.shield_review_epoch = self.shield_review_epoch.wrapping_add(1);
        let epoch = self.shield_review_epoch;
        let recipient_snapshot = recipient.clone();
        cx.notify();
        let client = self.signer.client();
        let chain_id = self.chain_id;
        let task = cx.background_spawn(async move {
            let intent = signer::build_shield_intent(chain_id, &recipient, value_wei)?;
            let decision =
                client.propose_blocking(&intent, deckard_contract::ProposalOrigin::App)?;
            Ok::<(Intent, Decision), anyhow::Error>((intent, decision))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Guard FIRST: a stale review must not even clear `busy` (a newer review may
                // own it now).
                if this.shield_review_epoch != epoch {
                    return;
                }
                this.shield_busy = false;
                match res {
                    Ok((intent, Decision::Allow)) => {
                        let request_id = SignerClient::request_id_for_intent(&intent);
                        this.shield_proposal = Some(ShieldProposal {
                            intent,
                            request_id,
                            recipient: recipient_snapshot,
                            needs_resolve: false,
                        });
                    }
                    // NeedsApproval (over-cap, or the daemon's mainnet guardrail): the
                    // review card + hold-to-confirm ARE the human approval surface — the
                    // hold resolves the pending record, then executes.
                    Ok((intent, Decision::NeedsApproval { request_id })) => {
                        this.shield_proposal = Some(ShieldProposal {
                            intent,
                            request_id,
                            recipient: recipient_snapshot,
                            needs_resolve: true,
                        });
                    }
                    Ok((_, Decision::Deny { reason })) => {
                        // An external STOP/lock ends the session — bounce to the unlock gate.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.shield_error =
                                Some(format!("Can't shield: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.shield_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Sign + broadcast the reviewed shield off-thread (the hold-to-confirm completed). On
    /// success the deposit is on its way to a private note; surface the broadcast.
    pub fn confirm_shield(&mut self, cx: &mut Context<Self>) {
        let Some(ShieldProposal {
            request_id,
            needs_resolve,
            ..
        }) = self.shield_proposal.clone()
        else {
            return;
        };
        if self.shield_busy {
            return;
        }
        self.shield_busy = true;
        self.shield_error = None;
        cx.notify();
        let client = self.signer.client();
        let control = self.signer.control();
        // For a NeedsApproval proposal the completed hold IS the approval: resolve over the
        // private capability channel, then execute (signer::approve_and_execute_blocking). An
        // Allow goes straight to execute.
        let task = cx.background_spawn(async move {
            signer::approve_and_execute_blocking(&client, &control, request_id, needs_resolve)
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.shield_busy = false;
                // Invalidate the proposal on EVERY execute attempt: a second hold must not be
                // able to re-broadcast. On an ambiguous timeout the deposit may already be in
                // flight, so retrying requires a fresh, deliberate review (new request id).
                this.shield_proposal = None;
                match res {
                    Ok(ExecuteResult::Broadcast { tx_hash }) => {
                        this.shield_tx = Some(tx_hash);
                        // Just broadcast — honestly `Sending` (we don't track confirmations).
                        // The re-sync surfaces the note; the watcher then settles to
                        // PrivateSpendable (or Failed), never a fabricated "spendable $0".
                        this.shield_status = Some(ShieldStatus::Sending);
                        if let Some(h) = &this.shielded {
                            h.resync();
                        }
                        this.watch_shielded_sync(true, cx);
                    }
                    Ok(ExecuteResult::Denied { reason }) => {
                        // An external STOP/lock ends the session — bounce to the unlock gate.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.shield_error =
                                Some(format!("Shield denied: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.shield_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Begin a confirm hold: start the amber fill-sweep and a timer that fires
    /// `confirm_shield` only if the hold survives [`SHIELD_HOLD`]. A per-hold epoch guards
    /// against a stale timer firing after an early release / re-press.
    pub fn shield_hold_start(&mut self, cx: &mut Context<Self>) {
        if self.shield_holding || self.shield_busy || self.shield_proposal.is_none() {
            return;
        }
        self.shield_holding = true;
        self.shield_hold_epoch = self.shield_hold_epoch.wrapping_add(1);
        let epoch = self.shield_hold_epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHIELD_HOLD).await;
            this.update(cx, |this, cx| {
                // Only fire if THIS hold is still active (not released, not superseded) AND
                // the user is still on the Shield surface — leaving via ⌘[ / palette / a
                // surface change must never let a held confirm sign after the screen is gone.
                if this.shield_holding
                    && this.shield_hold_epoch == epoch
                    && this.surface == Surface::Shield
                    && this.shield_proposal.is_some()
                {
                    this.shield_holding = false;
                    this.confirm_shield(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Release the confirm hold before it completed — reset the sweep; the epoch bump
    /// cancels the pending timer.
    pub fn shield_hold_cancel(&mut self, cx: &mut Context<Self>) {
        if self.shield_holding {
            self.shield_holding = false;
            self.shield_hold_epoch = self.shield_hold_epoch.wrapping_add(1);
            cx.notify();
        }
    }

    /// Open the native-send flow from the wallet home / palette. Refused while viewing a
    /// watched read-only account — a send signs from YOUR wallet, so it has no meaning there
    /// (the same guard `open_shield` uses).
    pub fn open_send(&mut self, cx: &mut Context<Self>) {
        if self.viewing_watch {
            return;
        }
        self.reset_send();
        self.open(Surface::Send, cx);
    }

    /// Clear all transient send state (proposal, error, broadcast, hold). Bumps the hold +
    /// review epochs so any in-flight hold timer or propose/resolve reply lands as a no-op.
    fn reset_send(&mut self) {
        self.send_proposal = None;
        self.send_error = None;
        self.send_tx = None;
        self.send_busy = false;
        self.send_holding = false;
        self.send_hold_epoch = self.send_hold_epoch.wrapping_add(1);
        self.send_review_epoch = self.send_review_epoch.wrapping_add(1);
    }

    /// Resolve the recipient, then build + `propose` the send off-thread. A `0x…` recipient is
    /// parsed directly (works offline); anything else is treated as an ENS name and
    /// forward-resolved over the same path the watch-address field uses
    /// (`EthProvider::resolve_name`). On `Allow`/`NeedsApproval`, stash the proposal so the
    /// review card + hold-to-confirm appear; on a parse/resolve/`Deny` error, surface a clear
    /// line. Mirrors `review_shield` (epoch-guarded; the guard is checked before `busy`).
    pub fn review_send(&mut self, cx: &mut Context<Self>) {
        if self.send_busy {
            return;
        }
        let amount = self.send_amount.read(cx).value().to_string();
        let recipient = self.send_recipient.read(cx).value().to_string();
        let value_wei = match signer::parse_eth_to_wei(&amount) {
            Ok(w) if w > U256::ZERO => w,
            Ok(_) => {
                self.send_error = Some("Enter an amount greater than zero".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.send_error = Some(e);
                cx.notify();
                return;
            }
        };
        let recipient = recipient.trim().to_string();
        if recipient.is_empty() {
            self.send_error = Some("Enter a recipient address or ENS name".into());
            cx.notify();
            return;
        }
        self.send_error = None;
        self.send_proposal = None;
        self.send_busy = true;
        // Each review supersedes the last; a slow reply for a since-cancelled/re-issued review
        // checks this epoch before installing (and before touching `busy`).
        self.send_review_epoch = self.send_review_epoch.wrapping_add(1);
        let epoch = self.send_review_epoch;
        cx.notify();
        let client = self.signer.client();
        let chain_id = self.chain_id;
        let eth = self.eth.clone();
        // A literal 0x address skips ENS entirely; anything else is treated as a name.
        let parsed = recipient.parse::<Address>().ok();
        let recipient_for_task = recipient.clone();
        let task = cx.background_spawn(async move {
            let to = match parsed {
                Some(addr) => addr,
                None => eth
                    .resolve_name(recipient_for_task.clone())
                    .recv_async()
                    .await
                    .map_err(|_| anyhow::anyhow!("network worker stopped"))?
                    .map_err(|e| anyhow::anyhow!("couldn't resolve name — {}", short_err(e)))?,
            };
            let intent = signer::build_native_send_intent(chain_id, to, value_wei);
            let decision =
                client.propose_blocking(&intent, deckard_contract::ProposalOrigin::App)?;
            Ok::<(Intent, Address, Decision), anyhow::Error>((intent, to, decision))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Guard FIRST: a stale review must not even clear `busy`.
                if this.send_review_epoch != epoch {
                    return;
                }
                this.send_busy = false;
                match res {
                    Ok((intent, to, Decision::Allow)) => {
                        let request_id = SignerClient::request_id_for_intent(&intent);
                        this.send_proposal = Some(SendProposal {
                            intent,
                            request_id,
                            recipient: to.to_checksum(None),
                            needs_resolve: false,
                        });
                    }
                    // NeedsApproval (over-cap, or `Always` approval): the review card +
                    // hold-to-confirm ARE the human approval surface — the hold resolves the
                    // pending record, then executes.
                    Ok((intent, to, Decision::NeedsApproval { request_id })) => {
                        this.send_proposal = Some(SendProposal {
                            intent,
                            request_id,
                            recipient: to.to_checksum(None),
                            needs_resolve: true,
                        });
                    }
                    Ok((_, _, Decision::Deny { reason })) => {
                        // An external STOP/lock ends the session — bounce to the unlock gate.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.send_error =
                                Some(format!("Can't send: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.send_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Sign + broadcast the reviewed send off-thread (the hold-to-confirm completed). For a
    /// `NeedsApproval` proposal the completed hold IS the approval (resolve, then execute); an
    /// `Allow` goes straight to execute. On success the transfer is broadcast and the public
    /// balance is re-fetched. Mirrors `confirm_shield`.
    pub fn confirm_send(&mut self, cx: &mut Context<Self>) {
        let Some(SendProposal {
            request_id,
            needs_resolve,
            ..
        }) = self.send_proposal.clone()
        else {
            return;
        };
        if self.send_busy {
            return;
        }
        self.send_busy = true;
        self.send_error = None;
        cx.notify();
        let client = self.signer.client();
        let control = self.signer.control();
        // For a NeedsApproval proposal the completed hold IS the approval: resolve over the
        // private capability channel, then execute (a Resolve on the public socket is refused).
        let task = cx.background_spawn(async move {
            signer::approve_and_execute_blocking(&client, &control, request_id, needs_resolve)
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.send_busy = false;
                // Invalidate the proposal on EVERY execute attempt: a second hold must not be
                // able to re-broadcast. On an ambiguous timeout the transfer may already be in
                // flight, so retrying requires a fresh, deliberate review (new request id).
                this.send_proposal = None;
                match res {
                    Ok(ExecuteResult::Broadcast { tx_hash }) => {
                        this.send_tx = Some(tx_hash);
                        // The transfer left the wallet — re-fetch the public balance so home
                        // reflects it (a send has no private side to sync, unlike shield).
                        this.refresh_portfolio(cx);
                    }
                    Ok(ExecuteResult::Denied { reason }) => {
                        // An external STOP/lock ends the session — bounce to the unlock gate.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.send_error =
                                Some(format!("Send denied: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.send_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Begin a confirm hold: start the amber fill-sweep and a timer that fires `confirm_send`
    /// only if the hold survives [`SHIELD_HOLD`]. A per-hold epoch guards against a stale timer
    /// firing after an early release / re-press.
    pub fn send_hold_start(&mut self, cx: &mut Context<Self>) {
        if self.send_holding || self.send_busy || self.send_proposal.is_none() {
            return;
        }
        self.send_holding = true;
        self.send_hold_epoch = self.send_hold_epoch.wrapping_add(1);
        let epoch = self.send_hold_epoch;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHIELD_HOLD).await;
            this.update(cx, |this, cx| {
                // Fire only if THIS hold is still active AND the user is still on Send — leaving
                // via ⌘[ / palette / a surface change must never sign after the screen is gone.
                if this.send_holding
                    && this.send_hold_epoch == epoch
                    && this.surface == Surface::Send
                    && this.send_proposal.is_some()
                {
                    this.send_holding = false;
                    this.confirm_send(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Release the confirm hold before it completed — reset the sweep; the epoch bump
    /// cancels the pending timer.
    pub fn send_hold_cancel(&mut self, cx: &mut Context<Self>) {
        if self.send_holding {
            self.send_holding = false;
            self.send_hold_epoch = self.send_hold_epoch.wrapping_add(1);
            cx.notify();
        }
    }

    /// Re-install the theme from the current settings (mode).
    fn apply_theme(&self, cx: &mut Context<Self>) {
        theme::install(cx, self.settings.theme_mode.to_gpui());
    }

    pub fn set_mode(&mut self, mode: ThemeModePref, cx: &mut Context<Self>) {
        self.settings.theme_mode = mode;
        self.settings.save();
        self.apply_theme(cx);
        cx.notify();
    }

    pub fn toggle_mode(&mut self, cx: &mut Context<Self>) {
        let next = match self.settings.theme_mode {
            ThemeModePref::Dark => ThemeModePref::Light,
            ThemeModePref::Light => ThemeModePref::Dark,
        };
        self.set_mode(next, cx);
    }

    // --- agent approvals queue + activity ---

    /// Open the Approvals surface: clear any half-open review, route to the surface, focus its
    /// key handler so j/k/x/Enter land there, and kick a fresh `PendingList` fetch + the
    /// recurring poller. The queue is keyboard-first, so focusing the surface is load-bearing.
    pub fn open_approvals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.approvals_reviewing = None;
        self.open(Surface::Approvals, cx);
        window.focus(&self.approvals_focus, cx);
        self.refresh_pending(cx);
        self.start_pending_poller(cx);
    }

    /// Open the Activity feed (#60): the see-and-stop ledger of what the agent + you did. Unlike
    /// the old terminal-only view this is keyboard-first (proposed rows are inline-approvable),
    /// so it captures focus for its `key_context("Activity")` handler and kicks a fresh
    /// `ActivityFeed` fetch + the shared poller.
    pub fn open_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activity_reviewing = None;
        self.activity_stop_arming = false;
        self.open(Surface::Activity, cx);
        window.focus(&self.activity_focus, cx);
        self.refresh_activity(cx);
        self.start_pending_poller(cx);
    }

    /// Fetch the latest `ActivityFeed` off the UI thread and fold it into `self.activity`,
    /// epoch-guarded (a slow reply for a superseded fetch can't clobber a newer snapshot) and
    /// clamping `activity_selected` to the new approvable-row count. Mirrors `refresh_pending`.
    pub fn refresh_activity(&mut self, cx: &mut Context<Self>) {
        self.activity_epoch = self.activity_epoch.wrapping_add(1);
        let epoch = self.activity_epoch;
        self.activity_loading = true;
        cx.notify();
        let client = self.signer.client();
        let task = cx.background_spawn(async move { client.activity_feed_blocking() });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                if this.activity_epoch != epoch {
                    return;
                }
                this.activity_loading = false;
                match res {
                    Ok(records) => {
                        this.activity_error = None;
                        this.activity = records;
                        let len = crate::activity_view::activity_pending(&this.activity).len();
                        this.activity_selected = if len == 0 {
                            0
                        } else {
                            this.activity_selected.min(len - 1)
                        };
                    }
                    Err(e) => this.activity_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Move the feed highlight down one approvable (still-proposed) row, clamped (no wrap, no
    /// panic on an all-terminal feed). j / ↓ route here.
    pub fn activity_select_next(&mut self) {
        let len = crate::activity_view::activity_pending(&self.activity).len();
        if len > 0 {
            self.activity_selected = (self.activity_selected + 1).min(len - 1);
        }
    }

    /// Move the feed highlight up one row (saturating at the top). k / ↑ route here.
    pub fn activity_select_prev(&mut self) {
        self.activity_selected = self.activity_selected.saturating_sub(1);
    }

    /// Open the inline clear-signing review for the highlighted approvable feed row. No-op when
    /// there are no proposed rows / the selection points past them (guarded via `.get`).
    pub fn open_selected_activity_review(&mut self, cx: &mut Context<Self>) {
        let pending = crate::activity_view::activity_pending(&self.activity);
        if let Some(rec) = pending.get(self.activity_selected) {
            self.activity_reviewing = Some(rec.request_id);
            cx.notify();
        }
    }

    /// Open the inline review for a specific feed row (a click on a proposed row), aligning the
    /// keyboard selection with it so a subsequent ⌘Enter/Esc targets the same record.
    pub fn review_activity_row(
        &mut self,
        request_id: deckard_contract::RequestId,
        cx: &mut Context<Self>,
    ) {
        if let Some(i) = crate::activity_view::activity_pending(&self.activity)
            .iter()
            .position(|r| r.request_id == request_id)
        {
            self.activity_selected = i;
        }
        self.activity_reviewing = Some(request_id);
        cx.notify();
    }

    /// Leave the feed's inline review (Esc, or a completed approve/deny). Pure UI state.
    pub fn cancel_activity_review(&mut self, cx: &mut Context<Self>) {
        self.activity_reviewing = None;
        cx.notify();
    }

    /// Approve the reviewed feed row. Like the Approvals queue, approval ALWAYS routes through
    /// the open review (no blind-approve from a list row): with no review open, `⌘Enter` opens
    /// the highlighted row's review; with one open it resolves `approved == true` over the
    /// private capability channel (the agent then executes its own write once it flips to
    /// `Allowed` — the app never broadcasts).
    pub fn approve_activity(&mut self, cx: &mut Context<Self>) {
        if self.activity_reviewing.is_none() {
            self.open_selected_activity_review(cx);
            return;
        }
        self.resolve_activity_target(true, cx);
    }

    /// Deny the target feed row (the reviewed record, else the highlighted proposed row).
    pub fn deny_activity(&mut self, cx: &mut Context<Self>) {
        self.resolve_activity_target(false, cx);
    }

    /// Shared resolve body for the feed (mirrors `resolve_target` but refreshes the feed). Drives
    /// `Resolve` over the capability channel off-thread and reconciles against the daemon's reply;
    /// on success it leaves the review and re-fetches the feed (the now-decided row updates in
    /// place); on a control-channel failure it fails loud and stays put. No `Execute` ever.
    fn resolve_activity_target(&mut self, approved: bool, cx: &mut Context<Self>) {
        let target = self.activity_reviewing.or_else(|| {
            crate::activity_view::activity_pending(&self.activity)
                .get(self.activity_selected)
                .map(|r| r.request_id)
        });
        let Some(request_id) = target else {
            return;
        };
        let control = self.signer.control();
        self.activity_loading = true;
        cx.notify();
        let task =
            cx.background_spawn(
                async move { signer::resolve_blocking(&control, request_id, approved) },
            );
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| match res {
                Ok(()) => {
                    this.activity_reviewing = None;
                    this.refresh_activity(cx);
                }
                Err(e) => {
                    this.activity_loading = false;
                    this.activity_error = Some(short_err(e));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The feed's STOP control: a deliberate two-step so the irreversible key-zeroize is never a
    /// single click — the first call arms it (the button flips to "Confirm STOP"), the second
    /// fires [`stop_revoke_all`](Self::stop_revoke_all). Esc on the surface disarms.
    pub fn stop_button_clicked(&mut self, cx: &mut Context<Self>) {
        if self.activity_stop_arming {
            self.activity_stop_arming = false;
            self.stop_revoke_all(cx);
        } else {
            self.activity_stop_arming = true;
            cx.notify();
        }
    }

    /// STOP / panic brake from the feed (or the ⌘K command): zeroize the key + deny in-flight via
    /// `revoke_all` off-thread, then refresh so the feed SHOWS the revoke (the daemon answers
    /// `ActivityFeed` while locked). The wallet is now locked; a banner tells the operator to
    /// unlock to re-arm. We deliberately stay on the feed (not jump to the unlock gate) so the
    /// kill is visible — #60 acceptance 3.
    pub fn stop_revoke_all(&mut self, cx: &mut Context<Self>) {
        self.activity_stop_arming = false;
        let client = self.signer.client();
        self.activity_loading = true;
        cx.notify();
        let task = cx.background_spawn(async move { client.revoke_all_blocking() });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| match res {
                Ok(()) => {
                    this.activity_stopped = true;
                    this.agent_acting = false;
                    this.refresh_activity(cx);
                }
                Err(e) => {
                    this.activity_loading = false;
                    this.activity_error = Some(short_err(e));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The Activity feed's per-surface key handler (mirrors `on_approvals_key`): j/↓ k/↑ move the
    /// highlight, x denies, Enter opens the selected row's review, ⌘Enter approves, Esc disarms
    /// STOP then leaves an open review. Scoped via `key_context("Activity")` so it never collides
    /// with the Approvals queue's identical bindings (the two surfaces never render at once).
    fn on_activity_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;
        match key {
            "j" | "down" => {
                self.activity_select_next();
                cx.notify();
            }
            "k" | "up" => {
                self.activity_select_prev();
                cx.notify();
            }
            "x" => self.deny_activity(cx),
            "escape" => {
                if self.activity_stop_arming {
                    self.activity_stop_arming = false;
                    cx.notify();
                } else if self.activity_reviewing.is_some() {
                    self.cancel_activity_review(cx);
                }
            }
            "enter" => {
                if m.platform {
                    self.approve_activity(cx);
                } else {
                    self.open_selected_activity_review(cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Leave the clear-signing review and fall back to the queue (Esc, or a completed
    /// approve/deny). Pure UI state — it never resolves anything on its own.
    pub fn cancel_review(&mut self, cx: &mut Context<Self>) {
        self.approvals_reviewing = None;
        cx.notify();
    }

    /// Open the clear-signing review for the currently-highlighted queue row. No-op when the
    /// queue is empty / the selection somehow points past it (guarded via `.get`), so Enter on
    /// an empty inbox can't panic.
    pub fn open_selected_review(&mut self, cx: &mut Context<Self>) {
        let queue = crate::approvals_view::approvals_queue(&self.pending);
        if let Some(record) = queue.get(self.approvals_selected) {
            self.approvals_reviewing = Some(record.request_id);
            cx.notify();
        }
    }

    /// Fetch the latest `PendingList` off the UI thread and fold it into `self`, epoch-guarded
    /// so a slow reply for a superseded fetch can't clobber a newer snapshot (mirrors
    /// `review_send`/`review_shield`). On success it stores the snapshot and clamps
    /// `approvals_selected` to the new queue length; on transport failure it fails loud.
    pub fn refresh_pending(&mut self, cx: &mut Context<Self>) {
        self.approvals_epoch = self.approvals_epoch.wrapping_add(1);
        let epoch = self.approvals_epoch;
        self.approvals_loading = true;
        cx.notify();
        let client = self.signer.client();
        let task = cx.background_spawn(async move { client.pending_list_blocking() });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Guard FIRST: a stale reply must not even clear `loading` (a newer fetch owns it).
                if this.approvals_epoch != epoch {
                    return;
                }
                this.approvals_loading = false;
                match res {
                    Ok(records) => {
                        this.approvals_error = None;
                        this.pending = records;
                        // Clamp the highlight to the (possibly shorter) new pending queue.
                        let len = crate::approvals_view::approvals_queue(&this.pending).len();
                        this.approvals_selected = if len == 0 {
                            0
                        } else {
                            this.approvals_selected.min(len - 1)
                        };
                    }
                    Err(e) => this.approvals_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Approve the reviewed pending record. Money-safety: approval ALWAYS routes through the
    /// open clear-signing review — you can never blind-approve a spend from a list row or the
    /// palette (DESIGN/spec: "you cannot blind-approve from a list row"). With no review open,
    /// `⌘Enter` / "Approve selected" OPENS the highlighted row's review instead of resolving;
    /// the deliberate step is the review screen plus a second `⌘Enter`. Once a review is open it
    /// resolves `approved == true` over the **private capability channel** and STOPS there — for
    /// an agent-origin proposal the agent executes its own write once the record flips to
    /// `Allowed`; the app never broadcasts.
    pub fn approve_selected(&mut self, cx: &mut Context<Self>) {
        if self.approvals_reviewing.is_none() {
            self.open_selected_review(cx);
            return;
        }
        self.resolve_target(true, cx);
    }

    /// Deny the target pending record (same target resolution as `approve_selected`). Resolves
    /// `approved == false` over the capability channel — the daemon flips the record to
    /// `Denied`; nothing is executed.
    pub fn deny_selected(&mut self, cx: &mut Context<Self>) {
        self.resolve_target(false, cx);
    }

    /// Shared body for approve/deny. Picks the target request id (the reviewed record for an
    /// approve — guaranteed by `approve_selected` — else the highlighted queue row, which is the
    /// spec's one-key list-row deny). A resolve is a money-movement decision, so it is NOT
    /// optimistic: it drives `Resolve` off-thread and reconciles against the daemon's
    /// authoritative reply — only on success does it leave the review and re-fetch (the
    /// now-terminal record drops out of the Pending queue, auto-advancing the highlight to the
    /// next proposal); on a control-channel failure it fails loud and stays put, so the operator
    /// sees the proposal is still unresolved. No `Execute` ever — the authority is the `Resolve`;
    /// the agent does the broadcast once the record flips to `Allowed`.
    fn resolve_target(&mut self, approved: bool, cx: &mut Context<Self>) {
        let target = self.approvals_reviewing.or_else(|| {
            crate::approvals_view::approvals_queue(&self.pending)
                .get(self.approvals_selected)
                .map(|r| r.request_id)
        });
        let Some(request_id) = target else {
            return;
        };
        let control = self.signer.control();
        self.approvals_loading = true;
        cx.notify();
        let task =
            cx.background_spawn(
                async move { signer::resolve_blocking(&control, request_id, approved) },
            );
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| match res {
                Ok(()) => {
                    this.approvals_reviewing = None;
                    this.refresh_pending(cx);
                }
                Err(e) => {
                    this.approvals_loading = false;
                    this.approvals_error = Some(short_err(e));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Move the queue highlight down one row, clamped to the pending queue (no wrap, no panic on
    /// an empty queue). j / ↓ route here.
    pub fn approvals_select_next(&mut self) {
        let len = crate::approvals_view::approvals_queue(&self.pending).len();
        if len > 0 {
            self.approvals_selected = (self.approvals_selected + 1).min(len - 1);
        }
    }

    /// Move the queue highlight up one row (saturating at the top). k / ↑ route here.
    pub fn approvals_select_prev(&mut self) {
        self.approvals_selected = self.approvals_selected.saturating_sub(1);
    }

    /// Start the recurring `PendingList` poller: a ~2s loop that re-fetches while the Approvals
    /// or Activity surface is open, so an agent-parked record (or a settled outcome) shows up
    /// without a manual refresh. Idempotent — guarded by `pending_poller_running` so a second
    /// open never spawns a second loop. The loop self-terminates the moment neither surface is
    /// active (and clears the flag so the next open restarts it). Mirrors `watch_shielded_sync`.
    fn start_pending_poller(&mut self, cx: &mut Context<Self>) {
        if self.pending_poller_running {
            return;
        }
        self.pending_poller_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let keep = this.update(cx, |this, cx| match this.surface {
                    // Each surface refreshes only its own data set (the feed retains executed
                    // rows the pending queue drops, so they are distinct fetches).
                    Surface::Approvals => {
                        this.refresh_pending(cx);
                        true
                    }
                    Surface::Activity => {
                        this.refresh_activity(cx);
                        true
                    }
                    _ => {
                        // Off both surfaces: stop polling and let the next open restart the loop.
                        this.pending_poller_running = false;
                        false
                    }
                });
                match keep {
                    Ok(true) => continue,
                    // Either the view is gone (Err) or we left both surfaces (Ok(false)) — stop.
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// The Approvals surface's per-surface key handler (mirrors `on_palette_key`): j/↓ and k/↑
    /// move the highlight, x denies, Esc leaves an open review, Enter opens the selected row's
    /// review, and ⌘Enter (`m.platform`) approves outright. Scoped via `key_context("Approvals")`
    /// + the focused `approvals_focus`, and `stop_propagation` keeps a bare key off the globals.
    fn on_approvals_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let m = ks.modifiers;
        match key {
            "j" | "down" => {
                self.approvals_select_next();
                cx.notify();
            }
            "k" | "up" => {
                self.approvals_select_prev();
                cx.notify();
            }
            "x" => self.deny_selected(cx),
            "escape" => {
                if self.approvals_reviewing.is_some() {
                    self.cancel_review(cx);
                }
            }
            "enter" => {
                if m.platform {
                    self.approve_selected(cx);
                } else {
                    self.open_selected_review(cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    // --- Action handlers (wired in `render`) ---
    //
    // While the palette is open it OWNS the keyboard: gpui dispatches these global ⌘-shortcuts
    // (None-context bindings) BEFORE the palette panel's `on_key_down`, so without a guard they'd
    // fire behind the open overlay (e.g. ⌘, opening Settings under the palette). Each gates on
    // `palette_open` so the shortcut is inert while the palette is up — its action is reachable as a
    // palette command instead. ⌘K (`on_toggle_palette`) and ⌘Q (Quit) are intentionally NOT gated.

    fn on_new_item(&mut self, _: &NewItem, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        self.created += 1;
        cx.notify();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        self.open(Surface::Settings, cx);
    }

    fn on_open_approvals(
        &mut self,
        _: &OpenApprovals,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette_open {
            return;
        }
        self.open_approvals(window, cx);
    }

    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        // Back = leave any action surface and return to the selection's Home view.
        if self.surface != Surface::Home {
            self.open(Surface::Home, cx);
        }
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        self.toggle_mode(cx);
    }

    fn on_toggle_palette(
        &mut self,
        _: &TogglePalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_palette(window, cx);
    }

    /// Open or close the command palette. Both the ⌘K binding (`on_toggle_palette`) and the
    /// breadcrumb ⌘K affordance route here, so the open path is identical from either entry —
    /// it MUST capture focus + recompute results + focus the panel, or the overlay renders inert.
    pub fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            // Toggling closed restores the focus we captured on open.
            self.close_palette(window, cx);
        } else {
            // Remember what had focus so closing can return there, then open with a clean
            // query, recompute the (frecency-ordered) results, and focus the palette panel so
            // its `key_context("CommandPalette")` listener starts receiving keys immediately.
            self.palette_prev_focus = window.focused(cx);
            self.palette_query.clear();
            self.palette_selected = 0;
            self.repalette(cx);
            self.palette_open = true;
            window.focus(&self.palette_focus, cx);
            cx.notify();
        }
    }

    /// Recompute the palette results from the current query (pure `palette_commands::rank` over
    /// the static registry, with the frecency store + reused matcher). Clamp the selection so it
    /// can never point past the (possibly shorter) new result list.
    pub(crate) fn repalette(&mut self, _cx: &mut Context<Self>) {
        self.palette_results = crate::palette_commands::rank(
            &self.palette_query,
            crate::palette_commands::COMMANDS,
            &self.palette_usage,
            crate::palette_usage::now_unix_secs(),
            &mut self.palette_matcher,
        );
        self.palette_selected = self
            .palette_selected
            .min(self.palette_results.len().saturating_sub(1));
    }

    /// Run the palette command with stable `id`: record the use (frecency), perform the action
    /// (the bodies migrated from the old click-only palette), then close the palette. Unknown ids
    /// are a no-op (still closes) — the registry and this match are kept in lockstep.
    pub fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_usage.record(id);
        match id {
            "portfolio" => {
                self.select(Selection::Wallet, cx);
                self.open(Surface::Home, cx);
            }
            "send" => self.open_send(cx),
            "receive" => self.open(Surface::Receive, cx),
            "shield" => self.open_shield(cx),
            "approvals" => self.open_approvals(window, cx),
            "activity" => self.open_activity(window, cx),
            "approve-selected" => self.approve_selected(cx),
            "deny-selected" => self.deny_selected(cx),
            // STOP / panic brake — its OWN id, never overloading the demo `agent` toggle. The
            // ⌘K selection is itself the deliberate act, so this fires the kill directly.
            "revoke-all" => self.stop_revoke_all(cx),
            "settings" => self.open(Surface::Settings, cx),
            "copy" => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    self.wallet_address_string(),
                ));
                cx.notify();
            }
            "theme" => self.toggle_mode(cx),
            "mask" => self.toggle_mask(cx),
            "agent" => self.toggle_agent_acting(cx),
            // `lock` returns to the unlock gate and already clears `palette_open`; closing again
            // below is a harmless no-op (the prev-focus restore targets a now-replaced view).
            "lock" => self.lock(cx),
            _ => {}
        }
        self.close_palette(window, cx);
    }

    /// Close the palette and restore the focus captured on open (codex m7) so dismissing the
    /// overlay never strands the keyboard in a now-hidden panel. Idempotent.
    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        if let Some(prev) = self.palette_prev_focus.take() {
            window.focus(&prev, cx);
        }
        cx.notify();
    }

    fn on_toggle_mask(&mut self, _: &ToggleMask, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        self.toggle_mask(cx);
    }

    // Tab / Shift-Tab in the palette (bound in the `CommandPalette` context so they fire only while
    // it's focused, shadowing Root's focus-traversal). They mirror ↓/↑ in `on_palette_key`.
    fn on_palette_next(&mut self, _: &PaletteNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.palette_open {
            return;
        }
        self.palette_select_next();
        cx.notify();
    }

    fn on_palette_prev(&mut self, _: &PalettePrev, _: &mut Window, cx: &mut Context<Self>) {
        if !self.palette_open {
            return;
        }
        self.palette_select_prev();
        cx.notify();
    }

    /// Move the palette selection down/up, clamped to the current results. Shared by ↑/↓ in
    /// `on_palette_key` and the Tab/Shift-Tab actions; the caller notifies.
    pub(crate) fn palette_select_next(&mut self) {
        let n = self.palette_results.len();
        if n > 0 {
            self.palette_selected = (self.palette_selected + 1).min(n - 1);
        }
    }

    pub(crate) fn palette_select_prev(&mut self) {
        self.palette_selected = self.palette_selected.saturating_sub(1);
    }

    /// Clear a prior session's shield inputs once after lock, then pre-fill the recipient with
    /// the user's own 0zk address once the grant arrives (still editable). Runs from `render`,
    /// the only place with a `Window` for `set_value`.
    fn prepare_shield_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_shield_clear {
            self.pending_shield_clear = false;
            self.shield_amount
                .update(cx, |i, cx| i.set_value("", window, cx));
            self.shield_recipient
                .update(cx, |i, cx| i.set_value("", window, cx));
            // The send inputs share the lock-clear: a prior wallet's recipient/amount must not
            // linger into the next unlock.
            self.send_amount
                .update(cx, |i, cx| i.set_value("", window, cx));
            self.send_recipient
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        if self.recipient_autofilled {
            return;
        }
        if let Some(addr) = self.railgun_address.clone() {
            self.shield_recipient.update(cx, |input, cx| {
                input.set_value(addr.as_str(), window, cx);
            });
            self.recipient_autofilled = true;
        }
    }

    /// Push the capture-block state to the OS when `capture_block && mask` changes.
    /// Called once per `render`; the change-guard makes it a no-op on most frames. On a
    /// non-macOS or non-`tray` build `apply_capture_block` is itself an inert no-op.
    fn sync_capture_block(&mut self) {
        // `allow_screen_capture` (the DECKARD_ALLOW_SCREEN_CAPTURE recording override) forces the
        // block off even when the setting + mask would otherwise engage it.
        let desired = self.settings.capture_block && self.mask && !self.allow_screen_capture;
        if desired != self.capture_applied {
            crate::capture::apply_capture_block(desired);
            self.capture_applied = desired;
        }
    }

    /// A bare macOS title bar: just the traffic-light inset + the app name. Its old
    /// settings/theme controls now live in the breadcrumb (`shell_chrome.rs`).
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        TitleBar::new().child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .child(APP_NAME),
        )
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;

        // Keep the OS capture-block in sync with `capture_block && mask` (no-op unless it
        // changed, and a no-op entirely off a macOS `--features tray` build).
        self.sync_capture_block();

        let body = if self.auth == AuthStep::Ready {
            // The unlocked app: macOS title bar above the two-pane shell grid
            // (sidebar | [breadcrumb / content / status strip]) + command palette.
            self.prepare_shield_inputs(window, cx);
            let title_bar = self.render_title_bar(cx);
            // Each scrollable surface inlines its OWN `.overflow_y_scrollbar()` (don't factor into
            // a helper): gpui-component keys the scroll offset by call site, so per-arm calls give
            // each surface an independent offset. Receive/Shield are short centered cards — no wrapper.
            let content = match (self.selection, self.surface) {
                (_, Surface::Settings) => div()
                    .id("scroll-settings")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(self.render_settings(window, cx))
                    .into_any_element(),
                (_, Surface::Receive) => self.render_receive(cx).into_any_element(),
                (_, Surface::Send) => self.render_send(cx).into_any_element(),
                (_, Surface::Shield) => self.render_shield(cx).into_any_element(),
                // Approvals owns the keyboard while it's the active surface: track its focus
                // handle + scope the in-queue keys (j/k/x/Enter/⌘Enter/Esc) to `key_context`,
                // so they never leak to a global binding behind it.
                (_, Surface::Approvals) => div()
                    .id("scroll-approvals")
                    .size_full()
                    .overflow_y_scrollbar()
                    .track_focus(&self.approvals_focus)
                    .key_context("Approvals")
                    .on_key_down(cx.listener(Self::on_approvals_key))
                    .child(self.render_approvals(cx))
                    .into_any_element(),
                // The feed owns the keyboard while it's the active surface: track its focus +
                // scope j/k/x/Enter/⌘Enter/Esc to `key_context("Activity")` so they never collide
                // with the Approvals queue's identical bindings or leak to a global behind it.
                (_, Surface::Activity) => div()
                    .id("scroll-activity")
                    .size_full()
                    .overflow_y_scrollbar()
                    .track_focus(&self.activity_focus)
                    .key_context("Activity")
                    .on_key_down(cx.listener(Self::on_activity_key))
                    .child(self.render_activity(cx))
                    .into_any_element(),
                (Selection::Wallet, Surface::Home) => div()
                    .id("scroll-wallet")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(self.render_wallet_home(cx))
                    .into_any_element(),
                (Selection::Project, Surface::Home) => div()
                    .id("scroll-project")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(self.render_project_home(cx))
                    .into_any_element(),
                (Selection::Agent, Surface::Home) => div()
                    .id("scroll-agent")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(self.render_agent_home(cx))
                    .into_any_element(),
            };
            v_flex()
                .size_full()
                .child(title_bar)
                .child(
                    h_flex().size_full().child(self.render_sidebar(cx)).child(
                        // Fill the full pane height (like the sidebar's `.h_full()`): `h_flex`
                        // centers its children vertically, so without this the content column
                        // collapses to its intrinsic height and floats mid-pane — the
                        // breadcrumb, content, and bottom status strip then bunch up and overlap
                        // whenever a view is shorter than the viewport.
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .min_h_0()
                            .child(self.render_breadcrumb(cx))
                            // The slot is a `v_flex`, not a plain `div` (gpui defaults to
                            // `display: block`): the centered Receive/Shield roots use `flex_1` +
                            // `justify_center`, which only fill + center inside a flex parent.
                            .child(v_flex().flex_1().min_h_0().child(content))
                            .child(self.render_status_strip(cx)),
                    ),
                )
                .children(self.palette_open.then(|| self.render_palette(cx)))
                .into_any_element()
        } else {
            // Auto-focus the step's primary input the first time it appears.
            if self.focused_step != Some(self.auth) {
                self.focus_auth_input(window, cx);
                self.focused_step = Some(self.auth);
            }
            // The auth gate: a minimal title bar + the onboarding / unlock surface.
            v_flex()
                .size_full()
                .child(self.render_auth_title_bar(cx))
                .child(self.render_auth(cx))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .relative()
            .bg(background)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_item))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_approvals))
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_toggle_palette))
            .on_action(cx.listener(Self::on_toggle_mask))
            .on_action(cx.listener(Self::on_palette_next))
            .on_action(cx.listener(Self::on_palette_prev))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::is_session_ended;

    #[test]
    fn session_ended_matches_only_stop_states() {
        // A locked daemon answers `locked` to a propose; an execute after STOP answers
        // `revoked`. Both must bounce the app back to the unlock gate.
        assert!(is_session_ended("locked"));
        assert!(is_session_ended("revoked"));
        // Ordinary policy denials stay inline (the app stays Ready, shows the reason).
        for inline in [
            "over_cap",
            "off_allowlist",
            "chain_mismatch",
            "shield_to_mismatch",
            "not_approved",
            "already_executed",
            "broadcast_timeout",
        ] {
            assert!(!is_session_ended(inline), "{inline} must stay inline");
        }
    }
}
