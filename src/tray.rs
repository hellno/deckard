//! Optional native menu-bar tray icon + dock hiding (`cargo run --features tray`).
//!
//! ## Does a tray icon fit the GPUI flow? Yes — and it adds no second renderer.
//!
//! A macOS menu-bar item (`NSStatusItem`) is, like the dock icon, just a native
//! **image plus a native menu** — there is nothing to "render" with a UI
//! framework. So the tray icon is drawn by AppKit (via the `tray-icon` crate),
//! while every actual *window* you show stays 100% GPUI. No second rendering
//! system enters the project.
//!
//! Integration is clean because GPUI's run loop *is* the standard AppKit
//! `NSApplication.run` loop. `tray-icon` posts click/menu events to a global
//! channel; we drain that channel on GPUI's own executor and act on the main
//! thread. The only other native call is flipping the activation policy to
//! `Accessory` so the app has no dock icon (a true menu-bar-only app).

use std::time::Duration;

use gpui::{App, Global};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::theme::Accent;

/// Holds the live tray icon so it (a) stays alive for the process and (b) can be
/// restyled when the user changes accent in settings. Stored as a GPUI global.
struct TrayState {
    tray: TrayIcon,
}

impl Global for TrayState {}

/// Build the tray icon, hide the dock, and bridge tray-menu clicks into GPUI.
/// Call once from `app.run` (it must run on the main thread, after launch).
pub fn install(cx: &mut App, accent: Accent) {
    hide_dock_icon();

    let menu = Menu::new();
    let show = MenuItem::new(format!("Show {}", crate::APP_NAME), true, None);
    let quit = MenuItem::new(format!("Quit {}", crate::APP_NAME), true, None);
    let _ = menu.append(&show);
    let _ = menu.append(&quit);

    let tray = TrayIconBuilder::new()
        .with_tooltip(crate::APP_NAME)
        .with_icon(brand_icon(accent))
        .with_menu(Box::new(menu))
        .build()
        .expect("failed to build tray icon");

    // Keep the status item alive for the whole process, and reachable so accent
    // changes can restyle it (see `set_accent`).
    cx.set_global(TrayState { tray });

    let show_id = show.id().clone();
    let quit_id = quit.id().clone();

    // Bridge: poll the native menu-event channel on GPUI's executor, act on the
    // main thread via `cx.update`. 120ms is imperceptible for human clicks.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let rx = MenuEvent::receiver();
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            while let Ok(event) = rx.try_recv() {
                if event.id == quit_id {
                    let _ = cx.update(|cx| cx.quit());
                } else if event.id == show_id {
                    // Bring the (still-GPUI) window back to the foreground.
                    let _ = cx.update(|cx| cx.activate(true));
                }
            }
        }
    })
    .detach();
}

/// Restyle the tray icon to match a new accent. No-op if the tray isn't running
/// (i.e. the feature is on but `install` wasn't called). Wired from `Shell`.
pub fn set_accent(cx: &mut App, accent: Accent) {
    if cx.has_global::<TrayState>() {
        let icon = brand_icon(accent);
        let _ = cx.global::<TrayState>().tray.set_icon(Some(icon));
    }
}

/// Make the app a menu-bar "accessory": no dock icon, no ⌘-Tab entry.
/// macOS-only — GPUI hardcodes `Regular` at launch, so we override it here at
/// runtime via objc2. On Linux/Windows there's no dock; whether a window shows
/// in the taskbar is the window manager's concern, so this is a no-op there.
#[cfg(target_os = "macos")]
fn hide_dock_icon() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
fn hide_dock_icon() {}

/// A simple 32×32 rounded-square icon in the current accent color. Replace with
/// your own — e.g. `Icon::from_path("…")`, or a black template image so macOS
/// tints it to match the menu bar automatically.
fn brand_icon(accent: Accent) -> Icon {
    const SIZE: u32 = 32;
    let hex = accent.rgb();
    let (r, g, b) = ((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if rounded_square(x as f32, y as f32, SIZE as f32, 6.0) {
                let i = ((y * SIZE + x) * 4) as usize;
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 0xFF;
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid tray icon")
}

fn rounded_square(x: f32, y: f32, size: f32, radius: f32) -> bool {
    let (lo, hi) = (2.0, size - 2.0);
    if x < lo || x > hi || y < lo || y > hi {
        return false;
    }
    let cx = x.clamp(lo + radius, hi - radius);
    let cy = y.clamp(lo + radius, hi - radius);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= radius * radius + 0.5
}
