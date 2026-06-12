//! macOS screen-capture block — `NSWindow.sharingType = .none` (DESIGN §Trust,
//! deckard-demo-ux-locked). Opt-in and **default OFF**: tied to the privacy mask,
//! it removes the app's windows from screen recordings / screenshots so a masked
//! balance can't be captured anyway. For a demo *recording* you leave it off (or the
//! recording itself goes blank), which is exactly why the default is OFF.
//!
//! An automated recording (an agent driving the demo GIF) can guarantee the block is off without
//! reaching into the settings UI by launching with `DECKARD_ALLOW_SCREEN_CAPTURE=1`
//! ([`deckard_core::screen_capture_allowed`]); the override is logged at startup and never silent.
//!
//! ## Why this reuses the tray feature's objc2 dep (no new dependency)
//!
//! The native call lives behind `#[cfg(all(target_os = "macos", feature = "tray"))]`
//! so it compiles against the SAME `objc2` / `objc2-app-kit` crates the tray icon
//! already pulls — no manifest churn, no `raw-window-handle`. We reach the window
//! through `NSApplication.windows` (the app owns exactly one window) rather than
//! bridging GPUI's `Window` to a raw handle, mirroring `tray.rs`'s
//! `NSApplication::sharedApplication` activation-policy call. Every other build
//! (no `tray`, or non-macOS) gets the inert no-op twin below.

/// Apply (or clear) the capture block to all of the app's native windows.
///
/// `on == true` → `NSWindowSharingType::None` (content cannot be captured by other
/// processes); `on == false` → `NSWindowSharingType::ReadOnly` (the system default,
/// capturable). Must run on the main thread — the caller invokes it from `render`,
/// which is already on GPUI's main UI thread.
#[cfg(all(target_os = "macos", feature = "tray"))]
pub fn apply_capture_block(on: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowSharingType};

    // Off the main thread we have no AppKit access; bail rather than risk UB. (render
    // always runs on the main thread, so this is just a guard, never hit in practice.)
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let sharing = if on {
        NSWindowSharingType::None
    } else {
        NSWindowSharingType::ReadOnly
    };
    for window in app.windows().iter() {
        window.setSharingType(sharing);
    }
}

/// No-op on every build without the macOS `tray` feature (Linux/Windows have no
/// `NSWindow`; a non-`tray` macOS build doesn't link AppKit). The setting still
/// persists and the toggle still renders — it just has no OS effect here.
#[cfg(not(all(target_os = "macos", feature = "tray")))]
pub fn apply_capture_block(_on: bool) {}
