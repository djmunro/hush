//! Floating pill overlay near the bottom-center of the screen.
//!
//! Three modes:
//! - Hidden — panel ordered out
//! - Recording — live audio-level bars driven by a ring buffer of recent
//!   RMS values that the audio worker writes
//! - Transcribing — animated dots while whisper does its thing
//!
//! Driven by an NSTimer firing at ~30Hz. The timer reads the shared
//! state and triggers a redraw / show / hide as needed. Audio thread
//! never touches AppKit — it only mutates the Mutex.

#![allow(deprecated)]

use std::cell::OnceCell;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSPanel, NSScreen, NSStatusWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSTimer,
};

const LEVEL_HISTORY: usize = 14;
const PILL_WIDTH: CGFloat = 140.0;
const PILL_HEIGHT: CGFloat = 36.0;
const BOTTOM_INSET: CGFloat = 80.0;
const FRAME_HZ: f64 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    Hidden,
    Recording,
    Transcribing,
}

pub struct OverlayState {
    pub mode: OverlayMode,
    /// Ring buffer of recent RMS levels in [0, 1]. Index 0 is oldest.
    pub levels: [f32; LEVEL_HISTORY],
    /// Monotonic frame counter for animation phase (transcribing dots).
    pub anim_phase: u32,
}

impl OverlayState {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            mode: OverlayMode::Hidden,
            levels: [0.0; LEVEL_HISTORY],
            anim_phase: 0,
        }))
    }

    /// Audio worker calls this each callback batch with the new RMS.
    /// Cheap rolling shift — rarely contended.
    pub fn push_level(state: &Arc<Mutex<Self>>, rms: f32) {
        let mut s = state.lock().unwrap();
        for i in 0..LEVEL_HISTORY - 1 {
            s.levels[i] = s.levels[i + 1];
        }
        s.levels[LEVEL_HISTORY - 1] = rms.clamp(0.0, 1.0);
    }

    pub fn set_mode(state: &Arc<Mutex<Self>>, mode: OverlayMode) {
        let mut s = state.lock().unwrap();
        s.mode = mode;
        if mode != OverlayMode::Recording {
            // Drop the bars so a future Recording starts from silence.
            s.levels = [0.0; LEVEL_HISTORY];
        }
    }
}

#[derive(Default)]
pub struct OverlayViewIvars {
    state: OnceCell<Arc<Mutex<OverlayState>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[name = "HushOverlayView"]
    #[ivars = OverlayViewIvars]
    pub struct OverlayView;

    impl OverlayView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            unsafe { self.do_draw() }
        }
    }

    unsafe impl NSObjectProtocol for OverlayView {}
);

impl OverlayView {
    fn new(mtm: MainThreadMarker, state: Arc<Mutex<OverlayState>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(OverlayViewIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let _ = this.ivars().state.set(state);
        this
    }

    unsafe fn do_draw(&self) {
        let bounds: NSRect = self.bounds();

        // Pill background — semi-opaque black, rounded.
        let radius = bounds.size.height / 2.0;
        let bg = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, radius, radius);
        let bg_color = NSColor::colorWithCalibratedWhite_alpha(0.0, 0.85);
        bg_color.setFill();
        bg.fill();

        // Snapshot state under the lock, then draw without holding it.
        let (mode, levels, phase) = {
            let s = self.ivars().state.get().unwrap().lock().unwrap();
            (s.mode, s.levels, s.anim_phase)
        };

        match mode {
            OverlayMode::Recording => draw_bars(bounds, &levels),
            OverlayMode::Transcribing => draw_dots(bounds, phase),
            OverlayMode::Hidden => {} // window will be ordered out
        }
    }
}

unsafe fn draw_bars(bounds: NSRect, levels: &[f32; LEVEL_HISTORY]) {
    NSColor::whiteColor().setFill();

    let n = LEVEL_HISTORY as CGFloat;
    let bar_w: CGFloat = 4.0;
    let gap: CGFloat = 3.0;
    let total_w = n * bar_w + (n - 1.0) * gap;
    let start_x = (bounds.size.width - total_w) / 2.0;
    let cy = bounds.size.height / 2.0;
    let max_h: CGFloat = bounds.size.height - 14.0;
    let min_h: CGFloat = 3.0;

    for (i, lvl) in levels.iter().enumerate() {
        // Perceptual curve — log-ish so quiet speech still moves the bars.
        let shaped = (*lvl as CGFloat).powf(0.6);
        let h = (min_h + (max_h - min_h) * shaped).clamp(min_h, max_h);
        let x = start_x + (i as CGFloat) * (bar_w + gap);
        let y = cy - h / 2.0;
        let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(bar_w, h));
        let path =
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, bar_w / 2.0, bar_w / 2.0);
        path.fill();
    }
}

unsafe fn draw_dots(bounds: NSRect, phase: u32) {
    let n: usize = 10;
    let dot_d: CGFloat = 3.0;
    let gap: CGFloat = 5.0;
    let total_w = (n as CGFloat) * dot_d + ((n - 1) as CGFloat) * gap;
    let start_x = (bounds.size.width - total_w) / 2.0;
    let cy = bounds.size.height / 2.0;

    // Walking-highlight: one dot brighter, sweeps left → right.
    let head = (phase as usize / 3) % n;
    for i in 0..n {
        let dist = ((i as i32 - head as i32).abs()) as CGFloat;
        let alpha = (1.0 - (dist / 4.0).min(1.0) * 0.7).max(0.3);
        NSColor::colorWithCalibratedWhite_alpha(1.0, alpha).setFill();

        let x = start_x + (i as CGFloat) * (dot_d + gap);
        let y = cy - dot_d / 2.0;
        let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(dot_d, dot_d));
        NSBezierPath::bezierPathWithOvalInRect(rect).fill();
    }
}

#[derive(Default)]
pub struct OverlayControllerIvars {
    panel: OnceCell<Retained<NSPanel>>,
    view: OnceCell<Retained<OverlayView>>,
    state: OnceCell<Arc<Mutex<OverlayState>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HushOverlayController"]
    #[ivars = OverlayControllerIvars]
    pub struct OverlayController;

    impl OverlayController {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&AnyObject>) {
            unsafe { self.do_tick() }
        }
    }

    unsafe impl NSObjectProtocol for OverlayController {}
);

impl OverlayController {
    pub fn install(mtm: MainThreadMarker, state: Arc<Mutex<OverlayState>>) -> Retained<Self> {
        // OverlayController is a plain NSObject — no main-thread alloc required.
        let this = <Self as objc2::AllocAnyThread>::alloc()
            .set_ivars(OverlayControllerIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };

        unsafe {
            // Borderless, transparent, non-activating, floating panel —
            // it sits above all windows but doesn't take focus or
            // appear in cmd-tab.
            let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PILL_WIDTH, PILL_HEIGHT));
            let panel: Retained<NSPanel> = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(true);
            panel.setIgnoresMouseEvents(true);
            panel.setHidesOnDeactivate(false);
            panel.setReleasedWhenClosed(false);
            // NSStatusWindowLevel-ish — well above normal windows.
            panel.setLevel(NSStatusWindowLevel);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::Stationary,
            );

            let view = OverlayView::new(mtm, state.clone());
            view.setFrame(frame);
            panel.setContentView(Some(&*view));

            position_panel(&panel, mtm);

            let _ = this.ivars().panel.set(panel);
            let _ = this.ivars().view.set(view);
            let _ = this.ivars().state.set(state);

            // 30Hz tick drives both animation phase and show/hide.
            let observer: &AnyObject = &this;
            let _timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / FRAME_HZ,
                observer,
                sel!(tick:),
                None,
                true,
            );
        }

        this
    }

    unsafe fn do_tick(&self) {
        let (mode, _) = {
            let mut s = self.ivars().state.get().unwrap().lock().unwrap();
            s.anim_phase = s.anim_phase.wrapping_add(1);
            (s.mode, s.anim_phase)
        };

        let panel = self.ivars().panel.get().unwrap();
        let view = self.ivars().view.get().unwrap();

        match mode {
            OverlayMode::Hidden => {
                if panel.isVisible() {
                    panel.orderOut(None);
                }
            }
            OverlayMode::Recording | OverlayMode::Transcribing => {
                if !panel.isVisible() {
                    if let Some(mtm) = MainThreadMarker::new() {
                        position_panel(panel, mtm);
                    }
                    panel.orderFrontRegardless();
                }
                let view_obj: &NSView = view;
                view_obj.setNeedsDisplay(true);
            }
        }
    }
}

unsafe fn position_panel(panel: &NSPanel, mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    let screen = if let Some(s) = NSScreen::mainScreen(mtm) {
        s
    } else if let Some(s) = screens.firstObject() {
        s
    } else {
        return;
    };
    let frame = screen.frame();
    let x = frame.origin.x + (frame.size.width - PILL_WIDTH) / 2.0;
    let y = frame.origin.y + BOTTOM_INSET;
    panel.setFrameOrigin(NSPoint::new(x, y));
}
