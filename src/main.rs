//! hush — local push-to-talk dictation. Hold fn, talk, release to paste.

mod audio;
mod autostart;
mod icon;
mod keyboard;
mod overlay;
mod perms;
mod ui;

use std::cell::Cell;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult,
};
use objc2_foundation::MainThreadMarker;

use crate::perms::PermStatus;

const FN_FLAG_BITS: u64 = 0x00800000; // kCGEventFlagMaskSecondaryFn

fn main() {
    let mtm = MainThreadMarker::new().expect("main() must run on the main thread");

    let overlay_state = overlay::OverlayState::new();
    // Overlay controller must be created on the main thread; it owns
    // its own NSTimer that drives show/hide + redraws.
    let _overlay_ctrl = overlay::OverlayController::install(mtm, overlay_state.clone());

    let (tx, rx) = mpsc::channel::<audio::Msg>();
    let worker_overlay = overlay_state.clone();
    std::thread::spawn(move || {
        let model_path = audio::ensure_model();
        audio::run_worker(&model_path, rx, worker_overlay);
    });

    let tap_handle = TapHandle::new(tx);
    let ui_handles = ui::install_menubar_and_window(mtm, tap_handle.clone());
    ui::maybe_show_settings_at_launch(&ui_handles.controller);

    // First attempt — succeeds if Input Monitoring is already granted.
    // If it fails, the UI's perm-poll timer calls try_install again
    // every tick, so once the user grants the tap comes online without
    // an app restart.
    tap_handle.try_install();

    ui::run_app(mtm);
}

/// Shared handle for the global event tap. The UI poll timer re-calls
/// `try_install` after each TCC refresh, so once Input Monitoring is
/// granted the tap comes online without an app restart.
#[derive(Clone)]
pub struct TapHandle {
    /// `Some(tx)` until the tap is installed; then `None` (the sender
    /// has been moved into the tap callback).
    pending: Arc<Mutex<Option<Sender<audio::Msg>>>>,
}

impl TapHandle {
    fn new(tx: Sender<audio::Msg>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(Some(tx))),
        }
    }

    pub fn try_install(&self) {
        let mut slot = self.pending.lock().unwrap();
        let tx = match slot.take() {
            Some(t) => t,
            None => return, // already attempted
        };
        if !PermStatus::check().input_monitoring {
            // Perm not granted yet; hold the sender so a later
            // attempt (after the user grants) can install.
            *slot = Some(tx);
            return;
        }
        if install_event_tap(tx) {
            eprintln!("[hush] event tap installed");
        } else {
            // Preflight said granted but CGEventTapCreate failed —
            // typically a TCC cdhash mismatch on ad-hoc dev builds.
            // Do NOT retry: each CGEventTapCreate against an
            // unauthorized cdhash re-fires the "Keystroke Receiving"
            // TCC prompt, trapping the user in a prompt loop. Drop
            // the sender; user can restart the app to retry.
            eprintln!(
                "[hush] event tap install failed despite granted perm; \
                 quit and reopen Hush to retry"
            );
        }
    }
}

/// Returns true if the tap was successfully installed.
fn install_event_tap(tx: Sender<audio::Msg>) -> bool {
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
        Err(_) => return false,
    };

    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(s) => s,
        Err(_) => return false,
    };

    unsafe {
        let main_loop = CFRunLoop::get_main();
        main_loop.add_source(&loop_source, kCFRunLoopCommonModes);
    }
    tap.enable();

    // Tap is intentionally leaked: it must outlive the program.
    std::mem::forget(tap);
    true
}
