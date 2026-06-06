//! Shell — the single root view. Owns the persisted `Settings`, the current
//! route (Welcome vs Settings), and a stateful text input. It renders the title
//! bar plus whichever page is active. The page bodies live in `welcome.rs` and
//! `settings_view.rs` as `impl Shell` methods (Rust lets you split an inherent
//! impl across modules), so this file stays focused on state + navigation.

use gpui::{
    div, App, AppContext, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    v_flex, ActiveTheme, IconName, TitleBar,
};

use deckard_core::{Address, EthProvider, KdfParams, Portfolio, UnlockedVault, Vault, WordCount};
use zeroize::Zeroizing;

use crate::settings::{Settings, ThemeModePref};
use crate::theme::{self, Accent};
use crate::wallet;
use crate::{GoBack, NewItem, OpenSettings, TogglePalette, ToggleTheme, APP_NAME};

/// Trim a noisy provider error down to one short line for the UI.
fn short_err(e: impl std::fmt::Display) -> String {
    let line = e.to_string();
    let line = line.lines().next().unwrap_or("").trim();
    line.chars().take(140).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Welcome,
    Receive,
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
    pub route: Route,
    pub settings: Settings,
    pub name_input: Entity<InputState>,
    pub rpc_input: Entity<InputState>,
    pub watch_input: Entity<InputState>,
    pub created: usize,
    pub palette_open: bool,

    // --- auth / keystore (Chunk 3) ---
    pub auth: AuthStep,
    pub auth_error: Option<String>,
    /// True while an Argon2 create/unlock runs on a background thread.
    pub auth_busy: bool,
    /// The unlocked wallet's own address (for Receive / copy). `None` until unlocked.
    pub wallet_address: Option<Address>,
    /// The in-memory unlocked wallet; dropped (and zeroized) on lock.
    unlocked: Option<UnlockedVault>,
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

        Self {
            focus_handle,
            route: Route::Welcome,
            settings,
            name_input,
            rpc_input,
            watch_input,
            created: 0,
            palette_open: false,
            auth,
            auth_error: None,
            auth_busy: false,
            wallet_address: None,
            unlocked: None,
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

    /// Lock the wallet: drop (zeroize) the unlocked secret and return to the unlock gate.
    pub fn lock(&mut self, cx: &mut Context<Self>) {
        self.unlocked = None;
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
        let task = cx.background_spawn(async move {
            vault.write_atomic(&path)?;
            vault.unlock(pass.as_str())
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(unlocked) => {
                        wallet::delete_legacy_key();
                        this.pending_phrase = None;
                        this.pending_pass = None;
                        this.pending_vault = None;
                        this.finish_unlock(unlocked, cx);
                    }
                    Err(e) => {
                        this.auth_error = Some(short_err(e));
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
        let task = cx.background_spawn(async move {
            let trimmed = secret.trim();
            // Route by shape, not word count: a pure-hex string (optional 0x) is a raw key;
            // anything with spaces/words is a mnemonic, so a short/long phrase gets a real
            // BIP-39 error rather than a misleading "must be 32 bytes".
            let h = trimmed.strip_prefix("0x").unwrap_or(trimmed);
            let looks_like_hex_key = !trimmed.contains(char::is_whitespace)
                && !h.is_empty()
                && h.chars().all(|c| c.is_ascii_hexdigit());
            let vault = if looks_like_hex_key {
                Vault::import_raw_key(trimmed, pass.as_str(), KdfParams::PRODUCTION)?
            } else {
                Vault::import_mnemonic(trimmed, pass.as_str(), KdfParams::PRODUCTION)?
            };
            vault.write_atomic(&path)?;
            vault.unlock(pass.as_str())
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(unlocked) => {
                        wallet::delete_legacy_key();
                        this.finish_unlock(unlocked, cx);
                    }
                    Err(e) => {
                        this.auth_error = Some(short_err(e));
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
        let Some(path) = wallet::vault_path() else {
            self.auth_error = Some("no config directory available".into());
            self.auth_busy = false;
            cx.notify();
            return;
        };
        let pass = Zeroizing::new(pass);
        let task = cx.background_spawn(async move {
            let vault = Vault::read(&path)?;
            vault.unlock(pass.as_str())
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(unlocked) => this.finish_unlock(unlocked, cx),
                    Err(e) => {
                        this.auth_error = Some(short_err(e));
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
        let hex = Zeroizing::new(hex);
        let task = cx.background_spawn(async move {
            let vault = Vault::import_raw_key(hex.as_str(), pass.as_str(), KdfParams::PRODUCTION)?;
            vault.write_atomic(&path)?;
            vault.unlock(pass.as_str())
        });
        cx.spawn(async move |this, cx| {
            let res = task.await;
            this.update(cx, |this, cx| {
                this.auth_busy = false;
                match res {
                    Ok(unlocked) => {
                        wallet::delete_legacy_key();
                        this.finish_unlock(unlocked, cx);
                    }
                    Err(e) => {
                        this.auth_error = Some(short_err(e));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Land in the unlocked app: stash the wallet, derive its address, fetch the portfolio.
    fn finish_unlock(&mut self, unlocked: UnlockedVault, cx: &mut Context<Self>) {
        match unlocked.primary_address() {
            Ok(addr) => {
                self.wallet_address = Some(addr);
                self.unlocked = Some(unlocked);
                self.auth = AuthStep::Ready;
                self.auth_error = None;
                self.route = Route::Welcome;
                self.retarget(cx);
            }
            Err(e) => {
                self.auth_error = Some(short_err(e));
                cx.notify();
            }
        }
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
                    Ok(Ok(p)) => {
                        // Ignore a stale reply for an address we're no longer viewing.
                        if p.address == this.display_address {
                            this.portfolio = Some(p);
                            this.portfolio_error = None;
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
            if let Ok(Ok(n)) = rx.recv_async().await {
                this.update(cx, |this, cx| {
                    this.synced_block = Some(n);
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
    pub fn respawn_provider(&mut self, cx: &mut Context<Self>) {
        let url = self.settings.effective_rpc();
        if url == self.current_rpc {
            return;
        }
        self.current_rpc = url.clone();
        self.eth = EthProvider::spawn(url);
        self.retarget(cx);
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.route = route;
        cx.notify();
    }

    /// Re-install the theme from the current settings (accent + mode).
    fn apply_theme(&self, cx: &mut Context<Self>) {
        theme::install(cx, self.settings.accent, self.settings.theme_mode.to_gpui());
    }

    pub fn set_accent(&mut self, accent: Accent, cx: &mut Context<Self>) {
        self.settings.accent = accent;
        self.settings.save();
        self.apply_theme(cx);
        // Keep the menu-bar tray icon (if running) in sync with the accent.
        #[cfg(feature = "tray")]
        crate::tray::set_accent(cx, accent);
        cx.notify();
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
        self.navigate(Route::Settings, cx);
    }

    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if self.route != Route::Welcome {
            self.navigate(Route::Welcome, cx);
        }
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mode(cx);
    }

    fn on_toggle_palette(&mut self, _: &TogglePalette, _: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        cx.notify();
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let is_settings = self.route == Route::Settings;
        let theme_icon = if self.settings.theme_mode == ThemeModePref::Dark {
            IconName::Sun
        } else {
            IconName::Moon
        };

        TitleBar::new().child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .children(is_settings.then(|| {
                            Button::new("back")
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.navigate(Route::Welcome, cx)),
                                )
                        }))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(muted)
                                .child(if is_settings { "Settings" } else { APP_NAME }),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .children((!is_settings).then(|| {
                            Button::new("open-settings")
                                .ghost()
                                .icon(IconName::Settings)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.navigate(Route::Settings, cx)
                                    }),
                                )
                        }))
                        .child(
                            Button::new("toggle-theme")
                                .ghost()
                                .icon(theme_icon)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_mode(cx))),
                        ),
                ),
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

        let body = if self.auth == AuthStep::Ready {
            // The unlocked app: full title bar + routes + command palette.
            let title_bar = self.render_title_bar(cx);
            let content = match self.route {
                Route::Welcome => self.render_welcome(cx).into_any_element(),
                Route::Receive => self.render_receive(cx).into_any_element(),
                Route::Settings => self.render_settings(window, cx).into_any_element(),
            };
            v_flex()
                .size_full()
                .child(title_bar)
                .child(content)
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
            .child(body)
    }
}
