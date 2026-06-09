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
    v_flex, ActiveTheme, TitleBar,
};

use alloy_primitives::B256;
use deckard_contract::{Decision, ExecuteResult, Intent, RequestId};
use deckard_core::{
    Address, EthProvider, KdfParams, Portfolio, ReadStatus, Vault, WordCount, U256,
};
use zeroize::Zeroizing;

use deckard_signerd::SignerClient;

use crate::settings::{Settings, ThemeModePref};
use crate::signer::{self, AppSigner};
use crate::theme;
use crate::wallet;
use crate::{GoBack, NewItem, OpenSettings, ToggleMask, TogglePalette, ToggleTheme, APP_NAME};

/// The chain the supervised daemon signs for. v1 is mainnet-first (the default RPC is
/// mainnet); multi-chain app config that re-points both the reader and the daemon is a
/// fast-follow.
const DAEMON_CHAIN_ID: u64 = 1;

/// The chain a **shield** deposit targets (T5 config seam, decision D1). It MUST equal the
/// daemon's chain or `propose` denies `chain_mismatch` — so it defaults to
/// [`DAEMON_CHAIN_ID`]. Railgun supports mainnet (1) and Sepolia (11155111); to record a
/// shield on Sepolia, relaunch the daemon + reader on Sepolia and set this to `11155111`
/// (a single switch point, kept out of the render path).
const SHIELD_CHAIN_ID: u64 = DAEMON_CHAIN_ID;

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
    match reason {
        "locked" => "unlock your wallet first".into(),
        "revoked" => "the signer is paused (STOP is active)".into(),
        "chain_mismatch" => {
            "the signer is on a different chain than this deposit — reconcile the chain first"
                .into()
        }
        "over_cap" => "it exceeds the agent's spending cap".into(),
        "off_allowlist" => "the recipient isn't on the allowlist".into(),
        "undecodable" => "the deposit calldata didn't validate".into(),
        "erc20_unsupported_v1" => "only native-ETH shields are supported in v1".into(),
        "unsupported_v1" => "that action isn't supported in v1".into(),
        other => other.to_string(),
    }
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
    /// The shield trigger flow (T5): compose a deposit → review card → hold-to-confirm.
    Shield,
    Settings,
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
    /// Privacy mask: when true, every money surface renders fixed bullets instead of a
    /// figure (DESIGN §Trust). Initialised from `Settings.mask_balances` and persisted on
    /// every toggle — the inverse of the seed reveal's momentary, default-hidden model.
    pub mask: bool,
    /// Demo stand-in for "Atlas is currently acting": drives the one sanctioned ambient
    /// motion (the ~1.2s breathing pulse on the agent squircle). Not persisted — it's a
    /// narrated demo toggle, since the real MCP agent is a fast-follow.
    pub agent_acting: bool,
    /// The capture-block state last pushed to the OS, so `render` only re-issues the
    /// native `setSharingType` call when `capture_block && mask` actually changes.
    capture_applied: bool,

    // --- shield trigger flow (T5) ---
    /// Deposit amount (ETH, free text) and the `0zk…` recipient. Free-text recipient is v1;
    /// auto-filling the user's OWN railgun address is Wave 2.
    pub shield_amount: Entity<InputState>,
    pub shield_recipient: Entity<InputState>,
    /// Set once `propose` returns `Allow`: the built intent + its (locally-derived) request
    /// id. `Some` means the review card + hold-to-confirm are live.
    pub shield_proposal: Option<(Intent, RequestId)>,
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
        // Enter on the recipient field reviews the deposit (keyboard-first).
        cx.subscribe(&shield_recipient, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.review_shield(cx);
            }
        })
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
        let eth = EthProvider::spawn(current_rpc.clone());

        // Spawn + supervise the process-isolated signer daemon. It owns the key; the app is a
        // key-less client that unlocks/signs over the socket. The daemon broadcasts via the
        // same RPC the app reads from.
        let signer = AppSigner::launch(current_rpc.clone(), DAEMON_CHAIN_ID);

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
            mask,
            agent_acting: false,
            capture_applied: false,
            shield_amount,
            shield_recipient,
            shield_proposal: None,
            shield_busy: false,
            shield_error: None,
            shield_tx: None,
            shield_holding: false,
            shield_hold_epoch: 0,
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
        self.retarget(cx);
    }

    /// The unlocked wallet's own address as an EIP-55 string (empty until unlocked).
    pub fn wallet_address_string(&self) -> String {
        self.wallet_address
            .map(|a| a.to_string())
            .unwrap_or_default()
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
    }

    /// Select a sidebar entity: switch the selection and reset to its Home view.
    pub fn select(&mut self, sel: Selection, cx: &mut Context<Self>) {
        self.selection = sel;
        self.surface = Surface::Home;
        cx.notify();
    }

    /// Open a full-pane surface (Home / Receive / Settings) over the current selection.
    pub fn open(&mut self, surface: Surface, cx: &mut Context<Self>) {
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
    /// persisted — Atlas is an openly-narrated manual stand-in for v1.
    pub fn toggle_agent_acting(&mut self, cx: &mut Context<Self>) {
        self.agent_acting = !self.agent_acting;
        cx.notify();
    }

    // --- shield trigger flow (T5) ---

    /// Open the shield flow with a clean slate (clears any prior proposal/error/result; the
    /// typed amount/recipient are left intact).
    pub fn open_shield(&mut self, cx: &mut Context<Self>) {
        self.reset_shield();
        self.open(Surface::Shield, cx);
    }

    /// Clear all transient shield state (proposal, error, broadcast, hold). Bumps the hold
    /// epoch so any in-flight hold timer is cancelled.
    fn reset_shield(&mut self) {
        self.shield_proposal = None;
        self.shield_error = None;
        self.shield_tx = None;
        self.shield_busy = false;
        self.shield_holding = false;
        self.shield_hold_epoch = self.shield_hold_epoch.wrapping_add(1);
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
        cx.notify();
        let client = self.signer.client();
        let task = cx.background_spawn(async move {
            let intent = signer::build_shield_intent(SHIELD_CHAIN_ID, &recipient, value_wei)?;
            let decision = client.propose_blocking(&intent)?;
            Ok::<(Intent, Decision), anyhow::Error>((intent, decision))
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.shield_busy = false;
                match res {
                    Ok((intent, Decision::Allow)) => {
                        let request_id = SignerClient::request_id_for_intent(&intent);
                        this.shield_proposal = Some((intent, request_id));
                    }
                    Ok((_, Decision::NeedsApproval { .. })) => {
                        this.shield_error = Some(
                            "This is over the agent cap — a human approval card is required (lands with the approvals flow)."
                                .into(),
                        );
                    }
                    Ok((_, Decision::Deny { reason })) => {
                        this.shield_error =
                            Some(format!("Can't shield: {}", humanize_deny(&reason)));
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
        let Some((_, request_id)) = self.shield_proposal.clone() else {
            return;
        };
        if self.shield_busy {
            return;
        }
        self.shield_busy = true;
        self.shield_error = None;
        cx.notify();
        let client = self.signer.client();
        let task = cx.background_spawn(async move { client.execute_blocking(request_id) });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.shield_busy = false;
                match res {
                    Ok(ExecuteResult::Broadcast { tx_hash }) => {
                        this.shield_proposal = None;
                        this.shield_tx = Some(tx_hash);
                    }
                    Ok(ExecuteResult::Denied { reason }) => {
                        this.shield_error =
                            Some(format!("Shield denied: {}", humanize_deny(&reason)));
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
                // Only fire if THIS hold is still active (not released, not superseded).
                if this.shield_holding && this.shield_hold_epoch == epoch {
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

    // --- Action handlers (wired in `render`) ---

    fn on_new_item(&mut self, _: &NewItem, _: &mut Window, cx: &mut Context<Self>) {
        self.created += 1;
        cx.notify();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _: &mut Window, cx: &mut Context<Self>) {
        self.open(Surface::Settings, cx);
    }

    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        // Back = leave any action surface and return to the selection's Home view.
        if self.surface != Surface::Home {
            self.open(Surface::Home, cx);
        }
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mode(cx);
    }

    fn on_toggle_palette(&mut self, _: &TogglePalette, _: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        cx.notify();
    }

    fn on_toggle_mask(&mut self, _: &ToggleMask, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mask(cx);
    }

    /// Push the capture-block state to the OS when `capture_block && mask` changes.
    /// Called once per `render`; the change-guard makes it a no-op on most frames. On a
    /// non-macOS or non-`tray` build `apply_capture_block` is itself an inert no-op.
    fn sync_capture_block(&mut self) {
        let desired = self.settings.capture_block && self.mask;
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
            let title_bar = self.render_title_bar(cx);
            let content = match (self.selection, self.surface) {
                (_, Surface::Settings) => self.render_settings(window, cx).into_any_element(),
                (_, Surface::Receive) => self.render_receive(cx).into_any_element(),
                (_, Surface::Shield) => self.render_shield(cx).into_any_element(),
                (Selection::Wallet, Surface::Home) => {
                    self.render_wallet_home(cx).into_any_element()
                }
                (Selection::Project, Surface::Home) => {
                    self.render_project_home(cx).into_any_element()
                }
                (Selection::Agent, Surface::Home) => self.render_agent_home(cx).into_any_element(),
            };
            v_flex()
                .size_full()
                .child(title_bar)
                .child(
                    h_flex().size_full().child(self.render_sidebar(cx)).child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(self.render_breadcrumb(cx))
                            .child(div().flex_1().min_h_0().child(content))
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
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_toggle_palette))
            .on_action(cx.listener(Self::on_toggle_mask))
            .child(body)
    }
}
