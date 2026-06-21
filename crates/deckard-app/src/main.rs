//! Deckard — the native desktop app (macOS + Linux), built on GPUI.
//!
//! `main` wires the native shell: a window with a custom title bar, the system
//! menu bar, global keyboard shortcuts, the bundled offline fonts, the refined
//! theme, and persisted settings. The views live in `shell.rs` + the per-surface
//! modules; shared leaf components live in `widgets.rs` (DESIGN.md §Enforcement).

mod activity_view;
mod agent_view;
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
mod widgets;

use gpui::{
    px, size, App, AppContext, Bounds, KeyBinding, Menu, MenuItem, OsAction, WindowBounds,
    WindowOptions,
};
use gpui_component::{Root, TitleBar};

use settings::Settings;
use shell::Shell;

/// The display name used in the menu bar and window. Change this first when forking.
pub const APP_NAME: &str = "Deckard";

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
        PalettePrev,
        ConfirmCommit
    ]
);

fn main() {
    // `with_assets` registers gpui-component's bundled icon SVGs + fonts so
    // `IconName::*` renders. This is the whole asset story for the UI kit.
    // `application()` lives in `gpui_platform` after Zed split gpui into core +
    // platform crates; it returns the same `gpui::Application` builder.
    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            // 1. Bring up gpui-component (themes, fonts, icon assets, input system).
            gpui_component::init(cx);

            // 1b. Register the bundled offline fonts (no web-font CDN). The theme
            //     sets the family names ("Schibsted Grotesk" / "JetBrains Mono");
            //     these files are embedded at build time so the running app renders
            //     in Deckard's real type. DESIGN.md §Enforcement: fonts are bundled,
            //     not optional — without this the app falls back to the OS system
            //     font and the mono-for-money + weight hierarchy is fiction. Both
            //     families are OFL-1.1, so the raw files are safe to ship in a
            //     public repo.
            use std::borrow::Cow;
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(
                        include_bytes!("../assets/fonts/SchibstedGrotesk-Regular.otf").as_slice(),
                    ),
                    Cow::Borrowed(
                        include_bytes!("../assets/fonts/SchibstedGrotesk-Medium.otf").as_slice(),
                    ),
                    Cow::Borrowed(
                        include_bytes!("../assets/fonts/SchibstedGrotesk-SemiBold.otf").as_slice(),
                    ),
                    Cow::Borrowed(
                        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
                    ),
                    Cow::Borrowed(
                        include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf").as_slice(),
                    ),
                ])
                // reason: bundled fonts are a build-time invariant; a failure here
                // is a packaging bug we want to surface loudly at startup.
                .expect("bundled fonts failed to register");

            // 2. Load persisted preferences and install the refined theme from them.
            let settings = Settings::load();
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
                // ⌘↵ confirms a clear-signing review. Scoped to the `Commit` context (the
                // focused Send/Shield/Swap review surface) so it never shadows Activity's own
                // ⌘⏎ approve. The handler re-checks the surface + the arm-delay.
                KeyBinding::new("secondary-enter", ConfirmCommit, Some("Commit")),
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
