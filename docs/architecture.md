# hush — architecture & implementation notes

## Module map

```
src/
├── main.rs       Entry point. Sets up NSApp, installs the global fn-key
│                 monitor (NSEvent.addGlobalMonitor for FlagsChanged),
│                 spawns the audio worker thread.
├── ui.rs         AppController (NSObject subclass via objc2 define_class!),
│                 NSStatusItem with template icon, NSMenu, settings
│                 NSWindow with the two permission cards, NSTimer poll
│                 for TCC state changes.
├── overlay.rs    Floating pill panel (NSPanel) near the bottom-center of
│                 the screen. Three modes (Hidden / Recording /
│                 Transcribing) driven by shared OverlayState. Custom
│                 NSView subclass with drawRect: for the bars and dots.
├── audio.rs      cpal capture, RMS computation, whisper.cpp invocation.
│                 Runs entirely on a worker thread because the cpal
│                 stream is !Send.
├── keyboard.rs   Native Cmd+V via CGEventPost. Replaces the original
│                 osascript shellout. Critical for correct TCC attribution
│                 — see docs/macos-permissions.md.
├── perms.rs      Permission probe + request helpers. Wraps AVCaptureDevice
│                 (mic) and AXIsProcessTrusted/AXIsProcessTrustedWithOptions
│                 (accessibility). No Input Monitoring.
├── autostart.rs  Per-user LaunchAgent for "Open Hush at login" checkbox.
└── icon.rs       Menubar template icon, drawn at runtime via NSBezierPath.
```

## Threading model

```
                ┌───────────────────────────────────────────┐
                │              MAIN THREAD                  │
                │                                           │
                │   NSApplication run loop                  │
                │   ├── NSEvent global monitor (.flagsChanged)
                │   │     block sends Msg::Start/Stop       │
                │   │     ↓ (mpsc channel)                  │
                │   ├── NSTimer 1.5s — TCC poll             │
                │   ├── NSTimer 30Hz — overlay redraw       │
                │   └── all NSView/NSWindow updates         │
                └────────────────────────────────────────────┘
                                    │ mpsc
                                    ↓
                ┌───────────────────────────────────────────┐
                │           AUDIO WORKER THREAD             │
                │                                           │
                │   cpal stream (!Send — must be born here) │
                │   audio callback computes RMS,            │
                │     pushes into OverlayState              │
                │   On Msg::Stop: drain buffer, run whisper,│
                │     paste via CGEventPost                 │
                └────────────────────────────────────────────┘
```

The cpal stream's `!Send` constraint forces the audio capture to live on
whatever thread first creates it. We put it on the worker so the main
thread (NSApp) is never blocked by the ~6 second whisper inference.

The NSEvent global monitor's block is the only piece that lives on main
but talks to the worker — it sends `Msg::Start` / `Msg::Stop` on an mpsc
channel. The block fires on the main thread (so a `Cell<bool>` for
edge-detection is fine — no `Mutex` needed).

## Shared state — `OverlayState`

`Arc<Mutex<OverlayState>>` is held by:

- The audio worker (mutates: pushes RMS levels each callback, sets `mode`
  on start/stop/done).
- The overlay's 30Hz NSTimer (reads: snapshots mode + levels under the
  lock, releases lock, then draws).

Mutex contention is uncontested in practice: audio writes ~50Hz, UI reads
30Hz, hold time is a few µs. Atomics would be premature optimization.

## Global fn-key monitor — NSEvent.addGlobalMonitor under Accessibility

`src/main.rs::install_fn_monitor` registers an
`NSEvent.addGlobalMonitorForEventsMatchingMask` block matching
`NSEventMask::FlagsChanged`. The block runs on the main thread, edge-
detects on `event.modifierFlags().contains(.Function)`, and sends
`Msg::Start` / `Msg::Stop` over the worker channel.

Why not `CGEventTap`? Because `CGEventTap` requires the **separate Input
Monitoring TCC permission** AND has a brutal failure mode where
`CGEventTapCreate` re-fires the "Keystroke Receiving" prompt every time
it's called against an unauthorized cdhash — and ad-hoc dev builds get
a new cdhash on every `cargo build`. We spent hours trapped in a prompt
loop before switching APIs. See `docs/macos-permissions.md` for the full
write-up.

`NSEvent.addGlobalMonitor` is gated only on Accessibility (which we
already need for `CGEventPost`-based pasting), and it doesn't fire any
TCC prompt itself — it silently no-ops without permission and starts
delivering events immediately when Accessibility is granted, no
reinstall on grant needed. This is the same approach Wispr Flow uses.

## objc2 patterns we use

```rust
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "HushAppController"]
    #[ivars = ControllerIvars]
    pub struct AppController;

    impl AppController {
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) { ... }
    }

    unsafe impl NSObjectProtocol for AppController {}
);
```

- `MainThreadMarker` is required to alloc most AppKit types
  (`NSWindow::alloc(mtm)`, `NSImage::alloc(mtm)`).
- `NSObject` subclasses (our controllers) use `<Self as
  AllocAnyThread>::alloc()` since they don't need a main-thread marker.
- ivars get a `#[derive(Default)]` plain struct stored as
  `OnceCell<Retained<...>>` for the views/windows we want to retain.
- Selectors must be exposed via `#[unsafe(method(name:))]` and called from
  AppKit via `sel!(name:)`.

## NSStatusItem template icon

`NSImage::setTemplate(true)` tells AppKit the image is a single-channel
mask: opaque pixels get tinted with the menubar's foreground color (white
in dark menubars, black in light). Our template image is drawn at runtime
in `src/icon.rs` with `NSBezierPath` — fills go into the image via
`lockFocus`/`unlockFocus`.

`lockFocus` is deprecated in modern AppKit, but the alternative
(`imageWithSize:flipped:drawingHandler:`) requires a block, which adds
`block2` round-tripping. The deprecation is `#![allow(deprecated)]`'d in
`icon.rs`.

## NSPanel for the overlay

The "always on top, doesn't take focus, ignored by cmd-tab" overlay needs
a specific cocktail of NSPanel settings:

```rust
style: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel
panel.setOpaque(false);
panel.setBackgroundColor(Some(&NSColor::clearColor()));
panel.setIgnoresMouseEvents(true);
panel.setLevel(NSStatusWindowLevel);          // = 25, above normal
panel.setCollectionBehavior(
    CanJoinAllSpaces                 // visible across desktops
  | FullScreenAuxiliary              // visible over fullscreen apps
  | Stationary                       // doesn't slide with space switches
);
```

`NonactivatingPanel` is essential — without it, every show/hide cycle steals
focus from whatever app the user is typing into.

## NSBox and NSStackView gotchas

We learned these the hard way in the settings window:

- `NSBox::setContentView` does not auto-pin the content via autolayout.
  Without explicit constraints to the box, the inner view collapses to
  zero size and the cards overlap. Fix: pin inner stack to box's
  leading/trailing/top/bottom anchors, and `setContentViewMargins(NSSize::ZERO)`
  so the box doesn't double-pad.
- `NSStackView` vertical orientation has alignment defaulting to `centerX`,
  which centers all children at their intrinsic width. To make children
  fill the stack's width, set `alignment = .leading` *and* add an explicit
  `widthAnchor` constraint binding each child to the stack's width minus
  edge insets.

## Quit and ggml-metal teardown

`NSApplication::terminate` calls `libc::exit`, which runs C++ atexit
destructors, which trigger ggml-metal's destructor, which asserts that its
Metal residency set is empty (`GGML_ASSERT([rsets->data count] == 0)`).
Since the worker thread may still hold the WhisperContext, this fails and
the app crashes during quit.

Fix: in the `quit:` selector we call `libc::_exit(0)` directly, which
skips C++ atexit handlers entirely. Acceptable for an app whose entire
state is in-memory model weights and audio buffers — there's nothing to
flush.

## Icon pipeline

Two icons:

1. **Menubar template image** — `src/icon.rs`. Drawn at runtime, monochrome
   with alpha, marked as a template so AppKit auto-tints it.
2. **App icon (`.icns`)** — `scripts/draw-icon.swift` renders a
   1024×1024 PNG into a temporary file, `sips` resizes to all the iconset
   sizes Apple wants, `iconutil -c icns` packages them.

Headless gotcha: `NSImage::lockFocus` doesn't work in a `swift script`
process — there's no graphics environment to attach to. We use
`NSBitmapImageRep` + `NSGraphicsContext::current = ctx` instead.

## Build & package

| Script | Purpose |
|---|---|
| `scripts/build-app.sh` | cargo build → `target/release/Hush.app` (Info.plist, .icns, ad-hoc sign — no hardened runtime) |
| `scripts/install-dev.sh` | build-app.sh → kill running → tccutil reset → swap `/Applications/Hush.app` → open |
| `scripts/package.sh` | build-app.sh → `dist/Hush-X.Y.Z.dmg` (with /Applications symlink) + `dist/Hush-X.Y.Z.zip` (via `ditto` to preserve resource forks) |
