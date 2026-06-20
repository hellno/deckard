//! Deck — an omakase native desktop app starter on GPUI (macOS + Linux).
//!
//! `main` wires the native shell: a window with a custom title bar, the system
//! menu bar, global keyboard shortcuts, a refined theme, and persisted settings.
//! The UI lives in `shell.rs` (+ `welcome.rs` / `settings_view.rs`).
//!
//! Fork checklist: rename the crate in `Cargo.toml`, change `APP_NAME` and the
//! bundle identifier, swap `assets/icon.png`, then start editing the views.

mod activity_view;
mod capture;
mod commit_flow;
mod commit_view;
mod errors;
mod money;
mod onboarding;
mod palette;
mod palette_commands;
mod palette_usage;
mod receive;
mod send_view;
mod settings;
mod settings_view;
mod shell;
mod shell_chrome;
mod shield_view;
mod signer;
mod swap;
mod swap_view;
mod theme;
#[cfg(feature = "tray")]
mod tray;
mod wallet;
mod welcome;

use gpui::{
    px, size, App, AppContext, Bounds, KeyBinding, Menu, MenuItem, OsAction, WindowBounds,
    WindowOptions,
};
use gpui_component::{Root, TitleBar};

use settings::Settings;
use shell::Shell;

/// The display name used in the menu bar and window. Change this first when forking.
pub const APP_NAME: &str = "Deckard";

/// How long the launch chain-id pre-flight waits for the RPC's `eth_chainId`. The happy path is
/// sub-second; this only bounds a hung/unreachable RPC, which then warns and continues rather than
/// blocking the launch.
const CHAIN_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

// Declare the app's actions. Each becomes a zero-sized struct you can bind a key
// to, hang a menu item off of, and handle in a view or globally. Add your own here.
gpui::actions!(
    deckard,
    [
        Quit,
        About,
        OpenSettings,
        OpenApprovals,
        ToggleTheme,
        NewItem,
        GoBack,
        TogglePalette,
        ToggleMask,
        PaletteNext,
        PalettePrev
    ]
);

fn main() {
    // Load persisted preferences before the GUI comes up so the launch pre-flight can resolve the
    // chain config (env > settings > registry default) from them.
    let settings = Settings::load();

    // Launch pre-flight: verify the RPC actually serves the chain Deckard is configured for, and
    // REFUSE TO START on a confirmed mismatch rather than silently reading the wrong chain. This is
    // EIP-3085's strongest clause ("reject if the declared chainId does not match the RPC's
    // eth_chainId") and protects the EIP-155 replay guarantee. The RPC-reported id is only ever
    // compared here, never used to pick the chain. An unreachable RPC cannot CONFIRM a mismatch, so
    // it warns and continues (offline tolerance): a node that serves no read can serve no wrong one.
    let chain_id = settings.effective_chain_id();
    let rpc = settings.effective_rpc(chain_id);
    match deckard_core::probe_rpc_chain_id(&rpc, chain_id, CHAIN_PROBE_TIMEOUT) {
        deckard_core::ChainIdProbe::Match => {}
        deckard_core::ChainIdProbe::Unreachable { error } => {
            eprintln!(
                "deckard: could not verify chain id against the RPC ({} · {error}); continuing — \
                 an unreachable RPC serves no read, so there is no wrong-chain risk",
                deckard_signerd::config::redact_url(&rpc),
            );
        }
        deckard_core::ChainIdProbe::Mismatch { declared, reported } => {
            eprintln!(
                "deckard: REFUSING TO START — configured for chain {declared} but the RPC ({}) \
                 reports chain {reported}. Point DECKARD_RPC_URL at a chain-{declared} endpoint (or \
                 fix DECKARD_CHAIN_ID). Refusing rather than silently reading the wrong chain.",
                deckard_signerd::config::redact_url(&rpc),
            );
            std::process::exit(1);
        }
    }

    // `with_assets` registers gpui-component's bundled icon SVGs + fonts so
    // `IconName::*` renders. This is the whole asset story for the UI kit.
    // `application()` lives in `gpui_platform` after Zed split gpui into core +
    // platform crates; it returns the same `gpui::Application` builder.
    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx: &mut App| {
            // 1. Bring up gpui-component (themes, fonts, icon assets, input system).
            gpui_component::init(cx);

            // 1b. Register the bundled offline fonts (no web-font CDN). The theme
            //     sets the family names ("General Sans" / "JetBrains Mono"); GPUI
            //     silently falls back to the system font until the files exist, so
            //     the family-name config alone is safe to ship now.
            //
            // TODO(fonts): a human must drop the licensed font files into
            //   crates/deckard-app/assets/fonts/ (see that dir's README.md), then
            //   uncomment the block below to embed + register them. Do NOT
            //   uncomment before the files exist — `include_bytes!` of a missing
            //   path is a compile error.
            //
            // use std::borrow::Cow;
            // cx.text_system()
            //     .add_fonts(vec![
            //         Cow::Borrowed(include_bytes!("../assets/fonts/GeneralSans-Regular.otf").as_slice()),
            //         Cow::Borrowed(include_bytes!("../assets/fonts/GeneralSans-Medium.otf").as_slice()),
            //         Cow::Borrowed(include_bytes!("../assets/fonts/GeneralSans-Semibold.otf").as_slice()),
            //         Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice()),
            //         Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf").as_slice()),
            //     ])
            //     // reason: bundled fonts are a build-time invariant; a failure here
            //     // is a packaging bug we want to surface loudly at startup.
            //     .expect("bundled fonts failed to register");

            // 2. Install the refined theme from the persisted preferences (loaded in `main`
            //    before the launch chain-id pre-flight).
            theme::install(cx, settings.theme_mode.to_gpui());

            // 3. Keyboard shortcuts. `secondary` = ⌘ on macOS, Ctrl on Linux /
            //    Windows — so these are portable. Context `None` = global.
            cx.bind_keys([
                KeyBinding::new("secondary-q", Quit, None),
                KeyBinding::new("secondary-n", NewItem, None),
                KeyBinding::new("secondary-,", OpenSettings, None),
                KeyBinding::new("secondary-shift-a", OpenApprovals, None),
                KeyBinding::new("secondary-shift-d", ToggleTheme, None),
                KeyBinding::new("secondary-[", GoBack, None),
                KeyBinding::new("secondary-k", TogglePalette, None),
                KeyBinding::new("secondary-shift-m", ToggleMask, None),
                // Tab / Shift-Tab navigate the palette. Scoped to the `CommandPalette` context so
                // they SHADOW gpui-component `Root`'s global `tab`→focus-traversal (a deeper-context
                // binding wins by depth). Without this, Tab moves focus OUT of the open palette onto a
                // hidden input behind the scrim — keystrokes would then land in an invisible wallet
                // field. (`Root` binds `tab` in its own context, which is always live.)
                KeyBinding::new("tab", PaletteNext, Some("CommandPalette")),
                KeyBinding::new("shift-tab", PalettePrev, Some("CommandPalette")),
            ]);

            // 4. Global action handlers. View-local actions (NewItem, OpenSettings,
            //    ToggleTheme, GoBack) are handled inside `Shell` so they can touch
            //    UI state and persist; only app-wide ones live here.
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            cx.on_action(|_: &About, _cx: &mut App| {
                println!("{APP_NAME} — a GPUI app.");
            });

            // 5. The native macOS menu bar.
            cx.set_menus(vec![
                Menu {
                    name: APP_NAME.into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action(format!("About {APP_NAME}"), About),
                        MenuItem::separator(),
                        MenuItem::action("Settings…", OpenSettings),
                        MenuItem::separator(),
                        MenuItem::action(format!("Quit {APP_NAME}"), Quit),
                    ],
                },
                Menu {
                    name: "File".into(),
                    disabled: false,
                    items: vec![MenuItem::action("New", NewItem)],
                },
                Menu {
                    name: "Edit".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::os_action("Undo", NewItem, OsAction::Undo),
                        MenuItem::separator(),
                        MenuItem::os_action("Cut", NewItem, OsAction::Cut),
                        MenuItem::os_action("Copy", NewItem, OsAction::Copy),
                        MenuItem::os_action("Paste", NewItem, OsAction::Paste),
                        MenuItem::os_action("Select All", NewItem, OsAction::SelectAll),
                    ],
                },
                Menu {
                    name: "View".into(),
                    disabled: false,
                    items: vec![MenuItem::action("Toggle Light / Dark", ToggleTheme)],
                },
            ]);

            // 6. Open the window. `TitleBar::title_bar_options()` makes the title
            //    bar transparent + insets the traffic lights so `Shell`'s custom
            //    `TitleBar` element draws edge-to-edge underneath.
            let bounds = Bounds::centered(None, size(px(880.0), px(620.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitleBar::title_bar_options()),
                window_min_size: Some(size(px(560.0), px(420.0))),
                ..Default::default()
            };

            cx.open_window(options, move |window, cx| {
                let view = cx.new(|cx| Shell::new(settings, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            // Optional: native menu-bar tray icon + dock hiding (`--features tray`).
            // The tray icon uses a fixed brand color.
            #[cfg(feature = "tray")]
            tray::install(cx);

            cx.activate(true);
        });
}
