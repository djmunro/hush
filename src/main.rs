//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

mod audio;
mod autostart;
mod icon;
mod keyboard;
mod overlay;
mod perms;
mod ui;

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Sender};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
use objc2_foundation::MainThreadMarker;

fn main() {
    let mtm = MainThreadMarker::new().expect("main() must run on the main thread");

    let overlay_state = overlay::OverlayState::new();
    let _overlay_ctrl = overlay::OverlayController::install(mtm, overlay_state.clone());

    let (tx, rx) = mpsc::channel::<audio::Msg>();
    let worker_overlay = overlay_state.clone();
    std::thread::spawn(move || {
        let model_path = audio::ensure_model();
        audio::run_worker(&model_path, rx, worker_overlay);
    });

    // Install the global fn-key monitor. NSEvent.addGlobalMonitor needs
    // only Accessibility (no separate Input Monitoring perm — this is
    // the same approach Wispr Flow uses). The monitor is registered
    // here and silently no-ops until Accessibility is granted; after
    // that, events flow without any reinstall.
    let monitor = install_fn_monitor(tx);
    if let Some(m) = monitor {
        // The OS retains the monitor; leak our handle so the block
        // never drops while the app is alive.
        std::mem::forget(m);
    }

    let ui_handles = ui::install_menubar_and_window(mtm);
    ui::maybe_show_settings_at_launch(&ui_handles.controller);

    ui::run_app(mtm);
}

fn install_fn_monitor(tx: Sender<audio::Msg>) -> Option<Retained<AnyObject>> {
    // Edge-detect fn press / release. Block fires on the main thread,
    // so a Cell suffices for the prev-state.
    let fn_down = Cell::new(false);
    let handler = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| {
        let event = unsafe { event_ptr.as_ref() };
        let pressed = event
            .modifierFlags()
            .contains(NSEventModifierFlags::Function);
        if pressed && !fn_down.get() {
            fn_down.set(true);
            let _ = tx.send(audio::Msg::Start);
        } else if !pressed && fn_down.get() {
            fn_down.set(false);
            let _ = tx.send(audio::Msg::Stop);
        }
    });
    NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::FlagsChanged,
        &handler,
    )
}
