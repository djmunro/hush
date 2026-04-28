//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

mod audio;
mod icon;
mod keyboard;
mod perms;
mod ui;

use std::cell::Cell;
use std::sync::mpsc;

use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};
use objc2_foundation::MainThreadMarker;

const FN_FLAG_BITS: u64 = 0x00800000; // kCGEventFlagMaskSecondaryFn

fn main() {
    let mtm = MainThreadMarker::new().expect("main() must run on the main thread");

    // UI is built first so the menubar appears immediately, regardless
    // of model download / load duration or permission state.
    let ui_handles = ui::install_menubar_and_window(mtm);

    // Open the settings window if any perm is missing — gives the user
    // somewhere to land instead of a silent menubar icon.
    ui::maybe_show_settings_at_launch(&ui_handles.controller);

    // Spin up audio + transcription worker on a background thread.
    let (tx, rx) = mpsc::channel::<audio::Msg>();
    std::thread::spawn(move || {
        let model_path = audio::ensure_model();
        audio::run_worker(&model_path, rx);
    });

    // Install the global fn-key event tap on the main run loop.
    install_event_tap(tx);

    // Hand control to AppKit. The event tap fires from the same run loop.
    ui::run_app(mtm);
}

fn install_event_tap(tx: mpsc::Sender<audio::Msg>) {
    // CGEventTap callbacks are `Fn` (not FnMut), so we lean on Cell for
    // edge-detection state. The tap fires from the main run loop, so the
    // !Sync-ness of Cell is fine.
    let fn_down = Cell::new(false);

    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::FlagsChanged],
        move |_proxy, _ty, event| {
            let pressed = (event.get_flags().bits() & FN_FLAG_BITS) != 0;
            if pressed && !fn_down.get() {
                fn_down.set(true);
                let _ = tx.send(audio::Msg::Start);
            } else if !pressed && fn_down.get() {
                fn_down.set(false);
                let _ = tx.send(audio::Msg::Stop);
            }
            CallbackResult::Keep
        },
    );

    let tap = match tap {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "[hush] could not install global event tap. Grant Input Monitoring \
                 in Settings, then relaunch."
            );
            return;
        }
    };

    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("[hush] could not create event tap run loop source");
            return;
        }
    };

    unsafe {
        let main_loop = CFRunLoop::get_main();
        main_loop.add_source(&loop_source, kCFRunLoopCommonModes);
    }
    tap.enable();

    // Tap is intentionally leaked: it must outlive the program.
    std::mem::forget(tap);
}
