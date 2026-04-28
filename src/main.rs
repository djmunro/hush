//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

mod audio;
mod autostart;
mod config;
mod dictation;
mod icon;
mod keyboard;
mod overlay;
mod perms;
mod shortcut;
mod ui;

use std::sync::mpsc;

use objc2_foundation::MainThreadMarker;

use dictation::{Dictation, Trigger};

fn main() {
    let mtm = MainThreadMarker::new().expect("main() must run on the main thread");

    let cfg = config::load();

    let overlay_state = overlay::OverlayState::new();
    let _overlay_ctrl = overlay::OverlayController::install(mtm, overlay_state.clone());

    let (tx, rx) = mpsc::channel::<Trigger>();
    Dictation::production(audio::ensure_model(), overlay_state.clone()).start_processing(rx);

    // Install the global shortcut monitor. NSEvent.addGlobalMonitor needs
    // only Accessibility (no separate Input Monitoring perm). It silently
    // no-ops until Accessibility is granted; after that, events flow
    // without any reinstall.
    let monitor = shortcut::ShortcutMonitor::install(cfg.shortcut.clone(), tx);

    let ui_handles = ui::install_menubar_and_window(mtm, cfg.shortcut, monitor);
    ui::maybe_show_settings_at_launch(&ui_handles.controller);

    ui::run_app(mtm);
}
