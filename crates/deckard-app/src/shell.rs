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

use deckard_contract::{
    Decision, ExecuteResult, Intent, Policy, ShieldStatus, SignerRequest, SignerResponse,
};
use deckard_core::{
    tokens_for, Address, CowOrderbook, EthProvider, KdfParams, Portfolio, QuoteResponse,
    ReadStatus, ShieldedHandle, Vault, WordCount, U256,
};
use zeroize::Zeroizing;

use deckard_signerd::SignerClient;

use crate::commit_flow::CommitFlow;
use crate::errors::{humanize_deny, humanize_swap_deny, is_session_ended, short_err};
use crate::settings::{Settings, ThemeModePref};
use crate::signer::{self, AppSigner};
use crate::theme;
use crate::wallet;
use crate::{
    ConfirmCommit, GoBack, NewItem, OpenApprovals, OpenSettings, PaletteNext, PalettePrev,
    ToggleMask, TogglePalette, ToggleTheme, APP_NAME,
};

/// Auto-refresh the public wallet balance every this many seconds while the home view is open.
const BALANCE_POLL_SECS: u64 = 20;

/// The confirm arm-delay: a clear-signing review must be on screen this long before ⌘↵ / a
/// click can confirm, so a keypress or click carried over from the previous screen can't approve
/// a money move (DESIGN §confirm pattern). The ⌘↵ chord plus this delay replace the old hold.
const COMMIT_ARM_DELAY: Duration = Duration::from_millis(450);

/// How long the recovery phrase stays visible on a single hold-to-reveal before it auto-hides
/// (DESIGN §Seed reveal: "auto-hides after a few seconds"). A defence against walking away with
/// the seed on screen — even while the button is still held, the words blur back after this.
const SEED_REVEAL_TIMEOUT: Duration = Duration::from_secs(10);

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
/// renders differently per selection (project / wallet). Demo scope is a single
/// project + wallet; and Atlas, the agent. Atlas is now a FIRST-CLASS entity
/// (DESIGN.md v2 §The agent interaction model): a standalone sidebar row that opens
/// its own surface (policy + controls + its activity), no longer folded into the
/// wallet home. It is still key-less automation on the same wallet EOA.
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
    /// The session activity feed (#60): the see-and-stop ledger of every tracked action —
    /// auto-allowed, pending, decided, and executed — newest-first, read from the daemon's
    /// `ActivityFeed`. Pending rows are inline-approvable; a header STOP control is the brake.
    Activity,
    /// The CoW swap flow (#25): compose (sell amount + token pickers) → get a quote → review the
    /// priced order → hold-to-confirm (propose + approve + resolve + sign + submit). The daemon
    /// signs the EIP-712 order; the app posts it to the orderbook. The app holds no key.
    Swap,
    Settings,
}

/// A reviewed-and-allowed shield, ready to sign. Carries a **recipient snapshot** taken at
/// review time so the clear-signing card always shows the recipient that is actually inside
/// `intent` — never a value the user edited in the input after `propose` landed.
///
/// Now the shared [`crate::commit_flow::Proposal`] (Shield + Send were field-identical and
/// collapsed in Step 0); the alias keeps every `ShieldProposal { .. }` construction site unchanged.
pub type ShieldProposal = crate::commit_flow::Proposal;

/// A reviewed-and-allowed native send, ready to sign. Carries a **recipient snapshot** (the
/// resolved, checksummed destination address that is actually inside `intent.to`) so the
/// clear-signing card always shows where the ETH is going — never the raw `0x…`/ENS text the
/// user could have since edited. The `needs_resolve` flag: an over-cap (or `Always`-approval)
/// send returns `NeedsApproval`, and the completed hold-to-confirm IS that human approval, so
/// confirm sends `Resolve{approved: true}` first.
///
/// The same shared [`crate::commit_flow::Proposal`] as [`ShieldProposal`] — the two flows were
/// field-identical and collapsed in Step 0; the alias keeps every `SendProposal { .. }` site
/// unchanged.
pub type SendProposal = crate::commit_flow::Proposal;

/// The auth gate that wraps the whole app. Until it reaches `Ready`, the portfolio and
/// every funds-touching surface are hidden behind onboarding or the unlock screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AuthStep {
    /// First run, no vault: choose to create or import.
    Choose,
    /// Create: set the passphrase.
    CreateSetup,
    /// Create: reveal the recovery phrase (read-only; verification is a separate step).
    CreateBackup,
    /// Create: verify the backup by retyping requested words, with the grid hidden.
    CreateVerify,
    /// Create: the vault is sealed + unlocked — a calm "you're ready" interstitial.
    CreateDone,
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
    /// The signer's live policy fence, rendered on the wallet home's "What Atlas may do"
    /// card so it shows the SAME numbers `deckard_policy_get` returns. Fetched from the
    /// daemon (`PolicyGet` deliberately succeeds while locked — the fence is config, not a
    /// secret); `None` until the first fetch lands or when the daemon is unreachable.
    pub agent_policy: Option<Policy>,

    // --- activity feed (#60: the see-and-stop ledger) ---
    /// The latest `ActivityFeed` snapshot — every tracked action (auto-allowed, pending, denied,
    /// executed), newest-first, with `tx_hash`/`timestamp_ms`/breached-cap. The Activity surface
    /// renders this; refreshed by `refresh_activity` + the poller. It is the feed's sole source —
    /// the still-proposed rows form the "NEEDS YOU" band (the inline triage queue), and the feed
    /// also retains executed/auto-allowed rows the pending-only view would drop.
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
    /// stale snapshot.
    activity_epoch: u64,
    /// The Activity surface's focus handle — `track_focus`'d so its `key_context("Activity")`
    /// listener owns the in-feed keys (j/k/x/Enter/⌘Enter/Esc) without leaking to a global behind it.
    activity_focus: gpui::FocusHandle,
    /// True while the recurring activity-feed poller loop is alive, so opening the surface twice
    /// can't spawn a second loop (the loop self-terminates when the feed is no longer active).
    activity_poller_running: bool,
    /// STOP confirm arming: a first click on the feed's STOP control arms it (shows "confirm"),
    /// a second confirms — so the irreversible key-zeroize is never a single click. Esc disarms.
    /// `pub` so the read-only `activity_view` can render the armed state.
    pub activity_stop_arming: bool,
    /// Set once a STOP from the feed succeeded — drives the "Stopped — key zeroized, unlock to
    /// re-arm" banner. The feed stays visible (the daemon answers `ActivityFeed` while locked) so
    /// the revoked rows are seen; the next unlock clears it. `pub` so the view can render it.
    pub activity_stopped: bool,
    /// The confirm arm-delay timestamp: set when a clear-signing review (Send/Shield/Swap) is
    /// installed. `commit_armed()` is true once [`COMMIT_ARM_DELAY`] has elapsed, gating the
    /// ⌘↵/click confirm so a carried-over keypress can't approve (DESIGN §confirm pattern).
    commit_review_at: Option<std::time::Instant>,
    /// Focus handle for the commit surfaces (Send/Shield/Swap) so the `key_context("Commit")`
    /// ⌘↵ binding dispatches to the confirm handler only when a review is on screen.
    commit_focus: gpui::FocusHandle,

    /// The capture-block state last pushed to the OS, so `render` only re-issues the
    /// native `setSharingType` call when `capture_block && mask` actually changes.
    capture_applied: bool,
    /// Recording override (`DECKARD_ALLOW_SCREEN_CAPTURE`): when set, force the capture block
    /// OFF regardless of the `capture_block` setting, so an automated agent can record the demo
    /// GIF without touching the settings UI. Resolved once at launch; default false (the setting
    /// governs) — a normal build never disables the trust feature behind the user's back.
    pub allow_screen_capture: bool,

    // --- shield trigger flow (T5) ---
    /// The shield trigger flow's state machine: the amount + `0zk…` recipient inputs (the
    /// recipient auto-fills the wallet's own 0zk address; free-text edit is allowed), the reviewed
    /// proposal, the busy/hold flags, the surfaced error/broadcast, and the review/hold epochs that
    /// fence stale background replies and stale hold timers. Migrated off the flat `shield_*` fields
    /// onto [`CommitFlow`] (Step 2); access its state via deref (`self.shield.proposal`,
    /// `self.shield.busy`, …). The deposit's private-side broadcast wiring (`shield_status` /
    /// resync / `watch_shielded_sync`) stays inline in `confirm_shield`.
    pub shield: CommitFlow,

    // --- native-ETH send flow (mirrors the shield trigger flow above) ---
    /// The native-ETH send flow's state machine: the amount + recipient inputs (a `0x…` address
    /// or an ENS name, forward-resolved at review time), the reviewed proposal, the busy/hold
    /// flags, the surfaced error/broadcast, and the review/hold epochs that fence stale background
    /// replies and stale hold timers. Migrated off the flat `send_*` fields onto [`CommitFlow`]
    /// (Step 1); access its state via deref (`self.send.proposal`, `self.send.busy`, …).
    pub send: CommitFlow,

    // --- CoW swap flow (#25) ---
    /// The swap flow's commit state machine. Reuses [`CommitFlow`]'s `amount` input (the sell
    /// amount) + the proposal/busy/hold/error/tx core for the review→hold→sign lifecycle; the
    /// `recipient` input is unused (a swap's receiver is always your own wallet). The bespoke
    /// compose (token pickers, quote summary) and the bespoke review/done live in `swap_view.rs`;
    /// the amber hold widget is shared via [`SWAP_VIEW`](crate::swap_view::SWAP_VIEW).
    pub swap: CommitFlow,
    /// The sell-side token, an address from [`tokens_for(chain_id)`](deckard_core::tokens_for).
    /// `None` until a chain with a curated list is active (mainnet/Sepolia) and the picker seeds it.
    pub swap_sell_token: Option<Address>,
    /// The buy-side token (same source). Seeded distinct from `swap_sell_token`.
    pub swap_buy_token: Option<Address>,
    /// The last fetched quote (drives the quote summary + the bound order). Cleared on every
    /// compose edit so a stale quote can't be signed against changed inputs.
    pub swap_quote: Option<deckard_core::QuoteResponse>,
    /// True while a `Get quote` round-trip runs on a background thread (the one allowed loading
    /// state on compose; never a spinner-forever — it clears on reply/error).
    pub swap_quoting: bool,
    /// The created order's CoW uid, set on a successful submit — drives the bespoke done screen
    /// (a swap produces a uid string, not a B256 tx hash, so it can't ride `CommitState.tx`).
    pub swap_uid: Option<String>,
    /// Bumped on every quote request (and on reset) so a slow quote reply for a since-changed
    /// compose can't install a stale quote. Mirrors the review/hold epoch pattern.
    swap_quote_epoch: u64,

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
    /// Monotonic guard so a stale auto-hide timer can't blank a *later* reveal (each
    /// reveal/hide bumps it; the timer only fires if its epoch still matches).
    reveal_epoch: u64,
    /// Monotonic guard for an in-flight `Vault::create`: bumped whenever the create flow is
    /// (re)started or abandoned, so a slow KDF that finishes *after* the user backed out drops
    /// its freshly-derived secrets instead of populating them into memory.
    create_epoch: u64,
    /// True briefly after the recovery phrase is copied, to flip the demoted Copy button to
    /// "Copied ✓" — never auto-set; only an explicit click sets it (DESIGN §Seed reveal).
    pub seed_copied: bool,
    /// True briefly after the new wallet address is copied on the "Ready" screen (inline "Copied ✓").
    pub address_copied: bool,
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
    /// True while a public portfolio read (or ENS resolution) is in flight — the single
    /// "refresh in flight" dedup flag. First-sync UI = `portfolio_loading && portfolio.is_none()`.
    pub portfolio_loading: bool,
    pub portfolio_error: Option<String>,
    /// Trust label for the last portfolio/block read: Helios-`Verified` vs visibly
    /// `Unsynced`/`Degraded`. Never silently "trusted" — surfaced in the status line.
    pub read_status: Option<ReadStatus>,
    /// Latest block height — a liveness/sync indicator for the status line.
    pub synced_block: Option<u64>,
    /// Handle to the running balance auto-refresh loop; dropped on lock to cancel it.
    poll_task: Option<gpui::Task<()>>,
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

        // Shield flow inputs (T5): amount in ETH + the 0zk recipient (auto-filled with the wallet's
        // own 0zk address; free-text edit is allowed).
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
        // The subscriptions above reference the entities (Enter-to-review / live-validity); now
        // move the two inputs into the shield flow's state machine (Step 2). The subscriptions keep
        // firing — they hold their own entity handles, independent of where the inputs now live.
        let shield = CommitFlow::new(shield_amount, shield_recipient);

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
        // The subscriptions above reference the entities (Enter-to-review / live-validity); now
        // move the two inputs into the send flow's state machine (Step 1). The subscriptions keep
        // firing — they hold their own entity handles, independent of where the inputs now live.
        let send = CommitFlow::new(send_amount, send_recipient);

        // Swap flow inputs (#25): the amount input doubles as the sell amount; the recipient input
        // is a throwaway (a swap's receiver is always your own wallet) — `CommitFlow::new` just
        // needs two entities. On a sell-amount edit, clear any stale quote (a quote priced for an
        // old amount must never survive into a confirm) and re-render; Enter gets a quote if there
        // isn't one yet, else reviews the priced order (keyboard-first).
        let swap_amount =
            cx.new(|cx| InputState::new(window, cx).placeholder("Amount to sell, e.g. 0.05"));
        let swap_recipient = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(
            &swap_amount,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    // A stale quote must never outlive an amount edit (codex must-do #4).
                    this.invalidate_swap_quote();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if this.swap_quote.is_some() {
                        this.review_swap(cx);
                    } else {
                        this.get_swap_quote(cx);
                    }
                }
                _ => {}
            },
        )
        .detach();
        let swap = CommitFlow::new(swap_amount, swap_recipient);

        // Submit-on-Enter for each auth field (keyboard-first).
        // The first passphrase field re-renders on every edit so the live strength meter tracks
        // it (DESIGN §Onboarding); Enter on the confirm field submits.
        cx.subscribe(&create_pass, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&create_pass2, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.do_create(cx);
            }
        })
        .detach();
        // The verify field re-renders on every edit so the "Confirm & finish" button can stay
        // disabled until the typed words match (DESIGN §Onboarding); Enter submits.
        cx.subscribe(
            &confirm_words,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.confirm_backup(cx),
                _ => {}
            },
        )
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

        // One resolved runtime chain id (env > settings > default), threaded to the daemon
        // launch, the shield builder, and the Railgun sync. Resolve it BEFORE the RPC so the
        // per-chain default RPC keys off the right chain.
        let chain_id = settings.effective_chain_id();
        // The network worker is always live (it serves the watch-only path too), but the
        // wallet portfolio isn't fetched until the vault is unlocked.
        let current_rpc = settings.effective_rpc(chain_id);
        let eth = EthProvider::spawn(current_rpc.clone(), chain_id);

        // Log the resolved runtime config once (the RPC is REDACTED to scheme://host — it may
        // carry an API key). Makes "which chain / RPC / mode am I on?" answerable from the log,
        // which matters most exactly when an env override re-points the demo off mainnet.
        eprintln!(
            "deckard: runtime — chain {chain_id} ({}) · rpc {} · verified-reads {} · fork-mode {}",
            deckard_core::network_name(chain_id).unwrap_or("unknown chain"),
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
            agent_policy: None,
            activity: Vec::new(),
            activity_selected: 0,
            activity_reviewing: None,
            activity_loading: false,
            activity_error: None,
            activity_epoch: 0,
            activity_focus: cx.focus_handle(),
            activity_poller_running: false,
            activity_stop_arming: false,
            activity_stopped: false,
            commit_review_at: None,
            commit_focus: cx.focus_handle(),
            capture_applied: false,
            allow_screen_capture,
            shield,
            send,
            swap,
            swap_sell_token: None,
            swap_buy_token: None,
            swap_quote: None,
            swap_quoting: false,
            swap_uid: None,
            swap_quote_epoch: 0,
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
            reveal_epoch: 0,
            create_epoch: 0,
            seed_copied: false,
            address_copied: false,
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
            poll_task: None,
            view_epoch: 0,
            current_rpc,
            chain_id,
        }
    }

    // --- auth / keystore actions (Chunk 3) ---

    pub fn start_create(&mut self, cx: &mut Context<Self>) {
        // Start clean + self-sufficient (don't rely on the caller having abandoned a prior try):
        // invalidate any still-running KDF, clear the busy flag, and wipe anything a previous
        // attempt staged.
        self.abandon_create();
        self.auth = AuthStep::CreateSetup;
        self.auth_error = None;
        cx.notify();
    }

    pub fn start_import(&mut self, cx: &mut Context<Self>) {
        // Leaving create for import abandons any in-flight KDF + clears anything it staged.
        self.abandon_create();
        self.auth = AuthStep::Import;
        self.auth_error = None;
        cx.notify();
    }

    pub fn auth_back_to_choose(&mut self, cx: &mut Context<Self>) {
        self.abandon_create();
        self.auth = AuthStep::Choose;
        self.auth_error = None;
        cx.notify();
    }

    /// Tear down any in-progress create: invalidate a still-running KDF (so its result is dropped,
    /// not stored), clear the busy flag, and wipe every secret it may have staged. Secrets live in
    /// `Zeroizing`, so dropping them here zeroizes them.
    fn abandon_create(&mut self) {
        self.create_epoch = self.create_epoch.wrapping_add(1);
        self.auth_busy = false;
        self.clear_pending_secrets();
    }

    /// Drop every staged-but-uncommitted create secret + the reveal/copy UI state. Each secret is
    /// `Zeroizing`, so `= None` zeroizes it.
    fn clear_pending_secrets(&mut self) {
        self.pending_phrase = None;
        self.pending_pass = None;
        self.pending_vault = None;
        self.reveal_seed = false;
        self.seed_copied = false;
    }

    pub fn set_reveal_seed(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.reveal_seed = reveal;
        // Every reveal/hide invalidates any pending auto-hide timer (so releasing then re-holding
        // restarts the clock instead of inheriting the old one).
        self.reveal_epoch = self.reveal_epoch.wrapping_add(1);
        if reveal {
            let epoch = self.reveal_epoch;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SEED_REVEAL_TIMEOUT).await;
                this.update(cx, |this, cx| {
                    // Only blank if this is still the same (un-superseded) reveal.
                    if this.reveal_epoch == epoch && this.reveal_seed {
                        this.reveal_seed = false;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Copy the recovery phrase to the clipboard — only ever from an explicit click on the demoted
    /// Copy button (never auto-copied; DESIGN §Seed reveal). The phrase lives in `Zeroizing`; this
    /// hands a plain copy to the OS clipboard at the user's deliberate request, then flips the
    /// button to "Copied ✓" for a moment.
    pub fn copy_recovery_phrase(&mut self, cx: &mut Context<Self>) {
        let Some(phrase) = self.pending_phrase.as_ref() else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(phrase.to_string()));
        self.seed_copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| {
                this.seed_copied = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Copy the new wallet's address to the clipboard from the "Ready" screen, then flash an inline
    /// "Copied ✓" for a moment (DESIGN §Trust: addresses are one-click-copy with inline feedback).
    pub fn copy_wallet_address(&mut self, cx: &mut Context<Self>) {
        let addr = self.wallet_address_string();
        if addr.is_empty() {
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(addr));
        self.address_copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |this, cx| {
                this.address_copied = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// CreateBackup → CreateVerify: the user confirms they saved the phrase; hide it and move to
    /// the separate verify step (the grid is not shown there).
    pub fn advance_to_verify(&mut self, cx: &mut Context<Self>) {
        self.reveal_seed = false;
        self.reveal_epoch = self.reveal_epoch.wrapping_add(1);
        self.seed_copied = false;
        self.auth_error = None;
        self.auth = AuthStep::CreateVerify;
        cx.notify();
    }

    /// CreateVerify → CreateBackup: let the user go back to re-read the phrase before verifying.
    pub fn back_to_backup(&mut self, cx: &mut Context<Self>) {
        self.reveal_seed = false;
        // Invalidate any pending auto-hide timer, same as advance_to_verify — keep the reveal-epoch
        // discipline symmetric across both backup↔verify transitions.
        self.reveal_epoch = self.reveal_epoch.wrapping_add(1);
        self.auth_error = None;
        self.auth = AuthStep::CreateBackup;
        cx.notify();
    }

    /// CreateDone → the live app: the vault is already sealed + unlocked (the verify step did it);
    /// this just wires up the app's live session (portfolio, pollers, agent policy).
    pub fn enter_after_create(&mut self, cx: &mut Context<Self>) {
        if let Some(addr) = self.wallet_address {
            self.finish_unlock(addr, cx);
        }
    }

    /// Whether the words typed on the verify step match the requested backup positions. Pure (no
    /// side effects) so the "Confirm & finish" button can read it to stay disabled until correct,
    /// and `confirm_backup` can gate on the same check. Case-insensitive, whitespace-tolerant.
    pub fn backup_words_match(&self, cx: &Context<Self>) -> bool {
        let Some(phrase) = self.pending_phrase.as_ref() else {
            return false;
        };
        let words: Vec<&str> = phrase.split_whitespace().collect();
        let expected: Vec<String> = self
            .confirm_positions
            .iter()
            .map(|&i| words.get(i).copied().unwrap_or("").to_lowercase())
            .collect();
        // A missing/empty expected word means the positions are out of sync with the phrase — never
        // treat that as a match (fail closed).
        if expected.is_empty() || expected.iter().any(|w| w.is_empty()) {
            return false;
        }
        let entered: Vec<String> = self
            .confirm_words
            .read(cx)
            .value()
            .split_whitespace()
            .map(|s| s.trim().to_lowercase())
            .collect();
        entered == expected
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
        self.portfolio_loading = false;
        self.portfolio_error = None;
        self.read_status = None;
        self.synced_block = None;
        // Dropping the task cancels the balance auto-refresh loop.
        self.poll_task = None;
        // Dropping the handle closes its channel → the sync worker thread exits.
        self.shielded = None;
        self.railgun_address = None;
        self.recipient_autofilled = false;
        self.shield_status = None;
        // Invalidate any in-flight grant fetch and clear shield inputs on the next render.
        self.auth_epoch = self.auth_epoch.wrapping_add(1);
        self.pending_shield_clear = true;
        self.shield.reset();
        self.send.reset();
        // Clear the swap flow + its compose-only state (tokens / quote / uid) so a prior wallet's
        // priced order can't linger into the next unlock.
        self.swap.reset();
        self.swap_sell_token = None;
        self.swap_buy_token = None;
        self.swap_quote = None;
        self.swap_quoting = false;
        self.swap_uid = None;
        self.swap_quote_epoch = self.swap_quote_epoch.wrapping_add(1);
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
        // Both passphrase copies stay in `Zeroizing` so neither the entry nor the confirm field's
        // plaintext lingers in freed heap after this call (the discipline the rest of the flow keeps).
        let p1 = Zeroizing::new(self.create_pass.read(cx).value().to_string());
        let p2 = Zeroizing::new(self.create_pass2.read(cx).value().to_string());
        if p1.chars().count() < 8 {
            self.auth_error = Some("Passphrase must be at least 8 characters".into());
            cx.notify();
            return;
        }
        if *p1 != *p2 {
            self.auth_error = Some("Passphrases don't match".into());
            cx.notify();
            return;
        }
        self.auth_error = None;
        self.auth_busy = true;
        cx.notify();
        // Tag this KDF so a result that lands after the user backed out (which bumps the epoch via
        // `abandon_create`) is dropped instead of staging orphaned secrets in memory.
        let epoch = self.create_epoch;
        let pass = p1;
        let task = cx.background_spawn(async move {
            let made = Vault::create(&pass, WordCount::Twelve, KdfParams::PRODUCTION);
            made.map(|(v, phrase)| (v, phrase, pass))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Abandoned mid-KDF: drop `res` (its `Zeroizing` secrets zeroize) and leave whatever
                // the user is doing now untouched — don't even clear `auth_busy`, which a newer
                // operation may legitimately own.
                if this.create_epoch != epoch {
                    return;
                }
                this.auth_busy = false;
                match res {
                    Ok((vault, phrase, pass)) => {
                        let wc = phrase.split_whitespace().count();
                        match deckard_core::random_word_positions(wc, 3) {
                            Ok(positions) => {
                                this.confirm_positions = positions;
                                this.pending_vault = Some(vault);
                                this.pending_phrase = Some(phrase);
                                this.pending_pass = Some(pass);
                                this.reveal_seed = false;
                                this.auth = AuthStep::CreateBackup;
                            }
                            Err(e) => this.auth_error = Some(short_err(e)),
                        }
                    }
                    Err(e) => this.auth_error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// CreateVerify → check the quizzed words, then write + unlock the vault and land on the
    /// "you're ready" interstitial.
    pub fn confirm_backup(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let (Some(vault), Some(pass)) = (self.pending_vault.clone(), self.pending_pass.clone())
        else {
            return;
        };
        // Same check the "Confirm & finish" button gates on — keep them on one predicate so the
        // disabled state and the submit can never disagree. (It also re-checks `pending_phrase`,
        // so a missing phrase fails closed here too.)
        if !self.backup_words_match(cx) {
            self.auth_error = Some("Those words don't match your backup. Try again.".into());
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
                        // The phrase is now backed up + verified — drop every pending secret. The
                        // vault is sealed and the daemon is unlocked; we hold only the address.
                        this.pending_phrase = None;
                        this.pending_pass = None;
                        this.pending_vault = None;
                        this.reveal_seed = false;
                        this.seed_copied = false;
                        // Land on the "you're ready" interstitial rather than dropping straight into
                        // the app. The address is shown on that screen and then handed to
                        // `finish_unlock` by the "Enter Deckard" click; the live app's portfolio /
                        // pollers don't start until then (they fence on `auth == Ready`).
                        this.wallet_address = Some(addr);
                        this.auth = AuthStep::CreateDone;
                        cx.notify();
                    }
                    Err(msg) => {
                        // Deliberately KEEP the pending secrets on a write/unlock failure: the user
                        // stays on Verify and the retry needs the same vault + phrase + passphrase
                        // (regenerating would hand them a *different* phrase they never backed up).
                        // The phrase already has to live in memory throughout backup/verify; an
                        // error doesn't widen that, and `Zeroizing` wipes it when the app exits.
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
        self.start_balance_poll(cx);
    }

    /// Fetch the daemon's live policy for the wallet home's "What Atlas may do" fence (off
    /// the UI thread). Key-less: `PolicyGet` is a read of the fence the daemon enforces — the
    /// daemon answers it even while locked, so this works from the unlock gate too. On any
    /// failure the card keeps its previous snapshot (or honestly shows none); never fabricates.
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
        let rpc = self.settings.effective_rpc(chain_id);
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

    /// The chain the daemon signs for (resolved once at startup). The swap surface reads it to
    /// pick the curated token list, the orderbook base, and the per-chain swatch.
    pub fn chain_id(&self) -> u64 {
        self.chain_id
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
        // The verify field holds real seed words (3 of the 12) the user typed to prove their
        // backup, and `InputState` is not `Zeroizing`. Once we've left the verify step — on success
        // to CreateDone, or Back to CreateBackup — wipe it so those words don't linger in memory for
        // the rest of the session. (On the verify step itself the `target` clear below handles it.)
        if self.auth != AuthStep::CreateVerify {
            self.confirm_words
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let target = match self.auth {
            AuthStep::CreateSetup => Some(&self.create_pass),
            // The backup screen is reveal-only (no text input); the verify step takes the words.
            AuthStep::CreateVerify => Some(&self.confirm_words),
            AuthStep::Import => Some(&self.import_secret),
            AuthStep::Migrate | AuthStep::Unlock => Some(&self.pass_input),
            AuthStep::Choose | AuthStep::CreateBackup | AuthStep::CreateDone | AuthStep::Ready => {
                None
            }
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
    fn kick_portfolio(
        eth: &EthProvider,
        addr: Address,
        auth_epoch: u64,
        view_epoch: u64,
        cx: &mut Context<Self>,
    ) {
        let rx = eth.portfolio(addr);
        cx.spawn(async move |this, cx| {
            let res = rx.recv_async().await;
            this.update(cx, |this, cx| {
                // Fence: reject a reply from a superseded session/view (lock, re-unlock, retarget).
                if this.auth != AuthStep::Ready
                    || this.auth_epoch != auth_epoch
                    || this.view_epoch != view_epoch
                    || addr != this.display_address
                {
                    return;
                }
                this.portfolio_loading = false;
                match res {
                    Ok(Ok(read)) => {
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

    fn kick_public_balance_refresh(&mut self, cx: &mut Context<Self>) -> bool {
        if self.auth != AuthStep::Ready || self.portfolio_loading {
            return false;
        }
        let addr = self.display_address;
        let auth_epoch = self.auth_epoch;
        let view_epoch = self.view_epoch;
        self.portfolio_loading = true;
        self.portfolio_error = None;
        Self::kick_portfolio(&self.eth, addr, auth_epoch, view_epoch, cx);
        Self::kick_block_number(&self.eth, auth_epoch, view_epoch, cx);
        true
    }

    /// Refresh the latest block height for the status line.
    fn kick_block_number(
        eth: &EthProvider,
        auth_epoch: u64,
        view_epoch: u64,
        cx: &mut Context<Self>,
    ) {
        let rx = eth.block_number();
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(read)) = rx.recv_async().await {
                this.update(cx, |this, cx| {
                    if this.auth != AuthStep::Ready
                        || this.auth_epoch != auth_epoch
                        || this.view_epoch != view_epoch
                    {
                        return;
                    }
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
        self.kick_public_balance_refresh(cx);
        // An MCP/CLI agent shields through the daemon WITHOUT this app in the loop, so a
        // manual refresh must re-scan the shielded balance too — otherwise an agent-path
        // deposit stays invisible until the next unlock.
        if let Some(h) = &self.shielded {
            h.resync();
            self.watch_shielded_sync(false, cx);
        }
        cx.notify();
    }

    /// Auto-refresh the PUBLIC balance while the wallet home is open, so funds that arrive
    /// out-of-band (a faucet top-up, an incoming transfer) appear without a manual refresh.
    /// Deliberately lightweight: re-reads ONLY the public balance + block height — the heavier
    /// shielded resync stays on the explicit refresh (header button / ⌘K command). Stored in
    /// `poll_task`; dropping it (on lock / re-unlock) cancels the loop. Epoch-fenced as
    /// belt-and-suspenders against a stale tick after a fast re-unlock.
    fn start_balance_poll(&mut self, cx: &mut Context<Self>) {
        let epoch = self.auth_epoch;
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(BALANCE_POLL_SECS))
                    .await;
                let keep = this.update(cx, |this, cx| {
                    // End the loop once this unlocked session is over (lock / re-unlock bumps the epoch).
                    if this.auth != AuthStep::Ready || this.auth_epoch != epoch {
                        return false;
                    }
                    // Only while the wallet home is showing, and never stacked on the first load.
                    if matches!(this.surface, Surface::Home) && this.selection == Selection::Wallet
                    {
                        this.kick_public_balance_refresh(cx);
                    }
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        }));
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
            self.portfolio_loading = false;
            self.refresh_portfolio(cx);
        } else if let Ok(addr) = target.parse::<Address>() {
            self.display_address = addr;
            self.viewing_watch = true;
            self.portfolio = None;
            self.portfolio_loading = false;
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
                            this.portfolio_loading = false;
                            this.refresh_portfolio(cx);
                        }
                        Ok(Err(e)) => {
                            this.portfolio_loading = false;
                            this.portfolio_error =
                                Some(format!("couldn't resolve name: {}", short_err(e)));
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
        let url = self
            .settings
            .effective_rpc(self.settings.effective_chain_id());
        if url == self.current_rpc {
            return;
        }
        self.current_rpc = url.clone();
        self.eth = EthProvider::spawn(url, self.settings.effective_chain_id());
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
        // The wallet home now carries the "What Atlas may do" policy fence — re-fetch the
        // daemon's live policy on every visit so an out-of-band edit to policy.json (or a
        // STOP) shows up without a relaunch.
        if matches!(sel, Selection::Wallet | Selection::Agent) {
            self.kick_agent_policy(cx);
        }
        cx.notify();
    }

    /// Open a full-pane surface (Home / Receive / Settings) over the current selection.
    pub fn open(&mut self, surface: Surface, cx: &mut Context<Self>) {
        // Leaving Shield (back, palette, a nav click) cancels any in-progress hold so its
        // timer can't fire a confirm after the screen is gone.
        if surface != Surface::Shield && self.shield.holding {
            self.shield.cancel_hold();
        }
        // Same for the send hold: leaving the Send surface must cancel an in-progress hold so
        // its timer can't fire a confirm after the screen is gone.
        if surface != Surface::Send && self.send.holding {
            self.send.cancel_hold();
        }
        // And the swap hold: leaving Swap cancels an in-progress hold so its timer can't fire a
        // confirm after the screen is gone.
        if surface != Surface::Swap && self.swap.holding {
            self.swap.cancel_hold();
        }
        // Drop a clear-signing review left open on the surface we're leaving. Its card only renders
        // on its OWN surface, so a stale `activity_reviewing` set on a now-hidden surface could
        // otherwise be resolved blind from the palette while a different surface is shown — the
        // second/blind approval path codex flagged. Clearing on every surface change closes it.
        if surface != Surface::Activity {
            self.activity_reviewing = None;
        }
        self.surface = surface;
        // The confirm arm-delay fails closed: clear it on every nav, then re-arm if we are landing
        // on a clear-signing review that already has a proposal (a re-entered review). A
        // stale-but-elapsed timestamp can never fire a confirm; ⌘↵/click stays gated by a fresh
        // delay every time the review appears.
        self.commit_review_at = None;
        let on_review = match surface {
            Surface::Send => self.send.proposal.is_some(),
            Surface::Shield => self.shield.proposal.is_some(),
            Surface::Swap => self.swap.proposal.is_some(),
            _ => false,
        };
        if on_review {
            self.arm_commit(cx);
        }
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

    // --- shield trigger flow (T5) ---

    /// Open the shield flow with a clean slate (clears any prior proposal/error/result; the
    /// typed amount/recipient are left intact). No-op while viewing a watched read-only
    /// account — a shield signs from YOUR wallet, so it must not be initiated from a
    /// someone-else's-address context.
    pub fn open_shield(&mut self, cx: &mut Context<Self>) {
        if self.viewing_watch {
            return;
        }
        self.shield.reset();
        self.open(Surface::Shield, cx);
    }

    /// Build + `propose` the shield off-thread. On `Allow`/`NeedsApproval`, stash the proposal so
    /// the review card + hold-to-confirm appear; on a parse/`Deny` error, surface a clear line.
    /// The recipient is validated SYNCHRONOUSLY (a non-empty 0zk string — no ENS resolution,
    /// unlike Send); the review TAIL is shared via [`Shell::finish_review`].
    pub fn review_shield(&mut self, cx: &mut Context<Self>) {
        if self.shield.busy {
            return;
        }
        let amount = self.shield.amount.read(cx).value().to_string();
        let recipient = self.shield.recipient.read(cx).value().to_string();
        let value_wei = match signer::parse_eth_to_wei(&amount) {
            Ok(w) if w > U256::ZERO => w,
            Ok(_) => {
                self.shield.error = Some("Enter an amount greater than zero".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.shield.error = Some(e);
                cx.notify();
                return;
            }
        };
        if recipient.trim().is_empty() {
            self.shield.error = Some("Enter a 0zk recipient address".into());
            cx.notify();
            return;
        }
        self.shield.error = None;
        self.shield.proposal = None;
        // begin_review bumps the epoch (each review supersedes the last) and sets `busy`; a slow
        // reply for a since-cancelled/re-issued review checks this epoch before installing.
        let epoch = self.shield.begin_review();
        let recipient_snapshot = recipient.clone();
        cx.notify();
        let client = self.signer.client();
        let chain_id = self.chain_id;
        let task = cx.background_spawn(async move {
            let intent = signer::build_shield_intent(chain_id, &recipient, value_wei)?;
            // The user's foreground shield from the app → App-origin (the wire requires origin).
            let decision =
                client.propose_blocking(&intent, deckard_contract::ProposalOrigin::App)?;
            // The recipient SNAPSHOT inside the signed intent: the 0zk string the user reviewed —
            // never a value they could have since edited in the input.
            Ok::<(Intent, String, Decision), anyhow::Error>((intent, recipient_snapshot, decision))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.finish_review(|s| &mut s.shield, epoch, res, "Can't shield: ", cx);
            })
            .ok();
        })
        .detach();
    }

    /// Sign + broadcast the reviewed shield off-thread (the hold-to-confirm completed). For a
    /// `NeedsApproval` proposal the completed hold IS the approval (resolve, then execute); an
    /// `Allow` goes straight to execute. On success the deposit is broadcast and on its way to a
    /// private note — set `Sending`, re-sync the private balance, and start the settle watcher.
    /// Mirrors `confirm_send` (plus the private-side broadcast wiring a send doesn't have).
    pub fn confirm_shield(&mut self, cx: &mut Context<Self>) {
        let Some(ShieldProposal {
            request_id,
            needs_resolve,
            ..
        }) = self.shield.proposal.clone()
        else {
            return;
        };
        if self.shield.busy {
            return;
        }
        self.shield.busy = true;
        self.shield.error = None;
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
                this.shield.busy = false;
                // Invalidate the proposal on EVERY execute attempt: a second hold must not be
                // able to re-broadcast. On an ambiguous timeout the deposit may already be in
                // flight, so retrying requires a fresh, deliberate review (new request id).
                this.shield.proposal = None;
                match res {
                    Ok(ExecuteResult::Broadcast { tx_hash }) => {
                        this.shield.tx = Some(tx_hash);
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
                            this.shield.error =
                                Some(format!("Shield denied: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.shield.error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Begin a confirm hold: start the amber fill-sweep and a timer that fires `confirm_shield`
    /// only if the hold survives [`SHIELD_HOLD`]. A per-hold epoch guards against a stale timer
    /// firing after an early release / re-press.
    pub fn shield_hold_start(&mut self, cx: &mut Context<Self>) {
        // The key-cap confirm trigger (a deliberate button click or ⌘↵, never a hold). Confirm
        // only while still on Shield AND once the review has ARMED, so a carried-over keypress
        // can't approve (DESIGN §confirm pattern).
        if self.surface == Surface::Shield && self.commit_armed() {
            self.confirm_shield(cx);
        }
    }

    /// Open the native-send flow from the wallet home / palette. Refused while viewing a
    /// watched read-only account — a send signs from YOUR wallet, so it has no meaning there
    /// (the same guard `open_shield` uses).
    pub fn open_send(&mut self, cx: &mut Context<Self>) {
        if self.viewing_watch {
            return;
        }
        self.send.reset();
        self.open(Surface::Send, cx);
    }

    /// The shared review TAIL: fold a `propose` reply into a [`CommitFlow`] on the UI thread.
    /// Every commit surface runs an identical post-`propose` sequence — re-acquire the flow,
    /// drop a stale (superseded) reply, clear `busy`, then install the proposal on
    /// `Allow`/`NeedsApproval`, bounce on a session-ended `Deny`, surface a humanized deny line
    /// otherwise, or a short error. Factored out of the per-surface `review_*` so Send (now) and
    /// Shield (Step 2) share one copy.
    ///
    /// `flow` re-acquires the surface's flow (it's only ever called *inside* `update`, never held
    /// across an await, so a `fn(&mut Shell) -> &mut CommitFlow` is sound). The
    /// `propose_result` carries the intent, the **recipient snapshot** (built in the prelude — the
    /// checksummed `to` for Send, the input string for Shield), and the daemon `Decision`.
    /// `review_deny_prefix` is the surface's leading copy on an inline deny ("Can't send: ").
    fn finish_review(
        &mut self,
        flow: fn(&mut Shell) -> &mut CommitFlow,
        epoch: u64,
        propose_result: Result<(Intent, String, Decision), anyhow::Error>,
        review_deny_prefix: &str,
        cx: &mut Context<Self>,
    ) {
        // Guard FIRST: a stale review must not even clear `busy` (a newer review may own it now).
        if !flow(self).review_is_current(epoch) {
            return;
        }
        flow(self).busy = false;
        match propose_result {
            Ok((intent, recipient, Decision::Allow)) => {
                let request_id = SignerClient::request_id_for_intent(&intent);
                flow(self).proposal = Some(crate::commit_flow::Proposal {
                    intent,
                    request_id,
                    recipient,
                    needs_resolve: false,
                });
            }
            // NeedsApproval (over-cap, or the daemon's auto-approval guardrail): the review card +
            // hold-to-confirm ARE the human approval surface — the hold resolves the pending
            // record, then executes.
            Ok((intent, recipient, Decision::NeedsApproval { request_id })) => {
                flow(self).proposal = Some(crate::commit_flow::Proposal {
                    intent,
                    request_id,
                    recipient,
                    needs_resolve: true,
                });
            }
            Ok((_, _, Decision::Deny { reason })) => {
                // An external STOP/lock ends the session — bounce to the unlock gate.
                if is_session_ended(&reason) {
                    self.handle_session_revoked(cx);
                } else {
                    flow(self).error =
                        Some(format!("{review_deny_prefix}{}", humanize_deny(&reason)));
                }
            }
            Err(e) => flow(self).error = Some(short_err(e)),
        }
        // Arm the confirm once the review lands: ⌘↵/click can confirm only after the arm-delay,
        // so a keypress carried over from the compose screen can't approve (DESIGN §confirm).
        if flow(self).proposal.is_some() {
            self.arm_commit(cx);
        }
        cx.notify();
    }

    /// Resolve the recipient, then build + `propose` the send off-thread. A `0x…` recipient is
    /// parsed directly (works offline); anything else is treated as an ENS name and
    /// forward-resolved over the same path the watch-address field uses
    /// (`EthProvider::resolve_name`). On `Allow`/`NeedsApproval`, stash the proposal so the
    /// review card + hold-to-confirm appear; on a parse/resolve/`Deny` error, surface a clear
    /// line. Mirrors `review_shield` (epoch-guarded; the guard is checked before `busy`).
    pub fn review_send(&mut self, cx: &mut Context<Self>) {
        if self.send.busy {
            return;
        }
        let amount = self.send.amount.read(cx).value().to_string();
        let recipient = self.send.recipient.read(cx).value().to_string();
        let value_wei = match signer::parse_eth_to_wei(&amount) {
            Ok(w) if w > U256::ZERO => w,
            Ok(_) => {
                self.send.error = Some("Enter an amount greater than zero".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.send.error = Some(e);
                cx.notify();
                return;
            }
        };
        let recipient = recipient.trim().to_string();
        if recipient.is_empty() {
            self.send.error = Some("Enter a recipient address or ENS name".into());
            cx.notify();
            return;
        }
        self.send.error = None;
        self.send.proposal = None;
        // begin_review bumps the epoch (each review supersedes the last) and sets `busy`; a slow
        // reply for a since-cancelled/re-issued review checks this epoch before installing.
        let epoch = self.send.begin_review();
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
                    .map_err(|e| anyhow::anyhow!("couldn't resolve name: {}", short_err(e)))?,
            };
            let intent = signer::build_native_send_intent(chain_id, to, value_wei);
            // The user's foreground send from the app → App-origin (the wire requires origin).
            let decision =
                client.propose_blocking(&intent, deckard_contract::ProposalOrigin::App)?;
            // The recipient SNAPSHOT inside the signed intent: the checksummed destination — never
            // the raw `0x…`/ENS text the user could have since edited.
            Ok::<(Intent, String, Decision), anyhow::Error>((
                intent,
                to.to_checksum(None),
                decision,
            ))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.finish_review(|s| &mut s.send, epoch, res, "Can't send: ", cx);
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
        }) = self.send.proposal.clone()
        else {
            return;
        };
        if self.send.busy {
            return;
        }
        self.send.busy = true;
        self.send.error = None;
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
                this.send.busy = false;
                // Invalidate the proposal on EVERY execute attempt: a second hold must not be
                // able to re-broadcast. On an ambiguous timeout the transfer may already be in
                // flight, so retrying requires a fresh, deliberate review (new request id).
                this.send.proposal = None;
                match res {
                    Ok(ExecuteResult::Broadcast { tx_hash }) => {
                        this.send.tx = Some(tx_hash);
                        // The transfer left the wallet — re-fetch the public balance so home
                        // reflects it (a send has no private side to sync, unlike shield).
                        this.refresh_portfolio(cx);
                    }
                    Ok(ExecuteResult::Denied { reason }) => {
                        // An external STOP/lock ends the session — bounce to the unlock gate.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.send.error =
                                Some(format!("Send denied: {}", humanize_deny(&reason)));
                        }
                    }
                    Err(e) => this.send.error = Some(short_err(e)),
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
        // The key-cap confirm trigger (a deliberate button click or ⌘↵, never a hold). Confirm
        // only while still on Send AND once the review has ARMED (the short arm-delay) so a
        // keypress carried over from the previous screen can't approve (DESIGN §confirm pattern).
        if self.surface == Surface::Send && self.commit_armed() {
            self.confirm_send(cx);
        }
    }

    // --- CoW swap flow (#25) ---

    /// Drop any priced quote (and the review proposal it backs) and fence a slow in-flight quote
    /// reply via the epoch bump, so a stale price can never be signed against changed compose
    /// inputs (codex must-do #4). Called on every sell-amount edit (the input subscription) and on
    /// every sell/buy token change. Does NOT `notify` — the caller decides when to re-render.
    fn invalidate_swap_quote(&mut self) {
        self.swap_quote = None;
        // A live review proposal was built from the now-cleared quote; drop it too so the user
        // can't confirm a card whose figures no longer have a backing quote.
        self.swap.proposal = None;
        self.swap.error = None;
        self.swap_quote_epoch = self.swap_quote_epoch.wrapping_add(1);
    }

    /// Open the swap flow from the wallet home / palette. Refused while viewing a watched
    /// read-only account (a swap signs from YOUR wallet) and refused on a chain with no curated
    /// token list (a plain anvil fork, chain 31337 — `tokens_for` is empty there, so there'd be
    /// nothing to pick); both surface a clear line rather than opening an unusable screen. Seeds
    /// the sell/buy tokens to the first two distinct tokens so the pickers are never empty.
    pub fn open_swap(&mut self, cx: &mut Context<Self>) {
        if self.viewing_watch {
            return;
        }
        let tokens = tokens_for(self.chain_id);
        if tokens.is_empty() {
            // Open the surface anyway so the refusal is visible (not a silently-inert button), but
            // with no quote/pickers possible — an honest "wrong network" line.
            self.swap.reset();
            self.swap_quote = None;
            self.swap_uid = None;
            self.swap.error = Some(
                "Swap needs a supported network (Sepolia or mainnet). Switch chains first.".into(),
            );
            self.open(Surface::Swap, cx);
            return;
        }
        // Fresh slate: clear the flow, the last quote, and any prior done-screen uid.
        self.swap.reset();
        self.swap_quote = None;
        self.swap_uid = None;
        self.swap_quote_epoch = self.swap_quote_epoch.wrapping_add(1);
        // Seed the pickers to the first two distinct tokens (only if not already chosen this
        // session) so compose has a valid default pair on first paint.
        if self.swap_sell_token.is_none() {
            self.swap_sell_token = tokens.first().map(|t| t.address);
        }
        if self.swap_buy_token.is_none() {
            self.swap_buy_token = tokens
                .iter()
                .map(|t| t.address)
                .find(|&a| Some(a) != self.swap_sell_token);
        }
        self.open(Surface::Swap, cx);
    }

    /// Choose the sell-side token. A different token invalidates the quote (it was priced for the
    /// old pair) and never lets the sell == buy degenerate case stand (it clears the buy side if
    /// they'd collide).
    pub fn set_swap_sell_token(&mut self, token: Address, cx: &mut Context<Self>) {
        if self.swap_sell_token == Some(token) {
            return;
        }
        self.swap_sell_token = Some(token);
        if self.swap_buy_token == Some(token) {
            self.swap_buy_token = None;
        }
        self.invalidate_swap_quote();
        cx.notify();
    }

    /// Choose the buy-side token (same staleness + collision rules as the sell side).
    pub fn set_swap_buy_token(&mut self, token: Address, cx: &mut Context<Self>) {
        if self.swap_buy_token == Some(token) {
            return;
        }
        self.swap_buy_token = Some(token);
        if self.swap_sell_token == Some(token) {
            self.swap_sell_token = None;
        }
        self.invalidate_swap_quote();
        cx.notify();
    }

    /// Fetch an indicative quote for the current compose inputs, off-thread. Epoch-fenced: a slow
    /// reply for a since-edited compose (different amount or pair) lands as a no-op. The quote is
    /// indicative only — `confirm_swap` re-quotes at confirm time for the binding figures.
    pub fn get_swap_quote(&mut self, cx: &mut Context<Self>) {
        if self.swap_quoting {
            return;
        }
        let amount = self.swap.amount.read(cx).value().to_string();
        let sell_wei = match signer::parse_eth_to_wei(&amount) {
            Ok(w) if w > U256::ZERO => w,
            Ok(_) => {
                self.swap.error = Some("Enter an amount greater than zero".into());
                cx.notify();
                return;
            }
            Err(e) => {
                self.swap.error = Some(e);
                cx.notify();
                return;
            }
        };
        let (Some(sell_token), Some(buy_token)) = (self.swap_sell_token, self.swap_buy_token)
        else {
            self.swap.error = Some("Pick a token to sell and a token to receive".into());
            cx.notify();
            return;
        };
        if sell_token == buy_token {
            self.swap.error = Some("Pick two different tokens".into());
            cx.notify();
            return;
        }
        let Some(base) = crate::swap::orderbook_base(self.chain_id) else {
            self.swap.error = Some("Swap needs a supported network (Sepolia or mainnet)".into());
            cx.notify();
            return;
        };
        let wallet = self.wallet_address.unwrap_or(Address::ZERO);

        self.swap.error = None;
        self.swap_quoting = true;
        // Fence this request: a reply for a since-changed compose is dropped on arrival.
        self.swap_quote_epoch = self.swap_quote_epoch.wrapping_add(1);
        let epoch = self.swap_quote_epoch;
        cx.notify();

        let req = crate::swap::quote_request(sell_token, buy_token, wallet, sell_wei);
        let task = cx.background_spawn(async move {
            // CoW HTTP (reqwest/hickory DNS) needs a tokio reactor, which GPUI's executor lacks —
            // so the quote goes through deckard-core's blocking wrapper (it owns the runtime). The
            // blocking call runs on this spawned background task, never the UI thread.
            let ob = CowOrderbook::new();
            ob.quote_blocking(base, &req)
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Drop a reply for a since-superseded quote request (a later edit / token change).
                if this.swap_quote_epoch != epoch {
                    return;
                }
                this.swap_quoting = false;
                match res {
                    Ok(quote) => {
                        this.swap_quote = Some(quote);
                        this.swap.error = None;
                    }
                    Err(e) => {
                        this.swap_quote = None;
                        this.swap.error = Some(crate::swap::humanize_quote_error(&e));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Build the bound order from the current quote, `propose_order` it off-thread, and on
    /// `NeedsApproval` install the review proposal so the clear-signing card + hold-to-confirm
    /// appear. A swap is ALWAYS `NeedsApproval` in v1 (the completed hold IS the approval); a
    /// `Deny` surfaces a swap-worded line. Validates a fresh quote + distinct tokens up front.
    pub fn review_swap(&mut self, cx: &mut Context<Self>) {
        if self.swap.busy {
            return;
        }
        let Some(quote) = self.swap_quote.clone() else {
            self.swap.error = Some("Get a quote first, then review the order".into());
            cx.notify();
            return;
        };
        let (Some(sell_token), Some(buy_token)) = (self.swap_sell_token, self.swap_buy_token)
        else {
            self.swap.error = Some("Pick a token to sell and a token to receive".into());
            cx.notify();
            return;
        };
        if sell_token == buy_token {
            self.swap.error = Some("Pick two different tokens".into());
            cx.notify();
            return;
        }
        let Some(wallet) = self.wallet_address else {
            self.swap.error = Some("Unlock your wallet first".into());
            cx.notify();
            return;
        };
        let chain_id = self.chain_id;
        let order = crate::swap::order_from_quote(&quote, chain_id, wallet);
        let bound = signer::bind_swap_order(&order, wallet);
        let request_id = SignerClient::request_id_for_swap_order(&bound);
        // A display-only "X SELL → at least Y BUY" summary the review card shows above the rows.
        let summary = self.swap_summary_line(&quote, chain_id);

        self.swap.error = None;
        self.swap.proposal = None;
        // begin_review bumps the review epoch + sets busy; a stale reply checks it before installing.
        let epoch = self.swap.begin_review();
        cx.notify();
        let client = self.signer.client();
        let order_for_task = order.clone();
        // App-origin: the user's foreground GUI swap → the feed labels the order "You", not "Atlas".
        let task = cx.background_spawn(async move {
            client.propose_order_blocking(&order_for_task, deckard_contract::ProposalOrigin::App)
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                // Guard FIRST: a stale review must not clear `busy` a newer review may own.
                if !this.swap.review_is_current(epoch) {
                    return;
                }
                this.swap.busy = false;
                match res {
                    // A valid swap is always NeedsApproval — the hold IS the approval. An Allow is
                    // unexpected (v1 swaps never auto-allow), but install it the same way (confirm
                    // re-derives + resolves regardless); the daemon stays the gate.
                    Ok(Decision::NeedsApproval { .. }) | Ok(Decision::Allow) => {
                        this.swap.proposal = Some(crate::commit_flow::Proposal {
                            // The swap path never reads `intent` (the orchestrator works off a
                            // fresh SwapInputs snapshot + re-quote); carry a synthetic placeholder
                            // so the shared `Proposal` shape is satisfied.
                            intent: signer::build_exact_approve_intent(
                                chain_id,
                                sell_token,
                                order.sell_amount,
                            ),
                            request_id,
                            recipient: summary,
                            needs_resolve: true,
                        });
                        // Arm the swap confirm once the priced review lands (DESIGN §confirm).
                        this.arm_commit(cx);
                    }
                    Ok(Decision::Deny { reason }) => {
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.swap.error =
                                Some(format!("Can't swap: {}", humanize_swap_deny(&reason)));
                        }
                    }
                    Err(e) => this.swap.error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A display-only "0.05 WETH → at least 92.1 COW" summary for the review card header, scaled
    /// by each side's curated decimals. Indicative (mirrors the quote summary); the binding figures
    /// are the rows the card renders from the same quote.
    fn swap_summary_line(&self, quote: &QuoteResponse, chain_id: u64) -> String {
        let sell_tok = quote.quote.sell_token;
        let buy_tok = quote.quote.buy_token;
        let sell_sym = crate::swap::token_symbol(chain_id, sell_tok);
        let buy_sym = crate::swap::token_symbol(chain_id, buy_tok);
        let sell_dec = crate::swap::token_decimals(chain_id, sell_tok);
        let buy_dec = crate::swap::token_decimals(chain_id, buy_tok);
        let gross_sell = quote
            .quote
            .sell_amount
            .saturating_add(quote.quote.fee_amount);
        let min_recv = deckard_core::apply_slippage(
            quote.quote.buy_amount,
            deckard_core::DEFAULT_SLIPPAGE_BPS,
        );
        format!(
            "{} {} → at least {} {}",
            deckard_core::format_amount(gross_sell, sell_dec, 6),
            sell_sym,
            deckard_core::format_amount(min_recv, buy_dec, 6),
            buy_sym,
        )
    }

    /// Confirm a reviewed swap (the hold-to-confirm completed): run the off-thread orchestrator
    /// (re-quote → propose-order → exact-gross approve if short → resolve+sign over the control
    /// channel → submit) and, on success, surface the order uid (the done screen). Mixes async I/O
    /// with the daemon's `*_blocking` calls, so it runs inside `cx.background_spawn` (the blocking
    /// calls block the spawned task, never the UI). Invalidates the proposal on every attempt — a
    /// second hold must not re-submit (codex must-do, mirrors confirm_send/confirm_shield).
    pub fn confirm_swap(&mut self, cx: &mut Context<Self>) {
        if self.swap.proposal.is_none() || self.swap.busy {
            return;
        }
        // Snapshot every value the orchestrator needs (codex must-do #5 — Shell isn't Send).
        let (Some(quote), Some(sell_token), Some(buy_token), Some(wallet)) = (
            self.swap_quote.as_ref(),
            self.swap_sell_token,
            self.swap_buy_token,
            self.wallet_address,
        ) else {
            self.swap.error = Some("Review the swap again. The order details are missing.".into());
            cx.notify();
            return;
        };
        let Some(base) = crate::swap::orderbook_base(self.chain_id) else {
            self.swap.error = Some("Swap needs a supported network (Sepolia or mainnet)".into());
            cx.notify();
            return;
        };
        let chain_id = self.chain_id;
        // The gross sell amount the relayer must be allowed to pull. The orchestrator re-quotes at
        // confirm time and re-derives its own gross, but THIS gross is the sell-in-atoms it
        // re-quotes against (`sellAmountBeforeFee`), so it must be the compose quote's gross.
        let sell_wei = quote
            .quote
            .sell_amount
            .saturating_add(quote.quote.fee_amount);

        self.swap.busy = true;
        self.swap.error = None;
        // Invalidate the proposal on EVERY confirm attempt (a second hold can't re-submit).
        self.swap.proposal = None;
        cx.notify();

        let client = self.signer.client();
        let control = self.signer.control();
        let eth = self.eth.clone();
        let inputs = crate::swap::SwapInputs {
            chain_id,
            wallet,
            sell_token,
            buy_token,
            sell_wei,
        };
        // The orchestrator is fully blocking: the CoW HTTP goes through deckard-core's `*_blocking`
        // wrappers (which own a tokio runtime — the GPUI app never touches tokio), and the signer
        // calls are already `*_blocking`. Run it on a background task so it blocks that task, never
        // the UI thread.
        let task = cx.background_spawn(async move {
            let ob = CowOrderbook::new();
            crate::swap::confirm_swap_blocking(&ob, &eth, &client, &control, base, inputs)
        });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            this.update(cx, |this, cx| {
                this.swap.busy = false;
                match outcome {
                    Ok(crate::swap::SwapConfirmOutcome::Submitted { uid }) => {
                        this.swap_uid = Some(uid);
                        // The order is on the orderbook (not yet on-chain) — no public balance
                        // change to refetch until a solver fills it.
                    }
                    Ok(crate::swap::SwapConfirmOutcome::Denied { reason }) => {
                        // A session-ended deny bounces to the unlock gate; the orchestrator returns
                        // the raw tag for those so we can detect it here.
                        if is_session_ended(&reason) {
                            this.handle_session_revoked(cx);
                        } else {
                            this.swap.error =
                                Some(format!("Can't swap: {}", humanize_swap_deny(&reason)));
                        }
                    }
                    Err(e) => this.swap.error = Some(short_err(e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Begin a confirm hold on the swap review: start the amber fill-sweep + a timer that fires
    /// `confirm_swap` only if the hold survives [`SHIELD_HOLD`] and the user is still on Swap.
    pub fn swap_hold_start(&mut self, cx: &mut Context<Self>) {
        // The key-cap confirm trigger (a deliberate button click or ⌘↵, never a hold). Confirm
        // only while still on Swap AND once the review has ARMED (DESIGN §confirm pattern).
        if self.surface == Surface::Swap && self.commit_armed() {
            self.confirm_swap(cx);
        }
    }

    /// Start the confirm arm-delay: stamp now AND schedule a single re-render at the arm boundary,
    /// so the key-cap button visibly flips from "arming" (dimmed) to active. GPUI only re-renders
    /// on notify, so without this wake the gate would flip silently and an early click would
    /// no-op with no feedback.
    fn arm_commit(&mut self, cx: &mut Context<Self>) {
        self.commit_review_at = Some(std::time::Instant::now());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COMMIT_ARM_DELAY).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }

    /// True once a clear-signing review has been on screen at least [`COMMIT_ARM_DELAY`] — the
    /// confirm arm gate so a carried-over keypress/click can't approve (DESIGN §confirm pattern).
    pub(crate) fn commit_armed(&self) -> bool {
        self.commit_review_at
            .map(|t| t.elapsed() >= COMMIT_ARM_DELAY)
            .unwrap_or(false)
    }

    /// The ⌘↵ confirm action (bound in the `Commit` key context). Routes to the right confirm by
    /// the live commit surface; each path re-checks the surface + the arm gate.
    pub fn confirm_commit(&mut self, cx: &mut Context<Self>) {
        match self.surface {
            Surface::Send => self.send_hold_start(cx),
            Surface::Shield => self.shield_hold_start(cx),
            Surface::Swap => self.swap_hold_start(cx),
            _ => {}
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

    // --- activity feed (#60: the see-and-stop ledger) ---

    /// Open the Activity feed (#60): the see-and-stop ledger of what the agent + you did, and the
    /// triage queue for what still needs you (the "NEEDS YOU" band). It is keyboard-first (proposed
    /// rows are inline-approvable), so it captures focus for its `key_context("Activity")` handler
    /// and kicks a fresh `ActivityFeed` fetch + the recurring poller.
    pub fn open_activity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activity_reviewing = None;
        self.activity_stop_arming = false;
        self.open(Surface::Activity, cx);
        window.focus(&self.activity_focus, cx);
        self.refresh_activity(cx);
        self.start_activity_poller(cx);
    }

    /// Fetch the latest `ActivityFeed` off the UI thread and fold it into `self.activity`,
    /// epoch-guarded (a slow reply for a superseded fetch can't clobber a newer snapshot) and
    /// clamping `activity_selected` to the new approvable-row count. This is the feed's sole
    /// source — the "NEEDS YOU" band derives its rows from `activity`, never a separate list.
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
                        // Preserve the highlight by request_id, NOT by raw index. The ~2s poller
                        // can insert/remove proposed rows between renders, so a stale numeric index
                        // could point at a DIFFERENT record than the operator sees selected — and
                        // `x`/deny + the palette deny-selected resolve the SELECTED record. Capture
                        // the id from the old snapshot, then re-key to it in the new pending subset;
                        // if it's gone (settled/expired), clamp. This closes a wrong-deny race.
                        let selected_id = crate::activity_view::activity_pending(&this.activity)
                            .get(this.activity_selected)
                            .map(|r| r.request_id);
                        this.activity = records;
                        let pending = crate::activity_view::activity_pending(&this.activity);
                        this.activity_selected = selected_id
                            .and_then(|id| pending.iter().position(|r| r.request_id == id))
                            .unwrap_or_else(|| {
                                this.activity_selected.min(pending.len().saturating_sub(1))
                            });
                        // CRITICAL: if the row whose review is OPEN has settled/expired (left the
                        // pending set), clear `activity_reviewing` IN THE SAME swap. The render is
                        // `&self` so it can't clear it — it just falls through to the feed — and a
                        // stale reviewing id would make a one-key APPROVE blind-approve the now-
                        // highlighted DIFFERENT row (`approve_activity` skips re-review while
                        // `reviewing.is_some()`). Clearing forces approve to re-open the highlighted
                        // row's own clear-signing review first — the no-blind-approve invariant.
                        if let Some(id) = this.activity_reviewing {
                            if !pending.iter().any(|r| r.request_id == id) {
                                this.activity_reviewing = None;
                            }
                        }
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
    /// keyboard selection with it so a subsequent ⌘Enter/Esc targets the same record. Sets
    /// `activity_reviewing` ONLY when the clicked row is STILL pending — a click can land a frame
    /// after a background poll settled the row, and opening a phantom review for a settled id would
    /// leave `activity_reviewing` stale.
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
            self.activity_reviewing = Some(request_id);
            cx.notify();
        }
    }

    /// Leave the feed's inline review (Esc, or a completed approve/deny). Pure UI state.
    pub fn cancel_activity_review(&mut self, cx: &mut Context<Self>) {
        self.activity_reviewing = None;
        cx.notify();
    }

    /// Approve the reviewed feed row. Approval resolves ONLY the actively-reviewed, STILL-PENDING
    /// record — NEVER the highlighted-row fallback that deny uses. If there is no valid open review
    /// (none, or the reviewed row settled/expired under a background poll, or a click that raced a
    /// settle left a stale id), `⌘Enter` instead opens the highlighted row's clear-signing review,
    /// so the operator must SEE that row's card before a second `⌘Enter` approves. This makes a
    /// blind-approve of an unreviewed spend STRUCTURALLY impossible no matter how `activity_reviewing`
    /// came to be stale. The agent executes its own write once the record flips to `Allowed` — the
    /// app never broadcasts.
    pub fn approve_activity(&mut self, cx: &mut Context<Self>) {
        let pending = crate::activity_view::activity_pending(&self.activity);
        match crate::activity_view::approve_target(self.activity_reviewing, &pending) {
            // The reviewed record is still pending → its clear-signing card is exactly what the
            // render shows (it keys the same id), so approving it is never blind.
            Some(id) => self.resolve_activity_id(id, true, cx),
            // No valid open review → resolve NOTHING. Drop any stale id and open the highlighted
            // row's review first; the operator approves only after seeing that row's card.
            None => {
                self.activity_reviewing = None;
                self.open_selected_activity_review(cx);
            }
        }
    }

    /// Deny the target feed row: the reviewed record while still pending, else the highlighted
    /// proposed row. Unlike approve, deny is one-key by design (it only REFUSES — the fail-safe
    /// direction), so it MAY fall back to the highlighted, on-screen row.
    pub fn deny_activity(&mut self, cx: &mut Context<Self>) {
        let pending = crate::activity_view::activity_pending(&self.activity);
        let target = self
            .activity_reviewing
            .filter(|id| pending.iter().any(|r| r.request_id == *id))
            .or_else(|| pending.get(self.activity_selected).map(|r| r.request_id));
        if let Some(request_id) = target {
            self.resolve_activity_id(request_id, false, cx);
        }
    }

    /// Drive `Resolve(request_id, approved)` over the capability channel off-thread and reconcile
    /// against the daemon's reply; on success leave the review and re-fetch the feed (the now-
    /// decided row updates in place); on a control-channel failure fail loud and stay put. No
    /// `Execute` ever — the agent/daemon broadcasts.
    fn resolve_activity_id(
        &mut self,
        request_id: deckard_contract::RequestId,
        approved: bool,
        cx: &mut Context<Self>,
    ) {
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
                    this.refresh_activity(cx);
                    // Re-fetch the policy so the wallet-home fence's STOP-brake row flips to
                    // "engaged" without waiting for a re-select (the fence lives in the wallet
                    // cockpit now; a STOP from the feed must not leave it reading "ready").
                    this.kick_agent_policy(cx);
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

    /// The Activity feed's per-surface key handler: j/↓ k/↑ move the highlight, x denies, Enter
    /// opens the selected row's review, ⌘Enter approves, Esc disarms STOP then leaves an open
    /// review. Scoped via `key_context("Activity")` + the focused `activity_focus`, and
    /// `stop_propagation` keeps a bare key off the globals.
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

    /// Start the recurring `ActivityFeed` poller: a ~2s loop that re-fetches while the Activity
    /// surface is open, so an agent-parked record (or a settled outcome) shows up in the feed
    /// without a manual refresh. Idempotent — guarded by `activity_poller_running` so a second
    /// open never spawns a second loop. The loop self-terminates the moment the feed is no longer
    /// the active surface (and clears the flag so the next open restarts it). Mirrors
    /// `watch_shielded_sync`.
    fn start_activity_poller(&mut self, cx: &mut Context<Self>) {
        if self.activity_poller_running {
            return;
        }
        self.activity_poller_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let keep = this.update(cx, |this, cx| {
                    if this.surface == Surface::Activity {
                        this.refresh_activity(cx);
                        true
                    } else {
                        // Off the feed: stop polling and let the next open restart the loop.
                        this.activity_poller_running = false;
                        false
                    }
                });
                match keep {
                    Ok(true) => continue,
                    // Either the view is gone (Err) or we left the feed (Ok(false)) — stop.
                    _ => break,
                }
            }
        })
        .detach();
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

    /// ⌘⇧A — opens the Activity feed. The Approvals surface collapsed into the feed's "NEEDS YOU"
    /// triage band, so the old `OpenApprovals` action (and its keybinding) now opens Activity.
    fn on_open_approvals(
        &mut self,
        _: &OpenApprovals,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette_open {
            return;
        }
        self.open_activity(window, cx);
    }

    fn on_confirm_commit(&mut self, _: &ConfirmCommit, _: &mut Window, cx: &mut Context<Self>) {
        // ⌘↵ on a clear-signing review. Scoped to the focused `Commit` context, so it never fires
        // on Activity (its own ⌘⏎ approve) or elsewhere. `confirm_commit` re-checks surface + arm.
        self.confirm_commit(cx);
    }

    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            return;
        }
        // Back = leave any action surface and return to the selection's Home view.
        if self.surface != Surface::Home {
            self.open(Surface::Home, cx);
            // Landing on the wallet cockpit OR the agent surface via back (not a re-select) must
            // still freshen the live policy — otherwise a STOP fired from the feed, then ⌘[ back,
            // shows a stale "ready" brake. select() kicks for both on click; do it here too.
            if matches!(self.selection, Selection::Wallet | Selection::Agent) {
                self.kick_agent_policy(cx);
            }
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
            "refresh" => self.refresh_portfolio(cx),
            "send" => self.open_send(cx),
            "receive" => self.open(Surface::Receive, cx),
            "shield" => self.open_shield(cx),
            "swap" => self.open_swap(cx),
            // "Approvals" collapsed into the Activity feed — its triage queue is the feed's
            // "NEEDS YOU" band — so this id now opens Activity, same as "activity".
            "approvals" => self.open_activity(window, cx),
            "activity" => self.open_activity(window, cx),
            // APPROVE requires the deliberate two-step (open the row's review, then a second
            // approve) — you can NEVER blind-approve a SPEND from a list row or the palette. DENY is
            // one-key by design (deny only REFUSES a spend — the fail-safe direction); it resolves
            // the highlighted record, which refresh_activity re-keys by request_id so it can't hit
            // the wrong row under poll churn. Open the feed FIRST (mirroring STOP) so the operator
            // triages against a fresh, on-screen snapshot — never a stale off-feed one.
            "approve-selected" => {
                self.open_activity(window, cx);
                self.approve_activity(cx);
            }
            "deny-selected" => {
                self.open_activity(window, cx);
                self.deny_activity(cx);
            }
            // STOP / panic brake — its OWN id. The ⌘K selection is itself the deliberate act, so
            // this fires the kill directly. Route to the Activity feed FIRST so the kill is visible
            // there (the revoked rows + the "Stopped" banner) — otherwise firing it from
            // Portfolio/Send would zeroize the key with no on-screen feedback (the daemon is now
            // locked but the surface looks Ready).
            "revoke-all" => {
                self.open_activity(window, cx);
                self.stop_revoke_all(cx);
            }
            "settings" => self.open(Surface::Settings, cx),
            "copy" => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    self.wallet_address_string(),
                ));
                cx.notify();
            }
            "theme" => self.toggle_mode(cx),
            "mask" => self.toggle_mask(cx),
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
            self.shield
                .amount
                .update(cx, |i, cx| i.set_value("", window, cx));
            self.shield
                .recipient
                .update(cx, |i, cx| i.set_value("", window, cx));
            // The send inputs share the lock-clear: a prior wallet's recipient/amount must not
            // linger into the next unlock.
            self.send
                .amount
                .update(cx, |i, cx| i.set_value("", window, cx));
            self.send
                .recipient
                .update(cx, |i, cx| i.set_value("", window, cx));
            // The swap sell-amount field shares the lock-clear too (the quote/tokens were already
            // cleared in `lock`; this clears the input text that a listener can't, needing a Window).
            self.swap
                .amount
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        if self.recipient_autofilled {
            return;
        }
        if let Some(addr) = self.railgun_address.clone() {
            self.shield.recipient.update(cx, |input, cx| {
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

        // Focus the commit surface on the REVIEW step (a proposal is installed, no input to type)
        // so the `key_context("Commit")` ⌘↵ confirm dispatches. On compose we leave focus with the
        // amount input; idempotent (grabs focus only when not already held) so it never steals
        // focus mid-type.
        let on_commit_review = match self.surface {
            Surface::Send => self.send.proposal.is_some(),
            Surface::Shield => self.shield.proposal.is_some(),
            Surface::Swap => self.swap.proposal.is_some(),
            _ => false,
        };
        if on_commit_review && !self.commit_focus.is_focused(window) {
            self.commit_focus.focus(window, cx);
        }

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
                // Send/Shield: short centered cards. Wrapped so the review step can hold focus for
                // the `key_context("Commit")` ⌘↵ confirm. Focus is grabbed on the review step only
                // (in render() below), so the compose amount input still types.
                (_, Surface::Send) => v_flex()
                    .size_full()
                    .track_focus(&self.commit_focus)
                    .key_context("Commit")
                    .child(self.render_commit(&crate::send_view::SEND_VIEW, cx))
                    .into_any_element(),
                (_, Surface::Shield) => v_flex()
                    .size_full()
                    .track_focus(&self.commit_focus)
                    .key_context("Commit")
                    .child(self.render_commit(&crate::shield_view::SHIELD_VIEW, cx))
                    .into_any_element(),
                // Activity owns its OWN scroll INSIDE the feed body (so the heading + STOP stay
                // pinned — the panic brake must never scroll off screen); this outer just holds the
                // focus + the j/k/x/Enter/⌘Enter key handling.
                (_, Surface::Activity) => v_flex()
                    .id("activity-surface")
                    .size_full()
                    .track_focus(&self.activity_focus)
                    .key_context("Activity")
                    .on_key_down(cx.listener(Self::on_activity_key))
                    .child(self.render_activity(cx))
                    .into_any_element(),
                // Swap is a bespoke render (token pickers + a quote summary the generic
                // `render_commit` can't express); wrap it in its own scroll surface since the
                // compose arm (pickers + summary) can run taller than the pane.
                // v_flex (not a block div) so render_swap's commit_shell flex_1/justify_center
                // actually centers the card — matching Send/Shield (see gpui-div-defaults-block).
                (_, Surface::Swap) => v_flex()
                    .id("scroll-swap")
                    .size_full()
                    .overflow_y_scrollbar()
                    .track_focus(&self.commit_focus)
                    .key_context("Commit")
                    .child(self.render_swap(cx))
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
                // The agent's own surface (DESIGN.md v2 §The agent interaction model): selected
                // from the sidebar Agents group. Rendered entirely from policy data + the agent's
                // activity slice.
                (Selection::Agent, Surface::Home) => div()
                    .id("scroll-agent")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(self.render_agent_surface(cx))
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
            .on_action(cx.listener(Self::on_confirm_commit))
            .child(body)
    }
}
