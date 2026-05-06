//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

mod audio;
mod autostart;
mod cleanup;
mod config;
mod dictation;
mod icon;
mod keyboard;
mod overlay;
mod perms;
mod shortcut;
mod ui;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use objc2_foundation::MainThreadMarker;

use dictation::{Dictation, Trigger};

fn main() {
    let mtm = MainThreadMarker::new().expect("main() must run on the main thread");

    let cfg = config::load();

    let overlay_state = overlay::OverlayState::new();
    let _overlay_ctrl = overlay::OverlayController::install(mtm, overlay_state.clone());

    // Two channels with a relay so the backend can be swapped live: the
    // shortcut monitor sends into `front_tx`; the relay forwards to whichever
    // pipeline sender is currently in `hub`. Replacing the hub's inner sender
    // drops the old one, which signals the previous pipeline thread to exit.
    let (front_tx, front_rx) = mpsc::channel::<Trigger>();
    let (pipeline_tx, pipeline_rx) = mpsc::channel::<Trigger>();
    Dictation::production(&cfg, overlay_state.clone()).start_processing(pipeline_rx);
    let hub = Arc::new(Mutex::new(pipeline_tx));
    let hub_for_relay = hub.clone();
    std::thread::spawn(move || {
        while let Ok(t) = front_rx.recv() {
            let _ = hub_for_relay.lock().unwrap().send(t);
        }
    });

    // Install the global shortcut monitor. NSEvent.addGlobalMonitor needs
    // only Accessibility (no separate Input Monitoring perm). It silently
    // no-ops until Accessibility is granted; after that, events flow
    // without any reinstall.
    let monitor = shortcut::ShortcutMonitor::install(cfg.shortcut.clone(), front_tx);

    let ui_handles =
        ui::install_menubar_and_window(mtm, cfg.shortcut, monitor, hub, overlay_state);
    ui::maybe_show_settings_at_launch(&ui_handles.controller);

    ui::run_app(mtm);
}
